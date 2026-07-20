//! Tauri IPC commands for the Matrix backend.
//!
//! Thin wrappers over `matrix-sdk`: login, restore-on-restart, logout, and a status
//! probe the UI polls to decide whether to show the login screen or the app. Errors are
//! flattened to strings (the UI's `errMessage` renders them).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use matrix_sdk::ruma::{RoomId, UInt};
use matrix_sdk::{room::MessagesOptions, Client};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::matrix::client::{build_client, new_passphrase, ClientSession, FullSession};
use crate::matrix::sync::MatrixMessage;
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
    state.syncing.store(false, Ordering::SeqCst);
    let _ = std::fs::remove_file(state.session_file());
    Ok(())
}

/// Start the background sync loop (once). Emits `matrix://message` on new messages.
#[tauri::command]
pub async fn matrix_start_sync(app: AppHandle, state: MxState<'_>) -> Result<(), String> {
    if state.syncing.swap(true, Ordering::SeqCst) {
        return Ok(()); // already running
    }
    let client = {
        let guard = state.client.read().await;
        guard.as_ref().ok_or("not logged in")?.clone()
    };
    crate::matrix::sync::start(client, app);
    Ok(())
}

/// A joined room, for the conversations list.
#[derive(Serialize)]
pub struct MatrixRoom {
    pub id: String,
    pub name: String,
    pub is_dm: bool,
}

/// List the rooms the user has joined.
#[tauri::command]
pub async fn matrix_rooms(state: MxState<'_>) -> Result<Vec<MatrixRoom>, String> {
    let guard = state.client.read().await;
    let client = guard.as_ref().ok_or("not logged in")?;
    let mut out = Vec::new();
    for room in client.joined_rooms() {
        let name = room.name().unwrap_or_else(|| room.room_id().to_string());
        let is_dm = room.is_direct().await.unwrap_or(false);
        out.push(MatrixRoom { id: room.room_id().to_string(), name, is_dm });
    }
    Ok(out)
}

/// Send a plain-text message to a room.
#[tauri::command]
pub async fn matrix_send_message(
    room_id: String,
    body: String,
    state: MxState<'_>,
) -> Result<(), String> {
    let guard = state.client.read().await;
    let client = guard.as_ref().ok_or("not logged in")?;
    let rid = RoomId::parse(&room_id).map_err(|e| e.to_string())?;
    let room = client.get_room(&rid).ok_or("unknown room")?;
    room.send(RoomMessageEventContent::text_plain(body))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Recent text history for a room (oldest-first), parsed from raw events so we don't
/// depend on the SDK's evolving timeline-item enums.
#[tauri::command]
pub async fn matrix_room_messages(
    room_id: String,
    state: MxState<'_>,
) -> Result<Vec<MatrixMessage>, String> {
    let guard = state.client.read().await;
    let client = guard.as_ref().ok_or("not logged in")?;
    let me = client.user_id().map(|u| u.to_string());
    let rid = RoomId::parse(&room_id).map_err(|e| e.to_string())?;
    let room = client.get_room(&rid).ok_or("unknown room")?;

    let mut opts = MessagesOptions::backward();
    opts.limit = UInt::from(50u32);
    let resp = room.messages(opts).await.map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for ev in resp.chunk {
        let json = ev.raw().json();
        let v: serde_json::Value = match serde_json::from_str(json.get()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("m.room.message") {
            continue;
        }
        let content = &v["content"];
        if content.get("msgtype").and_then(|m| m.as_str()) != Some("m.text") {
            continue;
        }
        let Some(body) = content.get("body").and_then(|b| b.as_str()) else { continue };
        let sender = v.get("sender").and_then(|s| s.as_str()).unwrap_or("").to_string();
        out.push(MatrixMessage {
            room_id: room_id.clone(),
            event_id: v.get("event_id").and_then(|e| e.as_str()).map(String::from),
            outgoing: me.as_deref() == Some(sender.as_str()),
            sender,
            body: body.to_string(),
            ts: v.get("origin_server_ts").and_then(|t| t.as_u64()).unwrap_or(0),
        });
    }
    out.reverse(); // backward() yields newest-first; the UI wants oldest-first
    Ok(out)
}
