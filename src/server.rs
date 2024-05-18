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
    default_runtime, Endpoint, EndpointConfig, RecvStream, SendStream, ServerConfig, VarInt,
};
use sha1::Digest;
use std::io;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

use crate::{
    common::{handle_unexpected_packet, receive_packet, send_packet, FileOrDir, Sha1Hash},
    packets::{ClientPacket, ServerPacket},
    utils::{progress_bars, self_signed_cert},
    FILE_BUF_SIZE, KEEP_ALIVE_INTERVAL_SECS, VERSION,
};

#[derive(Error, Debug)]
pub enum ReceiveError {
    IoError(#[from] io::Error),
    ConnectionError(#[from] quinn::ConnectionError),
    WriteError(#[from] quinn::WriteError),
    VersionMismatch,
    UnexpectedPacket(ClientPacket),
    CouldNotVerifyIntegrity(String),
}

impl std::fmt::Display for ReceiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IoError(e) => write!(f, "IO error: {}", e),
            Self::ConnectionError(e) => write!(f, "Connection error: {}", e),
            Self::VersionMismatch => write!(f, "Version mismatch"),
            Self::UnexpectedPacket(p) => write!(f, "Unexpected packet: {:?}", p),
            Self::WriteError(e) => write!(f, "Write error: {}", e),
            Self::CouldNotVerifyIntegrity(file) => {
                write!(f, "Could not verify integrity of file: {}", file)
            }
        }
    }
}

pub async fn receive_files(socket: UdpSocket, _sender: SocketAddr) -> Result<(), ReceiveError> {
    let rt = default_runtime()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no async runtime found"))?;

    let server = Endpoint::new(EndpointConfig::default(), Some(server_config()), socket, rt)?;

    if let Some(conn) = server.accept().await {
        let conn = conn.await?;
        tracing::info!(
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

        let packet = receive_packet::<ClientPacket>(&mut recv).await?;
        let file_meta = match packet {
            ClientPacket::FileMeta { files } => files,
            p => {
                handle_unexpected_packet(&p);
                return Err(ReceiveError::UnexpectedPacket(p));
            }
        };

        let accept = accept_files(&file_meta);
        if !accept {
            send_packet(ServerPacket::RejectFiles, &mut send).await?;
        } else {
            send_packet(ServerPacket::AcceptFiles, &mut send).await?;
            download_files(&file_meta, &mut send, &mut recv).await?;
            tracing::info!("Finished receiving files");
        }

        send.finish().await?;
    }

    server.close(VarInt::from_u32(0), &[]);

    Ok(())
}

fn accept_files(file_meta: &[FileOrDir]) -> bool {
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

async fn download_files(
    file_meta: &[FileOrDir],
    _send: &mut SendStream,
    recv: &mut RecvStream,
) -> Result<(), ReceiveError> {
    tracing::debug!("Downloading {} files", file_meta.len());

    let mut recv = GzipDecoder::new(tokio::io::BufReader::with_capacity(FILE_BUF_SIZE, recv));

    let (bars, total_bar) = progress_bars(file_meta);

    for (file, bar) in file_meta.iter().zip(bars.iter()) {
        match file {
            FileOrDir::File { name, size, hash } => {
                download_single_file(
                    Path::new(name),
                    *size,
                    &mut recv,
                    bar,
                    total_bar.as_ref(),
                    *hash,
                )
                .await?;
            }
            FileOrDir::Dir { name, sub } => {
                download_directory(
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

async fn download_single_file(
    file_path: &Path,
    size: u64,
    recv: &mut GzipDecoder<BufReader<&mut RecvStream>>,
    bar: &ProgressBar,
    total_bar: Option<&ProgressBar>,
    hash: Option<Sha1Hash>,
) -> Result<(), ReceiveError> {
    tracing::debug!("Downloading file: {:?} with size {}", file_path, size);

    let mut file = tokio::fs::File::create(file_path).await?;

    let mut buf = vec![0; FILE_BUF_SIZE];
    let mut bytes_written = 0;

    let mut hasher = sha1::Sha1::new();

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
        let hash: Sha1Hash = hasher.finalize().into();
        if expected != hash {
            return Err(ReceiveError::CouldNotVerifyIntegrity(
                file_path.display().to_string(),
            ));
        }
    }

    file.shutdown().await.unwrap();

    tracing::debug!("Finished downloading file: {:?}", file);

    Ok(())
}

#[async_recursion::async_recursion]
async fn download_directory(
    dir: &Path,
    sub: Vec<FileOrDir>,
    recv: &mut GzipDecoder<BufReader<&mut RecvStream>>,
    bar: &ProgressBar,
    total_bar: Option<&ProgressBar>,
) -> Result<(), ReceiveError> {
    tracing::debug!("Downloading directory: {:?}", dir);
    tokio::fs::create_dir(dir).await?;

    for sub in sub {
        let path = dir.join(sub.name());
        match sub {
            FileOrDir::File {
                name: _,
                size,
                hash,
            } => {
                download_single_file(&path, size, recv, bar, total_bar, hash).await?;
            }
            FileOrDir::Dir { name: _, sub } => {
                download_directory(&path, sub, recv, bar, total_bar).await?;
            }
        }
    }

    tracing::debug!("Finished downloading directory: {:?}", dir);
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
