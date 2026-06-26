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
use crate::core::packet::{GroupInvite, Packet};
use crate::core::session::SessionState;
use crate::core::vault::{Friend, Group, StoredMessage};
use crate::core::{conversation, group, identity, message, now_millis, vault, CoreError, CoreResult};
use crate::net;

type Session<'a> = State<'a, Arc<SessionState>>;

#[derive(Serialize)]
pub struct AppStatus {
    pub account_exists: bool,
    pub unlocked: bool,
}

/// A conversation as the UI lists it (1:1 or group).
#[derive(Serialize)]
pub struct ConversationDto {
    pub id: String,
    pub kind: &'static str, // "direct" | "group"
    pub title: String,
    /// For a 1:1, the peer's signing public key; `None` for groups.
    pub peer: Option<String>,
    pub members: usize,
}

#[tauri::command]
pub fn app_status(state: Session) -> AppStatus {
    AppStatus { account_exists: vault::exists(&state.data_dir), unlocked: state.is_unlocked() }
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

/// Import a friend's token. Adds them locally AND sends them a signed friend request
/// (carrying our profile) so they get a one-tap prompt to add us back — no second
/// token exchange needed.
#[tauri::command]
pub async fn import_friend(token: String, state: Session<'_>) -> CoreResult<Friend> {
    let profile = identity::decode_token(&token)?;
    let friend = identity::friend_from(&profile);

    let (friend, request_json) = {
        let data_dir = state.data_dir.clone();
        state.with_unlocked(|u| {
            if u.data.friends.iter().any(|f| f.signing_public == friend.signing_public) {
                return Err(CoreError::DuplicateFriend);
            }
            u.data.add_friend(friend.clone());
            vault::save(&data_dir, &u.vault_key, &u.data)?;
            let request = identity::signed_friend_request(&u.data)?;
            Ok((friend.clone(), Packet::FriendRequest(request).to_json()))
        })?
    };
    net::deliver(state.inner(), &friend.signing_public, &request_json).await;
    Ok(friend)
}

/// Pending incoming friend requests.
#[tauri::command]
pub fn list_requests(state: Session) -> CoreResult<Vec<Friend>> {
    state.with_unlocked(|u| Ok(u.data.pending_requests.clone()))
}

/// Accept a friend request: add them as a friend (and reciprocate the request so they
/// have us too, in case they imported us only after sending).
#[tauri::command]
pub async fn accept_request(signing: String, state: Session<'_>) -> CoreResult<()> {
    let request_json = {
        let data_dir = state.data_dir.clone();
        state.with_unlocked(|u| {
            let friend = u
                .data
                .pending_by_signing(&signing)
                .ok_or(CoreError::UnknownSender)?
                .clone();
            u.data.add_friend(friend);
            vault::save(&data_dir, &u.vault_key, &u.data)?;
            let request = identity::signed_friend_request(&u.data)?;
            Ok(Packet::FriendRequest(request).to_json())
        })?
    };
    // Reciprocate so the requester definitely has us (idempotent on their side).
    net::deliver(state.inner(), &signing, &request_json).await;
    Ok(())
}

/// Decline a friend request: drop it.
#[tauri::command]
pub fn decline_request(signing: String, state: Session) -> CoreResult<()> {
    let data_dir = state.data_dir.clone();
    state.with_unlocked(|u| {
        u.data.remove_pending(&signing);
        vault::save(&data_dir, &u.vault_key, &u.data)
    })
}

#[tauri::command]
pub fn list_friends(state: Session) -> CoreResult<Vec<Friend>> {
    state.with_unlocked(|u| Ok(u.data.friends.clone()))
}

/// All conversations (1:1 and group) for the conversations list.
#[tauri::command]
pub fn list_conversations(state: Session) -> CoreResult<Vec<ConversationDto>> {
    state.with_unlocked(|u| {
        let my_signing = hex::encode(u.data.identity()?.public().signing_public);
        let mut out = Vec::new();
        for f in &u.data.friends {
            out.push(ConversationDto {
                id: conversation::direct_conversation_id(&my_signing, &f.signing_public),
                kind: "direct",
                title: f.display_name.clone(),
                peer: Some(f.signing_public.clone()),
                members: 2,
            });
        }
        for g in &u.data.groups {
            out.push(ConversationDto {
                id: g.id.clone(),
                kind: "group",
                title: g.name.clone(),
                peer: None,
                members: g.members.len() + 1,
            });
        }
        Ok(out)
    })
}

/// History for any conversation, by its id (direct or group).
#[tauri::command]
pub fn get_history(conversation_id: String, state: Session) -> CoreResult<Vec<StoredMessage>> {
    state.with_unlocked(|u| Ok(u.data.history(&conversation_id)))
}

/// The other members of a group (everyone but this account).
#[tauri::command]
pub fn group_members(group_id: String, state: Session) -> CoreResult<Vec<Friend>> {
    state.with_unlocked(|u| {
        Ok(u.data.group_by_id(&group_id).map(|g| g.members.clone()).unwrap_or_default())
    })
}

/// A comparable safety number for the 1:1 with `friend`, to verify their identity
/// out-of-band (guards against a tampered profile exchange).
#[tauri::command]
pub fn safety_number(friend: String, state: Session) -> CoreResult<String> {
    state.with_unlocked(|u| {
        let mine = u.data.identity()?.public().signing_public;
        let theirs: [u8; 32] = hex::decode(&friend)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or(CoreError::BadProfile("bad friend key".into()))?;
        Ok(seqr_crypto::fingerprint::safety_number(&mine, &theirs))
    })
}

/// Seal, sign, persist, and send a 1:1 message to a friend (by signing public key).
#[tauri::command]
pub async fn send_message(friend: String, body: String, state: Session<'_>) -> CoreResult<StoredMessage> {
    let (stored, packet_json) = {
        let data_dir = state.data_dir.clone();
        state.with_unlocked(|u| {
            let me = u.data.identity()?;
            let friend_rec =
                u.data.friend_by_signing(&friend).ok_or(CoreError::UnknownSender)?.clone();
            let my_signing = hex::encode(me.public().signing_public);
            let conv_id = conversation::direct_conversation_id(&my_signing, &friend);
            let (epoch, key) = conversation::current_direct_key(&u.data, &me, &friend_rec)?;
            let seq = u.data.next_seq(&conv_id);
            let frame = message::build_frame(&me, &conv_id, epoch, &key, seq, &body);
            let msg = stored_outgoing(&conv_id, &my_signing, &body, seq);
            u.data.add_message(msg.clone());
            vault::save(&data_dir, &u.vault_key, &u.data)?;
            Ok((msg, Packet::Message(frame).to_json()))
        })?
    };
    net::debug_log(
        state.inner(),
        format!("SEND 1:1 -> {} seq={}", &friend.chars().take(8).collect::<String>(), stored.seq),
    );
    net::deliver(state.inner(), &friend, &packet_json).await;
    Ok(stored)
}

/// Create a group, generate its key, and distribute it (sealed) to each member.
#[tauri::command]
pub async fn create_group(
    name: String,
    members: Vec<String>,
    state: Session<'_>,
) -> CoreResult<ConversationDto> {
    // Phase 1 (locked): build the group, store it, and prepare per-member invites.
    let (group_id, member_count, invites) = {
        let data_dir = state.data_dir.clone();
        state.with_unlocked(|u| {
            let me = u.data.identity()?;
            let my_signing = hex::encode(me.public().signing_public);

            // Resolve selected friends (must be in roster).
            let mut roster_others = Vec::new();
            for m in &members {
                roster_others
                    .push(u.data.friend_by_signing(m).ok_or(CoreError::UnknownSender)?.clone());
            }
            let kg = seqr_crypto::group::generate_group_key();
            let group_id = group::new_group_id();
            let epoch = 0u64;

            // Full roster includes us, for the invitees' rosters.
            let mut full_roster = vec![identity::self_as_friend(&u.data)?];
            full_roster.extend(roster_others.iter().cloned());

            // Store the group locally (members = everyone but us).
            u.data.groups.push(Group {
                id: group_id.clone(),
                name: name.clone(),
                members: roster_others.clone(),
                epoch,
                key: hex::encode(kg),
            });
            vault::save(&data_dir, &u.vault_key, &u.data)?;

            // One sealed invite per member.
            let mut invites: Vec<(String, Vec<u8>)> = Vec::new();
            for recipient in &roster_others {
                let sealed = group::seal_group_key(&me, recipient, &group_id, epoch, &kg)?;
                let invite = GroupInvite {
                    group_id: group_id.clone(),
                    name: name.clone(),
                    epoch,
                    members: full_roster.clone(),
                    originator: my_signing.clone(),
                    sealed_key: sealed,
                };
                invites.push((recipient.signing_public.clone(), Packet::GroupInvite(invite).to_json()));
            }
            Ok((group_id, roster_others.len() + 1, invites))
        })?
    };

    // Phase 2 (unlocked): deliver invites (direct, or mailbox if offline).
    for (recipient, packet) in &invites {
        net::deliver(state.inner(), recipient, packet).await;
    }

    Ok(ConversationDto {
        id: group_id,
        kind: "group",
        title: name,
        peer: None,
        members: member_count,
    })
}

/// Seal, sign, persist, and fan out a group message to every member.
#[tauri::command]
pub async fn send_group_message(
    group_id: String,
    body: String,
    state: Session<'_>,
) -> CoreResult<StoredMessage> {
    let (stored, packet_json, recipients) = {
        let data_dir = state.data_dir.clone();
        state.with_unlocked(|u| {
            let me = u.data.identity()?;
            let my_signing = hex::encode(me.public().signing_public);
            let g = u.data.group_by_id(&group_id).ok_or(CoreError::UnknownSender)?.clone();
            let kg = u.data.group_key(&group_id).ok_or(CoreError::Crypto("no group key".into()))?;
            let seq = u.data.next_seq(&group_id);
            let frame = message::build_frame(&me, &group_id, g.epoch, &kg, seq, &body);
            let msg = stored_outgoing(&group_id, &my_signing, &body, seq);
            u.data.add_message(msg.clone());
            vault::save(&data_dir, &u.vault_key, &u.data)?;
            let recipients: Vec<String> = g.members.iter().map(|m| m.signing_public.clone()).collect();
            Ok((msg, Packet::Message(frame).to_json(), recipients))
        })?
    };
    for recipient in &recipients {
        net::deliver(state.inner(), recipient, &packet_json).await;
    }
    Ok(stored)
}

/// Rotate the key of a 1:1 conversation: mint a new key at the next epoch and send it
/// to the friend (sealed under the long-term pairwise key).
#[tauri::command]
pub async fn rotate_direct(friend: String, state: Session<'_>) -> CoreResult<()> {
    let (recipient, packet) = {
        let data_dir = state.data_dir.clone();
        state.with_unlocked(|u| {
            let me = u.data.identity()?;
            let friend_rec =
                u.data.friend_by_signing(&friend).ok_or(CoreError::UnknownSender)?.clone();
            let my_signing = hex::encode(me.public().signing_public);
            let conv_id = conversation::direct_conversation_id(&my_signing, &friend);
            let (cur_epoch, _) = conversation::current_direct_key(&u.data, &me, &friend_rec)?;
            let new_epoch = cur_epoch + 1;
            let new_key = seqr_crypto::group::generate_group_key();
            u.data.set_direct_key(&conv_id, new_epoch, hex::encode(new_key));
            vault::save(&data_dir, &u.vault_key, &u.data)?;

            let sealed =
                conversation::seal_direct_key(&me, &friend_rec, &conv_id, new_epoch, &new_key)?;
            let packet = Packet::KeyUpdate(crate::core::packet::KeyUpdate {
                conversation_id: conv_id,
                epoch: new_epoch,
                originator: my_signing,
                sealed_key: sealed,
            })
            .to_json();
            Ok((friend.clone(), packet))
        })?
    };
    net::deliver(state.inner(), &recipient, &packet).await;
    Ok(())
}

/// Revoke a 1:1: remove the friend so no further messages are accepted or sent.
/// History is retained locally.
#[tauri::command]
pub fn remove_friend(friend: String, state: Session) -> CoreResult<()> {
    let data_dir = state.data_dir.clone();
    state.with_unlocked(|u| {
        u.data.remove_friend(&friend);
        vault::save(&data_dir, &u.vault_key, &u.data)
    })
}

/// Rotate a group key: any member may mint a new key at the next epoch and redistribute
/// it (sealed) to every current member.
#[tauri::command]
pub async fn rotate_group(group_id: String, state: Session<'_>) -> CoreResult<()> {
    let invites = rebuild_group_keys(&state, &group_id, None)?;
    for (recipient, packet) in &invites {
        net::deliver(state.inner(), recipient, packet).await;
    }
    Ok(())
}

/// Remove a member from a group (revocation): mint a new key at the next epoch and
/// distribute it to everyone *except* the removed member, cutting them off.
#[tauri::command]
pub async fn remove_member(
    group_id: String,
    member: String,
    state: Session<'_>,
) -> CoreResult<()> {
    let invites = rebuild_group_keys(&state, &group_id, Some(&member))?;
    for (recipient, packet) in &invites {
        net::deliver(state.inner(), recipient, packet).await;
    }
    Ok(())
}

/// Shared core of group rotation/removal: bump the epoch, mint a new key, update the
/// local group (optionally dropping `remove`), and return per-recipient sealed invites.
fn rebuild_group_keys(
    state: &Session,
    group_id: &str,
    remove: Option<&str>,
) -> CoreResult<Vec<(String, Vec<u8>)>> {
    let data_dir = state.data_dir.clone();
    state.with_unlocked(|u| {
        let me = u.data.identity()?;
        let my_signing = hex::encode(me.public().signing_public);
        let g = u.data.group_by_id(group_id).ok_or(CoreError::UnknownSender)?.clone();

        let remaining: Vec<Friend> = g
            .members
            .into_iter()
            .filter(|m| remove != Some(m.signing_public.as_str()))
            .collect();
        let new_kg = seqr_crypto::group::generate_group_key();
        let new_epoch = g.epoch + 1;

        // Update our local group.
        if let Some(group) = u.data.group_by_id_mut(group_id) {
            group.members = remaining.clone();
            group.epoch = new_epoch;
            group.key = hex::encode(new_kg);
        }
        vault::save(&data_dir, &u.vault_key, &u.data)?;

        let mut full_roster = vec![identity::self_as_friend(&u.data)?];
        full_roster.extend(remaining.iter().cloned());

        let mut invites = Vec::new();
        for recipient in &remaining {
            let sealed = group::seal_group_key(&me, recipient, group_id, new_epoch, &new_kg)?;
            let invite = GroupInvite {
                group_id: group_id.to_string(),
                name: g.name.clone(),
                epoch: new_epoch,
                members: full_roster.clone(),
                originator: my_signing.clone(),
                sealed_key: sealed,
            };
            invites.push((recipient.signing_public.clone(), Packet::GroupInvite(invite).to_json()));
        }
        Ok(invites)
    })
}

fn stored_outgoing(conversation_id: &str, my_signing: &str, body: &str, seq: u64) -> StoredMessage {
    StoredMessage {
        conversation_id: conversation_id.to_string(),
        sender: my_signing.to_string(),
        body: body.to_string(),
        ts: now_millis(),
        outgoing: true,
        seq,
    }
}
