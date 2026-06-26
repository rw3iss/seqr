//! Mailbox HTTP client — the desktop side of the store-and-forward helper.
//!
//! Used when a peer is offline: the sender `push`es ciphertext for the recipient, and
//! every client periodically `pull`s its own parked messages and `ack`s them once
//! stored. Pull/ack are authenticated by signing a challenge with the account's
//! Ed25519 key (the same key that is the iroh/identity public key), so only the owner
//! can fetch their mail. The helper sees ciphertext only.

use reqwest::{Certificate, Client};

use seqr_crypto::keys::Identity;
use seqr_crypto::sign::sign;
use seqr_protocol::mailbox::{
    AckRequest, LogRequest, PresenceRequest, PresenceResponse, PullRequest, PullResponse,
    PulledMessage, PushRequest, PushResponse,
};

use super::{now_millis, CoreError};

fn merr(e: impl std::fmt::Display) -> CoreError {
    CoreError::Transport(format!("mailbox: {e}"))
}

pub struct MailboxClient {
    base: String,
    http: Client,
}

impl MailboxClient {
    /// Build a client for `base_url`. When `cert_pem` is provided (the mailbox's
    /// self-signed certificate), it is pinned as the *only* trusted root — system/CA
    /// roots are disabled, so no certificate authority can impersonate the mailbox.
    pub fn new(base_url: &str, cert_pem: Option<&str>) -> Self {
        let mut builder = Client::builder();
        if let Some(pem) = cert_pem {
            match Certificate::from_pem(pem.as_bytes()) {
                Ok(cert) => {
                    builder = builder.add_root_certificate(cert).tls_built_in_root_certs(false);
                }
                Err(e) => eprintln!("seqr: ignoring invalid mailbox cert: {e}"),
            }
        }
        let http = builder.build().unwrap_or_else(|_| Client::new());
        Self { base: base_url.trim_end_matches('/').to_string(), http }
    }

    /// Park `payload` (an opaque, already-sealed frame) for recipient `to`
    /// (their signing public key, hex).
    pub async fn push(&self, to: &str, payload: &str) -> Result<String, CoreError> {
        let req = PushRequest { to: to.to_string(), payload: payload.to_string() };
        let resp = self
            .http
            .post(format!("{}/v1/push", self.base))
            .json(&req)
            .send()
            .await
            .map_err(merr)?;
        if !resp.status().is_success() {
            return Err(merr(format!("push status {}", resp.status())));
        }
        Ok(resp.json::<PushResponse>().await.map_err(merr)?.id)
    }

    /// Fetch this account's parked messages (authenticated).
    pub async fn pull(&self, identity: &Identity) -> Result<Vec<PulledMessage>, CoreError> {
        let id_hex = hex::encode(identity.public().signing_public);
        let ts = now_millis();
        let signature = hex::encode(sign(&identity.signing_key, &PullRequest::signing_bytes(&id_hex, ts)));
        let req = PullRequest { identity: id_hex, ts, signature };
        let resp = self
            .http
            .post(format!("{}/v1/pull", self.base))
            .json(&req)
            .send()
            .await
            .map_err(merr)?;
        if !resp.status().is_success() {
            return Err(merr(format!("pull status {}", resp.status())));
        }
        Ok(resp.json::<PullResponse>().await.map_err(merr)?.messages)
    }

    /// Ask which of `ids` are currently online (have polled recently).
    pub async fn presence(&self, ids: Vec<String>) -> Result<Vec<String>, CoreError> {
        let resp = self
            .http
            .post(format!("{}/v1/presence", self.base))
            .json(&PresenceRequest { ids })
            .send()
            .await
            .map_err(merr)?;
        if !resp.status().is_success() {
            return Err(merr(format!("presence status {}", resp.status())));
        }
        Ok(resp.json::<PresenceResponse>().await.map_err(merr)?.online)
    }

    /// Post a diagnostic line to the server's debug log (best-effort, unauthenticated).
    pub async fn debug(&self, tag: &str, msg: &str) -> Result<(), CoreError> {
        let req = LogRequest { tag: tag.to_string(), msg: msg.to_string() };
        let _ = self.http.post(format!("{}/v1/log", self.base)).json(&req).send().await;
        Ok(())
    }

    /// Delete delivered messages (authenticated).
    pub async fn ack(&self, identity: &Identity, ids: Vec<String>) -> Result<(), CoreError> {
        if ids.is_empty() {
            return Ok(());
        }
        let id_hex = hex::encode(identity.public().signing_public);
        let ts = now_millis();
        let signature =
            hex::encode(sign(&identity.signing_key, &AckRequest::signing_bytes(&id_hex, ts, &ids)));
        let req = AckRequest { identity: id_hex, ts, signature, ids };
        let resp = self
            .http
            .post(format!("{}/v1/ack", self.base))
            .json(&req)
            .send()
            .await
            .map_err(merr)?;
        if !resp.status().is_success() {
            return Err(merr(format!("ack status {}", resp.status())));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Live round-trip against the deployed VPS mailbox. Ignored by default (network);
    // run with: cargo test -p desktop mailbox_live -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn mailbox_live_roundtrip() {
        let url = std::env::var("SEQR_MAILBOX_URL")
            .unwrap_or_else(|_| "https://37.27.248.79:8443".to_string());
        // Optional path to the pinned cert (SEQR_MAILBOX_CERT) for HTTPS endpoints.
        let cert = std::env::var("SEQR_MAILBOX_CERT").ok().and_then(|p| std::fs::read_to_string(p).ok());
        let client = MailboxClient::new(&url, cert.as_deref());
        let me = Identity::generate();
        let id_hex = hex::encode(me.public().signing_public);

        // Park a payload addressed to ourselves.
        client.push(&id_hex, "desktop-client-probe").await.expect("push");

        // Pull it back (signed) and verify.
        let msgs = client.pull(&me).await.expect("pull");
        assert!(msgs.iter().any(|m| m.payload == "desktop-client-probe"));

        // Ack everything and confirm the mailbox is empty.
        let ids: Vec<String> = msgs.into_iter().map(|m| m.id).collect();
        client.ack(&me, ids).await.expect("ack");
        let after = client.pull(&me).await.expect("pull2");
        assert!(after.is_empty(), "mailbox should be empty after ack");
    }
}
