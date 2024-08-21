#![allow(clippy::suspicious_open_options)]

use crate::{
    common::{get_files_available, receive_packet, send_packet, FileSendRecvTree, FilesAvailable},
    packets::{ReceiverToSender, RoundezvousFromServer, RoundezvousToServer, SenderToReceiver},
    unsafe_client_config,
    utils::self_signed_cert,
    BUF_SIZE, CODE_LEN, KEEP_ALIVE_INTERVAL_SECS, ROUNDEZVOUS_SERVER_NAME, VERSION,
};
use async_compression::tokio::bufread::GzipDecoder;
use quinn::{default_runtime, Connection, Endpoint, EndpointConfig, ServerConfig};
use std::{
    io,
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time,
};
use thiserror::Error;
use tokio::io::AsyncWriteExt;

/// Generic receive function
pub async fn receive_file<R, W>(
    recv: &mut R,
    file: &mut W,
    skip: u64,
    size: u64,
    read_callback: &mut impl FnMut(u64),
) -> std::io::Result<()>
where
    R: tokio::io::AsyncReadExt + Unpin,
    W: tokio::io::AsyncWriteExt + tokio::io::AsyncSeekExt + Unpin,
{
    file.seek(tokio::io::SeekFrom::Start(skip)).await?;

    let mut buf = vec![0; BUF_SIZE];
    let mut written = skip;

    while written < size {
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

    Ok(())
}

/// Receive a directory
// #[async_recursion]
pub fn receive_directory<S>(
    send: &mut S,
    root_path: &std::path::Path,
    files: &[FileSendRecvTree],
    read_callback: &mut impl FnMut(u64),
) -> std::io::Result<()>
where
    S: tokio::io::AsyncReadExt + Unpin + Send,
{
    for file in files {
        match file {
            FileSendRecvTree::File { name, skip, size } => {
                let path = root_path.join(name);
                tokio::task::block_in_place(|| {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    rt.block_on(async {
                        let mut file = tokio::fs::OpenOptions::new()
                            .write(true)
                            .create(true)
                            .open(&path)
                            .await?;
                        receive_file(send, &mut file, *skip, *size, read_callback).await?;

                        file.sync_all().await?;
                        file.shutdown().await?;
                        Ok::<(), std::io::Error>(())
                    })
                })?;
            }
            FileSendRecvTree::Dir { name, files } => {
                let root_path = root_path.join(name);

                if !root_path.exists() {
                    std::fs::create_dir(&root_path)?;
                }
                receive_directory(send, &root_path, files, read_callback)?;
            }
        }
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum ReceiveError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("connect error: {0}")]
    Connect(#[from] quinn::ConnectError),
    #[error("connection error: {0}")]
    Connection(#[from] quinn::ConnectionError),
    #[error("write error: {0}")]
    Write(#[from] quinn::WriteError),
    #[error("read error {0}")]
    Read(#[from] quinn::ReadError),
    #[error("version mismatch, expected: {0}, got: {1}")]
    WrongVersion(String, String),
    #[error("unexpected data packet: {0:?}")]
    UnexpectedDataPacket(SenderToReceiver),
    #[error("unexpected roundezvous data packet: {0:?}")]
    UnexpectedRoundezvousDataPacket(RoundezvousFromServer),
    #[error("files rejected")]
    FilesRejected,
    #[error("unknown peer: {0}")]
    UnknownPeer(SocketAddr),
    #[error("invalid code")]
    InvalidCode,
}

pub struct Receiver {
    /// Receiver arguments
    args: ReceiverArgs,
    /// The connection to the sender
    conn: Connection,
    /// The local endpoint
    endpoint: Endpoint,
}

pub struct ReceiverArgs {
    /// Resume interrupted transfer
    pub resume: bool,
    /// Output path,
    pub output_path: PathBuf,
}

impl Receiver {
    pub async fn connect(
        socket: UdpSocket,
        sender: SocketAddr,
        args: ReceiverArgs,
    ) -> Result<Self, ReceiveError> {
        let rt = default_runtime()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no async runtime found"))?;

        let endpoint = Endpoint::new(EndpointConfig::default(), Some(server_config()), socket, rt)?;
        if let Some(conn) = endpoint.accept().await {
            let conn = conn.await?;
            tracing::debug!(
                "Server accepted connection from: {:?}",
                conn.remote_address()
            );

            if conn.remote_address() != sender {
                return Err(ReceiveError::UnknownPeer(conn.remote_address()));
            }

            return Ok(Self {
                args,
                conn,
                endpoint,
            });
        }

        Err(ReceiveError::Io(io::Error::new(
            io::ErrorKind::Other,
            "no connection found",
        )))
    }

    /// Wait for the sender to close the connection
    pub async fn close(&mut self) -> Result<(), ReceiveError> {
        self.conn.close(0u32.into(), &[0]);
        self.endpoint.close(0u32.into(), &[0]);
        Ok(())
    }

    /// Receive files
    /// # Arguments
    /// * `initial_progress_callback` - Callback with the initial progress of each file to send (name, current, total)
    /// * `accept_files_callback` - Callback to accept or reject the files
    /// * `read_callback` - Callback every time data is written to disk
    pub async fn receive_files(
        &mut self,
        mut initial_progress_callback: impl FnMut(&[(String, u64, u64)]),
        mut accept_files_callback: impl FnMut(&[FilesAvailable]) -> bool,
        read_callback: &mut impl FnMut(u64),
    ) -> Result<(), ReceiveError> {
        match receive_packet::<SenderToReceiver>(&self.conn).await? {
            SenderToReceiver::ConnRequest { version_num } => {
                if version_num != VERSION {
                    send_packet(
                        ReceiverToSender::WrongVersion {
                            expected: VERSION.to_string(),
                        },
                        &self.conn,
                    )
                    .await?;
                    return Err(ReceiveError::WrongVersion(VERSION.to_string(), version_num));
                }
                send_packet(ReceiverToSender::Ok, &self.conn).await?;
            }
            p => return Err(ReceiveError::UnexpectedDataPacket(p)),
        }

        let files_offered = match receive_packet::<SenderToReceiver>(&self.conn).await? {
            SenderToReceiver::FileInfo { files } => files,
            p => return Err(ReceiveError::UnexpectedDataPacket(p)),
        };

        if !accept_files_callback(&files_offered) {
            send_packet(ReceiverToSender::RejectFiles, &self.conn).await?;
            return Ok(());
        }

        let files_available = {
            let mut files = Vec::new();
            for file in &files_offered {
                let path = PathBuf::from_str(file.name()).unwrap();
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

        for file in to_receive.into_iter().flatten() {
            match file {
                FileSendRecvTree::File { name, skip, size } => {
                    let path = self.args.output_path.join(name);
                    let mut file = tokio::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .open(&path)
                        .await?;

                    receive_file(&mut recv, &mut file, skip, size, read_callback).await?;
                    file.sync_all().await?;
                    file.shutdown().await?;
                }
                FileSendRecvTree::Dir { name, files } => {
                    let path = self.args.output_path.join(name);

                    if !path.exists() {
                        std::fs::create_dir(&path)?;
                    }

                    receive_directory(&mut recv, &path, &files, read_callback)?;
                }
            }
        }

        self.close().await?;
        Ok(())
    }
}

pub async fn roundezvous_connect(
    socket: UdpSocket,
    external_addr: SocketAddr,
    server_addr: SocketAddr,
    code: [u8; CODE_LEN],
) -> Result<SocketAddr, ReceiveError> {
    let rt = default_runtime()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no async runtime found"))?;

    let mut endpoint = Endpoint::new(EndpointConfig::default(), None, socket, rt)?;

    endpoint.set_default_client_config(unsafe_client_config());

    let conn = endpoint
        .connect(server_addr, ROUNDEZVOUS_SERVER_NAME)?
        .await?;

    send_packet(
        RoundezvousToServer::Connect {
            version: VERSION.to_string(),
            socket_addr: external_addr,
            code,
        },
        &conn,
    )
    .await?;

    let sender_addr = match receive_packet::<RoundezvousFromServer>(&conn).await? {
        RoundezvousFromServer::SocketAddr { socket_addr } => socket_addr,
        RoundezvousFromServer::WrongVersion { expected } => {
            return Err(ReceiveError::WrongVersion(expected, VERSION.to_string()))
        }
        p => return Err(ReceiveError::UnexpectedRoundezvousDataPacket(p)),
    };

    endpoint.close(0u32.into(), b"exchange complete");

    Ok(sender_addr)
}

fn server_config() -> ServerConfig {
    let (cert, key) = self_signed_cert().expect("failed to generate self signed cert");

    let mut transport_config = quinn::TransportConfig::default();
    transport_config.keep_alive_interval(Some(time::Duration::from_secs(KEEP_ALIVE_INTERVAL_SECS)));

    ServerConfig::with_single_cert(vec![cert], key)
        .unwrap()
        .transport_config(Arc::new(transport_config))
        .to_owned()
}
