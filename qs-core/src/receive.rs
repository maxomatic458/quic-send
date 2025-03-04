#![allow(clippy::suspicious_open_options)]

use crate::{
    common::{
        get_files_available, receive_packet, send_packet, FileSendRecvTree, FilesAvailable,
        PacketRecvError,
    },
    packets::{ReceiverToSender, SenderToReceiver},
    BUF_SIZE, QS_ALPN, QS_VERSION,
};
use async_compression::tokio::bufread::GzipDecoder;
use std::{io, path::PathBuf};
use thiserror::Error;
use tokio::io::AsyncWriteExt;

/// Generic receive function
///
/// # Returns
/// * `Ok(true)` if the transfer should continue
/// * `Ok(false)` if the transfer should stop
pub async fn receive_file<R, W>(
    recv: &mut R,
    file: &mut W,
    skip: u64,
    size: u64,
    read_callback: &mut impl FnMut(u64),
    should_continue: &mut impl FnMut() -> bool,
) -> std::io::Result<bool>
where
    R: tokio::io::AsyncReadExt + Unpin,
    W: tokio::io::AsyncWriteExt + tokio::io::AsyncSeekExt + Unpin,
{
    file.seek(tokio::io::SeekFrom::Start(skip)).await?;

    let mut buf = vec![0; BUF_SIZE];
    let mut written = skip;

    while written < size {
        if !should_continue() {
            return Ok(false);
        }

        let to_write = std::cmp::min(BUF_SIZE as u64, size - written);
        let n = recv.read_exact(&mut buf[..to_write as usize]).await?;

        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected eof",
            ));
        }

        file.write_all(&buf[..n]).await?;
        written += n as u64;

        read_callback(n as u64);
    }

    Ok(true)
}

/// # Returns
/// * `Ok(true)` if the transfer should continue
/// * `Ok(false)` if the transfer should stop
pub fn receive_directory<S>(
    send: &mut S,
    root_path: &std::path::Path,
    files: &[FileSendRecvTree],
    read_callback: &mut impl FnMut(u64),
    should_continue: &mut impl FnMut() -> bool,
) -> std::io::Result<bool>
where
    S: tokio::io::AsyncReadExt + Unpin + Send,
{
    for file in files {
        match file {
            FileSendRecvTree::File { name, skip, size } => {
                let path = root_path.join(name);

                let continues = tokio::task::block_in_place(|| {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    rt.block_on(async {
                        let mut file = tokio::fs::OpenOptions::new()
                            .write(true)
                            .create(true)
                            .open(&path)
                            .await?;
                        let continues = receive_file(
                            send,
                            &mut file,
                            *skip,
                            *size,
                            read_callback,
                            should_continue,
                        )
                        .await?;

                        file.sync_all().await?;
                        file.shutdown().await?;
                        Ok::<bool, std::io::Error>(continues)
                    })
                })?;

                if !continues {
                    return Ok(false);
                }
            }
            FileSendRecvTree::Dir { name, files } => {
                let root_path = root_path.join(name);

                if !root_path.exists() {
                    std::fs::create_dir(&root_path)?;
                }

                if !receive_directory(send, &root_path, files, read_callback, should_continue)? {
                    return Ok(false);
                }
            }
        }
    }

    Ok(true)
}

