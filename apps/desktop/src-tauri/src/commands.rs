//! Tauri IPC commands — the thin seam between the Preact UI and the core.
//!
//! Each command maps a UI intent to core modules and returns serializable data or a
//! `CoreError` (rendered to a friendly string for the UI). No business logic lives
//! here; that belongs in the core modules and `net`.

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};

use seqr_protocol::ProfileBlob;

use crate::core::config::AppConfig;
use crate::core::mailbox::MailboxClient;
use crate::core::session::SessionState;
use crate::core::vault::{Friend, StoredMessage};
use crate::core::{conversation, identity, message, now_millis, vault, CoreError, CoreResult};
use crate::net;

type Session<'a> = State<'a, Arc<SessionState>>;

#[derive(Serialize)]
pub struct AppStatus {
    pub account_exists: bool,
    pub unlocked: bool,
}

#[tauri::command]
pub fn app_status(state: Session) -> AppStatus {
    AppStatus {
        account_exists: vault::exists(&state.data_dir),
        unlocked: state.is_unlocked(),
    }
}

#[tauri::command]
pub fn app_config(config: State<AppConfig>) -> AppConfig {
    config.inner().clone()
}

#[tauri::command]
pub async fn create_account(
    display_name: String,
    password: String,
    state: Session<'_>,
    app: AppHandle,
) -> CoreResult<ProfileBlob> {
    let (key, data) = vault::create(&state.data_dir, &display_name, &password)?;
    let profile = identity::profile_for(&data)?;
    let (_, signing) = data.identity()?.secret_bytes();
    state.set_unlocked(key, data);
    net::start_transport(Arc::clone(state.inner()), app, signing).await?;
    Ok(profile)
}

#[tauri::command]
pub async fn unlock(password: String, state: Session<'_>, app: AppHandle) -> CoreResult<ProfileBlob> {
    let (key, data) = vault::unlock(&state.data_dir, &password)?;
    let profile = identity::profile_for(&data)?;
    let (_, signing) = data.identity()?.secret_bytes();
    state.set_unlocked(key, data);
    net::start_transport(Arc::clone(state.inner()), app, signing).await?;
    Ok(profile)
}

#[tauri::command]
pub fn lock(state: Session) {
    state.lock();
}

#[tauri::command]
pub fn my_profile(state: Session) -> CoreResult<ProfileBlob> {
    state.with_unlocked(|u| identity::profile_for(&u.data))
}

#[tauri::command]
pub fn export_profile(state: Session) -> CoreResult<String> {
    state.with_unlocked(|u| {
        let profile = identity::profile_for(&u.data)?;
        identity::encode_token(&profile)
    })
}

#[tauri::command]
pub fn import_friend(token: String, state: Session) -> CoreResult<Friend> {
    let profile = identity::decode_token(&token)?;
    let friend = identity::friend_from(&profile);
    let data_dir = state.data_dir.clone();
    state.with_unlocked(|u| {
        if u.data.friends.iter().any(|f| f.signing_public == friend.signing_public) {
            return Err(CoreError::DuplicateFriend);
        }
        u.data.friends.push(friend.clone());
        vault::save(&data_dir, &u.vault_key, &u.data)?;
        Ok(friend)
    })
}

#[tauri::command]
pub fn list_friends(state: Session) -> CoreResult<Vec<Friend>> {
    state.with_unlocked(|u| Ok(u.data.friends.clone()))
}

/// History for the 1:1 conversation with a friend (by their signing public key).
#[tauri::command]
pub fn get_history(friend: String, state: Session) -> CoreResult<Vec<StoredMessage>> {
    state.with_unlocked(|u| {
        let my_signing = hex::encode(u.data.identity()?.public().signing_public);
        let conv_id = conversation::direct_conversation_id(&my_signing, &friend);
        Ok(u.data.history(&conv_id))
    })
}

/// Seal, sign, persist, and send a message to a friend (by their signing public key).
/// The message is stored locally regardless; delivery failure (peer offline) is
/// reported but the message remains in history. Offline delivery via mailbox is M3.
#[tauri::command]
pub async fn send_message(friend: String, body: String, state: Session<'_>) -> CoreResult<StoredMessage> {
    // Phase 1 (locked): build the frame, persist the outgoing message.
    let (stored, frame_json, friend_signing) = {
        let data_dir = state.data_dir.clone();
        state.with_unlocked(|u| {
            let me = u.data.identity()?;
            let friend_rec = u
                .data
                .friend_by_signing(&friend)
                .ok_or(CoreError::UnknownSender)?
                .clone();
            let my_signing = hex::encode(me.public().signing_public);
            let conv_id = conversation::direct_conversation_id(&my_signing, &friend);
            let key = conversation::pairwise_key(&me, &friend_rec)?;
            let seq = u.data.next_seq(&conv_id);
            let frame =
                message::build_frame(&me, &conv_id, conversation::DIRECT_EPOCH, &key, seq, &body);
            let frame_json =
                serde_json::to_vec(&frame).map_err(|e| CoreError::Storage(e.to_string()))?;

            let msg = StoredMessage {
                conversation_id: conv_id,
                sender: my_signing,
                body: body.clone(),
                ts: now_millis(),
                outgoing: true,
                seq,
            };
            u.data.add_message(msg.clone());
            vault::save(&data_dir, &u.vault_key, &u.data)?;

            let friend_signing: [u8; 32] = hex::decode(&friend_rec.signing_public)
                .ok()
                .and_then(|v| v.try_into().ok())
                .ok_or(CoreError::BadProfile("bad friend key".into()))?;
            Ok((msg, frame_json, friend_signing))
        })?
    };

    // Phase 2 (unlocked): try direct delivery; if the peer is unreachable, park the
    // (already-sealed) frame in the mailbox for offline delivery.
    let delivered = match state.transport() {
        Some(transport) => transport.send_to_id(&friend_signing, &frame_json).await.is_ok(),
        None => false,
    };
    if !delivered {
        let payload = String::from_utf8(frame_json).unwrap_or_default();
        let client = MailboxClient::new(&state.mailbox_url);
        if let Err(e) = client.push(&friend, &payload).await {
            eprintln!("seqr: mailbox push failed: {e}");
        }
    }

    Ok(stored)
}
