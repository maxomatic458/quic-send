use quinn::{crypto::rustls::QuicClientConfig, ClientConfig};
use rustls::{
    crypto,
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::sync::Arc;
use thiserror::Error;

pub mod common;
pub mod packets;
pub mod receive;
pub mod send;
pub mod utils;

pub const BUF_SIZE: usize = 8192;
pub const SEND_SERVER_NAME: &str = "quic-send";
pub const KEEP_ALIVE_INTERVAL_SECS: u64 = 5;
pub const QS_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const QS_ALPN: &[u8] = b"quic-send/0.4.0";

#[derive(Error, Debug)]
pub enum QuicSendError {
    #[error("rcgen error: {0}")]
    RcGen(#[from] rcgen::Error),
    #[error("send error: {0}")]
    Send(#[from] send::SendError),
    #[error("receive error: {0}")]
    Receive(#[from] receive::ReceiveError),
    #[error("bind error: {0}")]
    Bind(String),
}

/// Client config that ignores the server certificate
pub fn unsafe_client_config() -> ClientConfig {
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
pub struct SkipServerVerification(Arc<rustls::crypto::CryptoProvider>);

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
