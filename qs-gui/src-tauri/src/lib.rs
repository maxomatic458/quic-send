use qs_core::CODE_LEN;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn code_len() -> usize {
    CODE_LEN    
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![code_len])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
