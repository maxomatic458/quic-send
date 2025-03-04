use std::{
    path::PathBuf,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, AtomicU64},
        mpsc, Arc,
    },
    time::Duration,
};

use base64::{prelude::BASE64_STANDARD_NO_PAD, Engine};
use iroh::{Endpoint, RelayMode, SecretKey};
use qs_core::{
    common::FilesAvailable,
    receive::{Receiver, ReceiverArgs},
    send::{Sender, SenderArgs},
    QS_ALPN,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Listener};

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

const INITIAL_PROGRESS_EVENT: &str = "initial-progress";
const FILES_OFFERED_EVENT: &str = "files-offered";
const CONNECTED_WITH_CONN_TYPE: &str = "receiver-connected";
const CANCEL_TRANSFER_EVENT: &str = "cancel-transfer";
const FILES_DECISION_EVENT: &str = "files-decision";
const TRANSFER_CANCELLED_EVENT: &str = "transfer-cancelled";
const TRANSFER_FINISHED_EVENT: &str = "transfer-finished";
const TICKET_EVENT: &str = "server-connection-code";
const ACCEPT_FILES_EVENT: &str = "accept-files";
const CONNECTED_TO_SERVER_EVENT: &str = "connected-to-server";

#[derive(Clone, Serialize)]
struct InitialDownloadProgress {
    /// Filename, current, total
    data: Vec<(String, u64, u64)>,
}

#[derive(Clone, Serialize)]
struct FilesOffered {
    /// Filename, size in bytes
    files: Vec<(String, u64, bool)>,
}

#[tauri::command]
fn exit(handle: AppHandle, code: i32) {
    tracing::info!("exiting with code {}", code);
    handle.exit(code);
}

// Use an atomic to keep track of the bytes transferred
// So we can poll it from the frontend.
// This seems to be faster than using tauri events
lazy_static::lazy_static! {
    static ref BYTES_TRANSFERRED: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
}

#[tauri::command(async)]
async fn bytes_transferred() -> u64 {
    BYTES_TRANSFERRED.load(std::sync::atomic::Ordering::Relaxed)
}

/// # Returns
/// * `Ok(true)` if the download was successful
/// * `Ok(false)` if the download was cancelled (by the user)
#[tauri::command(async)]
async fn download_files(window: tauri::Window, ticket: String) -> Result<bool, String> {
    BYTES_TRANSFERRED.store(0, std::sync::atomic::Ordering::Relaxed);

    let secret_key = SecretKey::generate(rand::rngs::OsRng);

    let endpoint = Endpoint::builder()
        .secret_key(secret_key)
        .alpns(vec![QS_ALPN.to_vec()])
        .relay_mode(RelayMode::Default)
        .bind()
        .await
        .map_err(|_| "failed to iroh bind endpoint".to_string())?;

    let node_addr = BASE64_STANDARD_NO_PAD
        .decode(ticket.as_bytes())
        .map_err(|_| "failed to decode ticket".to_string())?;

    let node_addr: iroh::NodeAddr =
        bincode::serde::decode_from_slice(&node_addr, bincode::config::standard())
            .map_err(|_| "invalid ticket".to_string())?
            .0;

    let receiver_args = ReceiverArgs { resume: true };
    let mut receiver = Receiver::connect(endpoint, node_addr, receiver_args)
        .await
        .map_err(|e| format!("failed to connect to sender: {}", e))?;

    window.emit(CONNECTED_TO_SERVER_EVENT, ()).unwrap();

    std::thread::sleep(Duration::from_secs(4));
    let conn_type = receiver.connection_type().await;
    window.emit(CONNECTED_WITH_CONN_TYPE, conn_type).unwrap();

    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupted_clone = interrupted.clone();

    window.listen(CANCEL_TRANSFER_EVENT, move |_| {
        interrupted_clone.store(true, std::sync::atomic::Ordering::Relaxed);
    });

    receiver
        .receive_files(
            |files| {
                std::thread::sleep(Duration::from_millis(100));
                window
                    .emit(
                        INITIAL_PROGRESS_EVENT,
                        InitialDownloadProgress {
                            data: files.iter().map(|f| (f.0.clone(), f.1, f.2)).collect(),
                        },
                    )
                    .unwrap();
            },
            |files_offered| {
                let offered: Vec<(String, u64, bool)> = files_offered
                    .iter()
                    .map(|f| {
                        (
                            f.name().to_string(),
                            f.size(),
                            matches!(f, FilesAvailable::Dir { .. }),
                        )
                    })
                    .collect();
                window
                    .emit(FILES_OFFERED_EVENT, FilesOffered { files: offered })
                    .unwrap();

                let (tx, rx) = mpsc::channel();

                window.listen(ACCEPT_FILES_EVENT, move |event| {
                    let path_result = if event.payload() != "\"\"" {
                        let path_string = event.payload();
                        // Remove the quotes that wrap the path
                        let path_string = &path_string[1..path_string.len() - 1];

                        let path = PathBuf::from_str(path_string)
                            .expect("Failed to parse path from event payload");
                        Some(path)
                    } else {
                        None
                    };

                    let _ = tx.send(path_result);
                });

                rx.recv()
                    .expect("Failed to receive file acceptance decision")
            },
            &mut |bytes_read| {
                BYTES_TRANSFERRED.fetch_add(bytes_read, std::sync::atomic::Ordering::Relaxed);
            },
            &mut || !interrupted.load(std::sync::atomic::Ordering::Relaxed),
        )
        .await
        .map_err(|e| format!("failed to receive files: {}", e))?;

    let was_interrupted = interrupted.load(std::sync::atomic::Ordering::Relaxed);

    if was_interrupted {
        window.emit(TRANSFER_CANCELLED_EVENT, ()).unwrap();
    } else {
        window.emit(TRANSFER_FINISHED_EVENT, ()).unwrap();
    }

    BYTES_TRANSFERRED.store(0, std::sync::atomic::Ordering::Relaxed);
    Ok(!was_interrupted)
}

