use clap::{Parser, Subcommand};
use indicatif::{HumanBytes, MultiProgress, ProgressBar, ProgressStyle};
use qs_core::{
    common::FilesAvailable,
    receive::{roundezvous_connect, ReceiveError, Receiver, ReceiverArgs},
    send::{roundezvous_announce, SendError, Sender, SenderArgs},
    utils, QuicSendError, CODE_LEN, STUN_SERVER, VERSION,
};
use std::{
    cell::RefCell,
    net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket},
    path::PathBuf,
    rc::Rc,
};
use thiserror::Error;

// const DEFAULT_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 178, 47)), 9090);
const DEFAULT_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(209, 25, 141, 1)), 1182);

#[derive(Parser, Debug)]
#[clap(version = VERSION, author = env!("CARGO_PKG_AUTHORS"))]
struct Args {
    /// Log level
    #[clap(long, short, default_value = "info")]
    log_level: tracing::Level,
    /// Direct mode (no rendezvous server)
    #[clap(long, short, default_value = "false", conflicts_with = "server_addr")]
    pub direct: bool,
    /// Send or receive files
    #[clap(subcommand)]
    pub mode: Mode,
    /// override the default roundezvous server address, incompatible with direct mode
    #[clap(long, short, conflicts_with = "direct", default_value_t = DEFAULT_ADDR)]
    pub server_addr: SocketAddr,
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
        /// Overwrite files instead of resuming
        #[clap(long, short = 'f')]
        overwrite: bool,

        /// Custom output directory
        #[clap(long, short, default_value = ".")]
        output: PathBuf,

        /// The code to connect to the sender
        code: Option<String>,
    },
}

