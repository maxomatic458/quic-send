use std::{
    net::{Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket},
    path::PathBuf,
};

use clap::{Parser, Subcommand};
use color_eyre::owo_colors::OwoColorize;

use crate::utils::hole_punch;

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
// pub const MAX_HOLEPUNCH_TRIES: u64 = 5;

#[derive(Parser, Debug)]
#[clap(version = VERSION, author = env!("CARGO_PKG_AUTHORS"))]
struct Args {
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
    Receive,
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let subscriber = tracing_subscriber::fmt::Subscriber::builder()
        .with_max_level(tracing::Level::WARN)
        .without_time()
        .with_thread_ids(false)
        .with_target(false)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let args = Args::parse();

    // Check if the files even exist
    if let Mode::Send { files } = &args.mode {
        for file in files {
            if !file.exists() {
                eprintln!(
                    "File or directory does not exist: {}",
                    file.display().to_string().red()
                );
                std::process::exit(1);
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
            .interact()?;

    hole_punch(&socket, other)?;

    match args.mode {
        Mode::Send { files } => {
            client::send_files(&files, socket, other).await?;
        }
        Mode::Receive => {
            server::receive_files(socket, other).await?;
        }
    }

    Ok(())
}
