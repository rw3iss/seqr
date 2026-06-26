//! P2P transport over iroh (0.92 API).
//!
//! Our Ed25519 signing key doubles as the iroh `SecretKey`, so a peer's `NodeId` is
//! exactly their `signing_public` — no separate network address to exchange. The
//! endpoint hole-punches a direct QUIC connection when possible and falls back to a
//! relay; in both cases only ciphertext frames travel.
//!
//! Wire protocol (one frame per bi-directional stream):
//!   sender:   open_bi → write frame bytes → finish → read ack → close
//!   receiver: accept_bi → read frame bytes → write ack → finish
//! The ack lets the sender confirm delivery before closing.

use iroh::endpoint::Connection;
use iroh::{Endpoint, NodeAddr, NodeId, PublicKey, RelayMode, SecretKey, Watcher};

use super::CoreError;

/// Application protocol identifier negotiated on every connection.
pub const ALPN: &[u8] = b"seqr/chat/0";

/// Max bytes accepted for a single frame (generous; a chat message is tiny).
const MAX_FRAME: usize = 1 << 20;

fn terr(e: impl std::fmt::Display) -> CoreError {
    CoreError::Transport(e.to_string())
}

/// A live iroh endpoint. Cheaply cloneable (the inner endpoint is shared).
#[derive(Clone)]
pub struct Transport {
    pub endpoint: Endpoint,
}

// `start_local`/`id`/`addr`/`close` are part of the transport API and exercised by
// tests and diagnostics; they aren't all on the app's hot path yet.
#[allow(dead_code)]
impl Transport {
    /// Start the endpoint for production: relay + discovery enabled so peers are
    /// reachable across the internet by id alone.
    pub async fn start(signing_secret: &[u8; 32]) -> Result<Self, CoreError> {
        let secret = SecretKey::from_bytes(signing_secret);
        let endpoint = Endpoint::builder()
            .secret_key(secret)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .map_err(terr)?;
        Ok(Self { endpoint })
    }

    /// Start a relay-free endpoint for local/same-LAN direct connections (and tests).
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

    /// This endpoint's id (equals our signing public key).
    pub fn id(&self) -> NodeId {
        self.endpoint.node_id()
    }

    /// Our current dialable address (waits until at least one address is known).
    pub async fn addr(&self) -> NodeAddr {
        self.endpoint.node_addr().initialized().await
    }

    /// Send a frame to a peer identified by their signing public key, relying on
    /// discovery to locate them. Awaits delivery acknowledgement.
    pub async fn send_to_id(&self, peer_signing: &[u8; 32], frame: &[u8]) -> Result<(), CoreError> {
        let peer = PublicKey::from_bytes(peer_signing).map_err(terr)?;
        self.send(NodeAddr::new(peer), frame).await
    }

    /// Send a frame to a fully-specified address (direct dial).
    pub async fn send(&self, addr: NodeAddr, frame: &[u8]) -> Result<(), CoreError> {
        let conn = self.endpoint.connect(addr, ALPN).await.map_err(terr)?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(terr)?;
        send.write_all(frame).await.map_err(terr)?;
        send.finish().map_err(terr)?;
        // Wait for the receiver's ack so we know the frame landed.
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

/// Read a single frame from an accepted connection and acknowledge it.
///
/// After sending the ack we wait for the sender to close the connection gracefully,
/// which guarantees the ack has flushed before this task (and its endpoint) tears
/// down — otherwise the sender would see "connection lost" while awaiting the ack.
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

        // Bob's endpoint id must equal his signing public key.
        assert_eq!(tb.id().as_bytes(), &bob.public().signing_public);

        let bob_addr = tb.addr().await;

        // Bob accepts one connection and captures the frame.
        let recv_task = tokio::spawn(async move {
            let conn = tb.accept().await.expect("incoming connection");
            recv_frame(&conn).await.expect("read frame")
        });

        // Alice dials Bob directly and sends.
        ta.send(bob_addr, b"hello over quic").await.unwrap();

        let received = recv_task.await.unwrap();
        assert_eq!(received, b"hello over quic");
    }
}
