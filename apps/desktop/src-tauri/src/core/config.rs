//! Runtime configuration.
//!
//! The mailbox endpoint and its pinned self-signed certificate. Resolution order for
//! the URL: `SEQR_MAILBOX_URL` env → `seqr.toml` in the OS config dir → compiled-in
//! default. The pinned cert is read from `mailbox_cert.pem` in the same config dir (if
//! present); without it, an `https://` mailbox with a self-signed cert is untrusted.
//!
//! Kept deliberately tiny; richer settings can join later.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The user's commissioned mailbox helper (TLS-terminated by nginx, self-signed).
pub const DEFAULT_MAILBOX_URL: &str = "https://37.27.248.79:8443";

/// The mailbox's pinned CA certificate, compiled in so a freshly installed app trusts
/// the default mailbox with no manual setup. Override per-install with a
/// `mailbox_cert.pem` in the config dir.
pub const DEFAULT_MAILBOX_CERT: &str = "-----BEGIN CERTIFICATE-----
MIIBmTCCAT+gAwIBAgIUfCCcHaV7k1gBD7GLjugDvVBt3fAwCgYIKoZIzj0EAwIw
GjEYMBYGA1UEAwwPc2Vxci1tYWlsYm94LWNhMB4XDTI2MDYyNjA5MTQyOVoXDTM2
MDYyMzA5MTQyOVowGjEYMBYGA1UEAwwPc2Vxci1tYWlsYm94LWNhMFkwEwYHKoZI
zj0CAQYIKoZIzj0DAQcDQgAE1KY1+cTfyOCCTpdxvhiCT9ZdPiBeLhCAg1XphPvL
SFbcVxeXW8avBW+floDu6Vl1lLmK4E8bRuUwN1h4SiaQRaNjMGEwHQYDVR0OBBYE
FMLDkwhdVOkDZpFxl5UYkRWdaRD3MB8GA1UdIwQYMBaAFMLDkwhdVOkDZpFxl5UY
kRWdaRD3MA8GA1UdEwEB/wQFMAMBAf8wDgYDVR0PAQH/BAQDAgEGMAoGCCqGSM49
BAMCA0gAMEUCIQCuWhioV7/orkjprqN2ikjFb7q/o1NFo+iVCyTNuWwZ2wIgaGYa
DPvPWBKhv2aNWNRNlblRJZG7SR81ALwVgcqRGMY=
-----END CERTIFICATE-----
";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub mailbox_url: String,
    /// PEM of the mailbox's self-signed certificate to pin (None => system roots only).
    #[serde(default)]
    pub mailbox_cert: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            mailbox_url: DEFAULT_MAILBOX_URL.to_string(),
            mailbox_cert: Some(DEFAULT_MAILBOX_CERT.to_string()),
        }
    }
}

impl AppConfig {
    /// Resolve from `<config_dir>/seqr.toml` + `<config_dir>/mailbox_cert.pem`, with an
    /// env override for the URL.
    pub fn resolve(config_dir: &Path) -> Self {
        let mut cfg = AppConfig::default();
        if let Ok(text) = std::fs::read_to_string(config_dir.join("seqr.toml")) {
            // Minimal single-key parse to avoid a toml dependency for one line.
            for line in text.lines() {
                if let Some(rest) = line.trim().strip_prefix("mailbox_url") {
                    if let Some(v) = rest.split('=').nth(1) {
                        cfg.mailbox_url = v.trim().trim_matches('"').to_string();
                    }
                }
            }
        }
        if let Ok(url) = std::env::var("SEQR_MAILBOX_URL") {
            cfg.mailbox_url = url;
        }
        // Pin the mailbox cert if present alongside the config.
        if let Ok(pem) = std::fs::read_to_string(config_dir.join("mailbox_cert.pem")) {
            cfg.mailbox_cert = Some(pem);
        }
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_points_at_vps() {
        assert_eq!(AppConfig::default().mailbox_url, DEFAULT_MAILBOX_URL);
    }
}
