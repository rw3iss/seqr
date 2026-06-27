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

use iroh::endpoint::Connection;
use iroh::{Endpoint, NodeAddr, NodeId, PublicKey, RelayMode, SecretKey, Watcher};

use super::CoreError;

/// Application protocol identifier negotiated on every connection.
pub const ALPN: &[u8] = b"seqr/chat/0";

/// Max bytes accepted for a single frame. Must exceed one hex-encoded attachment chunk
/// (~1.05 MB for a 512 KB plaintext chunk); 4 MB gives headroom.
const MAX_FRAME: usize = 4 << 20;

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
            .alpns(vec![ALPN.to_vec()])
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
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .map_err(terr)?;
        Ok(Self { endpoint })
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
        let conn = self.endpoint.connect(addr, ALPN).await.map_err(terr)?;
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
