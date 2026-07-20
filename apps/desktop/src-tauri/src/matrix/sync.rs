//! Background sync for the Matrix backend.
//!
//! Drives `client.sync()` on a spawned task. The SDK transparently decrypts and updates
//! its stores, and invokes our typed handler for each new text message, which we relay to
//! the UI as a `matrix://message` event (mirroring the P2P `seqr://message` shape so the
//! chat UI can stay largely the same).

use matrix_sdk::{
    config::SyncSettings,
    event_handler::Ctx,
    ruma::events::{
        reaction::SyncReactionEvent, room::message::SyncRoomMessageEvent,
        room::redaction::SyncRoomRedactionEvent,
    },
    Client, Room,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const MATRIX_MESSAGE_EVENT: &str = "matrix://message";
/// Emitted (with the room id) when a room's timeline changed in a way that needs a
/// re-fetch — a reaction or a redaction (delete).
pub const MATRIX_ROOM_UPDATED_EVENT: &str = "matrix://room-updated";

/// An aggregated reaction on a message.
#[derive(Serialize, Clone)]
pub struct MatrixReaction {
    pub key: String,
    pub count: u64,
    pub mine: bool,
}

/// A decrypted message, flattened for the UI. For media (`m.image`/`m.file`/…) `body` is
/// the filename/caption; the UI fetches the bytes on demand via `matrix_read_media`.
#[derive(Serialize, Clone)]
pub struct MatrixMessage {
    pub room_id: String,
    pub event_id: Option<String>,
    pub sender: String,
    pub body: String,
    /// `m.text` | `m.image` | `m.file` | `m.video` | `m.audio` | …
    pub msgtype: String,
    /// Milliseconds since the Unix epoch (origin_server_ts).
    pub ts: u64,
    pub outgoing: bool,
    /// Aggregated reactions (populated by history fetch; empty for live events).
    #[serde(default)]
    pub reactions: Vec<MatrixReaction>,
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
                let msg = MatrixMessage {
                    room_id: room.room_id().to_string(),
                    event_id: Some(orig.event_id.to_string()),
                    sender: orig.sender.to_string(),
                    body: orig.content.body().to_string(),
                    msgtype: orig.content.msgtype.msgtype().to_string(),
                    ts: u64::from(orig.origin_server_ts.get()),
                    outgoing: me.as_deref() == Some(orig.sender.as_ref()),
                    reactions: Vec::new(),
                };
                let _ = ctx.emit(MATRIX_MESSAGE_EVENT, &msg);
            }
        },
    );

    // Reactions and redactions (deletes) change a room's timeline without being new
    // messages; ping the UI to re-fetch the affected room.
    client.add_event_handler(
        |_ev: SyncReactionEvent, room: Room, ctx: Ctx<AppHandle>| async move {
            let _ = ctx.emit(MATRIX_ROOM_UPDATED_EVENT, &room.room_id().to_string());
        },
    );
    client.add_event_handler(
        |_ev: SyncRoomRedactionEvent, room: Room, ctx: Ctx<AppHandle>| async move {
            let _ = ctx.emit(MATRIX_ROOM_UPDATED_EVENT, &room.room_id().to_string());
        },
    );

    tokio::spawn(async move {
        if let Err(e) = client.sync(SyncSettings::default()).await {
            eprintln!("matrix sync loop ended: {e}");
        }
    });
}
