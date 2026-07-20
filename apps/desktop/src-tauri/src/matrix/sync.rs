//! Background sync for the Matrix backend.
//!
//! Drives `client.sync()` on a spawned task. The SDK transparently decrypts and updates
//! its stores, and invokes our typed handler for each new text message, which we relay to
//! the UI as a `matrix://message` event (mirroring the P2P `seqr://message` shape so the
//! chat UI can stay largely the same).

use matrix_sdk::{
    config::SyncSettings,
    event_handler::Ctx,
    ruma::events::room::message::{MessageType, SyncRoomMessageEvent},
    Client, Room,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const MATRIX_MESSAGE_EVENT: &str = "matrix://message";

/// A decrypted text message, flattened for the UI.
#[derive(Serialize, Clone)]
pub struct MatrixMessage {
    pub room_id: String,
    pub event_id: Option<String>,
    pub sender: String,
    pub body: String,
    /// Milliseconds since the Unix epoch (origin_server_ts).
    pub ts: u64,
    pub outgoing: bool,
}

/// Spawn the sync loop and register the live message handler. Idempotency is the caller's
/// concern (guard with a flag so this runs once per login).
pub fn start(client: Client, app: AppHandle) {
    client.add_event_handler_context(app);

    let me = client.user_id().map(|u| u.to_owned());
    client.add_event_handler(
        move |ev: SyncRoomMessageEvent, room: Room, ctx: Ctx<AppHandle>| {
            let me = me.clone();
            async move {
                let Some(orig) = ev.as_original() else { return };
                let MessageType::Text(text) = &orig.content.msgtype else { return };
                let msg = MatrixMessage {
                    room_id: room.room_id().to_string(),
                    event_id: Some(orig.event_id.to_string()),
                    sender: orig.sender.to_string(),
                    body: text.body.clone(),
                    ts: u64::from(orig.origin_server_ts.get()),
                    outgoing: me.as_deref() == Some(orig.sender.as_ref()),
                };
                let _ = ctx.emit(MATRIX_MESSAGE_EVENT, &msg);
            }
        },
    );

    tokio::spawn(async move {
        if let Err(e) = client.sync(SyncSettings::default()).await {
            eprintln!("matrix sync loop ended: {e}");
        }
    });
}
