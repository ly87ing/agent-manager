#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engine;

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![engine::engine_invoke])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
