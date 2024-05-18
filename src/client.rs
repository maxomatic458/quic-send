use async_compression::tokio::write::GzipEncoder;
use sha1::Sha1;
use std::{
    net::{SocketAddr, UdpSocket},
    path::{Path, PathBuf},
    sync::Arc,
};

use indicatif::ProgressBar;
use quinn::{default_runtime, ClientConfig, Endpoint, EndpointConfig, RecvStream, SendStream};
use sha1::Digest;
use std::io;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    common::{handle_unexpected_packet, receive_packet, send_packet, FileOrDir},
    packets::{ClientPacket, ServerPacket},
    utils::progress_bars,
    FILE_BUF_SIZE, SERVER_NAME, VERSION,
};

#[derive(Error, Debug)]
pub enum SendError {
    IoError(#[from] io::Error),
    ConnectError(#[from] quinn::ConnectError),
    ConnectionError(#[from] quinn::ConnectionError),
    WriteError(#[from] quinn::WriteError),
    ReadError(#[from] quinn::ReadError),
    UnexpectedPacket(ServerPacket),
    WrongVersion(String),
    FilesRejected,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IoError(e) => write!(f, "IO error: {}", e),
            Self::ConnectError(e) => write!(f, "Connect error: {}", e),
            Self::ConnectionError(e) => write!(f, "Connection error: {}", e),
            Self::ReadError(e) => write!(f, "Read error: {}", e),
            Self::UnexpectedPacket(p) => write!(f, "Unexpected packet: {:?}", p),
            Self::WrongVersion(v) => write!(f, "Wrong version, the receiver expected: {}", v),
            Self::FilesRejected => write!(f, "The receiver rejected the files"),
            Self::WriteError(e) => write!(f, "Write error: {}", e),
        }
    }
}

pub async fn send_files(
    files: &[PathBuf],
    socket: UdpSocket,
    receiver: SocketAddr,
    checksums: bool,
) -> Result<(), SendError> {
    let rt = default_runtime()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no async runtime found"))?;

    let mut client = Endpoint::new(EndpointConfig::default(), None, socket, rt)?;

    client.set_default_client_config(client_config());
    let conn = client.connect(receiver, SERVER_NAME)?.await?;
    tracing::info!("Client connected to server: {:?}", conn.remote_address());

    let (mut send, mut recv) = conn.accept_bi().await?;
    recv.read_u8().await?; // Ignore opening byte

    send_packet(
        ClientPacket::ConnRequest {
            version_num: VERSION.to_string(),
        },
        &mut send,
    )
    .await?;

    let resp = receive_packet::<ServerPacket>(&mut recv).await?;
    match resp {
        ServerPacket::Ok => {}
        ServerPacket::WrongVersion { expected } => {
            return Err(SendError::WrongVersion(expected));
        }
        p => {
            handle_unexpected_packet(&p);
            return Err(SendError::UnexpectedPacket(p));
        }
    }

    if checksums {
        tracing::debug!("Calculating checksums for files");
        println!("Calculating checksums for files, this may take a while...")
    }

    let file_meta = file_meta(files, checksums).await?;
    send_packet(
        ClientPacket::FileMeta {
            files: file_meta.clone(),
        },
        &mut send,
    )
    .await?;

    println!("Waiting for server to accept files...");

    let resp = receive_packet::<ServerPacket>(&mut recv).await?;
    match resp {
        ServerPacket::AcceptFiles => {}
        ServerPacket::RejectFiles => {
            return Err(SendError::FilesRejected);
        }
        p => {
            handle_unexpected_packet(&p);
            return Err(SendError::UnexpectedPacket(p));
        }
    }

    upload_files(files, &file_meta, &mut send, &mut recv).await?;

    tracing::info!("Finished sending files");

    client.wait_idle().await;

    Ok(())
}

#[async_recursion::async_recursion]
async fn file_meta(files: &[PathBuf], checksums: bool) -> io::Result<Vec<FileOrDir>> {
    let mut out = Vec::new();

    for file in files {
        if file.is_file() {
            let file_size = file.metadata()?.len();

            let sha1_hash = if checksums {
                let mut hasher = Sha1::new();
                let mut file = tokio::fs::File::open(file).await?;
                let mut buf = vec![0; FILE_BUF_SIZE];
                loop {
                    let n = file.read(&mut buf).await?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                Some(hasher.finalize().into())
            } else {
                None
            };

            out.push(FileOrDir::File {
                name: file.file_name().unwrap().to_string_lossy().to_string(),
                size: file_size,
                hash: sha1_hash,
            });
        } else {
            let mut dir_contents = Vec::new();
            for sub in file.read_dir()? {
                let sub = sub?;
                dir_contents.push(sub.path());
            }

            out.push(FileOrDir::Dir {
                name: file.file_name().unwrap().to_string_lossy().to_string(),
                sub: file_meta(&dir_contents, checksums).await?,
            });
        }
    }

    Ok(out)
}

async fn upload_files(
    files: &[PathBuf],
    file_meta: &[FileOrDir],
    send: &mut SendStream,
    _recv: &mut RecvStream,
) -> Result<(), SendError> {
    tracing::debug!("Uploading {} files", files.len());

    let mut send = GzipEncoder::new(send);

    let (bars, total_bar) = progress_bars(file_meta);

    for (file, bar) in files.iter().zip(bars) {
        if file.is_file() {
            upload_single_file(
                file,
                file.metadata()?.len(),
                &mut send,
                &bar,
                total_bar.as_ref(),
            )
            .await?;
        } else {
            upload_directory(file, &mut send, &bar, total_bar.as_ref()).await?;
        }
    }

    send.shutdown().await?;

    Ok(())
}

async fn upload_single_file(
    path: &Path,
    size: u64,
    send: &mut GzipEncoder<&mut SendStream>,
    bar: &ProgressBar,
    total_bar: Option<&ProgressBar>,
) -> Result<(), SendError> {
    tracing::debug!("Uploading file: {:?}", path);

    let mut file = tokio::fs::File::open(path).await?;

    let mut buf = vec![0; FILE_BUF_SIZE];
    let mut bytes_read = 0;

    while bytes_read < size {
        let to_read = std::cmp::min(FILE_BUF_SIZE as u64, size - bytes_read);
        let n = file.read(&mut buf[..to_read as usize]).await?;

        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected EOF").into());
        }

        send.write_all(&buf[..n]).await?;
        bytes_read += n as u64;

        bar.inc(n as u64);
        if let Some(total_bar) = total_bar {
            total_bar.inc(n as u64);
        }
    }

    file.shutdown().await?;

    tracing::debug!("Finished uploading file: {:?}", path);

    Ok(())
}

#[async_recursion::async_recursion]
async fn upload_directory(
    dir: &Path,
    send: &mut GzipEncoder<&mut SendStream>,
    bar: &ProgressBar,
    total_bar: Option<&ProgressBar>,
) -> Result<(), SendError> {
    tracing::debug!("Uploading directory: {:?}", dir);
    for entry in dir.read_dir()? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            upload_single_file(&path, path.metadata()?.len(), send, bar, total_bar).await?;
        } else {
            upload_directory(&path, send, bar, total_bar).await?;
        }
    }

    tracing::debug!("Finished uploading directory: {:?}", dir);
    Ok(())
}

fn client_config() -> ClientConfig {
    let crypto = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_no_client_auth();

    ClientConfig::new(Arc::new(crypto))
}

/// Implementation of `ServerCertVerifier` that verifies everything as trustworthy.
/// https://quinn-rs.github.io/quinn/quinn/certificate.html
struct SkipServerVerification;

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl rustls::client::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}
