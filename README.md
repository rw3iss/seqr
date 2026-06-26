# Seqr

**Private, end-to-end-encrypted, peer-to-peer chat for you and your friends.**

Seqr is a local-first desktop app (Linux, Windows, macOS). Your messages live encrypted
on your own device; they travel directly between friends over an encrypted connection,
and no server can ever read them. Supports both 1:1 and group conversations, with keys
you can rotate or revoke at any time.

> **Status:** early development. The local account/vault, friends, UI, and the
> self-hosted mailbox helper are working. Live messaging (transport + offline delivery)
> and groups are the next milestones — see `CLAUDE.md` for the roadmap.

---

## How it works (in one picture)

```
   You ───────── direct, encrypted (iroh QUIC) ───────── Friend
     │                                                      │
     └──── if a friend is offline, ciphertext waits in ─────┘
                   your self-hosted mailbox (VPS)
                   (it only ever sees encrypted noise)
```

- **Your keys never leave your device.** When you add a friend you exchange only
  *public* keys; the shared encryption key is derived locally on both sides.
- **The mailbox is blind.** It stores and forwards messages while a friend is offline,
  but everything is already encrypted end-to-end — it cannot read anything.

---

## Installing the app

### Prerequisites

- [Rust](https://rustup.rs/) (stable) and [Node.js](https://nodejs.org/) 20+ with
  [pnpm](https://pnpm.io/) (`npm i -g pnpm`).
- **Linux only:** WebKitGTK + GTK dev packages, e.g. on Fedora:
  `sudo dnf install webkit2gtk4.1-devel gtk3-devel`
  (Debian/Ubuntu: `libwebkit2gtk-4.1-dev libgtk-3-dev build-essential`).

### Run from source

```bash
git clone <your-repo-url> seqr && cd seqr/apps/desktop
pnpm install
pnpm tauri dev          # launches the app
```

### Build an installer

```bash
cd apps/desktop
pnpm tauri build        # produces a native bundle for your OS in src-tauri/target
```

On first launch you'll set a **local password**. This password encrypts your vault
(identity keys + chat history) via Argon2id and **cannot be recovered** — choose well.

---

## Adding a friend

Adding a friend is a one-time exchange of **profile tokens**. A token looks like
`seqr:7b2276...` and contains only public information (your public keys and how to
reach you) — it is safe to send over any channel.

1. **Share your token.** In the app, click **+ Add friend → My profile**, then
   **Copy token**. Send it to your friend (message, email, whatever you trust).
2. **Import theirs.** When they send you their token, open **+ Add friend →
   Add a friend**, paste it, and click **Add friend**.
3. **Both sides import.** Once you've each imported the other, you'll appear in one
   another's friends list and can open a private conversation.

> Tip: exchange tokens over a channel you trust. Anyone who can both intercept *and*
> replace a token mid-exchange could impersonate a friend — the same caution as
> exchanging any contact detail.

### Groups

Create a group from friends you've already added. Any member can add another member
(whom they're already friends with), and **any member can rotate or revoke the group
key** — removing someone immediately cuts them off from all future messages.

---

## Hosting the mailbox helper (your VPS)

The mailbox lets messages reach friends who are offline. You host one small service on
any Linux server. It holds no accounts and can only ever see ciphertext.

```bash
# On your machine: build the static binary
rustup target add x86_64-unknown-linux-musl
RUSTFLAGS="-C target-feature=+crt-static" \
  cargo build --release --target x86_64-unknown-linux-musl -p seqr-mailbox

# Copy the binary + deploy files to the server
scp target/x86_64-unknown-linux-musl/release/seqr-mailbox user@your-server:/tmp/
scp services/mailbox/deploy/* user@your-server:/tmp/

# On the server: install (creates a hardened systemd service on :8787)
ssh user@your-server 'sudo bash /tmp/install.sh'

# Verify
curl http://your-server:8787/health     # -> ok
```

Then point the app at it via `~/.config/com.seqr.app/seqr.toml`
(see `config/seqr.example.toml`) or the `SEQR_MAILBOX_URL` environment variable.

The service config lives in `/etc/seqr-mailbox/seqr-mailbox.env` (port, limits);
edit and `sudo systemctl restart seqr-mailbox` to apply.

---

## Troubleshooting (Linux)

**`Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display`** (or a
blank/white window): WebKitGTK's DMABUF renderer can clash with some Wayland
compositors and GPU drivers. The dev script already sets
`WEBKIT_DISABLE_DMABUF_RENDERER=1` to avoid it. If a crash persists, also try forcing
the X11 (XWayland) backend:

```bash
GDK_BACKEND=x11 pnpm tauri dev
# or, if compositing still misbehaves:
WEBKIT_DISABLE_COMPOSITING_MODE=1 pnpm tauri dev
```

## Project structure & contributing

See **`CLAUDE.md`** for the full layout, build/test commands, cryptography summary, and
the milestone roadmap. The design spec is in `docs/superpowers/specs/`.

Run the test suite:

```bash
cargo test --workspace                     # backend (crypto, protocol, mailbox)
cd apps/desktop/src-tauri && cargo test    # desktop core
```

## License

MIT.
