//! Network wiring — the bridge between transport/mailbox and the app.
//!
//! Lives outside `core` because it touches Tauri (spawning tasks and emitting events).
//! On unlock it starts the iroh endpoint, runs the accept loop (direct delivery), and
//! runs the mailbox poll loop (offline delivery). Every inbound frame — whether direct
//! or pulled from the mailbox — flows through `process_incoming`: decrypt, verify
//! against the roster, deduplicate, store, and emit a `seqr://message` event.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use seqr_crypto::keys::Identity;
use seqr_protocol::MessageFrame;

use crate::core::mailbox::MailboxClient;
use crate::core::session::SessionState;
use crate::core::transport::{recv_frame, Transport};
use crate::core::vault::StoredMessage;
use crate::core::{conversation, message, now_millis, vault, CoreError};

/// Event name the UI listens on for incoming messages.
pub const MESSAGE_EVENT: &str = "seqr://message";

/// How often to poll the mailbox for offline-delivered messages.
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

async fn accept_loop(transport: Transport, state: Arc<SessionState>, app: AppHandle) {
    while let Some(conn) = transport.accept().await {
        let state = state.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            match recv_frame(&conn).await {
                Ok(bytes) => {
                    if let Err(e) = process_incoming(&state, &app, &bytes) {
                        eprintln!("seqr: dropped inbound frame: {e}");
                    }
                }
                Err(e) => eprintln!("seqr: recv error: {e}"),
            }
        });
    }
}

/// Periodically fetch parked messages from the mailbox until the account is locked.
async fn poll_mailbox_loop(state: Arc<SessionState>, app: AppHandle) {
    let client = MailboxClient::new(&state.mailbox_url);
    loop {
        if !state.is_unlocked() {
            break;
        }
        // Reconstruct the identity from the vault (don't hold the lock across awaits).
        let identity = match current_identity(&state) {
            Some(id) => id,
            None => break,
        };

        match client.pull(&identity).await {
            Ok(messages) => {
                let mut ids = Vec::with_capacity(messages.len());
                for m in &messages {
                    // Drop frames we can't process (e.g. unknown sender) but still ack
                    // them, so the mailbox doesn't redeliver forever.
                    if let Err(e) = process_incoming(&state, &app, m.payload.as_bytes()) {
                        eprintln!("seqr: dropped mailbox frame {}: {e}", m.id);
                    }
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

/// Decrypt, verify, dedupe, persist, and surface one inbound frame.
fn process_incoming(
    state: &Arc<SessionState>,
    app: &AppHandle,
    bytes: &[u8],
) -> Result<(), CoreError> {
    let frame: MessageFrame =
        serde_json::from_slice(bytes).map_err(|e| CoreError::BadProfile(e.to_string()))?;

    let stored = state.with_unlocked(|u| {
        // Ignore duplicates (a message may arrive both directly and via the mailbox).
        if u.data.has_incoming(&frame.conversation_id, &frame.sender, frame.seq) {
            return Ok(None);
        }
        let me = u.data.identity()?;
        let friend = u
            .data
            .friend_by_signing(&frame.sender)
            .ok_or(CoreError::UnknownSender)?
            .clone();
        let key = conversation::pairwise_key(&me, &friend)?;
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
        Ok(Some(msg))
    })?;

    if let Some(msg) = stored {
        let _ = app.emit(MESSAGE_EVENT, &msg);
    }
    Ok(())
}
