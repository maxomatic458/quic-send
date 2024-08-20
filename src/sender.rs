use async_compression::tokio::write::GzipEncoder;
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
    EndpointConfig, SendStream,
};
use std::io;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::{
    common::{
        apply_files_to_skip_tree, get_available_files_tree, receive_packet, send_packet,
        FileRecvSendTree,
    },
    packets::{Receiver2Sender, Sender2Receiver},
    utils::progress_bars,
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
    UnexpectedPacket(Receiver2Sender),
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

        Ok(Self { args, conn, client })
    }

    async fn wait_for_close(&mut self) -> Result<(), SendError> {
        self.client.wait_idle().await;
        Ok(())
    }

    fn get_file_info(&self, files: &[PathBuf]) -> io::Result<Vec<FileRecvSendTree>> {
        files
            .iter()
            .map(get_available_files_tree)
            .collect::<Result<Vec<_>, _>>()
    }

    /// Upload a single file
    async fn upload_file(
        &self,
        send: &mut GzipEncoder<SendStream>,
        path: &Path,
        skip: u64,
        size: u64,
        bar: &ProgressBar,
        total_bar: Option<&ProgressBar>,
    ) -> Result<(), SendError> {
        tracing::debug!("sending file: {:?}", path);
        let mut file = tokio::fs::File::open(path).await?;
        file.seek(io::SeekFrom::Start(skip)).await?;

        let mut bytes_read = skip;
        let mut buf = vec![0; FILE_BUF_SIZE];

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

        tracing::debug!("finished sending file: {:?}", path);

        file.shutdown().await?;
        Ok(())
    }

    /// Upload s single directory
    #[async_recursion::async_recursion]
    async fn upload_directory(
        &self,
        send: &mut GzipEncoder<SendStream>,
        files: &[FileRecvSendTree],
        path: &Path,
        bar: &ProgressBar,
        total_bar: Option<&ProgressBar>,
    ) -> Result<(), SendError> {
        tracing::debug!("sending directory: {:?}", path);

        for file in files {
            let path = path.join(&file.name());
            match file {
                FileRecvSendTree::File { size, skip, .. } => {
                    self.upload_file(send, &path, *skip, *size, bar, total_bar)
                        .await?;
                }
                FileRecvSendTree::Dir { files, .. } => {
                    self.upload_directory(send, files, &path, bar, total_bar)
                        .await?;
                }
            }
        }

        tracing::debug!("finished sending directory: {:?}", path);

        Ok(())
    }

    /// Upload all files and directories
    async fn send_files(
        &self,
        files: &[(&PathBuf, FileRecvSendTree)],
        bars: (Vec<ProgressBar>, Option<ProgressBar>),
    ) -> Result<(), SendError> {
        tracing::debug!("begin file sending");
        let (bars, total_bar) = bars;
        let mut send = self.conn.open_uni().await?;
        send.write_u8(1).await?; // Opening byte

        let mut send = GzipEncoder::new(send);

        for ((path, file_tree), bar) in files.iter().zip(bars.iter()) {
            match file_tree {
                FileRecvSendTree::File { size, skip, .. } => {
                    self.upload_file(&mut send, path, *skip, *size, bar, total_bar.as_ref())
                        .await?;
                }
                FileRecvSendTree::Dir { files, .. } => {
                    self.upload_directory(&mut send, files, path, bar, total_bar.as_ref())
                        .await?;
                }
            }
        }

        tracing::debug!("finished file sending");

        send.shutdown().await?;
        Ok(())
    }
}

pub async fn send_files(
    socket: UdpSocket,
    receiver: SocketAddr,
    args: SenderArgs,
) -> Result<(), SendError> {
    let mut sender = Sender::connect(socket, receiver, args).await?;

    send_packet(
        Sender2Receiver::ConnRequest {
            version_num: VERSION.to_string(),
        },
        &sender.conn,
    )
    .await?;

    match receive_packet::<Receiver2Sender>(&sender.conn).await? {
        Receiver2Sender::Ok => {}
        Receiver2Sender::WrongVersion { expected } => {
            return Err(SendError::WrongVersion(expected))
        }
        p => return Err(SendError::UnexpectedPacket(p)),
    };

    let file_info = sender.get_file_info(&sender.args.files)?;
    send_packet(
        Sender2Receiver::FileInfo {
            files: file_info.clone(),
        },
        &sender.conn,
    )
    .await?;

    let skip_requested = match receive_packet::<Receiver2Sender>(&sender.conn).await? {
        Receiver2Sender::AcceptFilesSkip { files } => files,
        Receiver2Sender::RejectFiles => return Err(SendError::FilesRejected),
        p => return Err(SendError::UnexpectedPacket(p)),
    };

    let to_send = {
        let mut to_send = vec![];
        for ((path, file_info), skip) in sender
            .args
            .files
            .iter()
            .zip(file_info.iter())
            .zip(skip_requested.iter())
        {
            match skip {
                Some(skip) => {
                    if let Some(trimmed) = apply_files_to_skip_tree(file_info, skip) {
                        to_send.push((path, trimmed));
                    }
                }
                None => {
                    to_send.push((path, file_info.clone()));
                }
            }
        }

        to_send
    };

    let bars = progress_bars(
        &file_info,
        &skip_requested
            .iter()
            .map(|x| x.as_ref().map(|x| x.skip()).unwrap_or_default())
            .collect::<Vec<_>>(),
    );

    sender.send_files(&to_send, bars).await?;

    sender.wait_for_close().await?;
    Ok(())
}

fn client_config() -> ClientConfig {
    let mut binding = rustls::ClientConfig::builder()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();

    let mut crypto = binding.dangerous();

    crypto.set_certificate_verifier(SkipServerVerification::new(Arc::new(
        crypto::ring::default_provider(),
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
