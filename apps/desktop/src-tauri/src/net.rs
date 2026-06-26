//! Network wiring — the bridge between transport/mailbox and the app.
//!
//! Lives outside `core` because it touches Tauri (spawning tasks and emitting events).
//! On unlock it starts the iroh endpoint, runs the accept loop (direct delivery), and
//! the mailbox poll loop (offline delivery). Every inbound `Packet` flows through
//! `process_incoming`, which dispatches chat messages (1:1 or group) and group invites.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use seqr_crypto::keys::Identity;

use crate::core::mailbox::MailboxClient;
use crate::core::packet::{GroupInvite, Packet};
use crate::core::session::SessionState;
use crate::core::transport::{recv_frame, Transport};
use crate::core::vault::{Friend, Group, StoredMessage};
use crate::core::{conversation, group, message, now_millis, vault, CoreError};

pub const MESSAGE_EVENT: &str = "seqr://message";
pub const GROUP_EVENT: &str = "seqr://group";
pub const REQUEST_EVENT: &str = "seqr://request";

const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// First 8 hex chars, for compact logging.
fn short(hex: &str) -> String {
    hex.chars().take(8).collect()
}

/// Emit a diagnostic line to stderr AND the mailbox's central debug log, tagged with
/// this account's identity so both machines are distinguishable. Best-effort.
pub fn debug_log(state: &Arc<SessionState>, msg: impl Into<String>) {
    let msg = msg.into();
    let tag = state
        .with_unlocked(|u| {
            let p = u.data.identity()?.public().signing_public;
            Ok(format!("{}/{}", u.data.display_name, short(&hex::encode(p))))
        })
        .unwrap_or_else(|_| "?".to_string());
    eprintln!("seqr[{tag}]: {msg}");
    let (url, cert) = (state.mailbox_url.clone(), state.mailbox_cert.clone());
    tauri::async_runtime::spawn(async move {
        let client = MailboxClient::new(&url, cert.as_deref());
        let _ = client.debug(&tag, &msg).await;
    });
}

/// Start the transport for the unlocked account; spawn the accept and poll loops.
pub async fn start_transport(
    state: Arc<SessionState>,
    app: AppHandle,
    signing_secret: [u8; 32],
) -> Result<(), CoreError> {
    let transport = Transport::start(&signing_secret).await?;
    let endpoint_id = hex::encode(transport.id().as_bytes());
    state.set_transport(transport.clone());
    debug_log(&state, format!("transport up, endpoint_id={}", short(&endpoint_id)));
    tauri::async_runtime::spawn(accept_loop(transport, state.clone(), app.clone()));
    tauri::async_runtime::spawn(poll_mailbox_loop(state, app));
    Ok(())
}

/// Try direct QUIC delivery to a recipient (by signing public key, hex); on failure
/// park the (already-sealed) packet in the mailbox for offline delivery.
pub async fn deliver(state: &Arc<SessionState>, recipient_hex: &str, payload: &[u8]) {
    let signing: Option<[u8; 32]> =
        hex::decode(recipient_hex).ok().and_then(|v| v.try_into().ok());
    let Some(signing) = signing else {
        debug_log(state, format!("deliver ABORT: bad recipient {}", short(recipient_hex)));
        return;
    };
    let direct = match state.transport() {
        Some(t) => match t.send_to_id(&signing, payload).await {
            Ok(()) => {
                debug_log(state, format!("deliver: DIRECT ok -> {}", short(recipient_hex)));
                true
            }
            Err(e) => {
                debug_log(state, format!("deliver: direct failed -> {} ({e})", short(recipient_hex)));
                false
            }
        },
        None => false,
    };
    if !direct {
        let client = MailboxClient::new(&state.mailbox_url, state.mailbox_cert.as_deref());
        let payload_str = String::from_utf8_lossy(payload).to_string();
        match client.push(recipient_hex, &payload_str).await {
            Ok(id) => debug_log(state, format!("deliver: MAILBOX push ok -> {} (id={id})", short(recipient_hex))),
            Err(e) => debug_log(state, format!("deliver: mailbox push FAILED -> {} ({e})", short(recipient_hex))),
        }
    }
}

