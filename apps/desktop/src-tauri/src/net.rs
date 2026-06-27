//! Network wiring — the bridge between transport/mailbox and the app.
//!
//! Lives outside `core` because it touches Tauri (spawning tasks and emitting events).
//! On unlock it starts the iroh endpoint, runs the accept loop (direct delivery), and
//! the mailbox poll loop (offline delivery). Every inbound `Packet` flows through
//! `process_incoming`, which dispatches chat messages (1:1 or group) and group invites.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use seqr_crypto::keys::Identity;
use seqr_crypto::SymmetricKey;

use crate::core::attachment::{self, Reassembler};
use crate::core::mailbox::MailboxClient;
use crate::core::packet::{AttachmentChunk, AttachmentMeta, GroupInvite, Packet};
use crate::core::session::SessionState;
use crate::core::transport::{accept_file, recv_frame, FileSend, Transport, ALPN_FILE};
use crate::core::vault::{AttachmentInfo, Friend, Group, StoredMessage, VaultData};
use crate::core::{conversation, group, message, now_millis, vault, CoreError};

pub const MESSAGE_EVENT: &str = "seqr://message";
pub const GROUP_EVENT: &str = "seqr://group";
pub const REQUEST_EVENT: &str = "seqr://request";
pub const PROGRESS_EVENT: &str = "seqr://attachment-progress";

/// Transfer progress for an attachment (sent or received), for the UI placeholder.
#[derive(Debug, Clone, Serialize)]
pub struct AttachmentProgress {
    pub att_id: String,
    pub conversation_id: String,
    pub filename: String,
    pub size: u64,
    pub received: u32,
    pub total: u32,
    pub outgoing: bool,
}

pub fn emit_progress(app: &AppHandle, p: AttachmentProgress) {
    let _ = app.emit(PROGRESS_EVENT, &p);
}

const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Cap a direct-delivery attempt; beyond this we assume the peer is unreachable and use
/// the mailbox. Short enough to feel responsive, long enough for a real hole-punch.
const DIRECT_TIMEOUT: Duration = Duration::from_secs(4);

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
    // Try direct, but cap the attempt so an offline peer doesn't stall the send for
    // the full QUIC/discovery timeout — fall back to the mailbox quickly.
    let direct = match state.transport() {
        Some(t) => {
            match tokio::time::timeout(DIRECT_TIMEOUT, t.send_to_id(&signing, payload)).await {
                Ok(Ok(())) => {
                    debug_log(state, format!("deliver: DIRECT ok -> {}", short(recipient_hex)));
                    true
                }
                Ok(Err(e)) => {
                    debug_log(state, format!("deliver: direct failed -> {} ({e})", short(recipient_hex)));
                    false
                }
                Err(_) => {
                    debug_log(state, format!("deliver: direct timeout -> {} (offline?)", short(recipient_hex)));
                    false
                }
            }
        }
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

/// Send an attachment to each recipient: a fast **direct file stream** when reachable
/// (one connection, all chunks), falling back to mailbox chunks when offline.
pub async fn deliver_attachment(
    state: &Arc<SessionState>,
    app: &AppHandle,
    recipients: Vec<String>,
    meta: AttachmentMeta,
    key: SymmetricKey,
    src: PathBuf,
) {
    let meta_json = Packet::AttachmentMeta(meta.clone()).to_json();
    for recipient in &recipients {
        let signing: Option<[u8; 32]> =
            hex::decode(recipient).ok().and_then(|v| v.try_into().ok());
        let Some(signing) = signing else { continue };

        let mut streamed = false;
        if let Some(t) = state.transport() {
            match t.open_file_send(&signing).await {
                Ok(mut fs) => match stream_attachment(&mut fs, &meta_json, &meta, &key, &src, app).await {
                    Ok(()) => match fs.finish().await {
                        Ok(()) => {
                            streamed = true;
                            debug_log(state, format!("file: streamed {} -> {}", meta.filename, short(recipient)));
                        }
                        Err(e) => debug_log(state, format!("file: finish failed -> {} ({e})", short(recipient))),
                    },
                    Err(e) => debug_log(state, format!("file: stream error -> {} ({e})", short(recipient))),
                },
                Err(e) => debug_log(state, format!("file: connect failed -> {} ({e}); mailbox fallback", short(recipient))),
            }
        }
        if !streamed {
            // Offline fallback: park the header + chunks in the mailbox.
            deliver(state, recipient, &meta_json).await;
            mailbox_chunks(state, recipient, &meta, &key, &src).await;
        }
    }
    emit_progress(app, progress_of(&meta, meta.chunks, true)); // final 100%
}

async fn stream_attachment(
    fs: &mut FileSend,
    meta_json: &[u8],
    meta: &AttachmentMeta,
    key: &SymmetricKey,
    src: &PathBuf,
    app: &AppHandle,
) -> Result<(), CoreError> {
    fs.write_frame(meta_json).await?;
    let mut file = std::fs::File::open(src).map_err(|e| CoreError::Storage(e.to_string()))?;
    let mut buf = vec![0u8; attachment::CHUNK_SIZE];
    for index in 0..meta.chunks {
        let filled = read_fill(&mut file, &mut buf)?;
        let ct = attachment::seal_chunk(key, &meta.att_id, index, &buf[..filled]);
        fs.write_frame(&ct).await?;
        emit_progress(app, progress_of(meta, index + 1, true));
    }
    Ok(())
}

async fn mailbox_chunks(
    state: &Arc<SessionState>,
    recipient: &str,
    meta: &AttachmentMeta,
    key: &SymmetricKey,
    src: &PathBuf,
) {
    let mut file = match std::fs::File::open(src) {
        Ok(f) => f,
        Err(e) => return debug_log(state, format!("file: reopen failed ({e})")),
    };
    let mut buf = vec![0u8; attachment::CHUNK_SIZE];
    for index in 0..meta.chunks {
        let filled = match read_fill(&mut file, &mut buf) {
            Ok(n) => n,
            Err(e) => return debug_log(state, format!("file: read failed ({e})")),
        };
        let ct = attachment::seal_chunk(key, &meta.att_id, index, &buf[..filled]);
        let chunk = AttachmentChunk { att_id: meta.att_id.clone(), index, data: hex::encode(ct) };
        deliver(state, recipient, &Packet::AttachmentChunk(chunk).to_json()).await;
    }
}

fn read_fill(file: &mut std::fs::File, buf: &mut [u8]) -> Result<usize, CoreError> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) => return Err(CoreError::Storage(e.to_string())),
        }
    }
    Ok(filled)
}

