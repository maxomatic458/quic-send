#![allow(clippy::suspicious_open_options)]

use crate::{
    common::{get_files_available, receive_packet, send_packet, FileSendRecvTree, PacketRecvError},
    packets::{ReceiverToSender, SenderToReceiver},
    BUF_SIZE, QS_PROTO_VERSION,
};
use async_compression::tokio::write::GzipEncoder;
use std::path::PathBuf;
use thiserror::Error;
use tokio::io::AsyncWriteExt;

/// Generic send function
///
/// # Returns
/// * `Ok(true)` if the transfer should continue
/// * `Ok(false)` if the transfer should stop
pub async fn send_file<S, R>(
    send: &mut S,
    file: &mut R,
    skip: u64,
    size: u64,
    write_callback: &mut impl FnMut(u64),
    should_continue: &mut impl FnMut() -> bool,
) -> std::io::Result<bool>
where
    S: tokio::io::AsyncWriteExt + Unpin,
    R: tokio::io::AsyncReadExt + tokio::io::AsyncSeekExt + Unpin,
{
    file.seek(tokio::io::SeekFrom::Start(skip)).await?;

    let mut buf = vec![0; BUF_SIZE];
    let mut read = skip;

    while read < size {
        if !should_continue() {
            return Ok(false);
        }

        let to_read = std::cmp::min(BUF_SIZE as u64, size - read);
        let n = file.read_exact(&mut buf[..to_read as usize]).await?;

        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "unexpected eof",
            ));
        }

        send.write_all(&buf[..n]).await?;
        read += n as u64;

        write_callback(n as u64);
    }

    Ok(true)
}

/// # Returns
/// * `Ok(true)` if the transfer should continue
/// * `Ok(false)` if the transfer should stop
pub fn send_directory<S>(
    send: &mut S,
    root_path: &std::path::Path,
    files: &[FileSendRecvTree],
    write_callback: &mut impl FnMut(u64),
    should_continue: &mut impl FnMut() -> bool,
) -> std::io::Result<bool>
where
    S: tokio::io::AsyncWriteExt + Unpin + Send,
{
    for file in files {
        match file {
            FileSendRecvTree::File { name, skip, size } => {
                let path = root_path.join(name);

                let continues = tokio::task::block_in_place(|| {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    rt.block_on(async {
                        let mut file = tokio::fs::OpenOptions::new().read(true).open(&path).await?;

                        if !send_file(
                            send,
                            &mut file,
                            *skip,
                            *size,
                            write_callback,
                            should_continue,
                        )
                        .await?
                        {
                            return Ok::<bool, std::io::Error>(false);
                        }

                        file.shutdown().await?;
                        Ok::<bool, std::io::Error>(true)
                    })
                })?;

                if !continues {
                    return Ok(false);
                }
            }
            FileSendRecvTree::Dir { name, files } => {
                let root_path = root_path.join(name);
                if !send_directory(send, &root_path, files, write_callback, should_continue)? {
                    return Ok(false);
                };
            }
        }
    }

    Ok(true)
}

#[derive(Debug, Error)]
pub enum SendError {
    #[error("files do not exist: {0}")]
    FileDoesNotExists(PathBuf),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    // #[error("connect error: {0}")]
    // Connect(#[from] iroh::endpoint::ConnectError),
    #[error("connection error: {0}")]
    Connection(#[from] iroh::endpoint::ConnectionError),
    #[error("read error: {0}")]
    Read(#[from] quinn::ReadError),
    #[error("wrong version, the receiver expected: {0}, but got: {1}")]
    WrongVersion(String, String),
    #[error(
        "wrong roundezvous protocol version, the roundezvous server expected {0}, but got: {1}"
    )]
    WrongRoundezvousVersion(u32, u32),
    #[error("unexpected data packet: {0:?}")]
    UnexpectedDataPacket(ReceiverToSender),
    #[error("files rejected")]
    FilesRejected,
    #[error("receive packet error: {0}")]
    ReceivePacket(#[from] PacketRecvError),
    #[error("failed to fetch node addr: {0}")]
    NodeAddr(String),
}

/// A client that can send files
pub struct Sender {
    /// Sender arguments
    args: SenderArgs,
    /// The connection to the receiver
    conn: iroh::endpoint::Connection,
    /// The local endpoint
    endpoint: iroh::Endpoint,
}

/// Arguments for the sender
pub struct SenderArgs {
    /// Files/Directories to send
    pub files: Vec<PathBuf>,
}

impl Sender {
    pub async fn connect(
        this_endpoint: iroh::Endpoint,
        args: SenderArgs,
    ) -> Result<Self, SendError> {
        if let Some(incoming) = this_endpoint.accept().await {
            let connecting = incoming.accept()?;
            let conn = connecting.await?;

            tracing::info!("receiver connected to sender");

            return Ok(Self {
                args,
                conn,
                endpoint: this_endpoint,
            });
        }

        unreachable!();
    }