async fn accept_loop(transport: Transport, state: Arc<SessionState>, app: AppHandle) {
    while let Some(conn) = transport.accept().await {
        let state = state.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            match recv_frame(&conn).await {
                Ok(bytes) => emit_inbound(&state, &app, &bytes, "direct"),
                Err(e) => debug_log(&state, format!("accept: recv error ({e})")),
            }
        });
    }
}

async fn poll_mailbox_loop(state: Arc<SessionState>, app: AppHandle) {
    let client = MailboxClient::new(&state.mailbox_url, state.mailbox_cert.as_deref());
    loop {
        if !state.is_unlocked() {
            break;
        }
        let identity = match current_identity(&state) {
            Some(id) => id,
            None => break,
        };
        match client.pull(&identity).await {
            Ok(messages) => {
                if !messages.is_empty() {
                    debug_log(&state, format!("poll: pulled {} from mailbox", messages.len()));
                }
                let mut ids = Vec::with_capacity(messages.len());
                for m in &messages {
                    emit_inbound(&state, &app, m.payload.as_bytes(), "mailbox");
                    ids.push(m.id.clone());
                }
                if let Err(e) = client.ack(&identity, ids).await {
                    debug_log(&state, format!("poll: ack failed ({e})"));
                }
            }
            Err(e) => debug_log(&state, format!("poll: pull failed ({e})")),
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn current_identity(state: &Arc<SessionState>) -> Option<Identity> {
    state
        .with_unlocked(|u| {
            let (a, s) = u.data.identity()?.secret_bytes();
            Ok((a, s))
        })
        .ok()
        .and_then(|(a, s)| Identity::from_secret_bytes(&a, &s).ok())
}

/// What an inbound packet produced, for the UI to be notified about.
enum Inbound {
    Message(StoredMessage),
    GroupUpdated(String),
    FriendRequested(String),
    Nothing,
}

fn emit_inbound(state: &Arc<SessionState>, app: &AppHandle, bytes: &[u8], source: &str) {
    match process_incoming(state, bytes) {
        Ok(Inbound::Message(m)) => {
            debug_log(
                state,
                format!("RECV[{source}] msg from {} conv={}", short(&m.sender), short(&m.conversation_id)),
            );
            let _ = app.emit(MESSAGE_EVENT, &m);
        }
        Ok(Inbound::GroupUpdated(id)) => {
            debug_log(state, format!("RECV[{source}] group update {}", short(&id)));
            let _ = app.emit(GROUP_EVENT, &id);
        }
        Ok(Inbound::FriendRequested(who)) => {
            debug_log(state, format!("RECV[{source}] friend request from {}", short(&who)));
            let _ = app.emit(REQUEST_EVENT, &who);
        }
        Ok(Inbound::Nothing) => {
            debug_log(state, format!("RECV[{source}] duplicate/ignored"));
        }
        Err(e) => debug_log(state, format!("RECV[{source}] DROPPED: {e}")),
    }
}

fn process_incoming(state: &Arc<SessionState>, bytes: &[u8]) -> Result<Inbound, CoreError> {
    let packet: Packet =
        serde_json::from_slice(bytes).map_err(|e| CoreError::BadProfile(e.to_string()))?;
    match packet {
        Packet::Message(frame) => state.with_unlocked(|u| {
            // Dedup (a message may arrive both directly and via the mailbox).
            if u.data.has_incoming(&frame.conversation_id, &frame.sender, frame.seq) {
                return Ok(Inbound::Nothing);
            }
            let me = u.data.identity()?;

            // Group message if we know the group; otherwise a 1:1.
            let key = if let Some(g) = u.data.group_by_id(&frame.conversation_id) {
                if !g.members.iter().any(|m| m.signing_public == frame.sender) {
                    return Err(CoreError::UnknownSender);
                }
                u.data.group_key(&frame.conversation_id)
                    .ok_or(CoreError::Crypto("no group key".into()))?
            } else {
                let friend = u
                    .data
                    .friend_by_signing(&frame.sender)
                    .ok_or(CoreError::UnknownSender)?
                    .clone();
                conversation::direct_key_for_epoch(&u.data, &me, &friend, frame.epoch)?
            };

            let body = message::open_frame(&key, &frame)?;
            let msg = StoredMessage {
                conversation_id: frame.conversation_id.clone(),
                sender: frame.sender.clone(),
                body,
                ts: now_millis(),
                outgoing: false,
                seq: frame.seq,
            };
            u.data.add_message(msg.clone());
            vault::save(&state.data_dir, &u.vault_key, &u.data)?;
            Ok(Inbound::Message(msg))
        }),
        Packet::GroupInvite(invite) => handle_invite(state, invite),
        Packet::FriendRequest(req) => state.with_unlocked(|u| {
            // Verify the profile signature (binds agreement key to the signing key).
            let sig: [u8; 64] = hex::decode(&req.signature)
                .ok()
                .and_then(|v| v.try_into().ok())
                .ok_or(CoreError::BadProfile("bad request signature".into()))?;
            let signer: [u8; 32] = hex::decode(&req.profile.signing_public)
                .ok()
                .and_then(|v| v.try_into().ok())
                .ok_or(CoreError::BadProfile("bad signing key".into()))?;
            let bytes = crate::core::packet::friend_req_signing_bytes(&req.profile);
            seqr_crypto::sign::verify_raw(&signer, &bytes, &sig)?;

            let friend = crate::core::identity::friend_from(&req.profile);
            let who = friend.signing_public.clone();
            if u.data.add_pending(friend) {
                vault::save(&state.data_dir, &u.vault_key, &u.data)?;
                Ok(Inbound::FriendRequested(who))
            } else {
                Ok(Inbound::Nothing) // already a friend or already pending
            }
        }),
        Packet::KeyUpdate(ku) => state.with_unlocked(|u| {
            let me = u.data.identity()?;
            // The originator must be a known friend (we share a long-term pairwise key).
            let originator = u
                .data
                .friend_by_signing(&ku.originator)
                .ok_or(CoreError::UnknownSender)?
                .clone();
            let key = conversation::open_direct_key(
                &me,
                &originator,
                &ku.conversation_id,
                ku.epoch,
                &ku.sealed_key,
            )?;
            u.data.set_direct_key(&ku.conversation_id, ku.epoch, hex::encode(key));
            vault::save(&state.data_dir, &u.vault_key, &u.data)?;
            Ok(Inbound::Nothing)
        }),
    }
}

fn handle_invite(state: &Arc<SessionState>, invite: GroupInvite) -> Result<Inbound, CoreError> {
    state.with_unlocked(|u| {
        let me = u.data.identity()?;
        let my_signing = hex::encode(me.public().signing_public);

        // The originator's public keys come from the invite roster (self-contained).
        let originator = invite
            .members
            .iter()
            .find(|m| m.signing_public == invite.originator)
            .cloned()
            .ok_or(CoreError::BadProfile("originator not in roster".into()))?;
        let kg = group::open_group_key(
            &me,
            &originator,
            &invite.group_id,
            invite.epoch,
            &invite.sealed_key,
        )?;

        // Our roster view: everyone but us.
        let members: Vec<Friend> =
            invite.members.into_iter().filter(|m| m.signing_public != my_signing).collect();

        match u.data.group_by_id_mut(&invite.group_id) {
            Some(g) => {
                // Accept only a newer (or equal) epoch.
                if invite.epoch >= g.epoch {
                    g.name = invite.name;
                    g.members = members;
                    g.epoch = invite.epoch;
                    g.key = hex::encode(kg);
                }
            }
            None => u.data.groups.push(Group {
                id: invite.group_id.clone(),
                name: invite.name,
                members,
                epoch: invite.epoch,
                key: hex::encode(kg),
            }),
        }
        vault::save(&state.data_dir, &u.vault_key, &u.data)?;
        Ok(Inbound::GroupUpdated(invite.group_id))
    })
}
