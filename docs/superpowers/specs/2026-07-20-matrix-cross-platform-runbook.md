# Seqr (Matrix) — cross-platform runbook (M5–M9)

Companion to `2026-07-05-matrix-migration-plan.md`. The desktop E2E client (M1–M4) is
**done as compile-verified code**. The phases below are gated on hardware/accounts I can't
provision from this environment, so each is written as **exact steps you run**, plus what's
already prepared in the repo.

Everything runs from `apps/desktop` unless noted. The active backend is chosen at launch
from config (`backend = "matrix"`, default) — see the plan §6.0/§5.

---

## M5 — Android

**✅ BUILT (2026-07-20).** `tauri android init` scaffolded `gen/android`, and a debug APK
**builds successfully** on this machine (Preact → Rust core incl. matrix-sdk cross-compiled
for `aarch64-linux-android`, then Gradle). Toolchain used: JDK 21, Gradle 8.14.3,
`ANDROID_HOME=~/Android/Sdk`, NDK r26 (`ndk/26.3.11579264`), all four Rust android targets.
`aws-lc-sys`/rustls cross-compiled cleanly. **Screen-capture protection** (`FLAG_SECURE`) is
applied in `MainActivity.kt` (parity with desktop). App id `com.seqr.app`.

Artifact: `gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`
(~818 MB **debug**, unstripped — a release build strips this to tens of MB).

**Reproduce / run:**
```bash
export ANDROID_HOME=~/Android/Sdk
export NDK_HOME=$ANDROID_HOME/ndk/26.3.11579264
cd apps/desktop
pnpm tauri android build --apk --debug --target aarch64   # debug APK (auto-signed)
# run on a device/emulator:
adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
# or live-reload dev (boots the AVD 'claimleo' or a plugged-in device):
pnpm tauri android dev
```

**Release (for distribution) — needs a signing keystore:**
```bash
keytool -genkey -v -keystore ~/seqr-release.jks -keyalg RSA -keysize 2048 \
  -validity 10000 -alias seqr
# add signing config to gen/android (keystore.properties) per Tauri docs, then:
pnpm tauri android build --apk --split-per-abi    # stripped release APKs, one per ABI
```

**Notes / gotchas**
- The Matrix SQLite stores use `bundled-sqlite`, so no system sqlite dep on-device. Good.
- `matrix-sdk` compiles for Android on stable dalek 2.x (same graph as desktop; see plan §5).
- File picker + save use `@tauri-apps/plugin-dialog`, which has Android support in v2.
- **Screen-capture protection**: the desktop code has a Linux no-op / macOS+Windows impl.
  Android equivalent is `FLAG_SECURE` on the activity window — add in the generated
  `gen/android` MainActivity if you want capture protection there (optional).
- First run: verify this device from an already-verified device (Security modal → Verify),
  or run `matrix_recover` with the recovery key to pull cross-signing + key backup.

---

## M6 — Push notifications

**✅ Gateway DEPLOYED (2026-07-20).** Sygnal (element-hq 0.17) runs on the VPS as
`sygnal.service` (Python 3.12 venv `/opt/sygnal`, user `sygnal`, `127.0.0.1:5000`),
configured with your **FCM v1** service account (`/etc/sygnal/fcm.json`, project `seqr-comm`,
app id `com.seqr.app.android`). It's exposed through the existing matrix vhost at
`/_matrix/push/`, so the pusher URL is **`https://matrix.rw3iss.com/_matrix/push/v1/notify`**
(verified: malformed POST → `400` from Sygnal). Client command **`matrix_register_pusher`**
is implemented (`api.matrixRegisterPusher(pushKey, appId)`).

**Remaining (gated on the Android build):** obtain the **FCM registration token** on-device
(needs the Firebase Android client config `google-services.json` + the FCM SDK wired into the
`gen/android` project from M5), then call `matrixRegisterPusher(token, "com.seqr.app.android")`
after login. **iOS/APNs:** add a `com.seqr.app.ios` app block to `/etc/sygnal/sygnal.yaml`
with your `.p8` key (needs the Apple push key from M7).

**Why a gateway:** Matrix push goes homeserver → **Sygnal** (push gateway) → FCM (Android) /
APNs (iOS). Continuwuity emits the push; Sygnal holds your FCM/APNs secrets and forwards.
Use `event_id_only` so no message content leaves the gateway; the app fetches + decrypts on
wake.

**You need:** a Firebase project (FCM v1 service-account JSON) for Android; an Apple push key
(`.p8`, APNs) for iOS.