    /// Close the connection
    pub async fn close(&mut self) {
        self.conn.close(0u32.into(), &[0]);
        self.endpoint.close().await;
    }

    /// Wait for the other peer to close the connection
    pub async fn wait_for_close(&mut self) {
        self.conn.closed().await;
    }

    /// Get the type of the connection
    pub async fn connection_type(&self) -> Option<iroh::endpoint::ConnectionType> {
        let node_id = self.conn.remote_node_id().ok()?;
        self.endpoint.conn_type(node_id).ok()?.get().ok()
    }

    /// Send files
    /// # Arguments
    /// * `wait_for_other_peer_to_accept_files_callback` - Callback to wait for the other peer to accept the files
    /// * `files_decision_callback` - Callback with the decision of the other peer to accept the files
    /// * `initial_progress_callback` - Callback with the initial progress of each file to send (name, current, total)
    /// * `write_callback` - Callback every time data is written to the connection
    /// * `should_continue` - Callback to check if the transfer should continue
    ///
    /// # Returns
    /// * `Ok(true)` if the transfer was finished successfully
    /// * `Ok(false)` if the transfer was stopped
    pub async fn send_files(
        &mut self,
        mut wait_for_other_peer_to_accept_files_callback: impl FnMut(),
        mut files_decision_callback: impl FnMut(bool),
        mut initial_progress_callback: impl FnMut(&[(String, u64, u64)]),
        write_callback: &mut impl FnMut(u64),
        should_continue: &mut impl FnMut() -> bool,
    ) -> Result<bool, SendError> {
        send_packet(
            SenderToReceiver::ConnRequest {
                version_num: QS_PROTO_VERSION.to_string(),
            },
            &self.conn,
        )
        .await?;

        match receive_packet::<ReceiverToSender>(&self.conn).await? {
            ReceiverToSender::Ok => (),
            ReceiverToSender::WrongVersion { expected } => {
                return Err(SendError::WrongVersion(expected, QS_PROTO_VERSION.to_string()));
            }
            p => return Err(SendError::UnexpectedDataPacket(p)),
        }

        let files_available = {
            let mut files = Vec::new();
            for file in &self.args.files {
                if !file.exists() {
                    return Err(SendError::FileDoesNotExists(file.clone()));
                }
                files.push(get_files_available(file)?);
            }
            files
        };

        send_packet(
            SenderToReceiver::FileInfo {
                files: files_available.clone(),
            },
            &self.conn,
        )
        .await?;

        wait_for_other_peer_to_accept_files_callback();

        let to_skip = match receive_packet::<ReceiverToSender>(&self.conn).await? {
            ReceiverToSender::AcceptFilesSkip { files } => {
                files_decision_callback(true);
                files
            }
            ReceiverToSender::RejectFiles => {
                files_decision_callback(false);
                self.close().await;
                return Err(SendError::FilesRejected);
            }
            p => return Err(SendError::UnexpectedDataPacket(p)),
        };

        let to_send: Vec<Option<FileSendRecvTree>> = files_available
            .iter()
            .zip(&to_skip)
            .map(|(file, skip)| {
                if let Some(skip) = skip {
                    file.remove_skipped(skip)
                } else {
                    Some(file.to_send_recv_tree())
                }
            })
            .collect();

        let mut progress: Vec<(String, u64, u64)> = Vec::with_capacity(files_available.len());
        for (file, skip) in files_available.iter().zip(to_skip) {
            progress.push((
                file.name().to_string(),
                skip.as_ref().map(|s| s.skip()).unwrap_or(0),
                file.size(),
            ));
        }

        initial_progress_callback(&progress);

        let send = self.conn.open_uni().await?;
        let mut send = GzipEncoder::new(send);

        let mut interrupted = false;

        for (path, file) in self.args.files.iter().zip(to_send) {
            if let Some(file) = file {
                match file {
                    FileSendRecvTree::File { skip, size, .. } => {
                        let mut file = tokio::fs::File::open(&path).await?;
                        if !send_file(
                            &mut send,
                            &mut file,
                            skip,
                            size,
                            write_callback,
                            should_continue,
                        )
                        .await?
                        {
                            interrupted = true;
                            break;
                        }
                    }
                    FileSendRecvTree::Dir { files, .. } => {
                        if !send_directory(
                            &mut send,
                            path,
                            &files,
                            write_callback,
                            should_continue,
                        )? {
                            interrupted = true;
                            break;
                        }
                    }
                }
            }
        }

        send.shutdown().await?;

        if !interrupted {
            self.wait_for_close().await;
        } else {
            tracing::info!("the transfer was interrupted");
        }

        Ok(!interrupted)
    }
}
