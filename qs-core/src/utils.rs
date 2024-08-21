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

/// Query the external address of the socket using a STUN server
pub fn external_addr(
    socket: &UdpSocket,
    stun_addr: SocketAddr,
) -> Result<SocketAddr, stunclient::Error> {
    let client = StunClient::new(stun_addr);

    client.query_external_address(socket)
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
