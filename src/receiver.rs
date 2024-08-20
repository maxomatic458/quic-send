use async_compression::tokio::bufread::GzipDecoder;
use async_recursion::async_recursion;
use core::time;
use std::{
    net::{SocketAddr, UdpSocket},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use indicatif::{HumanBytes, ProgressBar};
use quinn::{
    default_runtime, Connection, Endpoint, EndpointConfig, RecvStream, ServerConfig, VarInt,
};
use std::io;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

use crate::{
    common::{
        apply_files_to_skip_tree, get_available_files_tree, get_files_to_skip_tree, receive_packet,
        send_packet, FileRecvSendTree,
    },
    packets::{Receiver2Sender, Sender2Receiver},
    utils::{progress_bars, self_signed_cert},
    FILE_BUF_SIZE, KEEP_ALIVE_INTERVAL_SECS, VERSION,
};

#[derive(Error, Debug)]
pub enum ReceiveError {
    #[error("IO error: {0}")]
    IoError(#[from] io::Error),
    #[error("Connection error: {0}")]
    ConnectionError(#[from] quinn::ConnectionError),
    #[error("Write error: {0}")]
    WriteError(#[from] quinn::WriteError),
    #[error("Read error: {0}")]
    ReadError(#[from] quinn::ReadError),
    #[error("Version mismatch")]
    VersionMismatch,
    #[error("Unexpected packet: {0:?}")]
    UnexpectedPacket(Sender2Receiver),
    #[error("Could not verify integrity of file: {0}")]
    CouldNotVerifyIntegrity(String),
    #[error("Unknown peer")]
    UnknownPeer(SocketAddr),
}

pub struct Receiver {
    /// Arguments
    pub args: ReceiverArgs,
    /// The connection
    pub conn: Connection,
    /// Server endpoint
    pub server: Endpoint,
}

pub struct ReceiverArgs {
    pub resume: bool,
}

impl Receiver {
    pub async fn connect(
        socket: UdpSocket,
        sender: SocketAddr,
        args: ReceiverArgs,
    ) -> Result<Self, ReceiveError> {
        let rt = default_runtime()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no async runtime found"))?;

        let server = Endpoint::new(EndpointConfig::default(), Some(server_config()), socket, rt)?;
        if let Some(conn) = server.accept().await {
            let conn = conn.await?;
            tracing::debug!(
                "Server accepted connection from: {:?}",
                conn.remote_address()
            );

            if conn.remote_address() != sender {
                return Err(ReceiveError::UnknownPeer(conn.remote_address()));
            }

            return Ok(Self { args, conn, server });
        }

        Err(ReceiveError::IoError(io::Error::new(
            io::ErrorKind::Other,
            "no connection found",
        )))
    }

    fn accept_files(&self, files_offered: &[FileRecvSendTree]) -> bool {
        println!("The following files will be received:");

        let longest_name = files_offered
            .iter()
            .map(|f| f.name().len())
            .max()
            .unwrap_or(0);

        let total_size = files_offered.iter().map(|f| f.size()).sum::<u64>();

        for file in files_offered {
            let size = file.size();
            let size_human_bytes = HumanBytes(size).to_string();
            let name = file.name();

            println!(
                "- {:<width$} {:>10},",
                name,
                size_human_bytes,
                width = longest_name
            );
        }

        println!("\nTotal size: {}", HumanBytes(total_size));

        dialoguer::Confirm::new()
            .with_prompt("Do you want to receive these files?")
            .interact()
            .unwrap()
    }

    pub async fn close(&mut self) -> Result<(), ReceiveError> {
        self.conn.close(VarInt::from_u32(0), &[0]);
        Ok(())
    }

    /// Download a single file
    async fn download_file(
        &self,
        recv: &mut GzipDecoder<BufReader<RecvStream>>,
        path: &Path,
        skip: u64,
        size: u64,
        bar: &ProgressBar,
        total_bar: Option<&ProgressBar>,
    ) -> Result<(), ReceiveError> {
        tracing::debug!("receiving file: {:?}", path);

        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .append(skip != 0)
            .open(path)
            .await?;

        let mut bytes_written = skip;
        let mut buf = vec![0; FILE_BUF_SIZE];

        while bytes_written < size {
            let to_write = std::cmp::min(FILE_BUF_SIZE as u64, size - bytes_written);
            let n = recv.read_exact(&mut buf[..to_write as usize]).await?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected EOF").into());
            }

            file.write_all(&buf[..n]).await?;
            bytes_written += n as u64;

            bar.inc(n as u64);
            if let Some(total_bar) = total_bar {
                total_bar.inc(n as u64);
            }
        }

        tracing::debug!("finished receiving file: {:?}", path);

        file.sync_all().await?;
        file.shutdown().await?;

        Ok(())
    }

    /// Download a single directory
    #[async_recursion]
    async fn download_directory(
        &self,
        recv: &mut GzipDecoder<BufReader<RecvStream>>,
        files: &[FileRecvSendTree],
        path: &Path,
        bar: &ProgressBar,
        total_bar: Option<&ProgressBar>,
    ) -> Result<(), ReceiveError> {
        tracing::debug!("receiving directory: {:?}", path);

        for file in files {
            let path = path.join(&file.name());
            match file {
                FileRecvSendTree::File { size, skip, .. } => {
                    self.download_file(recv, &path, *skip, *size, bar, total_bar)
                        .await?;
                }
                FileRecvSendTree::Dir { files, .. } => {
                    if !path.exists() {
                        tokio::fs::create_dir(&path).await?;
                    }
                    self.download_directory(recv, files, &path, bar, total_bar)
                        .await?;
                }
            }
        }

        tracing::debug!("finished receiving directory: {:?}", path);

        Ok(())
    }

    /// Download all files and directories
    async fn receive_files(
        &self,
        files: &[FileRecvSendTree],
        bars: (Vec<ProgressBar>, Option<ProgressBar>),
    ) -> Result<(), ReceiveError> {
        tracing::debug!("begin file receiving");
        let (bars, total_bar) = bars;
        let mut recv = self.conn.accept_uni().await?;
        recv.read_u8().await?; // Opening byte

        let mut recv = GzipDecoder::new(BufReader::with_capacity(FILE_BUF_SIZE, recv));

        for (file, bar) in files.iter().zip(bars.iter()) {
            match file {
                FileRecvSendTree::File { size, skip, .. } => {
                    self.download_file(
                        &mut recv,
                        Path::new(&file.name()),
                        *skip,
                        *size,
                        bar,
                        total_bar.as_ref(),
                    )
                    .await?;
                }
                FileRecvSendTree::Dir { files, .. } => {
                    // tokio::fs::create_dir(&Path::new(&file.name())).await?;
                    if !Path::new(&file.name()).exists() {
                        tokio::fs::create_dir(&Path::new(&file.name())).await?;
                    }
                    self.download_directory(
                        &mut recv,
                        files,
                        Path::new(&file.name()),
                        bar,
                        total_bar.as_ref(),
                    )
                    .await?;
                }
            }
        }

        tracing::debug!("finished file receiving");

        Ok(())
    }
}

pub async fn receive_files(
    socket: UdpSocket,
    _sender: SocketAddr,
    args: ReceiverArgs,
) -> Result<(), ReceiveError> {
    let mut receiver = Receiver::connect(socket, _sender, args).await?;

    match receive_packet::<Sender2Receiver>(&receiver.conn).await? {
        Sender2Receiver::ConnRequest { version_num } => {
            if version_num != VERSION {
                send_packet(
                    Receiver2Sender::WrongVersion {
                        expected: VERSION.to_string(),
                    },
                    &receiver.conn,
                )
                .await?;
            }
        }
        p => return Err(ReceiveError::UnexpectedPacket(p)),
    };

    send_packet(Receiver2Sender::Ok, &receiver.conn).await?;

    let files_offered = match receive_packet::<Sender2Receiver>(&receiver.conn).await? {
        Sender2Receiver::FileInfo { files } => files,
        p => return Err(ReceiveError::UnexpectedPacket(p)),
    };

    if !receiver.accept_files(&files_offered) {
        send_packet(Receiver2Sender::RejectFiles, &receiver.conn).await?;
        receiver.close().await?;
    }

    let available_files = {
        let mut files = vec![];
        for file in &files_offered {
            let path = PathBuf::from_str(&file.name()).unwrap();
            files.push(get_available_files_tree(&path).ok());
        }

        files
    };

    let to_skip = if receiver.args.resume {
        let mut to_skip = vec![];
        for (available, offered) in available_files.iter().zip(files_offered.iter()) {
            match available {
                Some(available) => to_skip.push(get_files_to_skip_tree(available, offered)),
                None => to_skip.push(None),
            }
        }

        to_skip
    } else {
        // Do not skip any files
        vec![None; files_offered.len()]
    };

    let to_receive = {
        let mut to_receive = vec![];
        for (skip, offered) in to_skip.iter().zip(files_offered.iter()) {
            match skip {
                Some(skip) => {
                    if let Some(file) = apply_files_to_skip_tree(offered, skip) {
                        to_receive.push(file);
                    }
                }
                None => to_receive.push(offered.clone()),
            }
        }

        to_receive
    };

    let bars = progress_bars(
        &files_offered,
        &to_skip
            .iter()
            .map(|x| x.as_ref().map(|x| x.skip()).unwrap_or(0))
            .collect::<Vec<_>>(),
    );

    send_packet(
        Receiver2Sender::AcceptFilesSkip { files: to_skip },
        &receiver.conn,
    )
    .await?;

    receiver.receive_files(&to_receive, bars).await?;

    receiver.close().await?;
    Ok(())
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
