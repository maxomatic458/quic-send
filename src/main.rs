use std::{
    net::{Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket},
    path::PathBuf,
};

use crate::{
    receiver::ReceiverArgs,
    sender::{SendError, SenderArgs},
    utils::hole_punch,
};
use clap::{Parser, Subcommand};
use color_eyre::owo_colors::OwoColorize;
use receiver::ReceiveError;
use thiserror::Error;
// use utils::LogLevel;

pub mod common;
pub mod packets;
pub mod receiver;
pub mod sender;
pub mod utils;

pub const SERVER_NAME: &str = "server";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const STUN_SERVER: &str = "stun.l.google.com:19302";
pub const KEEP_ALIVE_INTERVAL_SECS: u64 = 5;
pub const FILE_BUF_SIZE: usize = 8192;
pub const HASH_BUF_SIZE: usize = 8192 * 6;

#[derive(Parser, Debug)]
#[clap(version = VERSION, author = env!("CARGO_PKG_AUTHORS"))]
struct Args {
    /// Log level
    #[clap(long, short, default_value = "info")]
    log_level: tracing::Level,
    /// Send or receive files
    #[clap(subcommand)]
    pub mode: Mode,
}

#[derive(Subcommand, Debug)]
enum Mode {
    #[clap(name = "send", about = "Send files")]
    Send {
        /// Files/directories to send
        #[clap(name = "files or directories", required = true)]
        files: Vec<PathBuf>,
    },
    #[clap(name = "receive", about = "Receive files")]
    Receive {
        /// Resume interrupted transfers, this will append to files if they already exist or skip them.
        /// If a existing file is corrupted (e.g larger than the original, or if the hash (if `checksums` was used) doesn't match)
        /// it will be overwritten.
        #[clap(long, short)]
        resume: bool,
    },
}

#[derive(Error, Debug)]
enum AppError {
    #[error("STUN error: {0}")]
    Stun(#[from] stunclient::Error),
    #[error("Send error: {0}")]
    Send(#[from] SendError),
    #[error("Receive error: {0}")]
    Receive(#[from] ReceiveError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install().ok();

    let args = Args::parse();
    let subscriber = tracing_subscriber::fmt::Subscriber::builder()
        .with_max_level(args.log_level)
        .without_time()
        .with_thread_ids(false)
        .with_target(false)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    // Check if the files even exist
    if let Mode::Send { files, .. } = &args.mode {
        for file in files {
            if !file.exists() {
                return Err(SendError::FileDoesNotExist(file.clone()).into());
            }
        }
    }

    let socket = UdpSocket::bind(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0))?;

    let external_addr = utils::external_addr(
        &socket,
        STUN_SERVER
            .to_socket_addrs()?
            .find(|x| x.is_ipv4())
            .unwrap(),
    )?;

    println!("External address: {}", external_addr.green());

    let other =
        dialoguer::Input::<SocketAddr>::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt("Enter the remote address")
            .interact()
            .unwrap();

    hole_punch(&socket, other)?;

    match args.mode {
        Mode::Send { files } => {
            let args = SenderArgs { files };
            sender::send_files(socket, other, args).await?;
        }
        Mode::Receive { resume } => {
            let args = ReceiverArgs { resume };
            receiver::receive_files(socket, other, args).await?;
        }
    };

    Ok(())
}
