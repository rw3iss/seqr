//! Network wiring — the bridge between the iroh transport and the app.
//!
//! Lives outside `core` because it touches Tauri (spawning tasks and emitting events).
//! Starts the endpoint on unlock and runs the accept loop: each inbound frame is
//! decrypted and verified against the friends roster, stored in the vault, and pushed
//! to the UI as a `seqr://message` event.

use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use seqr_protocol::MessageFrame;

use crate::core::session::SessionState;
use crate::core::transport::{recv_frame, Transport};
use crate::core::vault::StoredMessage;
use crate::core::{conversation, message, now_millis, vault, CoreError};

/// Event name the UI listens on for incoming messages.
pub const MESSAGE_EVENT: &str = "seqr://message";

/// Start the transport for the unlocked account and spawn the accept loop.
pub async fn start_transport(
    state: Arc<SessionState>,
    app: AppHandle,
    signing_secret: [u8; 32],
) -> Result<(), CoreError> {
    let transport = Transport::start(&signing_secret).await?;
    state.set_transport(transport.clone());
    tauri::async_runtime::spawn(accept_loop(transport, state, app));
    Ok(())
}

async fn accept_loop(transport: Transport, state: Arc<SessionState>, app: AppHandle) {
    while let Some(conn) = transport.accept().await {
        let state = state.clone();
        let app = app.clone();
        // Handle each connection concurrently; one frame per connection.
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

/// Decrypt, verify, persist, and surface one inbound frame. Returns an error (and the
/// frame is dropped) if the sender is unknown or the message fails authentication.
fn process_incoming(
    state: &Arc<SessionState>,
    app: &AppHandle,
    bytes: &[u8],
) -> Result<(), CoreError> {
    let frame: MessageFrame =
        serde_json::from_slice(bytes).map_err(|e| CoreError::BadProfile(e.to_string()))?;

    let stored = state.with_unlocked(|u| {
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
        };
        u.data.add_message(msg.clone());
        vault::save(&state.data_dir, &u.vault_key, &u.data)?;
        Ok(msg)
    })?;

    let _ = app.emit(MESSAGE_EVENT, &stored);
    Ok(())
}
