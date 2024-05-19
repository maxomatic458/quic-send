use async_compression::tokio::bufread::GzipDecoder;
use color_eyre::owo_colors::OwoColorize;
use core::time;
use std::{
    net::{SocketAddr, UdpSocket},
    path::Path,
    sync::Arc,
};

use indicatif::{HumanBytes, ProgressBar};
use quinn::{
    default_runtime, Connection, Endpoint, EndpointConfig, RecvStream, SendStream, ServerConfig,
    VarInt,
};
use std::io;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

use crate::{
    common::{handle_unexpected_packet, receive_packet, send_packet, Blake3Hash, FileOrDir},
    packets::{ClientPacket, ServerPacket},
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
    #[error("Version mismatch")]
    VersionMismatch,
    #[error("Unexpected packet: {0:?}")]
    UnexpectedPacket(ClientPacket),
    #[error("Could not verify integrity of file: {0}")]
    CouldNotVerifyIntegrity(String),
}

pub struct Receiver {
    /// The channel to send packets
    pub send: SendStream,
    /// The channel to receive packets
    pub recv: RecvStream,
    /// The connection
    pub conn: Connection,
    /// Server endpoint
    pub server: Endpoint,
}

impl Receiver {
    pub async fn connect(socket: UdpSocket, _sender: SocketAddr) -> Result<Self, ReceiveError> {
        let rt = default_runtime()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no async runtime found"))?;

        let server = Endpoint::new(EndpointConfig::default(), Some(server_config()), socket, rt)?;
        if let Some(conn) = server.accept().await {
            let conn = conn.await?;
            tracing::debug!(
                "Server accepted connection from: {:?}",
                conn.remote_address()
            );

            let (mut send, mut recv) = conn.open_bi().await?;
            send.write_u8(1).await?; // Open the stream

            let packet = receive_packet::<ClientPacket>(&mut recv).await?;
            match packet {
                ClientPacket::ConnRequest { version_num } => {
                    if version_num != VERSION {
                        send_packet(
                            ServerPacket::WrongVersion {
                                expected: VERSION.to_string(),
                            },
                            &mut send,
                        )
                        .await?;
                        return Err(ReceiveError::VersionMismatch);
                    }
                }
                p => {
                    handle_unexpected_packet(&p);
                    return Err(ReceiveError::UnexpectedPacket(p));
                }
            }

            send_packet(ServerPacket::Ok, &mut send).await?;

            return Ok(Self {
                send,
                recv,
                conn,
                server,
            });
        }

        Err(ReceiveError::IoError(io::Error::new(
            io::ErrorKind::Other,
            "no connection found",
        )))
    }

    pub async fn close(&mut self) -> Result<(), ReceiveError> {
        self.send.finish().await.ok();
        self.conn.close(VarInt::from_u32(0), b"");
        Ok(())
    }

    pub(crate) async fn receive_file_meta(&mut self) -> Result<Vec<FileOrDir>, ReceiveError> {
        let packet = receive_packet::<ClientPacket>(&mut self.recv).await?;
        match packet {
            ClientPacket::FileMeta { files } => Ok(files),
            p => {
                handle_unexpected_packet(&p);
                Err(ReceiveError::UnexpectedPacket(p))
            }
        }
    }

    fn accept_files(&self, file_meta: &[FileOrDir]) -> bool {
        println!("The following files will be received:");

        let longest_name = file_meta.iter().map(|f| f.name().len()).max().unwrap_or(0);
        let total_size = file_meta.iter().map(FileOrDir::size).sum::<u64>();

        for file in file_meta {
            let size = file.size();
            let size_human_bytes = format!("({})", HumanBytes(size)).red().to_string();
            let name = file.name().to_string();

            let name = if file.is_dir() {
                // adjust for width of directory indicator
                let name = format!("{}/", name);
                format!("{:width$}", name, width = longest_name)
                    .cyan()
                    .to_string()
            } else {
                format!("{:width$}", name, width = longest_name + 1)
                    .yellow()
                    .to_string()
            };

            let hash = if let FileOrDir::File {
                hash: Some(hash), ..
            } = file
            {
                hex::encode(hash)
            } else {
                String::new()
            };

            println!(
                "{:width$} {} {}",
                name,
                size_human_bytes,
                hash,
                width = longest_name
            );
        }

        let total_size_human_bytes = format!("({})", HumanBytes(total_size)).red().to_string();
        let prompt = format!(
            "\nDo you want to accept these files? {}",
            total_size_human_bytes
        );

        dialoguer::Confirm::new()
            .with_prompt(prompt)
            .interact()
            .unwrap()
    }

