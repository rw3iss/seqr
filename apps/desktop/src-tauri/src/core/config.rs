//! Runtime configuration.
//!
//! The one setting that matters today is the VPS mailbox endpoint. Resolution order:
//! 1. `SEQR_MAILBOX_URL` environment variable (handy for development), else
//! 2. a `seqr.toml` in the OS config dir (written at install/first-run), else
//! 3. the compiled-in default — this user's Hetzner server.
//!
//! Kept deliberately tiny; richer settings can join later.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The user's commissioned mailbox helper (see services/mailbox).
pub const DEFAULT_MAILBOX_URL: &str = "http://37.27.248.79:8787";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub mailbox_url: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self { mailbox_url: DEFAULT_MAILBOX_URL.to_string() }
    }
}

impl AppConfig {
    /// Resolve configuration from env, then an optional config file, then defaults.
    pub fn resolve(config_file: &Path) -> Self {
        let mut cfg = AppConfig::default();
        if let Ok(text) = std::fs::read_to_string(config_file) {
            // Minimal TOML-ish single-key parse to avoid a toml dependency for one line.
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
