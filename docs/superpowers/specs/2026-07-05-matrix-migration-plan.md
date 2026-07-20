# Seqr → Matrix Migration & Cross-Platform Plan

**Goal:** evolve Seqr from a bespoke P2P E2E chat into a **cross-platform** (desktop, Android,
iOS, later web) client on the **Matrix** protocol, backed by a **self-hosted homeserver** on
`rw3iss@162.35.181.92`, supporting **rooms (group chats), 1:1 DMs, media, and multi-device**.

Date: 2026-07-05 · Status: **Plan for review** (implementation follows approval)

---

## 1. Executive summary & the core decision

We adopt the **"Matrix-as-engine"** approach: keep Seqr's **Preact/Tauri UI** and product feel,
but **replace the entire custom backend** — crypto, transport, protocol, mailbox — with the
**`matrix-rust-sdk`** engine talking to a self-hosted homeserver.

**Why:** the remaining hard problems (multi-device E2E, key backup/recovery, device verification,
mobile push, reliable groups) are exactly what Matrix has spent years and audits solving.
`matrix-rust-sdk` is Rust-native (slots straight into Tauri's core), is production-grade, and
**powers Element X** — which is itself the proof that these crates run on iOS and Android.

**What we keep:**
- The Preact UI (conversation list, chat window, composer, media rendering, settings modal,
  image preview modal) — re-skinned onto Matrix data.
- The Tauri shell, SCSS/design tokens, build/CI patterns, `set_content_protected` screen security.

**What we retire (deleted or archived):**
- `crates/seqr-crypto` (→ Matrix Olm/Megolm via `matrix-sdk-crypto`).
- `crates/seqr-protocol` + `core/packet.rs`, `core/message.rs`, `core/group.rs`,
  `core/conversation.rs` (→ Matrix events & room state).
- `core/transport.rs` (iroh) (→ Matrix client-server HTTPS + sliding sync).
- `services/mailbox/` (→ the homeserver's own store-and-forward + push).
- `core/vault.rs` custom store (→ `matrix-sdk` SQLite stores: state, crypto, event-cache, media).

This is, honestly, a **backend rewrite with UI reuse**. The custom crypto/transport we built was a
great proof of concept and gave us a strong UI; the Matrix path trades "we own every byte" for
"multi-device + mobile + battle-tested crypto, fastest and safest."

---

## 2. Homeserver decision (honest recommendation)

The user requested **Dendrite**. A duty-bound flag from current research (July 2026):

- **Dendrite is in maintenance mode** since late 2024 — Element/Matrix.org can't fully resource it;
  development has slowed, and bridge/MSC support is partial. It's still a stable, memory-efficient
  (256–512 MB) Go homeserver (monolith binary + PostgreSQL) with full federation and Olm/Megolm
  support, good for the **10–100 user** range. See refs.
- The **Rust family — Continuwuity / conduwuit** (forks of Conduit) — now has the momentum: a
  **single binary with an embedded database (RocksDB), no Postgres**, low ops, actively developed.

**DECISION A — RESOLVED (2026-07-20): Continuwuity.** A single-binary, embedded-DB Rust homeserver
is the better fit than Dendrite + Postgres on both memory and future momentum. **Continuwuity**
(fork of Conduit; binary is named `conduwuit`, service `conduwuit.service`) ships a **fully-static
musl binary** (jemalloc + io_uring statically linked, no dependencies), uses **~32 MB fresh** (256 MB
generous), embeds RocksDB (no Postgres), listens on **`127.0.0.1:6167`** by default, and health-checks
at `/_conduwuit/server_version`. Dendrite is retired from this plan (maintenance mode).

---

## 3. Target architecture

```
 ┌───────────────────────── Clients (one shared Rust core + Preact UI) ─────────────────────────┐
 │  Tauri Desktop (Linux/Win/mac)   Tauri Android   Tauri iOS        Web (WASM or matrix-js-sdk)  │
 │        │   matrix-rust-sdk (matrix-sdk + matrix-sdk-ui + matrix-sdk-crypto) in the Rust core   │
 └────────┼──────────────────────────────────────────────────────────────────────────────────────┘
          │  Matrix Client-Server API over HTTPS (sliding sync, E2E via Olm/Megolm)
          ▼
 ┌──────────────────────────── Your server: matrix.<domain>  (162.35.181.92) ────────────────────┐
 │  Homeserver (Continuwuity | Dendrite)   ── media repo ──  Postgres/RocksDB   ── federation off  │
 │            │                                                                                     │
 │            └── Sygnal push gateway ──► FCM (Android/iOS) / APNs (iOS)   [+ optional ntfy/UnifiedPush] │
 └────────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **E2E is end-to-end:** the homeserver stores **ciphertext** (Megolm-encrypted room events);
  it cannot read messages. It *does* hold metadata (who is in which room, timing) — the accepted
  Signal/Matrix trade discussed previously.
- **Federation:** default **OFF** (closed server for your circle). Can be enabled later.
- **Push:** the homeserver applies push rules and calls **Sygnal**, which relays to FCM/APNs.

---

## 4. Concept mapping (Seqr → Matrix)

| Seqr today | Matrix equivalent | Notes |
|---|---|---|
| Account / identity (one keypair/vault) | Matrix user `@name:domain` + per-device keys | Multi-device is native. |
| Friend (import token) | **Invite** to a DM room (`m.direct`) | Or a shared user directory search. |
| Friend request accept/decline | Room **invite** accept/reject | Built in. |
| 1:1 conversation | **DM room** (2 members, `is_direct`) | |
| Group | **Room** (N members, name, topic, avatar) | Power levels for admin/remove. |
| Rotate key / revoke | Megolm **key rotation** + membership changes | Automatic on membership change. |
| Remove member | **Kick/ban** via power levels | |
| Safety number (verify) | **Cross-signing + SAS/QR device verification** | Stronger, standardized. |
| Attachment (encrypted, chunked) | **Media repo** upload (encrypted) `m.image/m.file/m.video` | See §10 for size limits. |
| Presence dot | Matrix **presence** (or last-seen) | Optional; can be disabled server-side. |
| Notifications setting | **Push rules** + local notifications | |
| Screen-capture protection | Keep as-is (client feature) | `set_content_protected`. |
| Multi-device history | **Server-stored (encrypted) timeline + key backup** | The big win. |

---

## 5. What survives in the codebase

**Reused (frontend):** `Dashboard`, `ChatWindow` composer, message bubbles, `AttachmentView` +
image modal, `SettingsModal`, `CreateGroupModal`/members UI, `FriendRequests` (→ invites),
`LoginView` (→ Matrix login/registration), SCSS tokens & components, Tauri config, screen-security.

**Added (backend):** a new `src/matrix/` module wrapping `matrix-rust-sdk` with a thin Tauri command
surface + events (login/session/timeline/…), running *alongside* the existing `core/`.

**Retained, not deleted — DECISION D (2026-07-20): runtime dual-backend.** The legacy P2P stack
(`core/*`, `seqr-crypto`, `seqr-protocol`, `net.rs`, the mailbox) is **kept compiled in** as a
selectable fallback for direct peer-to-peer chatting later. `AppConfig::backend` (`matrix` | `p2p`,
default `matrix`) picks the active backend at launch; both command sets are registered (names don't
collide). Cost accepted: both dependency trees compile (iroh + matrix-sdk), verified to share a single
`ed25519-dalek 2.2` / `curve25519-dalek 4.1` (vodozemac, iroh, seqr-crypto all unify — no conflict).
`services/mailbox` stays deployed but idle. (If the P2P path is ever formally dropped, archive on a tag.)

> **No data migration.** Because identities and crypto change entirely, existing Seqr installs start
> **fresh** on Matrix (new `@user:domain`, new keys). Old local history isn't carried over (could be
> offered as a read-only export later; low priority).

---

## 6. Server infrastructure (self-host on 162.35.181.92)

**Server facts (measured):** Fedora 44 Server, 2 vCPU, 118 GB disk (87 GB free), passwordless sudo,
**ports 80/443 already in use** (an existing web app), **~440 MB RAM available of 5.8 GB**.

### 6.0 Prerequisites (do first)
1. **Audit RAM — ✅ RESOLVED (2026-07-20).** The RAM was consumed by `trader-ml.service` (a
   Python/uvicorn+LightGBM sidecar) that had leaked to ~8.3 GB, almost all in swap, thrashing the box.
   Fixed without a reboot: capped it (`MemoryMax` drop-in), `systemctl disable --now` the whole trader
   stack (trader-api/ingest/ml/worker + timer), killed the user's procs, compacted swap
   (`swapoff -a && swapon -a`), and cleared 1.5 GB of Python core dumps. Result: RAM 5.3 GB → 1.6 GB
   used (**4.3 GB free**), swap 8.7 GB → 0. Continuwuity's actual footprint is ~22 MB, so headroom is ample.
2. **Domain + DNS — DECISION B RESOLVED (2026-07-20).**
   - **`server_name = rw3iss.com`** → user ids are **`@you:rw3iss.com`**. **This is permanent** for
     this homeserver (baked into every user/room id; cannot be changed without wiping the DB). A future
     custom Seqr domain would mean a *new* homeserver or a migration — so the **client's homeserver URL
     is made configurable** (like the old `mailbox_url`) to repoint the app, but the identity domain
     `rw3iss.com` is fixed here.
   - **Homeserver reachability:** `rw3iss.com` is **Cloudflare-proxied** (resolves to `104.21.x`, not
     the box) with **no origin LE cert** — and Cloudflare's free tier caps uploads at 100 MB and its
     100 s timeout breaks Matrix long-polling sync. So the client-server API must be reached over a
     **DNS-only (gray-cloud) subdomain pointing directly at `162.35.181.92`**, e.g.
     **`matrix.rw3iss.com`**, with its own Let's Encrypt cert.
   - **Delegation:** serve `/.well-known/matrix/client` on **`https://rw3iss.com`** (through Cloudflare
     is fine — it's a tiny static JSON, no upload/WS concerns) pointing `base_url` → `https://matrix.rw3iss.com`.
   - **⚠️ USER ACTION REQUIRED:** add a Cloudflare DNS record **`matrix.rw3iss.com` A `162.35.181.92`,
     proxy = OFF (gray cloud / DNS-only)**. Everything up to TLS can be staged without it; the LE cert
     and public exposure need this record to resolve.
3. **TLS.** The existing box already runs nginx + certbot elsewhere; on this box we terminate TLS for
   the homeserver (Let's Encrypt via certbot, or the existing web server if it's nginx).
4. **Reverse proxy** in front of the homeserver on 443 (path-routed `/_matrix/` + `/.well-known/matrix/`),
   with a **large `client_max_body_size`** (see §10).

### 6.1 Homeserver install — Option A: Continuwuity/conduwuit (recommended)
1. Install the single static binary (or container) as a hardened **systemd** service under an
   unprivileged user (mirror our existing `seqr-mailbox` deploy pattern: `/usr/local/bin`,
   `/etc/…/config.toml`, `/var/lib/…` state, firewall).
2. Config: `server_name`, `database_path` (RocksDB), `max_request_size` (media), **registration**
   (see 6.3), `allow_federation = false` (initially), listener on `127.0.0.1:6167`.
3. nginx vhost: `matrix.<domain>` → proxy `/_matrix` and `/_conduwuit` → `127.0.0.1:6167`; serve
   `/.well-known/matrix/{client,server}` for delegation & client discovery.

### 6.1-alt Homeserver install — Option B: Dendrite (as requested)
1. Install **PostgreSQL**, create the dendrite DB/user.
2. Install Dendrite (monolith) binary + `dendrite.yaml`; generate signing key & (self-)matrix key.
3. systemd service; nginx vhost proxying `/_matrix` (client 8008, federation 8448 if ever enabled)
   + `.well-known`.
4. Apply the **`bigint` media column** fix if large media is needed (see §10).

### 6.2 `.well-known` delegation (either option)
- `https://<domain>/.well-known/matrix/client` → `{ "m.homeserver": { "base_url": "https://matrix.<domain>" } }`
- `https://<domain>/.well-known/matrix/server` → `{ "m.server": "matrix.<domain>:443" }` (only if federating)

### 6.3 Registration policy
- **Invite/closed** by default: disable open registration; create accounts via admin (registration
  shared-secret or admin API), or enable **registration tokens** so you hand friends a one-time code.
- **Open decision C:** open-with-token vs admin-created accounts. Recommend **registration tokens**.

### 6.4 Ops
- Firewall: 443 (and 8448 only if federating). Backups of the DB + media + signing keys (critical:
  losing the homeserver signing key is unrecoverable). Basic monitoring (systemd + a `/health` check
  + disk alerts, since media grows). Log rotation.

### 6.5 Deployment record — ✅ DONE (2026-07-20)
Continuwuity **v26.6.2** is installed, running, and verified on `162.35.181.92`.

| Item | Value |
|------|-------|
| Binary | `/usr/local/bin/conduwuit` (`conduwuit-haswell-linux-static-amd64-maxperf`, static musl) |
| Service | `conduwuit.service` (systemd, hardened, `User=conduwuit`, `MemoryMax=1G`, enabled at boot) |
| Config | `/etc/conduwuit/conduwuit.toml` (0640) — `server_name=rw3iss.com`, `127.0.0.1:6167`, RocksDB, `allow_federation=false`, `max_request_size=1 GiB`, registration-token gated |
| Data | `/var/lib/conduwuit` (RocksDB) |
| Ops home | `/var/www/seqr-matrix/` — README + `deploy/` (staged nginx vhost, unit + config copies) |
| Footprint | ~22 MB RAM |
| Admin user | `@ryan:rw3iss.com` (created via the boot-log registration token) |

**Verified locally** (`127.0.0.1:6167`): `/_matrix/client/versions` → up to v1.18; password login
advertised; full round-trip **register → login → whoami → createRoom → send → sync → read-back** all `200`.
Documented in the box's `~/README.md` (Deployed apps) and `/var/www/seqr-matrix/README.md`.

**Public exposure — ✅ DONE (2026-07-20).** The `matrix.rw3iss.com` gray-cloud DNS record was added
(resolves directly to `162.35.181.92`). Issued a Let's Encrypt cert (`certbot certonly --webroot`,
valid to 2026-10-18, auto-renew scheduled) and activated the reverse-proxy vhost
(`/etc/nginx/conf.d/matrix.rw3iss.com.conf`, `client_max_body_size 1g`, 600 s proxy timeouts for
`/sync`). Verified over public TLS: `https://matrix.rw3iss.com/_matrix/client/versions`, the
`/.well-known/matrix/client` delegation hint, and a full **password login → `@ryan:rw3iss.com`**
round-trip through nginx. **The homeserver is publicly reachable and ready for the M2 client.**

---

## 7. Client architecture (Tauri + matrix-rust-sdk)

### 7.1 Rust core (new `core/`)
- Depend on **`matrix-sdk`** (features: `e2e-encryption`, `sqlite`, `sso-login` optional) +
  **`matrix-sdk-ui`** (Timeline, RoomListService) + **`matrix-sdk-crypto`** (transitive).
- **Session store:** SQLite stores (state, crypto, event-cache, media) under the Tauri app-data dir,
  encrypted with a passphrase derived from the login password (SDK supports a store passphrase).
- **Sync:** use **sliding sync / `RoomListService`** for the room list and **`Timeline`** for the open
  room — the SDK decrypts transparently and emits reactive updates.
- **Command surface (Tauri IPC)** — thin wrappers, mirroring today's `commands.rs` shape:
  `login`, `restore_session`, `logout`, `list_rooms` (via RoomListService), `room_timeline` +
  live subscription, `send_message`, `send_media`, `create_room`/`create_dm`, `invite`, `join`,
  `leave`, `kick`, `set_power_level`, `verify_device` (SAS/QR), `settings`, etc.
- **Events → UI:** forward SDK timeline/room-list/verification updates to the webview via Tauri
  events (replacing `seqr://message` etc.).

### 7.2 Known gotcha (from Cinny/Tauri)
Tauri's custom `tauri:` asset protocol can break service workers and, in some setups, session/key
persistence — plan storage under a stable path and **rely on server-side key backup** so a lost
session never loses keys. Verify session persistence early (M2 exit criteria).

### 7.3 Responsive UI
The two-pane desktop layout must collapse to a **single-pane, navigable** layout on phones
(room list → room → back). Add a small router/nav state; reuse existing components.

---

## 8. Cross-platform build-out

| Target | How | Blocking prereqs |
|---|---|---|
| **Desktop** (Linux/Win/mac) | Existing Tauri build; swap core. | none new |
| **Android** | `pnpm tauri android init` → Android Studio/SDK/NDK; APK/AAB. Buildable **on this Linux box**. | Android SDK; Play Console ($25) for store. |
| **iOS** | `pnpm tauri ios init` on the **Mac** + Xcode; APNs. | **Apple Developer ($99/yr)**; must build on macOS. |
| **Web** (later) | Either the same Preact UI on **`matrix-sdk` WASM**, or a separate build on **`matrix-js-sdk`**. | Different storage (IndexedDB); decide reuse vs. fork. |

> **Open decision D:** web via Rust-SDK-WASM (max code reuse, heavier) vs. `matrix-js-sdk` (mature web
> path, separate code). Defer until desktop+mobile land.

---

## 9. Push notifications (the mobile-critical piece)

Matrix push flow: client registers a **pusher** on the homeserver with a push key from FCM/APNs; on a
new event the homeserver calls **Sygnal**, which relays to FCM/APNs to wake the app, which then syncs
and shows the (locally decrypted) message.

**Plan:**
1. Run our **own Sygnal** (self-hosted, systemd) configured with **our app's** FCM + APNs secrets —
   end-users cannot supply these, so this is mandatory for a custom client.
2. **Android + iOS via FCM** (Firebase can relay to APNs), or **APNs directly** for iOS
   (token-based auth recommended). Set FCM `content_available` so **iOS wakes**.
3. Use the **`event_id_only`** push format so **message content never reaches Google/Apple** — the app
   fetches and decrypts after waking. (Privacy-preserving.)
4. **Optional Android privacy path:** **UnifiedPush + self-hosted ntfy**, avoiding Google entirely;
   iOS still needs APNs/Sygnal. A hybrid ("choose distributor") is the common pattern.

> **Open decision E:** you'll need a **Firebase/FCM project** and an **Apple Developer account** for
> real push. Confirm you want to create these (required for usable mobile).

---

## 10. Media & large files

- Matrix has a **media repository**; the SDK handles **encrypted** upload/download for `m.image`,
  `m.video`, `m.file`, with thumbnails.
- **Size:** raise the homeserver **max upload size** (e.g. `2048M`) **and** the reverse-proxy
  `client_max_body_size` to match, and **fully restart**. There is a **known ~2 GB ceiling** (DB
  column overflow on Postgres → apply the `media_length → bigint` fix; embedded-DB servers differ).
- **Our 1 GB requirement is achievable** with config, but **very large files are better shared as an
  external link** (Matrix isn't built for multi-GB blobs). Recommend: cap in-app media at ~1–2 GB,
  and revisit external object storage (S3/MinIO) for the media repo if volume grows.

---

## 11. Suggested UI extensions & improvements (enabled by Matrix)

Matrix gives us these largely "for free" — prioritized:

**Tier 1 (core parity + expected chat features):**
- **Device verification UX** (SAS emoji / QR) replacing the manual safety number — with clear
  "verify this device" prompts (essential for trust across devices).
- **Reactions** (emoji), **replies/threads**, **message edit & delete (redaction)**.
- **Read receipts & typing indicators**, **markdown/rich text** rendering.
- **Room management UI:** name/topic/avatar, member list with power levels (admin/kick/ban), invites,
  leave, mute/notifications per room.

**Tier 2 (polish & delight):**
- **User & room avatars**, **link previews (URL previews)**, **image galleries / lightbox** (extend
  our existing modal), **voice messages**, **file/download manager view**.
- **Search** (in-room + global), **pinned messages**, **unread badges & jump-to-unread**,
  **notification granularity** (per-room mute, keywords).
- **Cross-device session management** ("your devices" list, sign out remote sessions).
- **Key backup & recovery UX** (recovery key / passphrase) — so "forgot password" no longer means
  "lost history."

**Tier 3 (stretch):**
- Spaces (grouping rooms), public room directory, moderation tools, VoIP/calls (Matrix supports
  1:1 and group calls via MSC/Element Call), stickers, location sharing.

---

## 12. Phased milestones

- **M0 — Research & decisions** *(this doc)* — pick homeserver (A), user-id form (B), registration (C),
  domain, and confirm push accounts (E).
- **M1 — Homeserver up. ✅ DONE (2026-07-20, see §6.5).** Freed server RAM, installed Continuwuity
  v26.6.2 (static musl, systemd, federation-off, registration-token gated), admin `@ryan:rw3iss.com`,
  verified register→login→room→send→sync→read-back locally, documented. **Public exposure done too:**
  `https://matrix.rw3iss.com` live (gray-cloud DNS + Let's Encrypt + nginx proxy), public login verified.
- **M2 — Desktop client MVP.** *(in progress)* New `src/matrix/` core on `matrix-sdk`: login, restore
  session (persistent), RoomListService list, Timeline for one room, send/receive **E2E** text. Exit
  criteria: two desktop installs chat encrypted; session & keys survive restart.
  - ✅ **M2.0 — dual-backend scaffolding (2026-07-20):** `matrix-sdk 0.18` added (graph unifies with
    iroh, see §5); `AppConfig.backend`/`homeserver_url` (default `matrix` / `https://matrix.rw3iss.com`);
    `MatrixState` + commands `matrix_login`/`matrix_restore_session`/`matrix_logout`/`matrix_status`
    (SQLite stores, `FullSession` persist/restore per the SDK example); wired in `lib.rs`; TS `api.ts`
    types + methods. `cargo check` + `tsc` green. *Next:* route `LoginView` by backend; sync + room list.
- **M3 — Rooms + DMs + media.** Create/join rooms, DMs, invites/accept, membership/power levels,
  encrypted media upload/download with our attachment UI + image modal.
- **M4 — Trust & recovery.** Cross-signing, SAS/QR **device verification** UX, **key backup/recovery**.
- **M5 — Android.** `tauri android init`, responsive UI pass, native file picker, run on device/emulator
  (foreground delivery first).
- **M6 — Push.** Sygnal + FCM (+APNs), pushers, `event_id_only`; background delivery on Android/iOS.
- **M7 — iOS.** On the Mac: `tauri ios init`, Xcode signing, APNs, Keychain, capture protection.
- **M8 — Web (optional).** Decide WASM vs `matrix-js-sdk`; ship a web build.
- **M9 — Polish & release.** Tier-1/2 UI features, store assets, export-compliance, review submission,
  CI for all platforms.

Each milestone is a reviewable checkpoint; M1–M4 are the critical path to "it works E2E on desktop."

---

## 13. Risks & open decisions (consolidated)

**Decisions needed from you:**
- **A. ✅ RESOLVED — Continuwuity** (Rust, single binary, embedded DB).
- **B. ✅ RESOLVED — `@you:rw3iss.com`** (server_name fixed; client homeserver URL configurable).
  Homeserver served at gray-cloud `matrix.rw3iss.com` (⚠️ needs a Cloudflare DNS record).
- **C.** Registration: tokens (recommended) vs admin-created vs open.
- **D.** Web later: Rust-SDK-WASM vs `matrix-js-sdk` (defer).
- **E.** Push accounts: create a **Firebase/FCM** project and **Apple Developer** account? (required for
  mobile push). Domain name to use. Brand name (Seqr vs yapnet).

**Risks:**
- **Server RAM** currently too tight for Postgres-based Dendrite — must free/resize first.
- **Dendrite maintenance mode** — recommend the Rust homeserver for longevity.
- **iOS requires a Mac + paid Apple account**; push requires Google/Apple infrastructure (or
  UnifiedPush for Android only).
- **~2 GB media ceiling**; huge files better as external links.
- **Backend rewrite**: our custom crypto/transport is retired — a real scope of work, though the UI
  and much UX survive.
- **Tauri `tauri:`-protocol/session-persistence** pitfall — validate early (M2).

---

## 14. References

- Homeserver landscape: [matrix.org servers](https://matrix.org/ecosystem/servers/),
  [Dendrite (GitHub, "maintenance mode")](https://github.com/matrix-org/dendrite),
  [Synapse vs Dendrite vs Continuwuity 2026 (Pi Stack)](https://www.pistack.xyz/posts/2026-05-02-synapse-vs-dendrite-vs-continuwuity-self-hosted-matrix-server-guide/),
  [conduwuit](https://github.com/x86pup/conduwuit).
- `matrix-rust-sdk`: [GitHub](https://github.com/matrix-org/matrix-rust-sdk),
  [docs](https://matrix-org.github.io/matrix-rust-sdk/matrix_sdk/index.html),
  [sliding sync](https://matrix-org.github.io/matrix-rust-sdk/matrix_sdk/sliding_sync/index.html),
  [DeepWiki overview](https://deepwiki.com/matrix-org/matrix-rust-sdk).
- Push: [Sygnal](https://github.com/matrix-org/sygnal),
  [Push Gateway API](https://spec.matrix.org/unstable/push-gateway-api/),
  [ntfy/UnifiedPush](https://etke.cc/help/extras/ntfy).
- Media limits: [Synapse `max_upload_size` (2 GB barrier)](https://forum.cloudron.io/topic/11032/unable-to-increase-upload-size-above-2gb-matrix),
  [default 50M change](https://github.com/matrix-org/synapse/commit/ca2db5dd0c9fc430a931b4d456fea6a5300b8b42).
</content>
