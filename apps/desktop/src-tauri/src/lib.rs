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
    // WebKitGTK's DMABUF renderer crashes on many Wayland compositors with
    // "Error 71 (Protocol error)". Disable it before the web view initializes, unless
    // the user has set the variable themselves. Packaged builds rely on this (the dev
    // script alone doesn't reach the bundled binary).
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Per-user, OS-appropriate locations (e.g. ~/.local/share/com.seqr.app).
            let data_dir = app.path().app_data_dir().expect("resolve app data dir");
            let config_dir = app.path().app_config_dir().expect("resolve app config dir");
            std::fs::create_dir_all(&data_dir).ok();
            std::fs::create_dir_all(&config_dir).ok();

            let config = AppConfig::resolve(&config_dir);
            let mailbox_url = config.mailbox_url.clone();
            let mailbox_cert = config.mailbox_cert.clone();
            app.manage(config);
            app.manage(Arc::new(SessionState::new(data_dir, mailbox_url, mailbox_cert)));
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
            commands::list_requests,
            commands::accept_request,
            commands::decline_request,
            commands::list_friends,
            commands::get_settings,
            commands::set_settings,
            commands::list_conversations,
            commands::get_history,
            commands::presence,
            commands::group_members,
            commands::safety_number,
            commands::send_message,
            commands::send_attachment,
            commands::read_attachment,
            commands::save_attachment,
            commands::open_attachment,
            commands::create_group,
            commands::send_group_message,
            commands::rotate_direct,
            commands::remove_friend,
            commands::rotate_group,
            commands::remove_member,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
