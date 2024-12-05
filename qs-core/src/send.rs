#![allow(clippy::suspicious_open_options)]

use std::{
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
};

use crate::{
    common::{get_files_available, receive_packet, send_packet, FileSendRecvTree, PacketRecvError},
    packets::{ReceiverToSender, RoundezvousFromServer, RoundezvousToServer, SenderToReceiver},
    unsafe_client_config, BUF_SIZE, CODE_LEN, QS_VERSION, ROUNDEZVOUS_PROTO_VERSION,
    ROUNDEZVOUS_SERVER_NAME, SEND_SERVER_NAME,
};
use async_compression::tokio::write::GzipEncoder;
use quinn::{default_runtime, Connection, Endpoint, EndpointConfig};
use thiserror::Error;
use tokio::io::AsyncWriteExt;

/// Generic send function
pub async fn send_file<S, R>(
    send: &mut S,
    file: &mut R,
    skip: u64,
    size: u64,
    write_callback: &mut impl FnMut(u64),
) -> std::io::Result<()>
where
    S: tokio::io::AsyncWriteExt + Unpin,
    R: tokio::io::AsyncReadExt + tokio::io::AsyncSeekExt + Unpin,
{
    file.seek(tokio::io::SeekFrom::Start(skip)).await?;

    let mut buf = vec![0; BUF_SIZE];
    let mut read = skip;

    while read < size {
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

    Ok(())
}

pub fn send_directory<S>(
    send: &mut S,
    root_path: &std::path::Path,
    files: &[FileSendRecvTree],
    write_callback: &mut impl FnMut(u64),
) -> std::io::Result<()>
where
    S: tokio::io::AsyncWriteExt + Unpin + Send,
{
    for file in files {
        match file {
            FileSendRecvTree::File { name, skip, size } => {
                let path = root_path.join(name);
                tokio::task::block_in_place(|| {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    rt.block_on(async {
                        let mut file = tokio::fs::OpenOptions::new().read(true).open(&path).await?;

                        send_file(send, &mut file, *skip, *size, write_callback).await?;
                        file.shutdown().await?;
                        Ok::<(), std::io::Error>(())
                    })
                })?;
            }
            FileSendRecvTree::Dir { name, files } => {
                let root_path = root_path.join(name);
                send_directory(send, &root_path, files, write_callback)?;
            }
        }
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum SendError {
    #[error("files do not exist: {0}")]
    FileDoesNotExists(PathBuf),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("connect error: {0}")]
    Connect(#[from] quinn::ConnectError),
    #[error("connection error: {0}")]
    Connection(#[from] quinn::ConnectionError),
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
    #[error("unexpected roundezvous data packet: {0:?}")]
    UnexpectedRoundezvousDataPacket(RoundezvousFromServer),
    #[error("files rejected")]
    FilesRejected,
    #[error("receive packet error: {0}")]
    ReceivePacket(#[from] PacketRecvError),
}

/// A client that can send files
pub struct Sender {
    /// Sender arguments
    args: SenderArgs,
    /// The connection to the receiver
    conn: Connection,
    /// The local endpoint
    endpoint: Endpoint,
}

/// Arguments for the sender
pub struct SenderArgs {
    /// Files/Directories to send
    pub files: Vec<PathBuf>,
}

impl Sender {
    pub async fn connect(
        socket: UdpSocket,
        receiver: SocketAddr,
        args: SenderArgs,
    ) -> Result<Self, SendError> {
        let rt = default_runtime().unwrap();

        let mut endpoint = Endpoint::new(EndpointConfig::default(), None, socket, rt)?;

        endpoint.set_default_client_config(unsafe_client_config());
        let conn = endpoint.connect(receiver, SEND_SERVER_NAME)?.await?;
        tracing::debug!("Client connected to server: {:?}", conn.remote_address());

        Ok(Self {
            args,
            conn,
            endpoint,
        })
    }

    /// Close the connection
    pub async fn close(&mut self) {
        self.conn.close(0u32.into(), &[0]);
        self.endpoint.close(0u32.into(), &[0]);
    }

    /// Wait for the other peer to close the connection
    pub async fn wait_for_close(&mut self) {
        self.conn.closed().await;
        self.endpoint.wait_idle().await;
    }

    /// Send files
    /// # Arguments
    /// * `wait_for_other_peer_to_accept_files_callback` - Callback to wait for the other peer to accept the files
    /// * `files_decision_callback` - Callback with the decision of the other peer to accept the files
    /// * `initial_progress_callback` - Callback with the initial progress of each file to send (name, current, total)
    /// * `write_callback` - Callback every time data is written to the connection
    pub async fn send_files(
        &mut self,
        mut wait_for_other_peer_to_accept_files_callback: impl FnMut(),
        mut files_decision_callback: impl FnMut(bool),
        mut initial_progress_callback: impl FnMut(&[(String, u64, u64)]),
        write_callback: &mut impl FnMut(u64),
    ) -> Result<(), SendError> {
        send_packet(
            SenderToReceiver::ConnRequest {
                version_num: QS_VERSION.to_string(),
            },
            &self.conn,
        )
        .await?;

        match receive_packet::<ReceiverToSender>(&self.conn).await? {
            ReceiverToSender::Ok => (),
            ReceiverToSender::WrongVersion { expected } => {
                return Err(SendError::WrongVersion(expected, QS_VERSION.to_string()));
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

        for (path, file) in self.args.files.iter().zip(to_send) {
            if let Some(file) = file {
                match file {
                    FileSendRecvTree::File { skip, size, .. } => {
                        let mut file = tokio::fs::File::open(&path).await?;
                        send_file(&mut send, &mut file, skip, size, write_callback).await?;
                    }
                    FileSendRecvTree::Dir { files, .. } => {
                        send_directory(&mut send, path, &files, write_callback)?;
                    }
                }
            }
        }

        send.shutdown().await?;
        self.wait_for_close().await;
        Ok(())
    }
}

pub async fn roundezvous_announce(
    socket: UdpSocket,
    external_addr: SocketAddr,
    server_addr: SocketAddr,
    mut code_callback: impl FnMut([u8; CODE_LEN]),
) -> Result<SocketAddr, SendError> {
    let rt = default_runtime().unwrap();

    let mut endpoint = Endpoint::new(EndpointConfig::default(), None, socket, rt)?;

    endpoint.set_default_client_config(unsafe_client_config());

    let conn = endpoint
        .connect(server_addr, ROUNDEZVOUS_SERVER_NAME)?
        .await?;

    send_packet(
        RoundezvousToServer::Announce {
            version: ROUNDEZVOUS_PROTO_VERSION,
            socket_addr: external_addr,
        },
        &conn,
    )
    .await?;

    let code = match receive_packet::<RoundezvousFromServer>(&conn).await? {
        RoundezvousFromServer::Code { code } => code,
        RoundezvousFromServer::WrongVersion { expected } => {
            return Err(SendError::WrongRoundezvousVersion(
                expected,
                ROUNDEZVOUS_PROTO_VERSION,
            ))
        }
        p => return Err(SendError::UnexpectedRoundezvousDataPacket(p)),
    };

    code_callback(code);

    let receiver_addr = match receive_packet::<RoundezvousFromServer>(&conn).await? {
        RoundezvousFromServer::SocketAddr { socket_addr } => socket_addr,
        p => return Err(SendError::UnexpectedRoundezvousDataPacket(p)),
    };

    conn.closed().await;
    endpoint.wait_idle().await;
    tracing::debug!("exchange complete");

    Ok(receiver_addr)
}