#[derive(Serialize, Debug)]
enum UploadResult {
    /// The files were successfully uploaded
    Success,
    /// The file transfer was cancelled by the sender
    Cancelled,
    /// The files were rejected by the receiver
    Rejected,
}

#[tauri::command(async)]
async fn upload_files(window: tauri::Window, files: Vec<PathBuf>) -> Result<UploadResult, String> {
    BYTES_TRANSFERRED.store(0, std::sync::atomic::Ordering::Relaxed);

    let secret_key = SecretKey::generate(rand::rngs::OsRng);

    let endpoint = Endpoint::builder()
        .secret_key(secret_key)
        .alpns(vec![QS_ALPN.to_vec()])
        .relay_mode(RelayMode::Default)
        .bind()
        .await
        .map_err(|_| "failed to iroh bind endpoint".to_string())?;

    let node_addr = endpoint
        .node_addr()
        .await
        .map_err(|e| format!("failed to get node address: {}", e))?;

    let serialized = bincode::serde::encode_to_vec(node_addr, bincode::config::standard()).unwrap();
    let ticket: String = BASE64_STANDARD_NO_PAD.encode(&serialized);

    window.emit(TICKET_EVENT, ticket).unwrap();

    let mut sender = Sender::connect(endpoint, SenderArgs { files })
        .await
        .map_err(|e| format!("failed to connect to receiver: {}", e))?;

    window.emit(CONNECTED_TO_SERVER_EVENT, ()).unwrap();

    std::thread::sleep(Duration::from_secs(4));
    let conn_type = sender.connection_type().await;
    window.emit(CONNECTED_WITH_CONN_TYPE, conn_type).unwrap();

    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupted_clone = interrupted.clone();

    let mut rejected = false;

    window.listen(CANCEL_TRANSFER_EVENT, move |_| {
        interrupted_clone.store(true, std::sync::atomic::Ordering::Relaxed);
    });

    sender
        .send_files(
            || {},
            |accepted| {
                println!("FILES DECISION: {:?}", accepted);
                window.emit(FILES_DECISION_EVENT, accepted).unwrap();
                rejected = !accepted;
            },
            |initial_bytes_sent| {
                std::thread::sleep(Duration::from_millis(100));
                window
                    .emit(
                        INITIAL_PROGRESS_EVENT,
                        InitialDownloadProgress {
                            data: initial_bytes_sent
                                .iter()
                                .map(|f| (f.0.clone(), f.1, f.2))
                                .collect(),
                        },
                    )
                    .unwrap();
            },
            &mut |bytes_sent| {
                BYTES_TRANSFERRED.fetch_add(bytes_sent, std::sync::atomic::Ordering::Relaxed);
            },
            &mut || !interrupted.load(std::sync::atomic::Ordering::Relaxed),
        )
        .await
        .map_err(|e| format!("failed to send files: {}", e))?;

    let was_interrupted = interrupted.load(std::sync::atomic::Ordering::Relaxed);

    if rejected {
        return Ok(UploadResult::Rejected);
    }

    if was_interrupted {
        window.emit(TRANSFER_CANCELLED_EVENT, ()).unwrap();
        return Ok(UploadResult::Cancelled);
    }

    window.emit(TRANSFER_FINISHED_EVENT, ()).unwrap();
    BYTES_TRANSFERRED.store(0, std::sync::atomic::Ordering::Relaxed);
    Ok(UploadResult::Success)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileInfo {
    size_bytes: u64,
    is_directory: bool,
}

#[tauri::command(async)]
async fn file_info(path: PathBuf) -> Result<FileInfo, String> {
    let mut size = 0;

    let is_directory = path.is_dir();
    if is_directory {
        for entry in walkdir::WalkDir::new(&path) {
            let entry = entry.map_err(|e| format!("failed to read directory: {}", e))?;
            size += entry
                .metadata()
                .map_err(|e| format!("failed to read metadata: {}", e))?
                .len();
        }
    } else {
        size = path
            .metadata()
            .map_err(|e| format!("failed to read metadata: {}", e))?
            .len();
    }

    Ok(FileInfo {
        size_bytes: size,
        is_directory,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    std::env::set_var("RUST_LOG", "debug");
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            exit,
            bytes_transferred,
            download_files,
            file_info,
            upload_files,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
