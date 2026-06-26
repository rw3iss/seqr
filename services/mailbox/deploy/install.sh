#!/usr/bin/env bash
# Idempotent installer for the Seqr mailbox service.
#
# Usage (run on the server, expects the binary at /tmp/seqr-mailbox and the deploy
# files in the same directory as this script):
#   sudo bash install.sh
#
# Installs a dedicated unprivileged user, the binary, config, a hardened systemd
# unit, and opens the firewall port.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
BIN_SRC="${SEQR_BIN_SRC:-/tmp/seqr-mailbox}"
PORT="${SEQR_PORT:-8787}"

echo ">> Creating system user 'seqr' (if absent)"
id seqr &>/dev/null || useradd --system --no-create-home --shell /sbin/nologin seqr

echo ">> Installing binary to /usr/local/bin/seqr-mailbox"
install -m 0755 "$BIN_SRC" /usr/local/bin/seqr-mailbox

echo ">> Installing config to /etc/seqr-mailbox/"
mkdir -p /etc/seqr-mailbox
# Don't clobber an existing customized env file.
if [ ! -f /etc/seqr-mailbox/seqr-mailbox.env ]; then
    install -m 0644 "$HERE/seqr-mailbox.env" /etc/seqr-mailbox/seqr-mailbox.env
fi

echo ">> Installing systemd unit"
install -m 0644 "$HERE/seqr-mailbox.service" /etc/systemd/system/seqr-mailbox.service
systemctl daemon-reload
systemctl enable --now seqr-mailbox.service

echo ">> Opening firewall port ${PORT}/tcp"
if command -v firewall-cmd &>/dev/null && firewall-cmd --state &>/dev/null; then
    firewall-cmd --permanent --add-port="${PORT}/tcp" >/dev/null
    firewall-cmd --reload >/dev/null
fi

echo ">> Status:"
systemctl --no-pager --full status seqr-mailbox.service | head -12 || true
echo ">> Local health check:"
sleep 1
curl -fsS "http://127.0.0.1:${PORT}/health" && echo " <- OK" || echo "HEALTH CHECK FAILED"
