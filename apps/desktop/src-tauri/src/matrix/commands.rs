//! Tauri IPC commands for the Matrix backend.
//!
//! Thin wrappers over `matrix-sdk`: login, restore-on-restart, logout, and a status
//! probe the UI polls to decide whether to show the login screen or the app. Errors are
//! flattened to strings (the UI's `errMessage` renders them).

use std::sync::Arc;

use matrix_sdk::Client;
use serde::Serialize;
use tauri::State;

use crate::matrix::client::{build_client, new_passphrase, ClientSession, FullSession};
use crate::matrix::MatrixState;

type MxState<'a> = State<'a, Arc<MatrixState>>;

/// Snapshot of the Matrix backend for the UI.
#[derive(Serialize)]
pub struct MatrixStatus {
    pub homeserver_url: String,
    pub logged_in: bool,
    pub user_id: Option<String>,
    pub device_id: Option<String>,
}

fn status_of(client: Option<&Client>, homeserver: &str) -> MatrixStatus {
    match client {
        Some(c) => MatrixStatus {
            homeserver_url: homeserver.to_string(),
            // A client with a session has a user id; good enough to gate the login screen.
            logged_in: c.user_id().is_some(),
            user_id: c.user_id().map(|u| u.to_string()),
            device_id: c.device_id().map(|d| d.to_string()),
        },
        None => MatrixStatus {
            homeserver_url: homeserver.to_string(),
            logged_in: false,
            user_id: None,
            device_id: None,
        },
    }
}

/// Current backend status (does not touch the network).
#[tauri::command]
pub async fn matrix_status(state: MxState<'_>) -> Result<MatrixStatus, String> {
    let guard = state.client.read().await;
    Ok(status_of(guard.as_ref(), &state.homeserver_url))
}

/// Password login against the configured homeserver; persists the session on success.
#[tauri::command]
pub async fn matrix_login(
    username: String,
    password: String,
    state: MxState<'_>,
) -> Result<MatrixStatus, String> {
    let db_path = state.store_dir();
    std::fs::create_dir_all(&db_path).map_err(|e| e.to_string())?;

    let passphrase = new_passphrase();
    let client = build_client(&state.homeserver_url, &db_path, &passphrase)
        .await
        .map_err(|e| e.to_string())?;

    client
        .matrix_auth()
        .login_username(&username, &password)
        .initial_device_display_name("Seqr Desktop")
        .await
        .map_err(|e| e.to_string())?;

    // Persist the session so we come back logged-in on the next launch.
    let user_session = client
        .matrix_auth()
        .session()
        .ok_or_else(|| "no session after login".to_string())?;
    let full = FullSession {
        client_session: ClientSession {
            homeserver: state.homeserver_url.clone(),
            db_path,
            passphrase,
        },
        user_session,
        sync_token: None,
    };
    let json = serde_json::to_string(&full).map_err(|e| e.to_string())?;
    std::fs::write(state.session_file(), json).map_err(|e| e.to_string())?;

    let status = status_of(Some(&client), &state.homeserver_url);
    *state.client.write().await = Some(client);
    Ok(status)
}

/// Rebuild the client from a persisted session (called at startup). No-op if none saved.
#[tauri::command]
pub async fn matrix_restore_session(state: MxState<'_>) -> Result<MatrixStatus, String> {
    let file = state.session_file();
    if !file.exists() {
        return Ok(status_of(None, &state.homeserver_url));
    }
    let json = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let full: FullSession = serde_json::from_str(&json).map_err(|e| e.to_string())?;

    let client = build_client(
        &full.client_session.homeserver,
        &full.client_session.db_path,
        &full.client_session.passphrase,
    )
    .await
    .map_err(|e| e.to_string())?;
    client
        .restore_session(full.user_session)
        .await
        .map_err(|e| e.to_string())?;

    let status = status_of(Some(&client), &state.homeserver_url);
    *state.client.write().await = Some(client);
    Ok(status)
}

/// Log out (best-effort server-side) and drop the persisted session.
#[tauri::command]
pub async fn matrix_logout(state: MxState<'_>) -> Result<(), String> {
    if let Some(client) = state.client.write().await.take() {
        let _ = client.matrix_auth().logout().await;
    }
    let _ = std::fs::remove_file(state.session_file());
    Ok(())
}
