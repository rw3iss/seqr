# CLAUDE.md — Seqr

Secure, end-to-end-encrypted, peer-to-peer chat for small circles of friends.
Local-first desktop app (Linux/Windows/macOS) built on **Tauri (Rust core) + Preact/SCSS**,
with **iroh** for transport and a self-hosted **mailbox** helper for offline delivery.

Full design rationale: `docs/superpowers/specs/2026-06-26-seqr-e2e-chat-design.md`.

## Repository layout

```
seqr/
├── Cargo.toml                 # backend workspace (crypto, protocol, mailbox)
├── crates/
│   ├── seqr-crypto/           # PURE crypto primitives (the ONLY place that encrypts/signs)
│   └── seqr-protocol/         # shared wire types + canonical signing strings
├── services/
│   └── mailbox/               # VPS helper: store-and-forward encrypted mailbox (axum)
│       └── deploy/            # systemd unit, env, installer
├── apps/
│   └── desktop/               # Tauri + Preact app (its OWN cargo workspace)
│       ├── src/               # Preact UI (TS + SCSS, tab indent width 4)
│       └── src-tauri/src/core # Rust core: config, vault, identity, session
├── config/seqr.example.toml   # sample local config
└── docs/superpowers/specs/    # design spec
```

## Build, test, run

**Backend (crypto + protocol + mailbox)** — from repo root:
```bash
cargo test --workspace          # all backend tests
cargo build --workspace
```

**Mailbox static binary (for the VPS):**
```bash
rustup target add x86_64-unknown-linux-musl   # once
RUSTFLAGS="-C target-feature=+crt-static" \
  cargo build --release --target x86_64-unknown-linux-musl -p seqr-mailbox
# -> target/x86_64-unknown-linux-musl/release/seqr-mailbox
```

**Desktop app** — from `apps/desktop`:
```bash
pnpm install
pnpm build                      # tsc type-check + vite build (frontend)
pnpm tauri dev                  # run the app (needs a display)
(cd src-tauri && cargo test)    # core unit tests (vault/identity/config)
```
Linux build deps: `webkit2gtk-4.1`, GTK (already present on this machine).

## The VPS helper (Hetzner)

The encrypted mailbox is **deployed and running** on the user's server.

- **SSH:** `ssh rw3iss@37.27.248.79` (Fedora 43, passwordless sudo)
- **Service:** `seqr-mailbox.service` (systemd, enabled at boot), listens on `:8787`
- **Binary:** `/usr/local/bin/seqr-mailbox` (static musl), runs as unprivileged user `seqr`
- **Config:** `/etc/seqr-mailbox/seqr-mailbox.env`
- **Data:** `/var/lib/seqr-mailbox/` (ciphertext only; owned by `seqr`)
- **Firewall:** `8787/tcp` open (firewalld)
- **Health:** `curl http://37.27.248.79:8787/health` → `ok`

**Manage:**
```bash
ssh rw3iss@37.27.248.79 'sudo systemctl status seqr-mailbox'
ssh rw3iss@37.27.248.79 'sudo journalctl -u seqr-mailbox -n 50'
```

**Redeploy after changes:**
```bash
# 1. build the musl binary (see above)
scp target/x86_64-unknown-linux-musl/release/seqr-mailbox rw3iss@37.27.248.79:/tmp/
scp services/mailbox/deploy/* rw3iss@37.27.248.79:/tmp/
ssh rw3iss@37.27.248.79 'sudo bash /tmp/install.sh'   # idempotent
```
⚠️ Do **not** run the binary manually over SSH to "test" it — it binds `:8787` and will
orphan, blocking the service. Use the systemd unit and `curl /health`.

## Local app configuration

The app reads the mailbox URL from (in order): `SEQR_MAILBOX_URL` env →
`<config-dir>/com.seqr.app/seqr.toml` → compiled-in default (`http://37.27.248.79:8787`).
A local `seqr.toml` is already written at `~/.config/com.seqr.app/seqr.toml`.

## Cryptography (summary)

- **Identity:** long-term X25519 (agreement) + Ed25519 (signing) keypairs.
- **1:1 key:** X25519 ECDH → HKDF-SHA256. Both sides derive it; nothing secret is sent.
- **Group key:** random symmetric key, distributed to each member sealed under the
  pairwise key. Any member can rotate/revoke (mandatory rotation on member removal).
- **Messages:** ChaCha20-Poly1305 sealed (AAD binds conversation+epoch) **and**
  Ed25519-signed by the sender (prevents group members forging each other).
- **Vault:** local store encrypted with a key derived from the login password via
  Argon2id. (Implemented as an encrypted JSON file rather than the spec's SQLCipher —
  pure-Rust, no native dep, fine at friend scale; can migrate to a DB later.)
- **Profiles:** `seqr:<hex>` tokens carrying public keys + node address only.

## Conventions

- **Crypto lives only in `crates/seqr-crypto`.** Persistence only in the vault module.
  Network only in transport/mailbox. Do not duplicate these concerns.
- The Preact UI **never** handles private keys or plaintext — only via IPC commands.
- SCSS: one `.scss` per component, real class names (no CSS modules / CSS-in-JS),
  tokens in `src/styles/_tokens.scss`, **tab indentation (width 4)**.
- Tauri commands are a thin seam (`src-tauri/src/commands.rs`) — no business logic.

## Milestone status

- ✅ **M1 — vault + identity + friends + UI shell** (done): encrypted vault, account
  create/unlock, profile export/import, friends roster, two-pane dashboard.
- ✅ **VPS mailbox** built, tested, deployed, verified reachable.
- ⏳ **M2 — live 1:1 transport**: iroh connection, ECDH session, signed+sealed
  send/receive, message history in the vault.
- ⏳ **M3 — mailbox client**: push/pull/ack wiring for offline delivery.
- ⏳ **M4 — groups**: conversation generalisation, group-key distribution, fan-out, group UI.
- ⏳ **M5 — rotation/revocation**: rotate/remove for 1:1 and groups, concurrent-rotation resolution.
