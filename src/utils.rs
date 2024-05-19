use blake3::Hasher;
use clap::ValueEnum;
use indicatif::MultiProgress;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use rcgen;
use std::io;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::path::Path;
use stunclient::StunClient;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeekExt;

use crate::common::Blake3Hash;
use crate::common::FileOrDir;
use crate::HASH_BUF_SIZE;
 
#[derive(Debug, Clone, ValueEnum)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn to_tracing_level(&self) -> tracing::Level {
        match self {
            Self::Trace => tracing::Level::TRACE,
            Self::Debug => tracing::Level::DEBUG,
            Self::Info => tracing::Level::INFO,
            Self::Warn => tracing::Level::WARN,
            Self::Error => tracing::Level::ERROR,
        }
    }
}

impl From<&str> for LogLevel {
    fn from(s: &str) -> Self {
        match s {
            "trace" => Self::Trace,
            "debug" => Self::Debug,
            "info" => Self::Info,
            "warn" => Self::Warn,
            "error" => Self::Error,
            _ => Self::Info,
        }
    }
}

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
    tracing::debug!("Punching hole to {}", remote);
    // TODO: Make this more reliable
    socket.send_to(&[0], remote)?;

    Ok(())
}

/// Progress bars for uploading/downloading files
pub fn progress_bars(files: &[FileOrDir]) -> (Vec<ProgressBar>, Option<ProgressBar>) {
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
        let total_name = format!("{:width$}", total_name, width = longest_name);
        let bar = mp.add(ProgressBar::new(total_size));
        bar.set_style(total_style);
        bar.set_prefix(total_name);
        Some(bar)
    } else {
        None
    };

    (bars, total_bar)
}

pub async fn blake3_from_path(path: &Path) -> io::Result<Blake3Hash> {
    let mut file = File::open(path).await?;
    blake3_from_file_from(&mut file, 0).await
}

pub async fn blake3_from_file_from(file: &mut File, pos: u64) -> io::Result<Blake3Hash> {
    let mut buf = [0; HASH_BUF_SIZE];
    file.seek(io::SeekFrom::Start(pos)).await?;
    let mut hasher = Hasher::new();
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

/// Update a hasher with the contents of a file,
/// this is for continuing interrupted downloads
pub async fn update_hasher(hasher: &mut Hasher, file: &mut File) -> io::Result<()> {
    let mut buf = [0; HASH_BUF_SIZE];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(())
}
