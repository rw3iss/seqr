//! Runtime configuration, read from the environment so the systemd unit owns the
//! settings and the binary stays free of baked-in paths.

use std::path::PathBuf;

#[derive(Clone)]
pub struct Config {
    pub bind: String,
    pub data_dir: PathBuf,
    /// Reject payloads larger than this (bytes of the hex string).
    pub max_payload: usize,
    /// Accept pull/ack requests whose timestamp is within this many seconds of now.
    pub clock_skew_secs: u64,
    /// Maximum messages returned in one pull.
    pub pull_limit: usize,
}

impl Config {
    pub fn from_env() -> Self {
        let get = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        Self {
            bind: get("SEQR_MAILBOX_BIND", "0.0.0.0:8787"),
            data_dir: PathBuf::from(get("SEQR_MAILBOX_DATA", "./data")),
            max_payload: get("SEQR_MAILBOX_MAX_PAYLOAD", "131072").parse().unwrap_or(131072),
            clock_skew_secs: get("SEQR_MAILBOX_SKEW_SECS", "120").parse().unwrap_or(120),
            pull_limit: get("SEQR_MAILBOX_PULL_LIMIT", "256").parse().unwrap_or(256),
        }
    }
}
