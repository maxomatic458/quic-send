// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::OnceLock;

use serde::Serialize;
use tauri::{Manager, Window};
use thiserror::Error;
// use tauri::{, Window};

pub mod utils;

// keep track of the global window

static WINDOW: OnceLock<Option<Window>> = OnceLock::new();

#[derive(Debug, Error)]
enum AppError {
    #[error("tauri error: {0}")]
    Tauri(#[from] tauri::Error),
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![utils::get_file_size_and_is_dir])
        .setup(|app| {
            let binding = app.windows();
            let window = binding.iter().next().unwrap().1;
            WINDOW.set(Some(window.clone())).unwrap();
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn emit(event_name: &str, payload: impl Serialize + Clone) -> tauri::Result<()> {
    let window = WINDOW.get().unwrap().clone().unwrap();
    window.emit(event_name, payload)
}
