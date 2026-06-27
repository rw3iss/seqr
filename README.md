# Seqr

**Private, end-to-end-encrypted, peer-to-peer chat for you and your friends.**

Seqr is a local-first desktop app (Linux, Windows, macOS). Your messages live encrypted
on your own device; they travel directly between friends over an encrypted connection,
and no server can ever read them. Supports both 1:1 and group conversations, with keys
you can rotate or revoke at any time.

> **Status:** MVP feature-complete. Working: local account/vault, friends, 1:1 and
> group end-to-end chat over iroh QUIC, offline delivery via the self-hosted mailbox,
> and key rotation/revocation. Run two instances on separate machines to chat live.
> See `CLAUDE.md` for architecture and notes.

### 🛡️ Screen-capture protection

Seqr can hide its window from **screen recorders and screenshots**, so the contents of
your conversations don't leak into screen captures or screen-sharing. It's a toggle in
**Settings → Screen capture protection** (on by default).

- **macOS & Windows:** supported — the window is excluded from capture (recorders and
  screenshots see it as blank/omitted), via the OS (`NSWindowSharingNone` /
  `WDA_EXCLUDEFROMCAPTURE`).
- **Linux:** **not supported** — neither X11 nor Wayland offers a reliable way to exclude
  a window from capture, so the toggle has no effect there.

It defeats *software* capture, not a camera pointed at the screen, and isn't absolute
against an attacker with system-level access — but it stops casual/accidental leaks.

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

### Download a prebuilt installer (easiest)

Tagged releases publish native installers via GitHub Actions — **macOS** (`.dmg`),
Windows (`.msi`), and Linux (`.AppImage`/`.deb`). Grab the one for your OS from the
repo's **Releases** page and install as usual.

> **macOS first-launch note:** builds are unsigned, so macOS Gatekeeper will balk the
> first time. Right-click the app → **Open** (or System Settings → Privacy & Security →
> **Open Anyway**). After that it launches normally. The default mailbox URL and its
> pinned certificate are compiled in, so it connects with no further setup.

To cut a release yourself: `git tag v0.1.0 && git push origin v0.1.0` — the workflow in
`.github/workflows/release.yml` builds and drafts the release. On a Mac you can also
build locally in one command: `./scripts/setup-macos.sh`.

### Build from source

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

> Tip: exchange tokens over a channel you trust. To be certain no one tampered with the
> exchange, open the 1:1 chat and click **Verify** — both of you should see the same
> **safety number**. If the numbers match, your connection is genuine.

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

# Add TLS (self-signed CA + leaf via nginx; rebinds the mailbox to localhost)
scp services/mailbox/deploy/setup-tls.sh services/mailbox/deploy/nginx-seqr.conf user@your-server:/tmp/
ssh user@your-server 'sudo SEQR_HOST=<your-server-ip> bash /tmp/setup-tls.sh'
# ^ prints the CA certificate (PEM) + fingerprint at the end.

# Verify
curl --cacert ca.pem https://<your-server-ip>:8443/health   # -> ok
```

**TLS / certificate pinning.** `setup-tls.sh` creates a self-signed CA and serves the
mailbox over HTTPS on `:8443`. The app trusts **only** that certificate (no public CA),
so no certificate authority can impersonate your mailbox. Point clients at it via
`seqr.toml` (`mailbox_url = "https://<ip>:8443"`) and place the printed CA PEM beside it
as `mailbox_cert.pem` — or, for the default mailbox, the CA is already compiled into the
app so it works with no setup.

The service config lives in `/etc/seqr-mailbox/seqr-mailbox.env`; TLS certs in
`/etc/seqr-mailbox/tls/`. Edit and `sudo systemctl restart seqr-mailbox` (or
`reload nginx`) to apply.

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
