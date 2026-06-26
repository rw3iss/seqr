//! Tauri IPC commands — the thin seam between the Preact UI and the core.
//!
//! Each command maps a UI intent to core modules and returns serializable data or a
//! `CoreError` (rendered to a friendly string for the UI). No business logic lives
//! here; that belongs in the core modules.

use serde::Serialize;
use tauri::State;

use seqr_protocol::ProfileBlob;

use crate::core::config::AppConfig;
use crate::core::session::SessionState;
use crate::core::vault::Friend;
use crate::core::{identity, vault, CoreResult};

#[derive(Serialize)]
pub struct AppStatus {
    pub account_exists: bool,
    pub unlocked: bool,
}

#[tauri::command]
pub fn app_status(state: State<SessionState>) -> AppStatus {
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
pub fn create_account(
    display_name: String,
    password: String,
    state: State<SessionState>,
) -> CoreResult<ProfileBlob> {
    let (key, data) = vault::create(&state.data_dir, &display_name, &password)?;
    let profile = identity::profile_for(&data)?;
    state.set_unlocked(key, data);
    Ok(profile)
}

#[tauri::command]
pub fn unlock(password: String, state: State<SessionState>) -> CoreResult<ProfileBlob> {
    let (key, data) = vault::unlock(&state.data_dir, &password)?;
    let profile = identity::profile_for(&data)?;
    state.set_unlocked(key, data);
    Ok(profile)
}

#[tauri::command]
pub fn lock(state: State<SessionState>) {
    state.lock();
}

#[tauri::command]
pub fn my_profile(state: State<SessionState>) -> CoreResult<ProfileBlob> {
    state.with_unlocked(|u| identity::profile_for(&u.data))
}

#[tauri::command]
pub fn export_profile(state: State<SessionState>) -> CoreResult<String> {
    state.with_unlocked(|u| {
        let profile = identity::profile_for(&u.data)?;
        identity::encode_token(&profile)
    })
}

#[tauri::command]
pub fn import_friend(token: String, state: State<SessionState>) -> CoreResult<Friend> {
    let profile = identity::decode_token(&token)?;
    let friend = identity::friend_from(&profile);
    let data_dir = state.data_dir.clone();
    state.with_unlocked(|u| {
        if u.data.friends.iter().any(|f| f.signing_public == friend.signing_public) {
            return Err(crate::core::CoreError::DuplicateFriend);
        }
        u.data.friends.push(friend.clone());
        vault::save(&data_dir, &u.vault_key, &u.data)?;
        Ok(friend)
    })
}

#[tauri::command]
pub fn list_friends(state: State<SessionState>) -> CoreResult<Vec<Friend>> {
    state.with_unlocked(|u| Ok(u.data.friends.clone()))
}
