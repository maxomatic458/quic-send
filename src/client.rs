use async_compression::tokio::write::GzipEncoder;
use async_recursion::async_recursion;
use rustls::{
    crypto,
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::{
    net::{SocketAddr, UdpSocket},
    path::{Path, PathBuf},
    sync::Arc,
};

use indicatif::ProgressBar;
use quinn::{
    crypto::rustls::QuicClientConfig, default_runtime, ClientConfig, Connection, Endpoint,
    EndpointConfig,
};
use std::io;
use thiserror::Error;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};

use crate::{
    common::{handle_unexpected_packet, receive_packet, send_packet, FileOrDir},
    packets::{ClientPacket, ServerPacket},
    utils::{blake3_from_file, blake3_from_path, progress_bars},
    FILE_BUF_SIZE, SERVER_NAME, VERSION,
};

#[derive(Error, Debug)]
pub enum SendError {
    #[error("IO error: {0}")]
    IoError(#[from] io::Error),
    #[error("Connect error: {0}")]
    ConnectError(#[from] quinn::ConnectError),
    #[error("Connection error: {0}")]
    ConnectionError(#[from] quinn::ConnectionError),
    #[error("Write error: {0}")]
    WriteError(#[from] quinn::WriteError),
    #[error("Read error: {0}")]
    ReadError(#[from] quinn::ReadError),
    #[error("Unexpected packet: {0:?}")]
    UnexpectedPacket(ServerPacket),
    #[error("Wrong version, the receiver expected: {0}")]
    WrongVersion(String),
    #[error("The receiver rejected the files")]
    FilesRejected,
    #[error("File does not exist: {0:?}")]
    FileDoesNotExist(PathBuf),
}

pub struct Sender {
    /// Arguments
    pub args: SenderArgs,
    /// Connection
    pub conn: Connection,
    /// Client endpoint
    pub client: Endpoint,
}

pub struct SenderArgs {
    /// Calculate and send checksums (blake3)
    pub checksums: bool,
    /// Files to send
    pub files: Vec<PathBuf>,
}

impl Sender {
    pub async fn connect(
        socket: UdpSocket,
        receiver: SocketAddr,
        args: SenderArgs,
    ) -> Result<Self, SendError> {
        let rt = default_runtime()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no async runtime found"))?;

        let mut client = Endpoint::new(EndpointConfig::default(), None, socket, rt)?;

        client.set_default_client_config(client_config());
        let conn = client.connect(receiver, SERVER_NAME)?.await?;
        tracing::debug!("Client connected to server: {:?}", conn.remote_address());

        send_packet(
            ClientPacket::ConnRequest {
                version_num: VERSION.to_string(),
            },
            &conn,
        )
        .await?;

        let resp = receive_packet::<ServerPacket>(&conn).await?;
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

        let client = Self { args, conn, client };

        Ok(client)
    }

    pub async fn wait_for_close(&mut self) -> Result<(), SendError> {
        self.client.wait_idle().await;
        Ok(())
    }

    pub(crate) async fn send_file_meta(
        &mut self,
        file_meta: &[FileOrDir],
    ) -> Result<(), SendError> {
        send_packet(
            ClientPacket::FileMeta {
                files: file_meta.to_vec(),
            },
            &self.conn,
        )
        .await?;

        let resp = receive_packet::<ServerPacket>(&self.conn).await?;
        match resp {
            ServerPacket::AcceptFiles => Ok(()),
            ServerPacket::RejectFiles => Err(SendError::FilesRejected),
            p => {
                handle_unexpected_packet(&p);
                Err(SendError::UnexpectedPacket(p))
            }
        }
    }

    pub(crate) async fn upload_files(&mut self, file_meta: &[FileOrDir]) -> Result<(), SendError> {
        tracing::debug!("Opening file stream");

        let (bars, total_bar) = progress_bars(file_meta);

        for (file, bar) in self.args.files.clone().iter().zip(bars) {
            if file.is_file() {
                self.upload_single_file(
                    file,
                    file.metadata()?.len(),
                    // &mut send,
                    &bar,
                    total_bar.as_ref(),
                )
                .await?;
                bar.finish();
            } else {
                self.upload_directory(file, &bar, total_bar.as_ref())
                    .await?;
                bar.finish();
            }
        }

        Ok(())
    }

    /// Handles how/if the file should be sent
    /// See [``crate::server::Receiver::handle_save_mode``]
    async fn handle_receiver_save_mode(
        &mut self,
        path: &Path,
    ) -> Result<(Option<File>, u64), SendError> {
        let mut bytes_read = 0;
        let request = receive_packet::<ServerPacket>(&self.conn).await?;

        let file = match request {
            // Send the whole file
            ServerPacket::Ok => Some(File::open(path).await?),
            // Skip the file
            ServerPacket::SkipFile => {
                tracing::debug!("Skipping file: {:?}", path);
                None
            }
            // The server requests the file to be sent from a specific position
            ServerPacket::FileFromPos { pos } => {
                let mut file = File::open(path).await?;
                let hash = if self.args.checksums {
                    Some(blake3_from_file(&mut file, pos).await?)
                } else {
                    None
                };

                bytes_read = pos;
                send_packet(ClientPacket::FilePosHash { hash }, &self.conn).await?;

                file.seek(io::SeekFrom::Start(pos)).await?;
                Some(file)
            }
            p => {
                handle_unexpected_packet(&p);
                return Err(SendError::UnexpectedPacket(p));
            }
        };

        Ok((file, bytes_read))
    }

    async fn upload_single_file(
        &mut self,
        path: &Path,
        size: u64,
        bar: &ProgressBar,
        total_bar: Option<&ProgressBar>,
    ) -> Result<(), SendError> {
        tracing::debug!("Uploading file: {:?} size: {}", path, size);

        let mut send = self.conn.open_uni().await?;
        send.write_u8(1).await?; // Opening byte

        let mut send = GzipEncoder::new(&mut send);

        let (mut file, mut bytes_read) =
            if let (Some(file), bytes_read) = self.handle_receiver_save_mode(path).await? {
                if bytes_read > 0 {
                    tracing::info!("Resuming file: {:?}", path);
                }
                (file, bytes_read)
            } else {
                tracing::info!("Skipping file: {:?}", path);
                return Ok(());
            };

        let mut buf = vec![0; FILE_BUF_SIZE];

        bar.inc(bytes_read);

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

        send.write_u8(1).await?; // Prevent sending nothing, which can cause issues

        send.shutdown().await?;
        file.shutdown().await?;

        tracing::debug!("Finished uploading file: {:?}", path);

        Ok(())
    }

    #[async_recursion::async_recursion]
    async fn upload_directory(
        &mut self,
        dir: &Path,
        bar: &ProgressBar,
        total_bar: Option<&ProgressBar>,
    ) -> Result<(), SendError> {
        tracing::debug!("Uploading directory: {:?}", dir);
        for entry in dir.read_dir()? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                self.upload_single_file(&path, path.metadata()?.len(), bar, total_bar)
                    .await?;
            } else {
                self.upload_directory(&path, bar, total_bar).await?;
            }
        }

        tracing::debug!("Finished uploading directory: {:?}", dir);
        Ok(())
    }
}

#[async_recursion]
async fn file_meta(files: &[PathBuf], checksums: bool) -> io::Result<Vec<FileOrDir>> {
    let mut out = Vec::new();

    for file in files {
        if file.is_file() {
            let file_size = file.metadata()?.len();

            let blake3_hash = if checksums {
                Some(blake3_from_path(file).await?)
            } else {
                None
            };

            // tracing::debug!(
            //     "File: {:?}, size: {}, hash: {:?}",
            //     file,
            //     file_size,
            //     blake3_hash.map(hex::encode)
            // );

            out.push(FileOrDir::File {
                name: file.file_name().unwrap().to_string_lossy().to_string(),
                size: file_size,
                hash: blake3_hash,
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

pub async fn send_files(
    socket: UdpSocket,
    receiver: SocketAddr,
    args: SenderArgs,
) -> Result<(), SendError> {
    let mut client = Sender::connect(socket, receiver, args).await?;
    let file_meta = file_meta(&client.args.files, client.args.checksums).await?;
    client.send_file_meta(&file_meta).await?;
    client.upload_files(&file_meta).await?;
    tracing::debug!("Finished sending files");

    client.wait_for_close().await?;

    Ok(())
}

fn client_config() -> ClientConfig {
    let mut binding = rustls::ClientConfig::builder()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();

    let mut crypto = binding.dangerous();

    crypto.set_certificate_verifier(SkipServerVerification::new(Arc::new(
        crypto::aws_lc_rs::default_provider(),
    )));

    ClientConfig::new(Arc::new(
        QuicClientConfig::try_from(crypto.cfg.to_owned()).unwrap(),
    ))
}

/// Implementation of `ServerCertVerifier` that verifies everything as trustworthy.
/// https://quinn-rs.github.io/quinn/quinn/certificate.html
/// Dummy certificate verifier that treats any certificate as valid.
/// NOTE, such verification is vulnerable to MITM attacks, but convenient for testing.
#[derive(Debug)]
struct SkipServerVerification(Arc<rustls::crypto::CryptoProvider>);

impl SkipServerVerification {
    fn new(provider: Arc<rustls::crypto::CryptoProvider>) -> Arc<Self> {
        Arc::new(Self(provider))
    }
}

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