    async fn download_files(&self, file_meta: &[FileOrDir]) -> Result<(), ReceiveError> {
        let recv = self.conn.accept_uni().await?;
        let mut recv = GzipDecoder::new(tokio::io::BufReader::with_capacity(FILE_BUF_SIZE, recv));
        tracing::debug!("Accepted file stream");

        let (bars, total_bar) = progress_bars(file_meta);

        for (file, bar) in file_meta.iter().zip(bars.iter()) {
            match file {
                FileOrDir::File { name, size, hash } => {
                    let file = tokio::fs::File::create(Path::new(name)).await?;
                    self.download_single_file(
                        file,
                        name,
                        *size,
                        &mut recv,
                        bar,
                        total_bar.as_ref(),
                        *hash,
                    )
                    .await?;
                }
                FileOrDir::Dir { name, sub } => {
                    self.download_directory(
                        Path::new(name),
                        sub.clone(),
                        &mut recv,
                        bar,
                        total_bar.as_ref(),
                    )
                    .await?;
                }
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn download_single_file<F>(
        &self,
        mut file: F,
        name: &str,
        size: u64,
        recv: &mut GzipDecoder<BufReader<RecvStream>>,
        bar: &ProgressBar,
        total_bar: Option<&ProgressBar>,
        hash: Option<Blake3Hash>,
    ) -> Result<(), ReceiveError>
    // For testing
    where
        F: AsyncWriteExt + Unpin + std::fmt::Debug,
    {
        tracing::debug!("Downloading file: {:?} with size {}", file, size);

        let mut buf = vec![0; FILE_BUF_SIZE];
        let mut bytes_written = 0;

        let mut hasher = blake3::Hasher::new();

        while bytes_written < size {
            let to_read = std::cmp::min(FILE_BUF_SIZE as u64, size - bytes_written) as usize;
            let n = recv.read(&mut buf[..to_read]).await?;

            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected EOF").into());
            }

            file.write_all(&buf[..n]).await.unwrap();
            bytes_written += n as u64;

            if hash.is_some() {
                hasher.update(&buf[..n]);
            }

            bar.inc(n as u64);
            if let Some(total_bar) = total_bar {
                total_bar.inc(n as u64);
            }
        }

        if let Some(expected) = hash {
            let hash: Blake3Hash = hasher.finalize().into();
            if expected != hash {
                return Err(ReceiveError::CouldNotVerifyIntegrity(name.to_string()));
            }
            tracing::debug!("File integrity verified: {:?}", name)
        }

        file.shutdown().await.unwrap();

        tracing::debug!("Finished downloading file: {:?}", name);

        Ok(())
    }

    #[async_recursion::async_recursion]
    async fn download_directory(
        &self,
        dir: &Path,
        sub: Vec<FileOrDir>,
        recv: &mut GzipDecoder<BufReader<RecvStream>>,
        bar: &ProgressBar,
        total_bar: Option<&ProgressBar>,
    ) -> Result<(), ReceiveError> {
        tracing::debug!("Downloading directory: {:?}", dir);
        tokio::fs::create_dir(dir).await?;

        for sub in sub {
            let path = dir.join(sub.name());
            match sub {
                FileOrDir::File { name, size, hash } => {
                    let file = tokio::fs::File::create(&path).await?;
                    self.download_single_file(file, &name, size, recv, bar, total_bar, hash)
                        .await?;
                }
                FileOrDir::Dir { name: _, sub } => {
                    self.download_directory(&path, sub, recv, bar, total_bar)
                        .await?;
                }
            }
        }

        tracing::debug!("Finished downloading directory: {:?}", dir);
        Ok(())
    }
}

pub async fn receive_files(socket: UdpSocket, _sender: SocketAddr) -> Result<(), ReceiveError> {
    let mut server = Receiver::connect(socket, _sender).await?;

    let file_meta = server.receive_file_meta().await?;
    if server.accept_files(&file_meta) {
        send_packet(ServerPacket::AcceptFiles, &mut server.send).await?;
        server.download_files(&file_meta).await?;
    } else {
        send_packet(ServerPacket::RejectFiles, &mut server.send).await?;
    }

    server.close().await?;

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
