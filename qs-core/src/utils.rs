use bincode::{Decode, Encode};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;
use stunclient::StunClient;

use crate::common::FileSendRecvTree;
/// Generate a self signed certificate and private key
pub fn self_signed_cert() -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), rcgen::Error>
{
    let cert_key = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert = cert_key.cert.der();
    let key = cert_key.key_pair.serialize_der();

    // let cer = CertificateDer::from(cert);

    Ok((cert.to_owned(), key.try_into().unwrap()))
}

#[derive(Debug, thiserror::Error)]
pub enum StunError {
    #[error("STUN request error: {0}")]
    StunRequest(#[from] stunclient::Error),
    #[error("Your NAT type is not supported, you cannot use this program ):")]
    UnsupportedNatType,
}

/// Query the external address of the socket using a STUN server
/// When given 2 STUN servers it will check if the addresses match (no symmetric NAT)
pub fn external_addr(
    socket: &UdpSocket,
    stun_addr1: SocketAddr,
    stun_addr2: Option<SocketAddr>,
) -> Result<SocketAddr, StunError> {
    let client = StunClient::new(stun_addr1);
    let external_addr = client
        .query_external_address(socket)
        .map_err(StunError::StunRequest)?;

    if let Some(stun_addr2) = stun_addr2 {
        let client = StunClient::new(stun_addr2);
        let external_addr2 = client
            .query_external_address(socket)
            .map_err(StunError::StunRequest)?;

        if external_addr != external_addr2 {
            return Err(StunError::UnsupportedNatType);
        }
    }

    Ok(external_addr)
}

/// Perform a UDP hole punch to the remote address
pub fn hole_punch(socket: &UdpSocket, remote: SocketAddr) -> std::io::Result<()> {
    tracing::debug!("Punching hole to {}", remote);

    socket.connect(remote)?;

    const MSG: &[u8] = &[1];
    const ACK: &[u8] = &[2];

    let mut hole_punched = false;
    let timeout = Duration::from_secs(1);

    const MAX_HOLEPUNCH_TRIES: u8 = 5;
    let mut retries = 0;

    let mut buf = [0; 1];

    while !hole_punched && retries < MAX_HOLEPUNCH_TRIES {
        socket.send(MSG)?;
        socket.set_read_timeout(Some(timeout))?;

        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if let Ok((n, _)) = socket.recv_from(&mut buf) {
                if n != 1 {
                    continue;
                }

                if buf == MSG {
                    socket.send(ACK)?;
                    break;
                }

                if buf == ACK {
                    hole_punched = true;
                    break;
                }
            }
        }

        if !hole_punched {
            retries += 1;
            tracing::debug!("Retrying hole punch, attempt {}", retries);
        }
    }

    Ok(())
}

pub fn hash_files(files: FileSendRecvTree) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    files.hash(&mut hasher);
    hasher.finish()
}

/// Helper struct to store the version, because [semver::Version] does not implement [Encode] and [Decode]
#[derive(Debug, Clone, Encode, Decode)]
pub struct Version {
    /// The Major version represents the Protocol version used by qs-core & the roundezvous server
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    pub fn to_semver(&self) -> semver::Version {
        semver::Version::new(self.major, self.minor, self.patch)
    }

    pub fn from_semver(version: semver::Version) -> Self {
        Self {
            major: version.major,
            minor: version.minor,
            patch: version.patch,
        }
    }

    /// Check if the major version matches (only this matters for compatibility)
    pub fn matches_major(&self, other: &Version) -> bool {
        self.major == other.major
    }
}

impl From<&'static str> for Version {
    fn from(version: &str) -> Self {
        let version = semver::Version::parse(version).unwrap();
        Self::from_semver(version)
    }
}

impl From<semver::Version> for Version {
    fn from(version: semver::Version) -> Self {
        Self::from_semver(version)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}
