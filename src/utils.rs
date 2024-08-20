use indicatif::MultiProgress;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use rcgen;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;
use std::io;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::time::Duration;
use stunclient::StunClient;

use crate::common::FileRecvSendTree;

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
pub fn hole_punch(socket: &UdpSocket, remote: SocketAddr) -> io::Result<()> {
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

pub fn progress_bars(
    offered_files: &[FileRecvSendTree],
    skipped: &[u64],
) -> (Vec<ProgressBar>, Option<ProgressBar>) {
    let total_name = "Total";

    let style = ProgressStyle::default_bar()
        .template("{spinner:.green} {prefix} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        .unwrap()
        .progress_chars("##-");

    let total_style = ProgressStyle::default_bar()
        .template(
            "{spinner:.green} {prefix} [{bar:40.yellow/yellow}] {bytes}/{total_bytes} ({eta})",
        )
        .unwrap()
        .progress_chars("##-");

    let longest_name = offered_files
        .iter()
        .map(|f| f.name().len())
        .max()
        .unwrap_or(0);
    let total_size = offered_files
        .iter()
        .map(FileRecvSendTree::size)
        .sum::<u64>();
    let total_skip = skipped.iter().sum::<u64>();

    let mp = MultiProgress::new();

    let mut bars = vec![];

    for (file, skipped) in offered_files.iter().zip(skipped.iter()) {
        let name = format!("{:width$}", file.name(), width = longest_name);
        let bar = mp.add(ProgressBar::new(file.size()).with_position(*skipped));
        bar.set_style(style.clone());
        bar.set_prefix(name);
        bars.push(bar);
    }

    let total_bar = if bars.len() > 1 {
        // let total_name = format!("{:width$}", total_name, width = longest_name);
        // let pb = ProgressBar::new(total_size);
        // // pb.inc(total_skip);
        // pb.set_style(total_style);
        // pb.set_prefix(total_name);
        // let bar = mp.add(pb.with_position(total_skip));
        // Some(bar)

        let total_name = format!("{:width$}", total_name, width = longest_name);
        let pb = ProgressBar::new(total_size);
        pb.set_style(total_style);
        pb.set_prefix(total_name);
        pb.set_position(total_skip);
        let bar = mp.add(pb);
        Some(bar)
    } else {
        None
    };

    (bars, total_bar)
}