async fn accept_loop(transport: Transport, state: Arc<SessionState>, app: AppHandle) {
    while let Some(conn) = transport.accept().await {
        let state = state.clone();
        let app = app.clone();
        let is_file = conn.alpn().as_deref() == Some(ALPN_FILE);
        tauri::async_runtime::spawn(async move {
            if is_file {
                handle_file_connection(conn, state, app).await;
            } else {
                match recv_frame(&conn).await {
                    Ok(bytes) => emit_inbound(&state, &app, &bytes, "direct"),
                    Err(e) => debug_log(&state, format!("accept: recv error ({e})")),
                }
            }
        });
    }
}

/// Receive an attachment over a dedicated file stream: header, then chunks, with
/// progress events; finalize to disk and record the message.
async fn handle_file_connection(
    conn: iroh::endpoint::Connection,
    state: Arc<SessionState>,
    app: AppHandle,
) {
    let mut fr = match accept_file(conn).await {
        Ok(r) => r,
        Err(e) => return debug_log(&state, format!("file: accept failed ({e})")),
    };
    let header = match fr.read_frame().await {
        Ok(Some(h)) => h,
        _ => return debug_log(&state, "file: no header".to_string()),
    };
    let meta: AttachmentMeta = match serde_json::from_slice(&header) {
        Ok(m) => m,
        Err(e) => return debug_log(&state, format!("file: bad header ({e})")),
    };
    let mut reassembler = match make_reassembler(&state, &meta) {
        Ok(r) => r,
        Err(e) => return debug_log(&state, format!("file: rejected ({e})")),
    };
    debug_log(&state, format!("file: receiving {} ({} chunks)", meta.filename, meta.chunks));
    emit_progress(&app, progress_of(&meta, 0, false));

    for index in 0..meta.chunks {
        let chunk = match fr.read_frame().await {
            Ok(Some(c)) => c,
            _ => break,
        };
        if let Err(e) = reassembler.add_chunk(index, &chunk) {
            return debug_log(&state, format!("file: chunk {index} failed ({e})"));
        }
        emit_progress(&app, progress_of(&meta, index + 1, false));
    }
    let _ = fr.finish().await;

    let info = match reassembler.finalize() {
        Ok(i) => i,
        Err(e) => return debug_log(&state, format!("file: finalize failed ({e})")),
    };
    let stored = state.with_unlocked(|u| {
        let msg = StoredMessage {
            conversation_id: meta.conversation_id.clone(),
            sender: meta.sender.clone(),
            body: String::new(),
            ts: now_millis(),
            outgoing: false,
            seq: meta.seq,
            attachment: Some(info),
        };
        u.data.add_message(msg.clone());
        vault::save(&state.data_dir, &u.vault_key, &u.data)?;
        Ok(msg)
    });
    match stored {
        Ok(msg) => {
            debug_log(&state, format!("file: RECV complete {} from {}", meta.filename, short(&meta.sender)));
            let _ = app.emit(MESSAGE_EVENT, &msg);
        }
        Err(e) => debug_log(&state, format!("file: store failed ({e})")),
    }
}

