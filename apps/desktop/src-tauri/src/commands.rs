//! Tauri IPC commands — the thin seam between the Preact UI and the core.
//!
//! Each command maps a UI intent to core modules and returns serializable data or a
//! `CoreError` (rendered to a friendly string for the UI). No business logic lives
//! here; that belongs in the core modules and `net`.

use std::io::Read;
use std::sync::Arc;

use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, State};

use seqr_protocol::ProfileBlob;

use crate::core::config::AppConfig;
use crate::core::mailbox::MailboxClient;
use crate::core::packet::{AttachmentChunk, AttachmentMeta, GroupInvite, Packet};
use crate::core::session::SessionState;
use crate::core::vault::{AttachmentInfo, Friend, Group, Settings, StoredMessage};
use crate::core::{
    attachment, conversation, group, identity, message, now_millis, vault, CoreError, CoreResult,
};
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

/// Read the account's settings.
#[tauri::command]
pub fn get_settings(state: Session) -> CoreResult<Settings> {
    state.with_unlocked(|u| Ok(u.data.settings.clone()))
}

/// Update the account's settings.
#[tauri::command]
pub fn set_settings(settings: Settings, state: Session) -> CoreResult<()> {
    let data_dir = state.data_dir.clone();
    state.with_unlocked(|u| {
        u.data.settings = settings;
        vault::save(&data_dir, &u.vault_key, &u.data)
    })
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

/// Which of the given identities are currently online (polling the mailbox).
#[tauri::command]
pub async fn presence(ids: Vec<String>, state: Session<'_>) -> CoreResult<Vec<String>> {
    let client = MailboxClient::new(&state.mailbox_url, state.mailbox_cert.as_deref());
    client.presence(ids).await
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
            let msg = stored_outgoing(&conv_id, &my_signing, &body, seq, None);
            u.data.add_message(msg.clone());
            vault::save(&data_dir, &u.vault_key, &u.data)?;
            Ok((msg, Packet::Message(frame).to_json()))
        })?
    };
    net::debug_log(
        state.inner(),
        format!("SEND 1:1 -> {} seq={}", &friend.chars().take(8).collect::<String>(), stored.seq),
    );
    // Deliver in the background so the composer never blocks (offline sends just queue
    // in the mailbox; you can keep typing/sending).
    let st = Arc::clone(state.inner());
    tauri::async_runtime::spawn(async move {
        net::deliver(&st, &friend, &packet_json).await;
    });
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
            let msg = stored_outgoing(&group_id, &my_signing, &body, seq, None);
            u.data.add_message(msg.clone());
            vault::save(&data_dir, &u.vault_key, &u.data)?;
            let recipients: Vec<String> = g.members.iter().map(|m| m.signing_public.clone()).collect();
            Ok((msg, Packet::Message(frame).to_json(), recipients))
        })?
    };
    // Fan out in the background so the composer stays responsive.
    let st = Arc::clone(state.inner());
    tauri::async_runtime::spawn(async move {
        for recipient in &recipients {
            net::deliver(&st, recipient, &packet_json).await;
        }
    });
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

/// Resolve a conversation id to (symmetric key, epoch, recipient signing keys) for
/// sending. Works for both groups and 1:1 (matched by deterministic conversation id).
fn resolve_send_ctx(
    data: &vault::VaultData,
    conversation_id: &str,
    my_signing: &str,
) -> CoreResult<(seqr_crypto::SymmetricKey, u64, Vec<String>)> {
    if let Some(g) = data.group_by_id(conversation_id) {
        let key = data.group_key(conversation_id).ok_or(CoreError::Crypto("no group key".into()))?;
        let recipients = g.members.iter().map(|m| m.signing_public.clone()).collect();
        Ok((key, g.epoch, recipients))
    } else {
        let me = data.identity()?;
        let friend = data
            .friends
            .iter()
            .find(|f| conversation::direct_conversation_id(my_signing, &f.signing_public) == conversation_id)
            .cloned()
            .ok_or(CoreError::UnknownSender)?;
        let (epoch, key) = conversation::current_direct_key(data, &me, &friend)?;
        Ok((key, epoch, vec![friend.signing_public]))
    }
}

