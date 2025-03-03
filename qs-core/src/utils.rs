use bincode::{Decode, Encode};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

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