#[derive(Error, Debug)]
enum AppError {
    #[error("send error: {0}")]
    QuicSendCore(#[from] qs_core::QuicSendError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let args: Args = Args::parse();
    tracing_subscriber::fmt()
        .with_max_level(args.log_level)
        .init();

    // Check if the files even exist
    if let Mode::Send { files, .. } = &args.mode {
        for file in files {
            if !file.exists() {
                return Err(QuicSendError::Send(SendError::FileDoesNotExists(file.clone())).into());
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
    )
    .map_err(QuicSendError::Stun)?;

    let other;

    if args.direct {
        println!("External address: {}", external_addr);

        other =
            dialoguer::Input::<SocketAddr>::with_theme(&dialoguer::theme::ColorfulTheme::default())
                .with_prompt("Enter the remote address")
                .interact()
                .unwrap();
    } else {
        let socket_clone = socket.try_clone().unwrap();
        match args.mode {
            Mode::Send { .. } => {
                other = roundezvous_announce(socket_clone, external_addr, args.server_addr, |c| {
                    let code = String::from_utf8(c.to_vec()).unwrap();
                    println!("Code: {}\n", code);
                    println!("on the other peer, run the following command:\n");
                    println!(
                        "qs {} receive {}",
                        if args.server_addr != DEFAULT_ADDR {
                            format!("-s {}", args.server_addr)
                        } else {
                            "".to_string()
                        },
                        code
                    );
                })
                .await
                .map_err(QuicSendError::Send)?;
            }
            Mode::Receive { ref code, .. } => {
                let code = code.clone().unwrap_or_else(|| {
                    dialoguer::Input::<String>::with_theme(
                        &dialoguer::theme::ColorfulTheme::default(),
                    )
                    .with_prompt("Enter the code")
                    .interact()
                    .unwrap()
                });

                let code: [u8; CODE_LEN] = match code.as_bytes().try_into() {
                    Ok(c) => c,
                    Err(_) => return Err(QuicSendError::Receive(ReceiveError::InvalidCode).into()),
                };

                other = roundezvous_connect(socket_clone, external_addr, args.server_addr, code)
                    .await
                    .map_err(QuicSendError::Receive)?;
            }
        }
    }

    utils::hole_punch(&socket, other)?;

    let progress_bars: Rc<RefCell<Option<CliProgressBars>>> = Rc::new(RefCell::new(None));
    let rc_clone = Rc::clone(&progress_bars);

    match args.mode {
        Mode::Send { files } => {
            let mut sender = Sender::connect(socket, other, SenderArgs { files })
                .await
                .map_err(QuicSendError::Send)?;

            sender
                .send_files(
                    || {
                        println!("waiting for the other peer to accept the files...");
                    },
                    |initial_progress| {
                        *rc_clone.borrow_mut() = Some(CliProgressBars::new(initial_progress));
                    },
                    &mut |last_sent| {
                        if let Some(pb) = &mut *rc_clone.borrow_mut() {
                            pb.update(last_sent);
                        }
                    },
                )
                .await
                .map_err(QuicSendError::Send)?;
        }
        Mode::Receive {
            overwrite, output, ..
        } => {
            let mut receiver = Receiver::connect(
                socket,
                other,
                ReceiverArgs {
                    resume: !overwrite,
                    output_path: output,
                },
            )
            .await
            .map_err(QuicSendError::Receive)?;

            receiver
                .receive_files(
                    |initial_progress| {
                        *progress_bars.borrow_mut() = Some(CliProgressBars::new(initial_progress));
                    },
                    accept_files,
                    &mut |last_received| {
                        if let Some(pb) = &mut *progress_bars.borrow_mut() {
                            pb.update(last_received);
                        }
                    },
                )
                .await
                .map_err(QuicSendError::Receive)?;
        }
    }

    Ok(())
}

/// Ask the receiver if they want to accept the files
fn accept_files(files_offered: &[FilesAvailable]) -> bool {
    println!("The following files will be received:");

    let longest_name = files_offered
        .iter()
        .map(|f| f.name().len())
        .max()
        .unwrap_or(0);

    let total_size = files_offered.iter().map(|f| f.size()).sum::<u64>();

    for file in files_offered {
        let size = file.size();
        let size_human_bytes = HumanBytes(size).to_string();
        let name = file.name();

        println!(
            " - {:<width$} {:>10}",
            name,
            size_human_bytes,
            width = longest_name
        );
    }

    println!("\nTotal size: {}", HumanBytes(total_size));

    dialoguer::Confirm::new()
        .with_prompt("Do you want to receive these files?")
        .interact()
        .unwrap_or_default()
}

/// Send and receive progress bars
struct CliProgressBars {
    /// Per file/dir progress bars
    progerss_bars: Vec<ProgressBar>,
    /// Only used when multiple files are sent
    total_bar: Option<ProgressBar>,
}

impl CliProgressBars {
    /// Creates the progress bars using the callbacks in
    /// [qs_core::send::Sender::send_files] and [qs_core::receive::Receiver::receive_files]
    /// # Arguments
    /// * `callback_data` - The initial progress data for each file (name, current, total)
    fn new(callback_data: &[(String, u64, u64)]) -> Self {
        let total_name = "Total";

        let style = ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("#>-");

        let total_style = ProgressStyle::default_bar()
            .template(
                "{spinner:.green} {prefix} [{bar:40.yellow/yellow}] {bytes}/{total_bytes} ({eta})",
            )
            .unwrap()
            .progress_chars("#>-");

        let (mut longest_name, total_progress, total_size) = callback_data.iter().fold(
            (0, 0, 0),
            |(longest_name, total_progress, total_size), (name, progress, size)| {
                (
                    longest_name.max(name.len()),
                    total_progress + progress,
                    total_size + size,
                )
            },
        );

        longest_name = longest_name.max(total_name.len());

        let mp = MultiProgress::new();

        let mut bars = Vec::new();
        for (name, progress, size) in callback_data {
            let pb = mp.add(ProgressBar::new(*size));
            pb.set_prefix(format!("{:<width$}", name, width = longest_name));
            pb.set_style(style.clone());
            pb.set_position(*progress);
            pb.reset_eta();
            bars.push(pb);
        }

        let total_bar = if bars.len() > 1 {
            let pb = mp.add(ProgressBar::new(total_size));
            pb.set_prefix(format!("{:<width$}", total_name, width = longest_name));
            pb.set_style(total_style);
            pb.set_position(total_progress);
            pb.reset_eta();
            Some(pb)
        } else {
            None
        };

        Self {
            progerss_bars: bars,
            total_bar,
        }
    }

    /// Update the progress bars
    /// it is expected that the files will be downloaded in order
    pub fn update(&mut self, mut progress: u64) {
        if let Some(pb) = &self.total_bar {
            pb.inc(progress);
        }

        for pb in &self.progerss_bars {
            let remaining = pb
                .length()
                .unwrap_or_default()
                .saturating_sub(pb.position());

            let this_bar_progress = progress.min(remaining);
            pb.inc(this_bar_progress);
            progress -= this_bar_progress;

            if progress == 0 {
                break;
            }
        }
    }
}
