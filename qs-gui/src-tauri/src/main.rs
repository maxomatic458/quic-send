// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod utils;

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            utils::get_file_size_and_is_dir
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