fn progress_of(meta: &AttachmentMeta, received: u32, outgoing: bool) -> AttachmentProgress {
    AttachmentProgress {
        att_id: meta.att_id.clone(),
        conversation_id: meta.conversation_id.clone(),
        filename: meta.filename.clone(),
        size: meta.size,
        received,
        total: meta.chunks,
        outgoing,
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
            let key = conv_key_for(&u.data, &frame.conversation_id, &frame.sender, frame.epoch)?;
            let body = message::open_frame(&key, &frame)?;
            let msg = StoredMessage {
                conversation_id: frame.conversation_id.clone(),
                sender: frame.sender.clone(),
                body,
                ts: now_millis(),
                outgoing: false,
                seq: frame.seq,
                attachment: None,
            };
            u.data.add_message(msg.clone());
            vault::save(&state.data_dir, &u.vault_key, &u.data)?;
            Ok(Inbound::Message(msg))
        }),
        Packet::AttachmentMeta(meta) => handle_attachment_meta(state, meta),
        Packet::AttachmentChunk(chunk) => handle_attachment_chunk(state, chunk),
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

/// Resolve the symmetric key to open a message/attachment in a conversation: the group
/// key for a known group (sender must be a member), else the 1:1 key for the epoch
/// (sender must be a friend).
fn conv_key_for(
    data: &VaultData,
    conversation_id: &str,
    sender: &str,
    epoch: u64,
) -> Result<SymmetricKey, CoreError> {
    let me = data.identity()?;
    if let Some(g) = data.group_by_id(conversation_id) {
        if !g.members.iter().any(|m| m.signing_public == sender) {
            return Err(CoreError::UnknownSender);
        }
        data.group_key(conversation_id).ok_or(CoreError::Crypto("no group key".into()))
    } else {
        let friend = data.friend_by_signing(sender).ok_or(CoreError::UnknownSender)?.clone();
        conversation::direct_key_for_epoch(data, &me, &friend, epoch)
    }
}

/// Verify an attachment header and build a fresh reassembler (resolving the conversation
/// key). Shared by the mailbox-chunk path and the direct file-stream path.
fn make_reassembler(state: &Arc<SessionState>, meta: &AttachmentMeta) -> Result<Reassembler, CoreError> {
    if meta.size > attachment::MAX_ATTACHMENT {
        return Err(CoreError::Storage("attachment exceeds size cap".into()));
    }
    state.with_unlocked(|u| {
        let signer: [u8; 32] = hex::decode(&meta.sender)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or(CoreError::BadProfile("bad sender".into()))?;
        let sig: [u8; 64] = hex::decode(&meta.signature)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or(CoreError::BadProfile("bad signature".into()))?;
        seqr_crypto::sign::verify_raw(&signer, &meta.signing_bytes(), &sig)?;
        let key = conv_key_for(&u.data, &meta.conversation_id, &meta.sender, meta.epoch)?;
        let info = AttachmentInfo {
            id: meta.att_id.clone(),
            filename: meta.filename.clone(),
            mime: meta.mime.clone(),
            size: meta.size,
        };
        Reassembler::new(
            &state.data_dir,
            info,
            meta.conversation_id.clone(),
            meta.sender.clone(),
            meta.seq,
            key,
            meta.chunks,
        )
    })
}

fn handle_attachment_meta(
    state: &Arc<SessionState>,
    meta: AttachmentMeta,
) -> Result<Inbound, CoreError> {
    // Already reassembling this one? Ignore the duplicate header.
    if state.reassembly.lock().expect("reassembly mutex").contains_key(&meta.att_id) {
        return Ok(Inbound::Nothing);
    }
    let reassembler = make_reassembler(state, &meta)?;
    state.reassembly.lock().expect("reassembly mutex").insert(meta.att_id, reassembler);
    Ok(Inbound::Nothing)
}

fn handle_attachment_chunk(
    state: &Arc<SessionState>,
    chunk: AttachmentChunk,
) -> Result<Inbound, CoreError> {
    let bytes = hex::decode(&chunk.data).map_err(|_| CoreError::Crypto("bad chunk hex".into()))?;

    // Add the chunk; if it completes the attachment, take the reassembler out.
    let completed = {
        let mut map = state.reassembly.lock().expect("reassembly mutex");
        match map.get_mut(&chunk.att_id) {
            Some(r) => {
                if r.add_chunk(chunk.index, &bytes)? {
                    map.remove(&chunk.att_id)
                } else {
                    None
                }
            }
            None => return Ok(Inbound::Nothing), // no header yet, or already finished
        }
    };

    let Some(reassembler) = completed else {
        return Ok(Inbound::Nothing);
    };

    // Finalize to disk and record the message.
    let conversation_id = reassembler.conversation_id.clone();
    let sender = reassembler.sender.clone();
    let seq = reassembler.seq;
    let info = reassembler.finalize()?;

    let msg = state.with_unlocked(|u| {
        let msg = StoredMessage {
            conversation_id,
            sender,
            body: String::new(),
            ts: now_millis(),
            outgoing: false,
            seq,
            attachment: Some(info),
        };
        u.data.add_message(msg.clone());
        vault::save(&state.data_dir, &u.vault_key, &u.data)?;
        Ok(msg)
    })?;
    Ok(Inbound::Message(msg))
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
