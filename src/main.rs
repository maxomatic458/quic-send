use std::{
    net::{Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket},
    path::PathBuf,
};

use crate::{
    client::{SendError, SenderArgs},
    server::{ReceiverArgs, SaveMode},
    utils::hole_punch,
};
use clap::{Parser, Subcommand};
use color_eyre::owo_colors::OwoColorize;
use server::ReceiveError;
use thiserror::Error;
use utils::LogLevel;

pub mod client;
pub mod common;
pub mod packets;
pub mod server;
pub mod utils;

pub const SERVER_NAME: &str = "server";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const STUN_SERVER: &str = "stun.l.google.com:19302";
pub const KEEP_ALIVE_INTERVAL_SECS: u64 = 5;
pub const FILE_BUF_SIZE: usize = 8192;
pub const HASH_BUF_SIZE: usize = 8192 * 6;
// pub const MAX_HOLEPUNCH_TRIES: u64 = 5;

#[derive(Parser, Debug)]
#[clap(version = VERSION, author = env!("CARGO_PKG_AUTHORS"))]
struct Args {
    /// Log level
    #[clap(long, short, default_value = "info")]
    log_level: LogLevel,
    /// Send or receive files
    #[clap(subcommand)]
    pub mode: Mode,
}

#[derive(Subcommand, Debug)]
enum Mode {
    #[clap(name = "send", about = "Send files")]
    Send {
        /// Use file checksums to verify the integrity of the files,
        /// this takes some time on the sender side at the start
        /// and might reduce the overall transfer speed
        #[clap(long, short)]
        checksums: bool,
        /// Files/directories to send
        #[clap(name = "files or directories", required = true)]
        files: Vec<PathBuf>,
    },
    #[clap(name = "receive", about = "Receive files")]
    Receive {
        /// Overwrite files if they already exist
        #[clap(long, short, conflicts_with = "resume")]
        overwrite: bool,
        /// Append to files if they already exist (if a previous transfer was interrupted)
        #[clap(long, short, conflicts_with = "overwrite")]
        resume: bool,
        /// Ask for every file which is already present whether to overwrite or append
        #[clap(long, short, conflicts_with = "overwrite", conflicts_with = "resume")]
        per_file: bool,
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
        .with_max_level(args.log_level.to_tracing_level())
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
        Mode::Send { checksums, files } => {
            let args = SenderArgs { checksums, files };
            client::send_files(socket, other, args).await?;
        }
        Mode::Receive {
            overwrite,
            resume,
            per_file,
        } => {
            let args = ReceiverArgs {
                save_mode: SaveMode::from_flags(overwrite, resume, per_file),
            };
            server::receive_files(socket, other, args).await?;
            #[cfg(feature = "toast-notifications")]
            {
                use utils::notify;
                notify("quic-send", "Transfer complete");
            }
        }
    };

    Ok(())
}
