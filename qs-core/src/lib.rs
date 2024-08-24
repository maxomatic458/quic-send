use quinn::{crypto::rustls::QuicClientConfig, ClientConfig};
use rustls::{
    crypto,
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};
use thiserror::Error;

pub mod common;
pub mod packets;
pub mod receive;
pub mod send;
pub mod utils;

pub const BUF_SIZE: usize = 8192;
pub const STUN_SERVER: &str = "stun.l.google.com:19302";
pub const SEND_SERVER_NAME: &str = "quic-send";
pub const ROUNDEZVOUS_SERVER_NAME: &str = "quic-roundezvous";
pub const KEEP_ALIVE_INTERVAL_SECS: u64 = 5;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The roundezvous server im hosting is running behind the playit.gg proxy.
/// the IP should be static, but i should probably get a domain name for it
pub const DEFAULT_ADDR: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(209, 25, 141, 16)), 1172);
// When changing this also change that in qs-gui/src/Pages/Receive.tsx
pub const CODE_LEN: usize = 8;

#[derive(Error, Debug)]
pub enum QuicSendError {
    #[error("stun error: {0}")]
    Stun(#[from] stunclient::Error),
    #[error("rcgen error: {0}")]
    RcGen(#[from] rcgen::Error),
    #[error("send error: {0}")]
    Send(#[from] send::SendError),
    #[error("receive error: {0}")]
    Receive(#[from] receive::ReceiveError),
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
