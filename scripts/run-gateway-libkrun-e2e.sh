#!/usr/bin/env bash
#
# Run the joined shared-Caddy → daemon → PostgreSQL gateway authority →
# released libkrun handler proof. The libkrun harness owns its disposable
# PostgreSQL, NATS, root filesystem, KVM worker, and cgroup fixture; this
# wrapper owns only the disposable shared Caddy edge.

set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly script_dir
repo_root="$(cd -- "${script_dir}/.." && pwd -P)"
readonly repo_root
source "${repo_root}/scripts/shell-failure-diagnostics.sh"
heph_shell_failure_init gateway-libkrun-e2e gateway
caddy_image="${HEPHAESTUS_CADDY_TEST_IMAGE:-docker.io/library/caddy@sha256:d8c17a862962def15cde69863a3a463f25a2664942eafd7bdbf050e9c3116b83}"
readonly caddy_image
container_name="hephaestus-gateway-libkrun-caddy-${PPID}-${RANDOM}"
fixture_root="$(mktemp -d)"

cleanup() {
    local status=$?
    heph_shell_failure_on_exit "${status}" "${LINENO}"
    heph_shell_failure_begin_cleanup
    podman rm --force "${container_name}" >/dev/null 2>&1 || true
    rm -rf "${fixture_root}"
    return "${status}"
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

admin_port="$(reserve_port)"
public_port="$(reserve_port)"
readonly admin_port public_port
admin_url="http://127.0.0.1:${admin_port}"
public_url="http://127.0.0.1:${public_port}"
public_listen="127.0.0.1:${public_port}"
readonly admin_url public_url public_listen

printf '{\n    auto_https off\n    admin 127.0.0.1:%s\n}\n\nhttp://%s {\n    respond "gateway configuration pending" 503\n}\n' \
    "${admin_port}" "${public_listen}" >"${fixture_root}/Caddyfile"

podman run --detach --rm \
    --name "${container_name}" \
    --network host \
    --volume "${fixture_root}/Caddyfile:/etc/caddy/Caddyfile:ro,Z" \
    "${caddy_image}" \
    caddy run --config /etc/caddy/Caddyfile --adapter caddyfile >/dev/null

for attempt in $(seq 1 30); do
    if curl --silent --fail "${admin_url}/config/" >/dev/null; then
        break
    fi
    sleep 1
    if (( attempt == 30 )); then
        podman logs "${container_name}" >&2 || true
        printf 'Caddy private administration API did not become ready\n' >&2
        exit 1
    fi
done

HEPHAESTUS_APP_LIBKRUN_E2E=1 \
HEPHAESTUS_APP_GATEWAY_CADDY_E2E=1 \
HEPHAESTUS_CADDY_TEST_ADMIN_URL="${admin_url}" \
HEPHAESTUS_CADDY_TEST_PUBLIC_URL="${public_url}" \
HEPHAESTUS_CADDY_TEST_LISTEN="${public_listen}" \
    "${script_dir}/run-libkrun-integration.sh"
