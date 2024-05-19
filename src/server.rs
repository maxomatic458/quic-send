use async_compression::tokio::bufread::GzipDecoder;
use blake3::Hasher;
use color_eyre::owo_colors::OwoColorize;
use core::time;
use std::{
    net::{SocketAddr, UdpSocket},
    path::Path,
    sync::Arc,
};

use indicatif::{HumanBytes, ProgressBar};
use quinn::{
    default_runtime, Connection, Endpoint, EndpointConfig, RecvStream, SendStream, ServerConfig,
    VarInt,
};
use std::io;
use thiserror::Error;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
};

use crate::{
    common::{handle_unexpected_packet, receive_packet, send_packet, Blake3Hash, FileOrDir},
    packets::{ClientPacket, ServerPacket},
    utils::{progress_bars, self_signed_cert, update_hasher},
    FILE_BUF_SIZE, KEEP_ALIVE_INTERVAL_SECS, VERSION,
};

#[derive(Error, Debug)]
pub enum ReceiveError {
    #[error("IO error: {0}")]
    IoError(#[from] io::Error),
    #[error("Connection error: {0}")]
    ConnectionError(#[from] quinn::ConnectionError),
    #[error("Write error: {0}")]
    WriteError(#[from] quinn::WriteError),
    #[error("Version mismatch")]
    VersionMismatch,
    #[error("Unexpected packet: {0:?}")]
    UnexpectedPacket(ClientPacket),
    #[error("Could not verify integrity of file: {0}")]
    CouldNotVerifyIntegrity(String),
}

pub struct Receiver {
    /// Arguments
    pub args: ReceiverArgs,
    /// The channel to send packets
    pub send: SendStream,
    /// The channel to receive packets
    pub recv: RecvStream,
    /// The connection
    pub conn: Connection,
    /// Server endpoint
    pub server: Endpoint,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SaveMode {
    /// If the file already exists, skip it
    SkipIfNotExists,
    /// Overwrite files
    Overwrite,
    /// Resume a download (will skip if the senders file size >= the receivers file size)
    Resume,
    /// Ask for each file
    PerFile,
}

impl SaveMode {
    pub fn from_flags(overwrite: bool, append: bool, per_file: bool) -> Self {
        match (overwrite, append, per_file) {
            (true, false, false) => Self::Overwrite,
            (false, true, false) => Self::Resume,
            (false, false, true) => Self::PerFile,
            _ => Self::SkipIfNotExists,
        }
    }
}

pub struct ReceiverArgs {
    pub save_mode: SaveMode,
}

impl Receiver {
    pub async fn connect(
        socket: UdpSocket,
        _sender: SocketAddr,
        args: ReceiverArgs,
    ) -> Result<Self, ReceiveError> {
        let rt = default_runtime()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no async runtime found"))?;

        let server = Endpoint::new(EndpointConfig::default(), Some(server_config()), socket, rt)?;
        if let Some(conn) = server.accept().await {
            let conn = conn.await?;
            tracing::debug!(
                "Server accepted connection from: {:?}",
                conn.remote_address()
            );

            let (mut send, mut recv) = conn.open_bi().await?;
            send.write_u8(1).await?; // Open the stream

            let packet = receive_packet::<ClientPacket>(&mut recv).await?;
            match packet {
                ClientPacket::ConnRequest { version_num } => {
                    if version_num != VERSION {
                        send_packet(
                            ServerPacket::WrongVersion {
                                expected: VERSION.to_string(),
                            },
                            &mut send,
                        )
                        .await?;
                        return Err(ReceiveError::VersionMismatch);
                    }
                }
                p => {
                    handle_unexpected_packet(&p);
                    return Err(ReceiveError::UnexpectedPacket(p));
                }
            }

            send_packet(ServerPacket::Ok, &mut send).await?;

            return Ok(Self {
                args,
                send,
                recv,
                conn,
                server,
            });
        }

        Err(ReceiveError::IoError(io::Error::new(
            io::ErrorKind::Other,
            "no connection found",
        )))
    }

    pub async fn close(&mut self) -> Result<(), ReceiveError> {
        self.send.finish().await.ok();
        self.conn.close(VarInt::from_u32(0), b"");
        Ok(())
    }

    pub(crate) async fn receive_file_meta(&mut self) -> Result<Vec<FileOrDir>, ReceiveError> {
        let packet = receive_packet::<ClientPacket>(&mut self.recv).await?;
        match packet {
            ClientPacket::FileMeta { files } => Ok(files),
            p => {
                handle_unexpected_packet(&p);
                Err(ReceiveError::UnexpectedPacket(p))
            }
        }
    }

    fn accept_files(&self, file_meta: &[FileOrDir]) -> bool {
        println!("The following files will be received:");

        let longest_name = file_meta.iter().map(|f| f.name().len()).max().unwrap_or(0);
        let total_size = file_meta.iter().map(FileOrDir::size).sum::<u64>();

        for file in file_meta {
            let size = file.size();
            let size_human_bytes = format!("({})", HumanBytes(size)).red().to_string();
            let name = file.name().to_string();

            let name = if file.is_dir() {
                // adjust for width of directory indicator
                let name = format!("{}/", name);
                format!("{:width$}", name, width = longest_name)
                    .cyan()
                    .to_string()
            } else {
                format!("{:width$}", name, width = longest_name + 1)
                    .yellow()
                    .to_string()
            };

            let hash = if let FileOrDir::File {
                hash: Some(hash), ..
            } = file
            {
                hex::encode(hash)
            } else {
                String::new()
            };

            println!(
                "{:width$} {} {}",
                name,
                size_human_bytes,
                hash,
                width = longest_name
            );
        }

        let total_size_human_bytes = format!("({})", HumanBytes(total_size)).red().to_string();
        let prompt = format!(
            "\nDo you want to accept these files? {}",
            total_size_human_bytes
        );

        dialoguer::Confirm::new()
            .with_prompt(prompt)
            .interact()
            .unwrap()
    }

    async fn download_files(&mut self, file_meta: &[FileOrDir]) -> Result<(), ReceiveError> {
        let mut recv = self.conn.accept_uni().await?;
        recv.read_u8().await?; // Read opening byte
        tracing::debug!("Accepted file stream");
        let mut recv = GzipDecoder::new(tokio::io::BufReader::with_capacity(FILE_BUF_SIZE, recv));

        let (bars, total_bar) = progress_bars(file_meta);

        for (file, bar) in file_meta.iter().zip(bars.iter()) {
            tracing::debug!("Downloading file: {:?}", file.name());
            match file {
                FileOrDir::File { name, size, hash } => {
                    self.download_single_file(
                        Path::new(name),
                        *size,
                        &mut recv,
                        bar,
                        total_bar.as_ref(),
                        *hash,
                    )
                    .await?;
                    bar.finish();
                }
                FileOrDir::Dir { name, sub } => {
                    self.download_directory(
                        Path::new(name),
                        sub.clone(),
                        &mut recv,
                        bar,
                        total_bar.as_ref(),
                    )
                    .await?;
                    bar.finish();
                }
            }
        }

        Ok(())
    }

    /// Either
    /// - (default) skip a file if it already exists
    /// - ``-a`` append to a file if a download was interrupted
    /// - ``-o`` overwrite a file
    /// - ``-p`` ask for each file
    ///
    /// [``crate::client::Sender::handle_receiver_save_mode``] is the client counterpart
    async fn handle_save_mode(
        &mut self,
        path: &Path,
        hasher: Option<&mut Hasher>,
        mode: SaveMode,
    ) -> Result<(Option<File>, u64), ReceiveError> {
        let mut bytes_written = 0;
        let mode = if mode == SaveMode::PerFile {
            // if the file doesnt exist there is nothing to do
            if !path.exists() {
                SaveMode::SkipIfNotExists
            } else {
                let items = &["Skip", "Resume", "Overwrite"];
                let selection =
                    dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
                        .with_prompt(format!(
                            "File: {:?} already exists, what do you want to do?",
                            path
                        ))
                        .items(items)
                        .default(0)
                        .interact()
                        .unwrap();

                match selection {
                    0 => {
                        tracing::debug!("Skipping file: {:?}", path);
                        send_packet(ServerPacket::SkipFile, &mut self.send).await?;
                        return Ok((None, 0));
                    }
                    1 => SaveMode::Resume,
                    2 => SaveMode::Overwrite,
                    _ => unreachable!(),
                }
            }
        } else {
            mode
        };

        let file = match mode {
            SaveMode::SkipIfNotExists => {
                if path.exists() {
                    tracing::debug!("Skipping file: {:?}", path);
                    send_packet(ServerPacket::SkipFile, &mut self.send).await?;
                    return Ok((None, 0));
                }
                send_packet(ServerPacket::Ok, &mut self.send).await?;
                Some(File::create(path).await?)
            }
            SaveMode::Overwrite => {
                tracing::debug!("Overwriting file: {:?}", path);
                send_packet(ServerPacket::Ok, &mut self.send).await?;
                Some(File::create(path).await?)
            }
            SaveMode::Resume => {
                tracing::debug!("Appending to file: {:?}", path);
                // first get the length of the file
                let mut file = tokio::fs::OpenOptions::new()
                    .append(true)
                    .open(path)
                    .await?;

                let hash: Option<Blake3Hash> = if let Some(hasher) = hasher {
                    update_hasher(hasher, &mut file).await?;
                    Some(hasher.clone().finalize().into())
                } else {
                    None
                };

                let size = file.metadata().await?.len();
                bytes_written = size;
                send_packet(ServerPacket::FileFromPos { pos: size }, &mut self.send).await?;

                let resp = receive_packet::<ClientPacket>(&mut self.recv).await?;
                match resp {
                    ClientPacket::FilePosHash { hash: client_hash } => {
                        if client_hash != hash {
                            return Err(ReceiveError::CouldNotVerifyIntegrity(
                                path.display().to_string(),
                            ));
                        }
                    }
                    p => {
                        handle_unexpected_packet(&p);
                        return Err(ReceiveError::UnexpectedPacket(p));
                    }
                }

                Some(file)
            }
            SaveMode::PerFile => unreachable!(),
        };

        Ok((file, bytes_written))
    }

    /// Download a single file
    pub(crate) async fn download_single_file(
        &mut self,
        file_path: &Path, // Output path of the file
        size: u64,        // Expected size of the file
        recv: &mut GzipDecoder<BufReader<RecvStream>>,
        bar: &ProgressBar, // Progress bar of the file or of the parent directory
        total_bar: Option<&ProgressBar>, // Total progress barw
        hash: Option<Blake3Hash>, // Expected hash of the file
    ) -> Result<(), ReceiveError> {
        tracing::debug!("Downloading file: {:?} with size {}", file_path, size);

        let mut hasher = if hash.is_some() {
            Some(blake3::Hasher::new())
        } else {
            None
        };

        let (mut file, mut bytes_written) = if let (Some(file), bytes_written) = self
            .handle_save_mode(file_path, hasher.as_mut(), self.args.save_mode)
            .await?
        {
            (file, bytes_written)
        } else {
            return Ok(());
        };

        let mut buf = vec![0; FILE_BUF_SIZE];

        bar.inc(bytes_written);

        while bytes_written < size {
            let to_read = std::cmp::min(FILE_BUF_SIZE as u64, size - bytes_written) as usize;
            let n = recv.read(&mut buf[..to_read]).await?;

            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected EOF").into());
            }

            file.write_all(&buf[..n]).await.unwrap();
            bytes_written += n as u64;

            if let Some(hasher) = hasher.as_mut() {
                hasher.update(&buf[..n]);
            }

            bar.inc(n as u64);
            if let Some(total_bar) = total_bar {
                total_bar.inc(n as u64);
            }
        }

        if let Some(expected) = hash {
            let hash: Blake3Hash = hasher.unwrap().finalize().into();
            if expected != hash {
                return Err(ReceiveError::CouldNotVerifyIntegrity(
                    file_path.display().to_string(),
                ));
            }
            tracing::debug!("File integrity verified: {:?}", file_path);
        }

        file.shutdown().await.unwrap();

        tracing::debug!("Finished downloading file: {:?}", file_path);

        Ok(())
    }

    #[async_recursion::async_recursion]
    async fn download_directory(
        &mut self,
        dir: &Path,
        sub: Vec<FileOrDir>,
        recv: &mut GzipDecoder<BufReader<RecvStream>>,
        bar: &ProgressBar,
        total_bar: Option<&ProgressBar>,
    ) -> Result<(), ReceiveError> {
        tracing::debug!("Downloading directory: {:?}", dir);
        tokio::fs::create_dir(dir).await?;

        for sub in sub {
            let path = dir.join(sub.name());
            match sub {
                FileOrDir::File { size, hash, .. } => {
                    self.download_single_file(&path, size, recv, bar, total_bar, hash)
                        .await?;
                }
                FileOrDir::Dir { sub, .. } => {
                    self.download_directory(&path, sub, recv, bar, total_bar)
                        .await?;
                }
            }
        }

        tracing::debug!("Finished downloading directory: {:?}", dir);
        Ok(())
    }
}

pub async fn receive_files(
    socket: UdpSocket,
    _sender: SocketAddr,
    args: ReceiverArgs,
) -> Result<(), ReceiveError> {
    let mut server = Receiver::connect(socket, _sender, args).await?;

    let file_meta = server.receive_file_meta().await?;
    if server.accept_files(&file_meta) {
        send_packet(ServerPacket::AcceptFiles, &mut server.send).await?;
        server.download_files(&file_meta).await?;
    } else {
        send_packet(ServerPacket::RejectFiles, &mut server.send).await?;
    }

    server.close().await?;

    Ok(())
}

fn server_config() -> ServerConfig {
    let (cert, key) = self_signed_cert().expect("failed to generate self signed cert");

    let mut transport_config = quinn::TransportConfig::default();
    transport_config.keep_alive_interval(Some(time::Duration::from_secs(KEEP_ALIVE_INTERVAL_SECS)));

    ServerConfig::with_single_cert(vec![cert], key)
        .unwrap()
        .transport_config(Arc::new(transport_config))
        .to_owned()
}
