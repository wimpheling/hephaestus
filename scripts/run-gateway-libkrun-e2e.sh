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

# With no arguments this wrapper runs the historical libkrun integration
# harness.  A command after `--` is run under the same disposable Caddy
# environment, which lets a composed workload bootstrap Caddy before its
# browser/OIDC setup.
command_args=("${script_dir}/run-libkrun-integration.sh")
if (($# > 0)); then
    [[ "$1" == -- ]] || {
        printf 'usage: %s [-- command [args...]]\n' "${BASH_SOURCE[0]}" >&2
        exit 2
    }
    shift
    (($# > 0)) || {
        printf 'a command is required after --\n' >&2
        exit 2
    }
    command_args=("$@")
fi
readonly command_args

source "${repo_root}/scripts/shell-failure-diagnostics.sh"
heph_shell_failure_init gateway-libkrun-e2e gateway
caddy_image="${HEPHAESTUS_CADDY_TEST_IMAGE:-docker.io/library/caddy@sha256:d8c17a862962def15cde69863a3a463f25a2664942eafd7bdbf050e9c3116b83}"
readonly caddy_image
container_name="hephaestus-gateway-libkrun-caddy-${PPID}-${RANDOM}"
fixture_root="$(mktemp -d)"
phase_timing_path="${HEPH_GCP_PHASE_TIMING_PATH:-}"
phase_timing_open=''

phase_timing_start() {
    local name="$1"
    [[ -n "$phase_timing_path" ]] || return 0
    local source_sha="${HEPH_GCP_PHASE_TIMING_SOURCE_SHA:-}"
    [[ -n "$source_sha" ]] || source_sha="$(git -C "$repo_root" rev-parse HEAD)"
    python3 "$repo_root/scripts/gcp_phase_timing.py" start \
        --path "$phase_timing_path" --phase "$name" --trust workload \
        --clock-domain workload-gateway --source-sha "$source_sha"
    phase_timing_open="$name"
}

phase_timing_end() {
    local name="$1" outcome="$2"
    [[ -n "$phase_timing_path" ]] || return 0
    local source_sha="${HEPH_GCP_PHASE_TIMING_SOURCE_SHA:-}"
    [[ -n "$source_sha" ]] || source_sha="$(git -C "$repo_root" rev-parse HEAD)"
    python3 "$repo_root/scripts/gcp_phase_timing.py" end \
        --path "$phase_timing_path" --phase "$name" --trust workload \
        --clock-domain workload-gateway --source-sha "$source_sha" --outcome "$outcome"
    phase_timing_open=''
}

phase_timing_finish_open() {
    local status="$1" outcome='failed'
    [[ -n "$phase_timing_open" ]] || return 0
    if ((status == 124)); then
        outcome='timed-out'
    elif ((status == 130 || status == 143)); then
        outcome='cancelled'
    fi
    phase_timing_end "$phase_timing_open" "$outcome" || true
}

cleanup() {
    local status=$?
    heph_shell_failure_on_exit "${status}" "${LINENO}"
    heph_shell_failure_begin_cleanup
    phase_timing_finish_open "${status}"
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
public_port="${HEPHAESTUS_CADDY_TEST_PUBLIC_PORT:-$(reserve_port)}"
[[ "${public_port}" =~ ^[1-9][0-9]{0,4}$ && "${public_port}" -le 65535 ]] || {
    printf 'HEPHAESTUS_CADDY_TEST_PUBLIC_PORT must be a valid TCP port\n' >&2
    exit 1
}
tls_enabled="${HEPHAESTUS_CADDY_TEST_TLS:-0}"
case "${tls_enabled}" in
    0|1) ;;
    *)
        printf 'HEPHAESTUS_CADDY_TEST_TLS must be 0 or 1\n' >&2
        exit 1
        ;;
esac
admin_url="http://127.0.0.1:${admin_port}"
if [[ "${tls_enabled}" == 1 ]]; then
    public_url="https://127.0.0.1:${public_port}"
else
    public_url="http://127.0.0.1:${public_port}"
fi
public_listen="127.0.0.1:${public_port}"
ca_cert_path="${fixture_root}/caddy-local-root.pem"
readonly admin_port public_port tls_enabled admin_url public_url public_listen ca_cert_path

if [[ "${tls_enabled}" == 1 ]]; then
    printf '{\n    auto_https disable_redirects\n    admin 127.0.0.1:%s\n}\n\nhttps://127.0.0.1:%s {\n    tls internal\n    respond "gateway configuration pending" 503\n}\n' \
        "${admin_port}" "${public_port}" >"${fixture_root}/Caddyfile"
else
    printf '{\n    auto_https off\n    admin 127.0.0.1:%s\n}\n\nhttp://%s {\n    respond "gateway configuration pending" 503\n}\n' \
        "${admin_port}" "${public_listen}" >"${fixture_root}/Caddyfile"
fi

phase_timing_start gateway-edge-ready
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
phase_timing_end gateway-edge-ready passed

if [[ "${tls_enabled}" == 1 ]]; then
    for command in awk grep; do
        command -v "${command}" >/dev/null || {
            printf 'required command is unavailable: %s\n' "${command}" >&2
            exit 1
        }
    done
    : >"${ca_cert_path}"
    chmod 0600 "${ca_cert_path}"
    if ! curl --silent --show-error --fail --connect-timeout 2 --max-time 5 \
        "${admin_url}/pki/ca/local/certificates" |
        awk '
            /-----BEGIN CERTIFICATE-----/ { certificate_count++ }
            certificate_count == 1 { print }
            /-----END CERTIFICATE-----/ && certificate_count == 1 { exit }
        ' >"${ca_cert_path}"; then
        podman logs "${container_name}" >&2 || true
        printf 'Caddy local CA certificate was unavailable\n' >&2
        exit 1
    fi
    if ! grep -q '^-----BEGIN CERTIFICATE-----$' "${ca_cert_path}" ||
        ! grep -q '^-----END CERTIFICATE-----$' "${ca_cert_path}"; then
        podman logs "${container_name}" >&2 || true
        printf 'Caddy local CA response did not contain one PEM certificate\n' >&2
        exit 1
    fi

    public_status=''
    for attempt in $(seq 1 30); do
        if public_status="$(curl --silent --show-error --cacert "${ca_cert_path}" \
            --connect-timeout 2 --max-time 5 --output /dev/null \
            --write-out '%{http_code}' "${public_url}/" 2>/dev/null)" &&
            [[ "${public_status}" == 503 ]]; then
            break
        fi
        if (( attempt == 30 )); then
            podman logs "${container_name}" >&2 || true
            printf 'Caddy HTTPS public listener did not become ready with its local CA\n' >&2
            exit 1
        fi
        sleep 1
    done
fi

integration_env=(
    HEPHAESTUS_APP_LIBKRUN_E2E=1
    HEPHAESTUS_APP_GATEWAY_CADDY_E2E=1
    "HEPHAESTUS_CADDY_TEST_ADMIN_URL=${admin_url}"
    "HEPHAESTUS_CADDY_TEST_PUBLIC_URL=${public_url}"
    "HEPHAESTUS_CADDY_TEST_LISTEN=${public_listen}"
    "HEPHAESTUS_CADDY_TEST_PUBLIC_PORT=${public_port}"
    "HEPHAESTUS_CADDY_TEST_ENV_COMPLETE=${tls_enabled}"
)
if [[ "${tls_enabled}" == 1 ]]; then
    integration_env+=(
        HEPHAESTUS_CADDY_TEST_TLS=1
        "HEPHAESTUS_CADDY_TEST_CA_CERT=${ca_cert_path}"
    )
fi
env "${integration_env[@]}" "${command_args[@]}"
