//! P2P transport over iroh (0.92 API).
//!
//! Our Ed25519 signing key doubles as the iroh `SecretKey`, so a peer's `NodeId` is
//! exactly their `signing_public` — no separate network address to exchange. With n0
//! discovery enabled, each endpoint publishes its address and resolves peers by id, so
//! `connect(by id)` works; the relay provides connectivity even when hole-punching
//! fails. **Without discovery, connect-by-id fails and every message falls back to the
//! mailbox poll — the cause of multi-second latency.**
//!
//! Wire protocol (one frame per connection):
//!   sender:   open_bi → write frame → finish → read ack → close
//!   receiver: accept_bi → read frame → write ack → finish → await close

use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{Endpoint, NodeAddr, NodeId, PublicKey, RelayMode, SecretKey, Watcher};

use super::CoreError;

/// ALPN for chat messages (one JSON frame per bi-stream).
pub const ALPN_CHAT: &[u8] = b"seqr/chat/0";
/// ALPN for attachment transfers (a header + length-prefixed chunks streamed over one
/// bi-stream — avoids the per-chunk reconnect that makes message-path file sending slow).
pub const ALPN_FILE: &[u8] = b"seqr/file/0";

/// Max bytes accepted for a single frame/chunk. Exceeds one chunk (~1.05 MB) with headroom.
const MAX_FRAME: usize = 4 << 20;
/// How long to wait to open a file connection before falling back to the mailbox.
const FILE_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(6);

fn terr(e: impl std::fmt::Display) -> CoreError {
    CoreError::Transport(e.to_string())
}

/// A live iroh endpoint. Cheaply cloneable (the inner endpoint is shared).
#[derive(Clone)]
pub struct Transport {
    pub endpoint: Endpoint,
}

// `start_local`/`addr`/`send`/`close` are used by tests and diagnostics.
#[allow(dead_code)]
impl Transport {
    /// Production endpoint: relay (default) + n0 discovery, so peers are reachable
    /// across the internet by id alone (direct or relayed — both real-time).
    pub async fn start(signing_secret: &[u8; 32]) -> Result<Self, CoreError> {
        let secret = SecretKey::from_bytes(signing_secret);
        let endpoint = Endpoint::builder()
            .secret_key(secret)
            .alpns(vec![ALPN_CHAT.to_vec(), ALPN_FILE.to_vec()])
            .discovery_n0()
            .bind()
            .await
            .map_err(terr)?;
        Ok(Self { endpoint })
    }

