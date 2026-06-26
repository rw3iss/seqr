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

const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Start the transport for the unlocked account; spawn the accept and poll loops.
pub async fn start_transport(
    state: Arc<SessionState>,
    app: AppHandle,
    signing_secret: [u8; 32],
) -> Result<(), CoreError> {
    let transport = Transport::start(&signing_secret).await?;
    state.set_transport(transport.clone());
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
        eprintln!("seqr: bad recipient key {recipient_hex}");
        return;
    };
    let delivered = match state.transport() {
        Some(t) => t.send_to_id(&signing, payload).await.is_ok(),
        None => false,
    };
    if !delivered {
        let client = MailboxClient::new(&state.mailbox_url);
        let payload_str = String::from_utf8_lossy(payload).to_string();
        if let Err(e) = client.push(recipient_hex, &payload_str).await {
            eprintln!("seqr: mailbox push failed: {e}");
        }
    }
}

async fn accept_loop(transport: Transport, state: Arc<SessionState>, app: AppHandle) {
    while let Some(conn) = transport.accept().await {
        let state = state.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            match recv_frame(&conn).await {
                Ok(bytes) => emit_inbound(&state, &app, &bytes),
                Err(e) => eprintln!("seqr: recv error: {e}"),
            }
        });
    }
}

async fn poll_mailbox_loop(state: Arc<SessionState>, app: AppHandle) {
    let client = MailboxClient::new(&state.mailbox_url);
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
                let mut ids = Vec::with_capacity(messages.len());
                for m in &messages {
                    emit_inbound(&state, &app, m.payload.as_bytes());
                    ids.push(m.id.clone());
                }
                if let Err(e) = client.ack(&identity, ids).await {
                    eprintln!("seqr: mailbox ack failed: {e}");
                }
            }
            Err(e) => eprintln!("seqr: mailbox pull failed: {e}"),
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
    Nothing,
}

fn emit_inbound(state: &Arc<SessionState>, app: &AppHandle, bytes: &[u8]) {
    match process_incoming(state, bytes) {
        Ok(Inbound::Message(m)) => {
            let _ = app.emit(MESSAGE_EVENT, &m);
        }
        Ok(Inbound::GroupUpdated(id)) => {
            let _ = app.emit(GROUP_EVENT, &id);
        }
        Ok(Inbound::Nothing) => {}
        Err(e) => eprintln!("seqr: dropped inbound packet: {e}"),
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