**Deploy Sygnal on the VPS (162.35.181.92)** — mirrors the mailbox pattern (systemd, `/etc`,
`/var/lib`, nginx TLS on a gray-cloud subdomain, e.g. `push.rw3iss.com`):

```bash
# on 162.35.181.92
python3 -m venv /opt/sygnal && /opt/sygnal/bin/pip install matrix-sygnal
sudo install -d -o sygnal -g sygnal /etc/sygnal /var/lib/sygnal
# /etc/sygnal/sygnal.yaml — apps:
#   com.seqr.app.android: { type: gcm, api_version: v1,
#     project_id: <fcm-project>, service_account_file: /etc/sygnal/fcm.json }
#   com.seqr.app.ios:     { type: apns, keyfile: /etc/sygnal/apns.p8,
#     key_id: <..>, team_id: <..>, topic: com.seqr.app }
# systemd unit -> /opt/sygnal/bin/python -m sygnal, listen 127.0.0.1:5000
# nginx: push.rw3iss.com (gray-cloud DNS + certbot) -> 127.0.0.1:5000
```

**Client side (to implement):** register a pusher after login with the FCM/APNs token:
`client.pusher().set(pusher)` (matrix-sdk `Pusher` with the Sygnal `url`,
`app_id = com.seqr.app.<platform>`, `pushkey = <device push token>`, `event_id_only`
format). Wire a Tauri command `matrix_register_pusher(token)` called from the mobile
push-token plugin callback. UnifiedPush/ntfy is a Google-free alternative for Android.

**Status:** un-exercisable here (no push creds). Architecture + deploy above are the plan.

---

## M7 — iOS

**You need:** a Mac, Xcode, an Apple Developer account (signing), APNs key (shared with M6).

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cd apps/desktop
pnpm tauri ios init            # generates gen/apple (one-time)
pnpm tauri ios dev             # run on simulator/device
pnpm tauri ios build           # archive; sign in Xcode (Team + provisioning)
```

- Screen-capture protection on iOS: `isSecureTextEntry`-style overlay / `UIScreen.isCaptured`
  observer in the generated project (optional; parity with desktop).
- Keychain: matrix-sdk session/store passphrase should live in the iOS Keychain rather than a
  plaintext file (also the desktop `TODO(security)` in `matrix/client.rs`).

---

## M8 — Web (optional)

**Decision: `matrix-js-sdk`, as a separate build.** The Tauri desktop client's backend is a
**Rust core reached over Tauri IPC** — that seam doesn't exist in a browser, so a web build
can't reuse `src/matrix/*`. Options:

1. **`matrix-js-sdk`** in a Preact web app reusing the *presentational* components
   (`MatrixChat`/`MatrixLogin` refactored to take a data layer via props/context). Recommended.
2. Compile the Rust core to **WASM** (`matrix-sdk` has a `js`/`indexeddb` path) and keep one
   codebase — heavier, less mature for full app use.

Either way it's a distinct target; scoped, not built. Reuse the SCSS tokens + component markup.

---

## M9 — Polish & release

**Already in place:** `.github/workflows/release.yml` (tauri-action) builds macOS `.dmg`
(universal), Windows `.msi`, Linux `.AppImage`/`.deb` on a `v*` tag → drafts a GitHub Release.
Unsigned (no Apple/Windows certs) — see repo `CLAUDE.md` "Packaging / distribution".

**Remaining (itemized):**
- **Tier-1 UI parity** (plan §11): reactions, replies/threads, edits/redaction, read receipts,
  typing, markdown render, richer room-management (name/topic/avatar, power levels).
- **Store assets**: icons (have `com.seqr.app` identifier), screenshots, descriptions,
  privacy labels; **export-compliance** (uses standard crypto → self-classification/ECCN).
- **Signing** for store distribution: Apple Developer cert (macOS/iOS), Play signing key,
  optional Windows Authenticode.
- **CI extension**: add Android (`--aab`) + iOS jobs to `release.yml` once M5/M7 init'd.
- **Security TODO**: encrypt the Matrix session file at rest (Argon2id from login password),
  per `apps/desktop/src-tauri/src/matrix/client.rs`.

---

## What's runtime-verifiable *now* (desktop, your machine)

```bash
cd apps/desktop && pnpm install && pnpm tauri dev
# Sign in as @ryan:rw3iss.com (or register another account with the token in
#   /etc/conduwuit/conduwuit.toml on the server), create a room / DM, send text + a file,
#   open Security → enable key backup. Run a second instance on another profile/machine to
#   confirm E2E + that the session survives a restart.
```
