//! Seqr desktop backend entry point.
//!
//! Resolves the per-user data/config directories, loads configuration (the VPS
//! mailbox endpoint), registers the session state, and exposes the command surface
//! to the Preact UI.

mod commands;
mod core;
mod net;

use std::sync::Arc;

use tauri::Manager;

use crate::core::config::AppConfig;
use crate::core::session::SessionState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Per-user, OS-appropriate locations (e.g. ~/.local/share/com.seqr.app).
            let data_dir = app.path().app_data_dir().expect("resolve app data dir");
            let config_dir = app.path().app_config_dir().expect("resolve app config dir");
            std::fs::create_dir_all(&data_dir).ok();
            std::fs::create_dir_all(&config_dir).ok();

            let config = AppConfig::resolve(&config_dir.join("seqr.toml"));
            let mailbox_url = config.mailbox_url.clone();
            app.manage(config);
            app.manage(Arc::new(SessionState::new(data_dir, mailbox_url)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::app_config,
            commands::create_account,
            commands::unlock,
            commands::lock,
            commands::my_profile,
            commands::export_profile,
            commands::import_friend,
            commands::list_friends,
            commands::list_conversations,
            commands::get_history,
            commands::send_message,
            commands::create_group,
            commands::send_group_message,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
