use indicatif::MultiProgress;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use rcgen;
use std::io;
use std::net::SocketAddr;
use std::net::UdpSocket;
use stunclient::StunClient;

use crate::common::FileOrDir;

/// Generate a self signed certificate and private key
pub fn self_signed_cert() -> Result<(rustls::Certificate, rustls::PrivateKey), rcgen::Error> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let key = rustls::PrivateKey(cert.key_pair.serialize_der());
    Ok((rustls::Certificate(cert.cert.der().to_vec()), key))
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
    // TODO: Make this more reliable
    socket.send_to(&[0], remote)?;

    Ok(())
}

/// Progress bars for uploading/downloading files
pub fn progress_bars(files: &[FileOrDir]) -> (Vec<ProgressBar>, Option<ProgressBar>) {
    let total_name = "Total";

    let style = ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})").unwrap()
        .progress_chars("##-");

    let total_style = ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.yellow/yellow}] {bytes}/{total_bytes} ({eta})").unwrap()
        .progress_chars("##-");

    let longest_name = files.iter().map(|f| f.name().len()).max().unwrap_or(0);
    let total_size = files.iter().map(FileOrDir::size).sum::<u64>();

    let mp = MultiProgress::new();

    let mut bars = Vec::new();
    for file in files {
        let name = format!("{:width$}", file.name(), width = longest_name);
        let bar = mp.add(ProgressBar::new(file.size()));
        bar.set_style(style.clone());
        bar.set_prefix(name);
        bars.push(bar);
    }

    let total_bar = if bars.len() > 1 {
        let bar = mp.add(ProgressBar::new(total_size));
        bar.set_style(total_style);
        bar.set_prefix(total_name);
        Some(bar)
    } else {
        None
    };

    (bars, total_bar)
}
