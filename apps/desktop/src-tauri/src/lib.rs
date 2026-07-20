// matrix-sdk's deeply-nested futures overflow the default auto-trait (Send) recursion
// limit when spawned; raise it. (Standard workaround for the SDK.)
#![recursion_limit = "512"]

//! Seqr desktop backend entry point.
//!
//! Resolves the per-user data/config directories, loads configuration (the VPS
//! mailbox endpoint), registers the session state, and exposes the command surface
//! to the Preact UI.

mod commands;
mod core;
mod matrix;
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
            let homeserver_url = config.homeserver_url.clone();
            app.manage(config);
            // Both backends are constructed; the active one is chosen by the UI from the
            // config `backend` field (surfaced via `app_config`). P2P (legacy) state:
            app.manage(Arc::new(SessionState::new(data_dir.clone(), mailbox_url, mailbox_cert)));
            // Matrix (default) state:
            app.manage(Arc::new(matrix::MatrixState::new(data_dir, homeserver_url)));
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
            commands::set_screen_security,
            commands::list_conversations,
            commands::get_history,
            commands::presence,
            commands::group_members,
            commands::safety_number,
            commands::send_message,
            commands::send_attachment,
            commands::read_attachment,
            commands::save_attachment,
            commands::stage_pasted_file,
            commands::create_group,
            commands::send_group_message,
            commands::rotate_direct,
            commands::remove_friend,
            commands::rotate_group,
            commands::remove_member,
            // Matrix backend (default).
            matrix::commands::matrix_status,
            matrix::commands::matrix_login,
            matrix::commands::matrix_register,
            matrix::commands::matrix_has_session,
            matrix::commands::matrix_unlock,
            matrix::commands::matrix_logout,
            matrix::commands::matrix_start_sync,
            matrix::commands::matrix_rooms,
            matrix::commands::matrix_send_message,
            matrix::commands::matrix_room_messages,
            matrix::commands::matrix_react,
            matrix::commands::matrix_redact,
            matrix::commands::matrix_create_dm,
            matrix::commands::matrix_create_room,
            matrix::commands::matrix_invite,
            matrix::commands::matrix_join,
            matrix::commands::matrix_leave,
            matrix::commands::matrix_room_members,
            matrix::commands::matrix_send_file,
            matrix::commands::matrix_read_media,
            matrix::commands::matrix_save_media,
            matrix::commands::matrix_devices,
            matrix::commands::matrix_verify_device,
            matrix::commands::matrix_recovery_enable,
            matrix::commands::matrix_recover,
            matrix::commands::matrix_verification_status,
            matrix::commands::matrix_register_pusher,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
