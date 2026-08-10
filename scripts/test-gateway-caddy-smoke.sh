#!/usr/bin/env bash
# Proves the loopback-only Caddy administration adapter and one shared-Caddy
# `/gateway/` request path. The Rust test starts the private dispatcher fixture;
# this script owns only the disposable Caddy edge.
set -euo pipefail

readonly repo_root="$(git rev-parse --show-toplevel)"
readonly caddy_image="${HEPHAESTUS_CADDY_TEST_IMAGE:-docker.io/library/caddy:2.10.2-alpine}"
readonly container_name="hephaestus-gateway-caddy-${PPID}-${RANDOM}"
readonly fixture_root="$(mktemp -d)"

cleanup() {
    podman rm --force "${container_name}" >/dev/null 2>&1 || true
    rm -rf "${fixture_root}"
}
trap cleanup EXIT

for command in cargo curl podman python3; do
    command -v "${command}" >/dev/null || {
        printf 'required command is unavailable: %s\n' "${command}" >&2
        exit 1
    }
done

reserve_port() {
    python3 -c 'import socket; sock = socket.socket(); sock.bind(("127.0.0.1", 0)); print(sock.getsockname()[1]); sock.close()'
}

readonly admin_port="$(reserve_port)"
readonly public_port="$(reserve_port)"
readonly admin_url="http://127.0.0.1:${admin_port}"
readonly public_url="http://127.0.0.1:${public_port}"
readonly public_listen="127.0.0.1:${public_port}"

cat >"${fixture_root}/Caddyfile" <<EOF
{
    auto_https off
    admin 127.0.0.1:${admin_port}
}

http://${public_listen} {
    respond "gateway configuration pending" 503
}
EOF

podman run --detach --rm \
    --name "${container_name}" \
    --network host \
    --volume "${fixture_root}/Caddyfile:/etc/caddy/Caddyfile:ro,Z" \
    "${caddy_image}" \
    caddy run --config /etc/caddy/Caddyfile --adapter caddyfile >/dev/null

for _attempt in $(seq 1 30); do
    if curl --silent --fail "${admin_url}/config/" >/dev/null; then
        break
    fi
    sleep 1
    if (( _attempt == 30 )); then
        podman logs "${container_name}" >&2 || true
        printf 'Caddy private administration API did not become ready\n' >&2
        exit 1
    fi
done

HEPHAESTUS_CADDY_TEST_ADMIN_URL="${admin_url}" \
HEPHAESTUS_CADDY_TEST_PUBLIC_URL="${public_url}" \
HEPHAESTUS_CADDY_TEST_LISTEN="${public_listen}" \
cargo test --manifest-path "${repo_root}/Cargo.toml" \
    --package gateway-edge --test caddy_ingress -- --nocapture

printf 'shared Caddy gateway ingress smoke test passed\n'