    /// Relay-free, discovery-free endpoint for local/same-LAN direct connections (tests).
    pub async fn start_local(signing_secret: &[u8; 32]) -> Result<Self, CoreError> {
        let secret = SecretKey::from_bytes(signing_secret);
        let endpoint = Endpoint::builder()
            .secret_key(secret)
            .alpns(vec![ALPN_CHAT.to_vec(), ALPN_FILE.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .map_err(terr)?;
        Ok(Self { endpoint })
    }

    /// Open a dedicated file-transfer connection + stream to a peer (by signing key).
    /// Fails fast if the peer is unreachable so the caller can fall back to the mailbox.
    pub async fn open_file_send(&self, peer: &[u8; 32]) -> Result<FileSend, CoreError> {
        let pk = PublicKey::from_bytes(peer).map_err(terr)?;
        let conn = tokio::time::timeout(
            FILE_CONNECT_TIMEOUT,
            self.endpoint.connect(NodeAddr::new(pk), ALPN_FILE),
        )
        .await
        .map_err(|_| CoreError::Transport("file connect timed out".into()))?
        .map_err(terr)?;
        let (send, recv) = conn.open_bi().await.map_err(terr)?;
        Ok(FileSend { conn, send, recv })
    }

    pub fn id(&self) -> NodeId {
        self.endpoint.node_id()
    }

    pub async fn addr(&self) -> NodeAddr {
        self.endpoint.node_addr().initialized().await
    }

    /// Send a frame to a peer (by signing public key), relying on discovery to locate
    /// them. Awaits delivery acknowledgement.
    pub async fn send_to_id(&self, peer: &[u8; 32], frame: &[u8]) -> Result<(), CoreError> {
        let pk = PublicKey::from_bytes(peer).map_err(terr)?;
        self.send(NodeAddr::new(pk), frame).await
    }

    /// Send a frame to a fully-specified address (direct dial).
    pub async fn send(&self, addr: NodeAddr, frame: &[u8]) -> Result<(), CoreError> {
        let conn = self.endpoint.connect(addr, ALPN_CHAT).await.map_err(terr)?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(terr)?;
        send.write_all(frame).await.map_err(terr)?;
        send.finish().map_err(terr)?;
        let _ack = recv.read_to_end(16).await.map_err(terr)?;
        conn.close(0u32.into(), b"done");
        Ok(())
    }

    /// Accept the next inbound connection, or `None` when the endpoint is closing.
    pub async fn accept(&self) -> Option<Connection> {
        let incoming = self.endpoint.accept().await?;
        incoming.await.ok()
    }

    pub async fn close(&self) {
        self.endpoint.close().await;
    }
}

// ---- File-transfer streaming (one connection, one bi-stream, many length-prefixed
// frames). Used for attachments to avoid per-chunk reconnect. ----

async fn write_lp(send: &mut SendStream, data: &[u8]) -> Result<(), CoreError> {
    send.write_all(&(data.len() as u32).to_be_bytes()).await.map_err(terr)?;
    send.write_all(data).await.map_err(terr)?;
    Ok(())
}

async fn read_lp(recv: &mut RecvStream) -> Result<Option<Vec<u8>>, CoreError> {
    let mut len_buf = [0u8; 4];
    // A clean end-of-stream (sender finished) surfaces as a read error here -> None.
    if recv.read_exact(&mut len_buf).await.is_err() {
        return Ok(None);
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(CoreError::Transport("file frame too long".into()));
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await.map_err(terr)?;
    Ok(Some(buf))
}

/// Sender half of a file stream. Write the header then chunks, then `finish`.
pub struct FileSend {
    conn: Connection,
    send: SendStream,
    recv: RecvStream,
}

impl FileSend {
    pub async fn write_frame(&mut self, data: &[u8]) -> Result<(), CoreError> {
        write_lp(&mut self.send, data).await
    }

    /// Finish sending and await the receiver's ack so all data flushes before close.
    pub async fn finish(mut self) -> Result<(), CoreError> {
        self.send.finish().map_err(terr)?;
        let _ack = self.recv.read_to_end(16).await.map_err(terr)?;
        self.conn.close(0u32.into(), b"done");
        Ok(())
    }
}

/// Receiver half of a file stream. Read the header then chunks until `None`.
pub struct FileRecv {
    conn: Connection,
    send: SendStream,
    recv: RecvStream,
}

impl FileRecv {
    pub async fn read_frame(&mut self) -> Result<Option<Vec<u8>>, CoreError> {
        read_lp(&mut self.recv).await
    }

    /// Acknowledge receipt and wait for the sender to close.
    pub async fn finish(mut self) -> Result<(), CoreError> {
        let _ = self.send.write_all(b"k").await;
        let _ = self.send.finish();
        self.conn.closed().await;
        Ok(())
    }
}

/// Accept the file stream on a connection whose ALPN is [`ALPN_FILE`].
pub async fn accept_file(conn: Connection) -> Result<FileRecv, CoreError> {
    let (send, recv) = conn.accept_bi().await.map_err(terr)?;
    Ok(FileRecv { conn, send, recv })
}

/// Read a single frame from an accepted connection and acknowledge it. Waits for the
/// sender's graceful close so the ack flushes before teardown.
pub async fn recv_frame(conn: &Connection) -> Result<Vec<u8>, CoreError> {
    let (mut send, mut recv) = conn.accept_bi().await.map_err(terr)?;
    let buf = recv.read_to_end(MAX_FRAME).await.map_err(terr)?;
    let _ = send.write_all(b"k").await;
    let _ = send.finish();
    conn.closed().await;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use seqr_crypto::keys::Identity;

    #[tokio::test]
    async fn two_endpoints_exchange_a_frame_directly() {
        let alice = Identity::generate();
        let bob = Identity::generate();
        let (_a, alice_secret) = alice.secret_bytes();
        let (_b, bob_secret) = bob.secret_bytes();

        let ta = Transport::start_local(&alice_secret).await.unwrap();
        let tb = Transport::start_local(&bob_secret).await.unwrap();
        assert_eq!(tb.id().as_bytes(), &bob.public().signing_public);

        let bob_addr = tb.addr().await;
        let recv_task = tokio::spawn(async move {
            let conn = tb.accept().await.expect("incoming connection");
            recv_frame(&conn).await.expect("read frame")
        });

        ta.send(bob_addr, b"hello over quic").await.unwrap();
        assert_eq!(recv_task.await.unwrap(), b"hello over quic");
    }
}
