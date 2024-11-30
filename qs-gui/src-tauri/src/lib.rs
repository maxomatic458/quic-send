use std::{
    net::{Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket},
    path::PathBuf,
    str::FromStr,
    sync::{Arc, RwLock},
};

use qs_core::{
    common::FilesAvailable,
    receive::{roundezvous_connect, Receiver, ReceiverArgs},
    send::{roundezvous_announce, Sender, SenderArgs},
    utils, CODE_LEN, STUN_SERVERS,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Listener};

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn code_len() -> usize {
    CODE_LEN
}
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

// #[tauri::command]
// fn send_notification()

#[tauri::command(async)]
async fn download_files(
    window: tauri::Window,
    code: String,
    server_addr: String,
) -> Result<(), String> {
    let server_addr = server_addr
        .to_socket_addrs()
        .map_err(|e| format!("failed to resolve server address: {}", e))?
        .find(|x| x.is_ipv4())
        .unwrap();

    let socket = UdpSocket::bind(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0))
        .map_err(|e| format!("failed to bind socket: {}", e))?;

    let external_addr = get_external_addr(&socket)?;

    let code: [u8; CODE_LEN] = match code.as_bytes().try_into() {
        Ok(c) => c,
        Err(_) => return Err("invalid code".to_string()),
    };

    let other = roundezvous_connect(
        socket.try_clone().unwrap(),
        external_addr,
        server_addr,
        code,
    )
    .await
    .map_err(|e| format!("failed to connect to server: {}", e))?;

    window.emit("server-connected", ()).unwrap();

    utils::hole_punch(&socket, other).map_err(|e| format!("failed to hole punch: {}", e))?;

    let mut receiver = Receiver::connect(socket, other, ReceiverArgs { resume: true })
        .await
        .map_err(|e| format!("failed to connect to sender: {}", e))?;

    window.emit("receiver-connected", ()).unwrap();

    receiver
        .receive_files(
            |files| {
                window
                    .emit(
                        "initial-download-progress",
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
                    .emit("files-offered", FilesOffered { files: offered })
                    .unwrap();

                let output_path = Arc::new(RwLock::new(None));
                let accepted_clone = output_path.clone();
                window.listen("accept-files", move |event| {
                    if !event.payload().is_empty() {
                        let path_string = event.payload();
                        // for some reason the path here starts and ends with a slash
                        let path_string = &path_string[1..path_string.len() - 1];

                        let path: PathBuf = PathBuf::from_str(path_string).unwrap();
                        *accepted_clone.write().unwrap() = Some(Some(path));
                    } else {
                        *accepted_clone.write().unwrap() = Some(None);
                    };
                });

                while output_path.read().unwrap().is_none() {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }

                let output_path = output_path.write().unwrap().take().unwrap();
                output_path
            },
            &mut |bytes_read| {
                window.emit("bytes-received", bytes_read).unwrap();
            },
        )
        .await
        .map_err(|e| format!("failed to receive files: {}", e))?;

    window.emit("transfer-complete", ()).unwrap();

    Ok(())
}

#[tauri::command(async)]
async fn upload_files(
    window: tauri::Window,
    server_addr: String,
    files: Vec<PathBuf>,
) -> Result<(), String> {
    let server_addr = server_addr
        .to_socket_addrs()
        .map_err(|e| format!("failed to resolve server address: {}", e))?
        .find(|x| x.is_ipv4())
        .unwrap();

    let socket = UdpSocket::bind(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0))
        .map_err(|e| format!("failed to bind socket: {}", e))?;

    let external_addr = get_external_addr(&socket)?;

    let socket_clone = socket.try_clone().unwrap();
    let other = roundezvous_announce(socket_clone, external_addr, server_addr, |c| {
        let code_string = String::from_utf8(c.to_vec()).unwrap();
        window.emit("server-connection-code", code_string).unwrap();
    })
    .await
    .map_err(|e| format!("failed to announce to server: {}", e))?;

    let mut sender = Sender::connect(socket, other, SenderArgs { files })
        .await
        .map_err(|e| format!("failed to connect to receiver: {}", e))?;

    window.emit("receiver-connected", ()).unwrap();

    sender
        .send_files(
            || {},
            |accepted| {
                window.emit("files-decision", accepted).unwrap();
            },
            |initial_bytes_sent| {
                window
                    .emit(
                        "initial-download-progress",
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
                window.emit("bytes-sent", bytes_sent).unwrap();
            },
        )
        .await
        .map_err(|e| format!("failed to send files: {}", e))?;

    window.emit("transfer-complete", ()).unwrap();

    Ok(())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileInfo {
    size_bytes: u64,
    is_directory: bool,
}

#[tauri::command(async)]
async fn file_info(path: PathBuf) -> Result<FileInfo, String> {
    tracing::info!("sdads");
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

fn get_external_addr(socket: &UdpSocket) -> Result<SocketAddr, String> {
    utils::external_addr(
        socket,
        STUN_SERVERS[0]
            .to_socket_addrs()
            .map_err(|e| format!("failed to resolve stun server: {}", e))?
            .find(|x| x.is_ipv4())
            .unwrap(),
        Some(
            STUN_SERVERS[1]
                .to_socket_addrs()
                .map_err(|e| format!("failed to resolve stun server: {}", e))?
                .find(|x| x.is_ipv4())
                .unwrap(),
        ),
    )
    .map_err(|e| format!("failed to get external address: {}", e))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    std::env::set_var("RUST_LOG", "debug");
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            exit,
            code_len,
            download_files,
            file_info,
            upload_files,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
