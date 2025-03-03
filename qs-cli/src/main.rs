use base64::{prelude::BASE64_STANDARD_NO_PAD, Engine};
use clap::{Parser, Subcommand};
use colored::Colorize;
use copypasta::{ClipboardContext, ClipboardProvider};
use dialoguer::theme::ColorfulTheme;
use indicatif::{HumanBytes, MultiProgress, ProgressBar, ProgressStyle};
use iroh::{Endpoint, RelayMode, SecretKey};
use qs_core::{
    common::FilesAvailable,
    receive::{ReceiveError, Receiver, ReceiverArgs},
    send::{SendError, Sender, SenderArgs},
    QuicSendError, QS_ALPN, QS_VERSION,
};
use std::{
    cell::RefCell,
    io::{self, Write},
    path::PathBuf,
    rc::Rc,
    str::FromStr,
    time::Duration,
};
use thiserror::Error;
use tracing::Level;

#[derive(Parser, Debug)]
#[clap(version = QS_VERSION, author = env!("CARGO_PKG_AUTHORS"))]
struct Args {
    /// Log level
    #[clap(long, short, default_value = "error")]
    log_level: tracing::Level,
    /// Send or receive files
    #[clap(subcommand)]
    mode: Mode,
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

        /// Automatically accept the files
        #[clap(long, short = 'y')]
        auto_accept: bool,
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
async fn main() -> color_eyre::Result<()> {
    let args: Args = Args::parse();

    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_max_level(Level::from_str(&args.log_level.to_string()).unwrap())
        .init();

    // Make sure colors work correclty in cmd.exe.
    #[cfg(windows)]
    {
        colored::control::set_virtual_terminal(true).unwrap();
    }

    tracing::debug!("qs {}", QS_VERSION);

    // Check if the files even exist
    if let Mode::Send { files, .. } = &args.mode {
        for file in files {
            if !file.exists() {
                return Err(QuicSendError::Send(SendError::FileDoesNotExists(file.clone())).into());
            }
        }
    }

    let secret_key = SecretKey::generate(rand::rngs::OsRng);

    let endpoint = Endpoint::builder()
        .secret_key(secret_key)
        .alpns(vec![QS_ALPN.to_vec()])
        .relay_mode(RelayMode::Default)
        .bind()
        .await
        .map_err(|e| AppError::QuicSendCore(QuicSendError::Bind(e.to_string())))?;

    let progress_bars: Rc<RefCell<Option<CliProgressBars>>> = Rc::new(RefCell::new(None));
    let rc_clone = Rc::clone(&progress_bars);

    match args.mode {
        Mode::Send { files } => {
            let node_addr = endpoint.node_addr().await.map_err(|e| {
                AppError::QuicSendCore(QuicSendError::Send(SendError::NodeAddr(e.to_string())))
            })?;

            let serialized =
                bincode::serde::encode_to_vec(node_addr, bincode::config::standard()).unwrap();
            let ticket: String = BASE64_STANDARD_NO_PAD.encode(&serialized);

            println!(
                "Ticket (copied to your clipboard):\n\n{}\n",
                ticket.bright_white()
            );
            println!("on the other peer, run the following command:\n");
            println!("{}", "qs receive <ticket>".yellow());

            if let Ok(mut ctx) = ClipboardContext::new() {
                let _ = ctx.set_contents(ticket);
            }

            let sender_args = SenderArgs { files };
            let mut sender = Sender::connect(endpoint, sender_args).await?;

            // Give iroh some time to switch the connection to direct
            std::thread::sleep(Duration::from_secs(4));
            let conn_type = sender.connection_type().await;
            tracing::debug!("connected with type: {:?}", conn_type);
            println!("Connection type: {}", connection_type_info_msg(conn_type));

            sender
                .send_files(
                    || {
                        print!("Waiting for the other peer to accept the files...");
                        io::stdout().flush().unwrap();
                    },
                    |_accepted| {},
                    |initial_progress| {
                        println!("\r{}", " ".repeat(49));
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
            overwrite,
            output,
            code,
            auto_accept,
        } => {
            let ticket = match code {
                Some(code) => code,
                None => dialoguer::Input::new()
                    .with_prompt("Enter the ticket to connect")
                    .interact()?,
            };

            let node_addr = BASE64_STANDARD_NO_PAD
                .decode(ticket.as_bytes())
                .map_err(|_| {
                    AppError::QuicSendCore(QuicSendError::Receive(ReceiveError::InvalidCode))
                })?;

            let node_addr: iroh::NodeAddr =
                bincode::serde::decode_from_slice(&node_addr, bincode::config::standard())
                    .map_err(|_| {
                        AppError::QuicSendCore(QuicSendError::Receive(ReceiveError::InvalidCode))
                    })?
                    .0;

            let receiver_args = ReceiverArgs { resume: !overwrite };
            let mut receiver = Receiver::connect(endpoint, node_addr, receiver_args).await?;

            // Give iroh some time to switch the connection to direct
            std::thread::sleep(Duration::from_secs(4));
            let conn_type = receiver.connection_type().await;
            tracing::debug!("connected with type: {:?}", conn_type);
            println!("Connection type: {}", connection_type_info_msg(conn_type));

            receiver
                .receive_files(
                    |initial_progress| {
                        *progress_bars.borrow_mut() = Some(CliProgressBars::new(initial_progress));
                    },
                    |files_offered| {
                        if auto_accept {
                            println!("auto accepting files");
                            tracing::debug!("auto accepting files");
                            Some(output.clone())
                        } else if accept_files(files_offered) {
                            Some(output.clone())
                        } else {
                            None
                        }
                    },
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
    println!("The following files will be received:\n");

    let longest_name = files_offered
        .iter()
        .map(|f| f.name().len())
        .max()
        .unwrap_or(0)
        + 1;

    let total_size = files_offered.iter().map(|f| f.size()).sum::<u64>();

    for file in files_offered {
        let size = file.size();
        let size_human_bytes = HumanBytes(size).to_string();
        let name = file.name();

        println!(
            " - {:<width$} {:>10}",
            if let FilesAvailable::Dir { .. } = file {
                format!("{}/", name).blue()
            } else {
                format!("{} ", name).blue()
            },
            size_human_bytes.red(),
            width = longest_name
        );
    }

    println!("\nTotal size: {}", HumanBytes(total_size).to_string().red());

    dialoguer::Confirm::with_theme(&ColorfulTheme::default())
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
            .template(
                "{spinner:.green} {prefix} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
            )
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

            if remaining == 0 {
                continue;
            }

            let this_bar_progress = progress.min(remaining);
            pb.inc(this_bar_progress);
            progress -= this_bar_progress;

            if progress == 0 {
                break;
            }
        }
    }
}

fn connection_type_info_msg(connection_type: Option<iroh::endpoint::ConnectionType>) -> String {
    if let Some(conn_type) = connection_type {
        return match conn_type {
            iroh::endpoint::ConnectionType::Direct(_) => "Direct".green().to_string(),
            iroh::endpoint::ConnectionType::Relay(_) => "Relay".red().to_string(),
            iroh::endpoint::ConnectionType::Mixed(_, _) => "Mixed".yellow().to_string(),
            iroh::endpoint::ConnectionType::None => "None".red().to_string(),
        };
    };

    "???".red().to_string()
}
