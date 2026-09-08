#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engine;
mod settings;

#[tauri::command]
fn settings_get(app: tauri::AppHandle) -> Result<settings::SettingsData, String> { settings::settings_data(&app) }

#[tauri::command]
fn settings_save(app: tauri::AppHandle, preferences: settings::SettingsPreferences) -> Result<settings::SettingsData, String> { settings::save_preferences(&app, preferences) }

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![engine::engine_invoke, settings_get, settings_save])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
