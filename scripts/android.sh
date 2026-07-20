#!/usr/bin/env bash
#
# Build + install (+ launch) the Seqr Android app on a connected device/emulator.
#
# Usage:
#   scripts/android.sh                 # debug APK for arm64 -> install -> launch
#   scripts/android.sh --target x86_64 # for an x86_64 emulator
#   scripts/android.sh --release       # release APK (needs a signing keystore configured)
#   scripts/android.sh --dev           # live-reload dev (tauri android dev)
#   scripts/android.sh --no-launch     # build + install, don't auto-launch
#   scripts/android.sh --uninstall-old # also remove the legacy com.seqr.app package first
#
# Env overrides: ANDROID_HOME, NDK_HOME, JAVA_HOME (auto-detected if unset).
set -euo pipefail

APP_ID="com.seqr.app.android"
LEGACY_APP_ID="com.seqr.app"        # pre-FCM applicationId (installs separately)
TARGET="aarch64"
PROFILE="--debug"
MODE="build"                        # build | dev
LAUNCH=1
UNINSTALL_OLD=0

while [ $# -gt 0 ]; do
	case "$1" in
		--release)        PROFILE="" ;;
		--debug)          PROFILE="--debug" ;;
		--target)         shift; TARGET="${1:?--target needs an ABI}" ;;
		--dev)            MODE="dev" ;;
		--no-launch)      LAUNCH=0 ;;
		--uninstall-old)  UNINSTALL_OLD=1 ;;
		-h|--help)        sed -n '2,20p' "$0"; exit 0 ;;
		*)                echo "unknown arg: $1 (see --help)"; exit 1 ;;
	esac
	shift
done

# --- toolchain env ------------------------------------------------------------
: "${ANDROID_HOME:=$HOME/Android/Sdk}"
export ANDROID_HOME
if [ ! -d "$ANDROID_HOME" ]; then
	echo "ANDROID_HOME not found at '$ANDROID_HOME'. Set ANDROID_HOME and retry." >&2
	exit 1
fi
# Newest installed NDK unless one is pinned.
if [ -z "${NDK_HOME:-}" ]; then
	NDK_HOME="$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1 || true)"
fi
export NDK_HOME
[ -n "$NDK_HOME" ] || { echo "No NDK found under $ANDROID_HOME/ndk. Install one via sdkmanager." >&2; exit 1; }
# JDK 17+ — prefer an explicit JAVA_HOME, else fall back to a Homebrew openjdk.
if [ -z "${JAVA_HOME:-}" ]; then
	for c in /home/linuxbrew/.linuxbrew/opt/openjdk@21/libexec \
	         /home/linuxbrew/.linuxbrew/opt/openjdk@17/libexec \
	         /usr/lib/jvm/java-21-openjdk /usr/lib/jvm/java-17-openjdk; do
		[ -d "$c" ] && { export JAVA_HOME="$c"; break; }
	done
fi

ADB="$ANDROID_HOME/platform-tools/adb"

echo "ANDROID_HOME=$ANDROID_HOME"
echo "NDK_HOME=$NDK_HOME"
echo "JAVA_HOME=${JAVA_HOME:-<system default>}"
echo "target=$TARGET  profile=${PROFILE:-release}  mode=$MODE"

# repo layout: this script lives in <repo>/scripts/
cd "$(dirname "$0")/../apps/desktop"

# --- require a device/emulator ------------------------------------------------
if ! "$ADB" get-state >/dev/null 2>&1; then
	echo ""
	echo "No device/emulator detected. Options:"
	echo "  • Plug in a phone with USB debugging enabled, or"
	echo "  • Start an emulator:"
	echo "      $ANDROID_HOME/emulator/emulator -list-avds"
	echo "      $ANDROID_HOME/emulator/emulator -avd <name> &"
	exit 1
fi

# --- dev mode short-circuits --------------------------------------------------
if [ "$MODE" = "dev" ]; then
	exec pnpm tauri android dev --target "$TARGET"
fi

# --- build --------------------------------------------------------------------
# shellcheck disable=SC2086
pnpm tauri android build --apk $PROFILE --target "$TARGET"

# --- locate the freshest APK --------------------------------------------------
OUT="src-tauri/gen/android/app/build/outputs/apk"
APK="$(find "$OUT" -name '*.apk' -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)"
[ -n "$APK" ] && [ -f "$APK" ] || { echo "No APK produced under $OUT" >&2; exit 1; }
echo ""
echo "APK: $APK"

# --- install (+ optional cleanup of the legacy package) -----------------------
if [ "$UNINSTALL_OLD" = "1" ]; then
	"$ADB" uninstall "$LEGACY_APP_ID" >/dev/null 2>&1 && echo "removed legacy $LEGACY_APP_ID" || true
fi
"$ADB" install -r "$APK"
echo "installed $APP_ID"

# --- launch -------------------------------------------------------------------
if [ "$LAUNCH" = "1" ]; then
	"$ADB" shell monkey -p "$APP_ID" -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 \
		&& echo "launched $APP_ID" \
		|| echo "(couldn't auto-launch; open Seqr from the app drawer)"
fi
