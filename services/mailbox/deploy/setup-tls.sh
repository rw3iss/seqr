#!/usr/bin/env bash
# Add self-signed TLS in front of the mailbox via nginx, and rebind the mailbox to
# localhost so it is only reachable through the TLS terminator.
#
# Usage (on the server, with the deploy files in the same dir):
#   sudo SEQR_HOST=37.27.248.79 bash setup-tls.sh
#
# Prints the certificate (PEM) and its SHA-256 fingerprint at the end — copy the PEM to
# each client as `mailbox_cert.pem` (the client pins it). Idempotent.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
HOST="${SEQR_HOST:-37.27.248.79}"
TLS_PORT="${SEQR_TLS_PORT:-8443}"
TLS_DIR=/etc/seqr-mailbox/tls

echo ">> Generating self-signed CA + leaf (EC P-256, 10y, SAN IP:${HOST}) if absent"
mkdir -p "$TLS_DIR"
if [ ! -f "$TLS_DIR/ca.pem" ]; then
    # CA (this is what the client pins).
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
        -keyout "$TLS_DIR/ca-key.pem" -out "$TLS_DIR/ca.pem" \
        -days 3650 -subj "/CN=seqr-mailbox-ca" \
        -addext "basicConstraints=critical,CA:TRUE" \
        -addext "keyUsage=critical,keyCertSign,cRLSign"
    # Leaf served by nginx: signed by the CA, with SAN + serverAuth (webpki-friendly).
    openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
        -keyout "$TLS_DIR/key.pem" -out "$TLS_DIR/leaf.csr" -subj "/CN=seqr-mailbox"
    openssl x509 -req -in "$TLS_DIR/leaf.csr" -CA "$TLS_DIR/ca.pem" -CAkey "$TLS_DIR/ca-key.pem" \
        -CAcreateserial -out "$TLS_DIR/cert.pem" -days 3650 \
        -extfile <(printf "subjectAltName=IP:%s\nextendedKeyUsage=serverAuth\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature" "$HOST")
    chmod 600 "$TLS_DIR/key.pem" "$TLS_DIR/ca-key.pem"
    rm -f "$TLS_DIR/leaf.csr"
fi

echo ">> Rebinding mailbox to localhost (only nginx reaches it now)"
sed -i 's#^SEQR_MAILBOX_BIND=.*#SEQR_MAILBOX_BIND=127.0.0.1:8787#' /etc/seqr-mailbox/seqr-mailbox.env
systemctl restart seqr-mailbox

echo ">> Installing nginx vhost on :${TLS_PORT}"
sed "s/__TLS_PORT__/${TLS_PORT}/g" "$HERE/nginx-seqr.conf" > /etc/nginx/conf.d/seqr-mailbox.conf
nginx -t
systemctl reload nginx

echo ">> Firewall: open ${TLS_PORT}/tcp, close public 8787/tcp"
if command -v firewall-cmd &>/dev/null && firewall-cmd --state &>/dev/null; then
    firewall-cmd --permanent --add-port="${TLS_PORT}/tcp" >/dev/null
    firewall-cmd --permanent --remove-port=8787/tcp >/dev/null 2>&1 || true
    firewall-cmd --reload >/dev/null
fi

echo ">> CA SHA-256 fingerprint:"
openssl x509 -in "$TLS_DIR/ca.pem" -noout -fingerprint -sha256
echo ">> ===== BEGIN CLIENT CERT (copy to mailbox_cert.pem) ====="
cat "$TLS_DIR/ca.pem"
echo ">> ===== END CLIENT CERT ====="