#[derive(Debug, Error)]
pub enum ReceiveError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("connect error: {0}")]
    Connect(String),
    #[error("connection error: {0}")]
    Connection(#[from] iroh::endpoint::ConnectionError),
    #[error("write error: {0}")]
    Write(#[from] quinn::WriteError),
    #[error("read error {0}")]
    Read(#[from] quinn::ReadError),
    #[error("version mismatch, expected: {0}, got: {1}")]
    WrongVersion(String, String),
    #[error(
        "wrong roundezvous protocol version, the roundezvous server expected {0}, but got: {1}"
    )]
    WrongRoundezvousVersion(u32, u32),
    #[error("unexpected data packet: {0:?}")]
    UnexpectedDataPacket(SenderToReceiver),
    #[error("files rejected")]
    FilesRejected,
    #[error("invalid code")]
    InvalidCode,
    #[error("receive packet error: {0}")]
    ReceivePacket(#[from] PacketRecvError),
}

/// A receiver that can receive files
pub struct Receiver {
    /// Receiver arguments
    args: ReceiverArgs,
    /// The connection to the sender
    conn: iroh::endpoint::Connection,
    /// The local endpoint
    endpoint: iroh::Endpoint,
}

/// Arguments for the receiver
pub struct ReceiverArgs {
    /// Resume interrupted transfer
    pub resume: bool,
}

impl Receiver {
    pub async fn connect(
        this_endpoint: iroh::Endpoint,
        node_addr: iroh::NodeAddr,
        args: ReceiverArgs,
    ) -> Result<Self, ReceiveError> {
        let conn = this_endpoint
            .connect(node_addr, QS_ALPN)
            .await
            .map_err(|e| ReceiveError::Connect(e.to_string()))?;

        tracing::info!("receiver connected to sender");

        Ok(Self {
            args,
            conn,
            endpoint: this_endpoint,
        })
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

    /// Receive files
    /// # Arguments
    /// * `initial_progress_callback` - Callback with the initial progress of each file to send (name, current, total)
    /// * `accept_files_callback` - Callback to accept or reject the files (Some(path) to accept, None to reject)
    /// * `read_callback` - Callback every time data is written to disk
    /// * `should_continue` - Callback to check if the transfer should continue
    ///
    /// # Returns
    /// * `Ok(true)` if the transfer was finished successfully
    /// * `Ok(false)` if the transfer was stopped
    pub async fn receive_files(
        &mut self,
        mut initial_progress_callback: impl FnMut(&[(String, u64, u64)]),
        mut accept_files_callback: impl FnMut(&[FilesAvailable]) -> Option<PathBuf>,
        read_callback: &mut impl FnMut(u64),
        should_continue: &mut impl FnMut() -> bool,
    ) -> Result<bool, ReceiveError> {
        match receive_packet::<SenderToReceiver>(&self.conn).await? {
            SenderToReceiver::ConnRequest { version_num } => {
                if version_num != QS_VERSION {
                    send_packet(
                        ReceiverToSender::WrongVersion {
                            expected: QS_VERSION.to_string(),
                        },
                        &self.conn,
                    )
                    .await?;
                    return Err(ReceiveError::WrongVersion(
                        QS_VERSION.to_string(),
                        version_num,
                    ));
                }
                send_packet(ReceiverToSender::Ok, &self.conn).await?;
            }
            p => return Err(ReceiveError::UnexpectedDataPacket(p)),
        }

        let files_offered = match receive_packet::<SenderToReceiver>(&self.conn).await? {
            SenderToReceiver::FileInfo { files } => files,
            p => return Err(ReceiveError::UnexpectedDataPacket(p)),
        };

        let output_path = match accept_files_callback(&files_offered) {
            Some(path) => path,
            None => {
                send_packet(ReceiverToSender::RejectFiles, &self.conn).await?;
                // Wait for the sender to acknowledge the rejection
                self.wait_for_close().await;
                return Err(ReceiveError::FilesRejected);
            }
        };

        let files_available = {
            let mut files = Vec::new();
            for file in &files_offered {
                let path = output_path.join(file.name());
                files.push(get_files_available(&path).ok());
            }

            files
        };

        let files_to_skip = if self.args.resume {
            let mut to_skip = Vec::new();
            for (available, offered) in files_available.iter().zip(&files_offered) {
                match available {
                    Some(available) => to_skip.push(offered.get_skippable(available)),
                    None => to_skip.push(None),
                }
            }

            to_skip
        } else {
            // Don't skip any files
            vec![None; files_offered.len()]
        };

        let to_receive: Vec<Option<FileSendRecvTree>> = files_offered
            .iter()
            .zip(&files_to_skip)
            .map(|(offered, skip)| {
                if let Some(skip) = skip {
                    offered.remove_skipped(skip)
                } else {
                    Some(offered.to_send_recv_tree())
                }
            })
            .collect();

        // progress callback
        let mut progress: Vec<(String, u64, u64)> = Vec::with_capacity(to_receive.len());
        for (offered, skip) in files_offered.iter().zip(&files_to_skip) {
            progress.push((
                offered.name().to_string(),
                skip.as_ref().map(|s| s.skip()).unwrap_or(0),
                offered.size(),
            ));
        }

        initial_progress_callback(&progress);

        send_packet(
            ReceiverToSender::AcceptFilesSkip {
                files: files_to_skip,
            },
            &self.conn,
        )
        .await?;

        let recv = self.conn.accept_uni().await?;
        let mut recv = GzipDecoder::new(tokio::io::BufReader::with_capacity(BUF_SIZE, recv));

        let mut interrupted = false;

        for file in to_receive.into_iter().flatten() {
            match file {
                FileSendRecvTree::File { name, skip, size } => {
                    let path = output_path.join(name);
                    let mut file = tokio::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .open(&path)
                        .await?;

                    interrupted = !receive_file(
                        &mut recv,
                        &mut file,
                        skip,
                        size,
                        read_callback,
                        should_continue,
                    )
                    .await?;
                    file.sync_all().await?;
                    file.shutdown().await?;

                    if interrupted {
                        break;
                    }
                }
                FileSendRecvTree::Dir { name, files } => {
                    let path = output_path.join(name);

                    if !path.exists() {
                        std::fs::create_dir(&path)?;
                    }

                    if !receive_directory(&mut recv, &path, &files, read_callback, should_continue)?
                    {
                        interrupted = true;
                        break;
                    }
                }
            }
        }

        self.close().await;

        if interrupted {
            tracing::info!("transfer interrupted");
        }

        Ok(!interrupted)
    }
}