fn valid_att_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Encrypt and send a file attachment to a conversation: announce it with a signed
/// AttachmentMeta, then stream encrypted chunks (direct or mailbox). Stores the file
/// locally so the sender sees their own attachment.
#[tauri::command]
pub async fn send_attachment(
    conversation_id: String,
    path: String,
    state: Session<'_>,
) -> CoreResult<StoredMessage> {
    let src = std::path::PathBuf::from(&path);
    let fs_meta = std::fs::metadata(&src).map_err(|e| CoreError::Storage(e.to_string()))?;
    let size = fs_meta.len();
    if size > attachment::MAX_ATTACHMENT {
        return Err(CoreError::Storage("file exceeds the 1 GB limit".into()));
    }
    let filename = src.file_name().and_then(|s| s.to_str()).unwrap_or("file").to_string();
    let mime = attachment::guess_mime(&filename);
    let att_id = attachment::new_attachment_id();
    let chunks = attachment::chunk_count(size);

    // Phase 1 (locked): build+sign meta, persist the outgoing message.
    let (stored, meta_json, recipients, key) = {
        let data_dir = state.data_dir.clone();
        state.with_unlocked(|u| {
            let me = u.data.identity()?;
            let my_signing = hex::encode(me.public().signing_public);
            let (key, epoch, recipients) = resolve_send_ctx(&u.data, &conversation_id, &my_signing)?;
            let seq = u.data.next_seq(&conversation_id);
            let mut meta = AttachmentMeta {
                att_id: att_id.clone(),
                conversation_id: conversation_id.clone(),
                epoch,
                sender: my_signing.clone(),
                seq,
                filename: filename.clone(),
                mime: mime.clone(),
                size,
                chunks,
                signature: String::new(),
            };
            meta.signature =
                hex::encode(seqr_crypto::sign::sign(&me.signing_key, &meta.signing_bytes()));
            let info = AttachmentInfo {
                id: att_id.clone(),
                filename: filename.clone(),
                mime: mime.clone(),
                size,
            };
            let msg = stored_outgoing(&conversation_id, &my_signing, "", seq, Some(info));
            u.data.add_message(msg.clone());
            vault::save(&data_dir, &u.vault_key, &u.data)?;
            Ok((msg, Packet::AttachmentMeta(meta).to_json(), recipients, key))
        })?
    };

    // Keep our own copy so the sender's UI can display it immediately (done before we
    // return, so read_attachment succeeds right away).
    std::fs::create_dir_all(attachment::attachments_dir(&state.data_dir)).ok();
    std::fs::copy(&src, attachment::attachment_path(&state.data_dir, &att_id))
        .map_err(|e| CoreError::Storage(e.to_string()))?;

    // Deliver in the background so the UI shows the message at once (and a slow or
    // offline transfer never blocks the composer).
    let st = Arc::clone(state.inner());
    tauri::async_runtime::spawn(async move {
        for r in &recipients {
            net::deliver(&st, r, &meta_json).await;
        }
        let mut file = match std::fs::File::open(&src) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("seqr: attachment read failed: {e}");
                return;
            }
        };
        let mut buf = vec![0u8; attachment::CHUNK_SIZE];
        for index in 0..chunks {
            let mut filled = 0usize;
            while filled < buf.len() {
                match file.read(&mut buf[filled..]) {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(e) => {
                        eprintln!("seqr: attachment read error: {e}");
                        return;
                    }
                }
            }
            let ct = attachment::seal_chunk(&key, &att_id, index, &buf[..filled]);
            let chunk = AttachmentChunk { att_id: att_id.clone(), index, data: hex::encode(ct) };
            let cj = Packet::AttachmentChunk(chunk).to_json();
            for r in &recipients {
                net::deliver(&st, r, &cj).await;
            }
        }
    });
    Ok(stored)
}

/// Return a small attachment as a `data:` URL for inline display (images). Refuses
/// files larger than 16 MB — use `open_attachment` for those.
#[tauri::command]
pub fn read_attachment(att_id: String, state: Session) -> CoreResult<String> {
    if !valid_att_id(&att_id) {
        return Err(CoreError::Storage("bad attachment id".into()));
    }
    let path = attachment::attachment_path(&state.data_dir, &att_id);
    let fs_meta = std::fs::metadata(&path).map_err(|e| CoreError::Storage(e.to_string()))?;
    if fs_meta.len() > 16 * 1024 * 1024 {
        return Err(CoreError::Storage("too large to preview inline".into()));
    }
    let bytes = std::fs::read(&path).map_err(|e| CoreError::Storage(e.to_string()))?;
    let mime = state.with_unlocked(|u| {
        Ok(u.data
            .messages
            .iter()
            .find_map(|m| m.attachment.as_ref().filter(|a| a.id == att_id).map(|a| a.mime.clone()))
            .unwrap_or_else(|| "application/octet-stream".to_string()))
    })?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{b64}"))
}

/// Copy an attachment's bytes to a destination path the user chose (download).
#[tauri::command]
pub fn save_attachment(att_id: String, dest: String, state: Session) -> CoreResult<()> {
    if !valid_att_id(&att_id) {
        return Err(CoreError::Storage("bad attachment id".into()));
    }
    let src = attachment::attachment_path(&state.data_dir, &att_id);
    std::fs::copy(&src, &dest).map_err(|e| CoreError::Storage(e.to_string()))?;
    Ok(())
}

/// Open an attachment with the OS default application.
#[tauri::command]
pub fn open_attachment(att_id: String, state: Session, app: AppHandle) -> CoreResult<()> {
    use tauri_plugin_opener::OpenerExt;
    if !valid_att_id(&att_id) {
        return Err(CoreError::Storage("bad attachment id".into()));
    }
    let path = attachment::attachment_path(&state.data_dir, &att_id);
    app.opener()
        .open_path(path.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| CoreError::Storage(e.to_string()))?;
    Ok(())
}

fn stored_outgoing(
    conversation_id: &str,
    my_signing: &str,
    body: &str,
    seq: u64,
    attachment: Option<AttachmentInfo>,
) -> StoredMessage {
    StoredMessage {
        conversation_id: conversation_id.to_string(),
        sender: my_signing.to_string(),
        body: body.to_string(),
        ts: now_millis(),
        outgoing: true,
        seq,
        attachment,
    }
}
