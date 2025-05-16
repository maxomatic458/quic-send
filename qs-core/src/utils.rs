use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::common::FileSendRecvTree;
/// Generate a self signed certificate and private key
pub fn self_signed_cert() -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), rcgen::Error>
{
    let cert_key = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert = cert_key.cert.der();
    let key = cert_key.key_pair.serialize_der();

    Ok((cert.to_owned(), key.try_into().unwrap()))
}

pub fn hash_files(files: FileSendRecvTree) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    files.hash(&mut hasher);
    hasher.finish()
}