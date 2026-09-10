#!/usr/bin/env bash
#
# Build a pinned Ubuntu guest fixture and run a real, non-root libkrun
# integration scenario. All generated files, containers, and cgroups are
# removed on exit.

set -Eeuo pipefail

readonly DEFAULT_UBUNTU_IMAGE="docker.io/library/ubuntu@sha256:52df9b1ee71626e0088f7d400d5c6b5f7bb916f8f0c82b474289a4ece6cf3faf"
readonly DEFAULT_POSTGRES_IMAGE="docker.io/library/postgres@sha256:af194ccf3e2d7fe367012c7b88ce8b816c5c889b18a5b316799a1f0d7eac746a"
readonly DEFAULT_NATS_IMAGE="docker.io/library/nats@sha256:e4bf19f15fd3218814a4e3c9e0064e1334bd8aa20d5984b9f1a0afd084f8cc00"
readonly ZOT_IMAGE="ghcr.io/project-zot/zot@sha256:6f7bf2b8e43437c7c3a121bc80214845c85f27321e66f2ff4be6bf4220775fd7"
readonly GUEST_TARGET="x86_64-unknown-linux-musl"
readonly REQUIRED_CONTROLLERS=(cpu io memory pids)

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly script_dir
repo_root="$(cd -- "${script_dir}/.." && pwd -P)"
readonly repo_root
ubuntu_image="${HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE:-${DEFAULT_UBUNTU_IMAGE}}"
readonly ubuntu_image
postgres_image="${HEPHAESTUS_POSTGRES_TEST_IMAGE:-${DEFAULT_POSTGRES_IMAGE}}"
readonly postgres_image
nats_image="${HEPHAESTUS_NATS_TEST_IMAGE:-${DEFAULT_NATS_IMAGE}}"
readonly nats_image

fixture_root=""
container_name=""
postgres_container_name=""
nats_container_name=""
zot_container_name=""
zot_port=""
cgroup_root=""
postgres_url="${HEPHAESTUS_POSTGRES_TEST_URL:-}"
nats_url="${HEPHAESTUS_NATS_TEST_URL:-}"
diagnostics_dir="${HEPHAESTUS_LIBKRUN_DIAGNOSTICS_DIR:-${HEPHAESTUS_COOKING_DIAGNOSTICS_DIR:-}}"
rust_builder_image="${HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE:-}"
rust_builder_root=""
builder_operation_root=""
verifier_operation_root=""
repository_image_workflow_file="${HEPHAESTUS_LOCAL_ROOT:-${repo_root}/.local/hephaestus}/repository-images/workflow.env"
builder_vm_image=""
builder_layout=""
verifier_vm_image=""
verifier_layout=""
base_layout_manifest=""
builder_image_loaded=false
verifier_image_loaded=false

reserve_port() {
    python3 -c 'import socket; sock = socket.socket(); sock.bind(("127.0.0.1", 0)); print(sock.getsockname()[1]); sock.close()'
}

die() {
    printf 'libkrun integration: %s\n' "$*" >&2
    exit 1
}

if [[ -n "${diagnostics_dir}" ]]; then
    [[ "${diagnostics_dir}" = /* && ! -L "${diagnostics_dir}" ]] ||
        die "diagnostics directory must be an absolute non-symlink path"
    mkdir -p -- "${diagnostics_dir}"
    chmod 700 -- "${diagnostics_dir}"
fi

# The source virtio-fs mount is serviced by the host-side libkrun worker. A
# default soft nofile limit of 1024 is exhausted by the vendored cooking
# workspace even when the guest child raises its own limit.
ulimit -n 65536 || die 'the host nofile limit cannot be raised to 65536'

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command is missing: $1"
}

require_digest_reference() {
    local name="$1"
    local reference="$2"
    [[ "${reference}" =~ ^[a-zA-Z0-9._:/-]+@sha256:[0-9a-f]{64}$ ]] ||
        die "${name} must be an immutable sha256 reference: ${reference}"
}

materialize_image() {
    local reference="$1"
    local destination="$2"
    local label="$3"

    mkdir -p -- "${destination}"
    chmod 0700 -- "${destination}"
    container_name="hephaestus-libkrun-${label}-$$"
    podman image exists "${reference}" || podman pull "${reference}"
    podman create --name "${container_name}" "${reference}" /bin/true >/dev/null
    podman export "${container_name}" | tar -C "${destination}" -xf -
    podman rm "${container_name}" >/dev/null
    container_name=""
}

materialize_layout_image() {
    local reference="$1"
    local layout="$2"
    local destination="$3"
    local label="$4"

    [[ -f "${layout}/index.json" && -f "${layout}/oci-layout" ]] ||
        die "reviewed OCI layout is unavailable: ${layout}"
    # The operation image is loaded from the reviewed local layout into the
    # disposable Podman store. This avoids depending on a registry pull for
    # the builder and verifier roots while preserving the exact digest.
    if ! podman image exists "${reference}"; then
        skopeo copy "oci:${layout}" "containers-storage:${reference}" >/dev/null
        case "${label}" in
            oci-builder) builder_image_loaded=true ;;
            oci-verifier) verifier_image_loaded=true ;;
        esac
    fi
    materialize_image "${reference}" "${destination}" "${label}"
}

workflow_value() {
    local key="$1"
    awk -F= -v expected="${key}" '$1 == expected { print substr($0, index($0, "=") + 1); found=1 } END { if (!found) exit 1 }' \
        "${repository_image_workflow_file}"
}

load_repository_image_workflow() {
    [[ -f "${repository_image_workflow_file}" ]] ||
        die "cooking project-image proof requires the reviewed repository-image workflow"
    builder_vm_image="$(workflow_value builder_vm_image)"
    builder_layout="$(workflow_value builder_layout)"
    verifier_vm_image="$(workflow_value verifier_vm_image)"
    verifier_layout="$(workflow_value verifier_layout)"
    base_layout_manifest="$(workflow_value base_layout_manifest)"
    [[ "${builder_vm_image}" =~ @sha256:[0-9a-f]{64} && "${verifier_vm_image}" =~ @sha256:[0-9a-f]{64} ]] ||
        die "repository-image workflow operation references must be digest-pinned"
    [[ "${base_layout_manifest}" = /* && -f "${base_layout_manifest}" ]] ||
        die "repository-image workflow base-layout manifest is unavailable"
}

prepare_guest_root() {
    local root="$1"
    install -D -m 0755 \
        "${repo_root}/target/${GUEST_TARGET}/release/heph-init" \
        "${root}/usr/libexec/hephaestus/heph-init"
    install -D -m 0755 \
        "${repo_root}/target/${GUEST_TARGET}/release/heph-integration-check" \
        "${root}/usr/libexec/hephaestus/integration-check.payload"
    # libkrun's embedded DHCP setup runs before this workload. Keep the
    # fixture diagnostic immediately before the check so a network failure
    # records only bounded guest state that the workload actually sees.
    install -D -m 0755 /dev/stdin \
        "${root}/usr/libexec/hephaestus/integration-check" <<'EOF'
#!/bin/sh
krun_dhcp='absent'
if [ -r /proc/cmdline ]; then
    krun_dhcp="$(awk '{ for (i = 1; i <= NF; i++) { if ($i == "KRUN_DHCP=1") { print "1"; exit } if ($i ~ /^KRUN_DHCP=/) { print "other"; exit } } }' /proc/cmdline)"
    krun_dhcp="${krun_dhcp:-absent}"
fi
{
    printf '%s\n' "guest-network-diagnostics: KRUN_DHCP=${krun_dhcp}"
    printf '%s\n' 'guest-network-diagnostics: interfaces'
    if [ -d /sys/class/net ]; then
        find /sys/class/net -mindepth 1 -maxdepth 1 -printf '%f\n' | sort | head -n 16
    else
        printf '%s\n' 'unavailable: /sys/class/net'
    fi
    printf '%s\n' 'guest-network-diagnostics: proc-net-dev'
    if [ -r /proc/net/dev ]; then
        sed -n '1,33p' /proc/net/dev
    else
        printf '%s\n' 'unavailable: /proc/net/dev'
    fi
    printf '%s\n' 'guest-network-diagnostics: nameservers'
    sed -n '/^[[:space:]]*nameserver[[:space:]]/p' /etc/resolv.conf 2>/dev/null | head -n 8
    printf '%s\n' 'guest-network-diagnostics: ipv4-routes'
    if [ -r /proc/net/route ]; then
        sed -n '1,33p' /proc/net/route
    else
        printf '%s\n' 'unavailable: /proc/net/route'
    fi
    printf '%s\n' 'guest-network-diagnostics: ipv4-addresses'
    if command -v ip >/dev/null 2>&1; then
        ip -4 addr show 2>&1 | sed -n '1,80p'
    else
        printf '%s\n' 'unavailable: ip command'
    fi
    printf '%s\n' 'guest-network-diagnostics: ipv4-route-command'
    if command -v ip >/dev/null 2>&1; then
        ip -4 route show 2>&1 | sed -n '1,40p'
    else
        printf '%s\n' 'unavailable: ip command'
    fi
} >&2
exec "$(dirname "$0")/integration-check.payload" "$@"
EOF
    grep -qE '(^|:)10001:' "${root}/etc/passwd" &&
        die "integration image already assigns guest UID 10001: ${root}"
    grep -qE '(^|:)10001:' "${root}/etc/group" &&
        die "integration image already assigns guest GID 10001: ${root}"
    printf 'heph-agent:x:10001:10001:Hephaestus agent:/nonexistent:/sbin/nologin\n' \
        >>"${root}/etc/passwd"
    printf 'heph-agent:x:10001:\n' >>"${root}/etc/group"
}

run_as_guest_owner() {
    if [[ "$(id -u)" -eq 10001 && "$(id -g)" -eq 10001 ]]; then
        "$@"
    else
        unshare --map-user 10001 --map-group 10001 "$@"
    fi
}

contains_word() {
    local words="$1"
    local expected="$2"
    [[ " ${words} " == *" ${expected} "* ]]
}

network_snapshot() {
    {
        find /sys/class/net -mindepth 1 -maxdepth 1 -printf '%f\n' | sort
        cat /proc/net/route
        cat /proc/net/ipv6_route
    } | sha256sum | awk '{ print $1 }'
}

published_port() {
    local container="$1"
    local container_port="$2"
    local mapping

    mapping="$(podman port "${container}" "${container_port}/tcp")"
    [[ "${mapping}" == 127.0.0.1:* ]] ||
        die "unexpected port mapping for ${container}: ${mapping}"
    printf '%s\n' "${mapping##*:}"
}

wait_for_postgres() {
    local container="$1"

    for _attempt in {1..300}; do
        if podman exec "${container}" \
            pg_isready --quiet --username postgres --dbname hephaestus; then
            return
        fi
        sleep 0.1
    done
    podman logs "${container}" >&2 || true
    die "PostgreSQL did not become ready"
}

wait_for_nats() {
    local container="$1"

    for _attempt in {1..300}; do
        if podman logs "${container}" 2>&1 | grep -q 'Server is ready'; then
            return
        fi
        sleep 0.1
    done
    podman logs "${container}" >&2 || true
    die "NATS did not become ready"
}

start_golden_services() {
    local port

    if [[ -z "${postgres_url}" ]]; then
        postgres_container_name="hephaestus-golden-postgres-$$"
        podman run --detach --rm \
            --name "${postgres_container_name}" \
            --env POSTGRES_PASSWORD=postgres \
            --env POSTGRES_DB=hephaestus \
            --publish 127.0.0.1::5432 \
            "${postgres_image}" >/dev/null
        wait_for_postgres "${postgres_container_name}"
        port="$(published_port "${postgres_container_name}" 5432)"
        postgres_url="postgres://postgres:postgres@127.0.0.1:${port}/hephaestus?sslmode=disable"
    fi

    if [[ -z "${nats_url}" ]]; then
        nats_container_name="hephaestus-golden-nats-$$"
        podman run --detach --rm \
            --name "${nats_container_name}" \
            --publish 127.0.0.1::4222 \
            "${nats_image}" -js >/dev/null
        wait_for_nats "${nats_container_name}"
        port="$(published_port "${nats_container_name}" 4222)"
        nats_url="nats://127.0.0.1:${port}"
    fi

    if [[ "${HEPHAESTUS_APP_COOKING_BUILD_PROOF:-0}" == "1" ]]; then
        local zot_config="${fixture_root}/zot/config.json"
        local zot_certificate="${fixture_root}/zot/verification.crt"
        local zot_storage="${fixture_root}/zot/storage"
        local zot_private_key="${fixture_root}/zot/registry-token-signing-key.pem"
        local registry_callback_token='cooking-fixture-notification-token-0123456789abcdef'
        zot_port="$(reserve_port)"
        mkdir -p -- "${fixture_root}/zot" "${zot_storage}"
        openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 \
            -out "${zot_private_key}" >/dev/null 2>&1
        openssl req -new -x509 -key "${zot_private_key}" \
            -out "${zot_certificate}" -days 1 \
            -subj '/CN=hephaestus-cooking-fixture' >/dev/null 2>&1
        sed \
            -e 's|{{ zot.storage_root }}|/var/lib/registry|g' \
            -e 's|{{ zot.private_address }}|0.0.0.0|g' \
            -e 's|{{ zot.private_port }}|55000|g' \
            -e 's|{{ hephaestus.registry_token_realm }}|http://127.0.0.1:1/v1/registry/token|g' \
            -e "s|{{ hephaestus.registry_service }}|127.0.0.1:${zot_port}|g" \
            -e 's|{{ hephaestus.registry_notification_sink_url }}|http://127.0.0.1:1/internal/zot-notifications|g' \
            -e "s|{{ hephaestus.registry_notification_callback_token }}|${registry_callback_token}|g" \
            "${repo_root}/deploy/zot/zot-config.json.tera" >"${zot_config}"
        chmod 0444 -- "${zot_config}" "${zot_certificate}"
        chmod 0400 -- "${zot_private_key}"
        chmod 0700 -- "${zot_storage}"
        zot_container_name="hephaestus-golden-zot-$$"
        podman run --detach --rm \
            --name "${zot_container_name}" \
            --publish "127.0.0.1:${zot_port}:55000" \
            --read-only --tmpfs /tmp:rw,noexec,nosuid,nodev \
            --cap-drop all --security-opt no-new-privileges \
            --security-opt label=disable \
            --volume "${zot_config}:/etc/zot/config.json:ro" \
            --volume "${zot_certificate}:/etc/zot/verification.crt:ro" \
            --volume "${zot_storage}:/var/lib/registry:rw" \
            --entrypoint /usr/local/bin/zot-linux-amd64 \
            "${ZOT_IMAGE}" serve /etc/zot/config.json >/dev/null
        for _attempt in {1..100}; do
            if [[ "$(curl --silent --output /dev/null --write-out '%{http_code}' "http://127.0.0.1:${zot_port}/v2/")" == "401" ]]; then
                break
            fi
            sleep 0.1
        done
        [[ "$(curl --silent --output /dev/null --write-out '%{http_code}' "http://127.0.0.1:${zot_port}/v2/")" == "401" ]] || {
            podman logs "${zot_container_name}" >&2 || true
            die "test-owned Zot registry did not become ready"
        }
        export HEPHAESTUS_TEST_REGISTRY_SERVICE="127.0.0.1:${zot_port}"
        export HEPHAESTUS_TEST_REGISTRY_ORIGIN="http://127.0.0.1:${zot_port}/"
        export HEPHAESTUS_TEST_REGISTRY_PRIVATE_KEY="${zot_private_key}"
        export HEPHAESTUS_TEST_REGISTRY_KEY_ID="local-v1"
    fi
}

usable_cgroup_parent() {
    local candidate="$1"
    local available
    local enabled

    [[ -d "${candidate}" ]] || return 1
    [[ -w "${candidate}" && -w "${candidate}/cgroup.subtree_control" ]] || return 1
    available="$(<"${candidate}/cgroup.controllers")"
    enabled="$(<"${candidate}/cgroup.subtree_control")"
    for controller in "${REQUIRED_CONTROLLERS[@]}"; do
        contains_word "${available}" "${controller}" || return 1
        contains_word "${enabled}" "${controller}" || return 1
    done
}

discover_cgroup_parent() {
    local user_group
    local candidate
    local current

    if [[ -n "${HEPHAESTUS_LIBKRUN_CGROUP_PARENT:-}" ]]; then
        candidate="${HEPHAESTUS_LIBKRUN_CGROUP_PARENT}"
        usable_cgroup_parent "${candidate}" ||
            die "configured cgroup parent is not writable with cpu/io/memory/pids delegated: ${candidate}"
        printf '%s\n' "${candidate}"
        return
    fi

    if command -v systemctl >/dev/null 2>&1; then
        user_group="$(systemctl --user show -p ControlGroup --value 2>/dev/null || true)"
        candidate="/sys/fs/cgroup${user_group}"
        if [[ -n "${user_group}" ]] && usable_cgroup_parent "${candidate}"; then
            printf '%s\n' "${candidate}"
            return
        fi
    fi

    current="$(awk -F: '$1 == "0" { print $3 }' /proc/self/cgroup)"
    candidate="/sys/fs/cgroup${current}"
    while [[ "${candidate}" == /sys/fs/cgroup/* ]]; do
        if usable_cgroup_parent "${candidate}"; then
            printf '%s\n' "${candidate}"
            return
        fi
        candidate="$(dirname -- "${candidate}")"
    done
    die "no writable delegated cgroup parent found; set HEPHAESTUS_LIBKRUN_CGROUP_PARENT"
}

cleanup_cgroup() {
    [[ -n "${cgroup_root}" && -d "${cgroup_root}" ]] || return
    if [[ -w "${cgroup_root}/cgroup.kill" ]]; then
        printf '1\n' >"${cgroup_root}/cgroup.kill" 2>/dev/null || true
    fi
    for _attempt in {1..100}; do
        if ! grep -q '^populated 1$' "${cgroup_root}/cgroup.events" 2>/dev/null; then
            break
        fi
        sleep 0.01
    done
    find "${cgroup_root}" -mindepth 1 -depth -type d -exec rmdir -- {} + 2>/dev/null || true
    rmdir -- "${cgroup_root}" 2>/dev/null || true
}

redact_diagnostics() {
    sed -E \
        -e 's/(authorization:[[:space:]]*Bearer[[:space:]]+)[^[:space:]]+/\1[REDACTED]/Ig' \
        -e 's/(x-telegram-bot-api-secret-token:[[:space:]]*)[^[:space:]]+/\1[REDACTED]/Ig' \
        -e 's/(HEPHAESTUS_[A-Z0-9_]*(SECRET|TOKEN|KEY)[A-Z0-9_]*=)[^[:space:]]+/\1[REDACTED]/g' \
        -e 's/golden-brokered-provider-sentinel-5d1a/[REDACTED]/g' \
        -e 's/cooking-inbound-only-fixture-sentinel/[REDACTED]/g' \
        -e 's/cooking-model-only-fixture-sentinel-724c/[REDACTED]/g' \
        -e 's/cooking-relay-only-fixture-sentinel-819e/[REDACTED]/g' \
        -e 's/cooking-model-rotated-fixture-sentinel-936f/[REDACTED]/g' \
        -e 's/cooking-inbound-rotated-fixture-sentinel-157a/[REDACTED]/g' \
        -e 's/cooking-relay-rotated-fixture-sentinel-482b/[REDACTED]/g'
}

diagnostics_body() {
    printf 'libkrun integration failed; redacted service diagnostics:\n'
    if [[ -n "${fixture_root}" && -d "${fixture_root}/runtime" ]]; then
        printf 'guest passt logs (bounded):\n'
        while IFS= read -r passt_log; do
            printf '%s\n' "--- ${passt_log} (last 200 lines) ---"
            tail -200 -- "${passt_log}" 2>&1 || true
        done < <(find "${fixture_root}/runtime" -type f -name passt.log -print | sort | head -n 4)
    fi
    for service in "${postgres_container_name}" "${nats_container_name}" "${zot_container_name}"; do
        [[ -n "${service}" ]] || continue
        if podman container exists "${service}" 2>/dev/null; then
            podman inspect --format \
                '  container={{.Name}} state={{.State.Status}} exit={{.State.ExitCode}}' \
                "${service}" 2>&1 || true
            podman logs "${service}" 2>&1 | tail -100 || true
        fi
    done
}

postgres_lifecycle_snapshot() {
    # This is intentionally limited to the disposable service started by this
    # script. An externally supplied database must never be queried or retained
    # by failure diagnostics.
    [[ -n "${postgres_container_name}" ]] || return 0

    local destination='/dev/stderr'
    local retained=false
    if [[ -n "${diagnostics_dir}" ]]; then
        destination="$(mktemp "${diagnostics_dir}/libkrun-postgres-${PPID}.XXXXXX")" || {
            printf 'postgres lifecycle snapshot unavailable\n' >&2
            return 0
        }
        chmod 600 -- "${destination}"
        retained=true
    fi

    # psql emits one JSON object per line. VM log bytes are decoded and passed
    # through the shared fixture-credential stream redactor before this file
    # can be retained; the sanitizer also enforces per-chunk and total caps.
    set +e
    timeout --kill-after=2s 8s podman exec -i "${postgres_container_name}" psql \
        --username postgres \
        --dbname hephaestus \
        --no-password \
        --no-psqlrc \
        --quiet \
        --tuples-only \
        --no-align \
        --field-separator='|' \
        --set=ON_ERROR_STOP=1 \
        2>/dev/null <<'SQL' | python3 "${repo_root}/scripts/redact-run-snapshot.py" >"${destination}"
SET statement_timeout = '3s';
SET lock_timeout = '1s';
SELECT json_build_object('surface', 'runs', 'count', count(*))::text FROM runs;
SELECT json_build_object(
           'surface', 'run', 'id', id, 'instance_id', instance_id,
           'command_id', command_id, 'state', state, 'outcome', outcome,
           'exit_code', exit_code, 'exit_signal', exit_signal,
           'volume_id', volume_id, 'lease_id', lease_id,
           'failure_present', failure IS NOT NULL,
           'created_at', created_at, 'updated_at', updated_at
       )::text
  FROM runs
 ORDER BY updated_at DESC, id
 LIMIT 100;
SELECT json_build_object(
           'surface', 'agent_update',
           'id', update_record.id,
           'instance_id', update_record.instance_id,
           'source_revision_id', update_record.expected_current_revision_id,
           'candidate_revision_id', update_record.candidate_revision_id,
           'state', CASE update_record.state
               WHEN 'candidate' THEN 'candidate'
               WHEN 'draining' THEN 'draining'
               WHEN 'hook_running' THEN 'hook_running'
               WHEN 'hook_committed' THEN 'hook_committed'
               WHEN 'activated' THEN 'activated'
               WHEN 'rejected' THEN 'rejected'
               WHEN 'compatibility_unknown' THEN 'compatibility_unknown'
               WHEN 'activation_recovery' THEN 'activation_recovery'
               ELSE 'unknown'
           END,
           'final_decision', CASE update_record.final_decision
               WHEN 'activated' THEN 'activated'
               WHEN 'agent_rejected' THEN 'agent_rejected'
               WHEN 'unknown' THEN 'unknown'
               WHEN 'recovery' THEN 'recovery'
               ELSE 'unknown'
           END,
           'hook_run_id', update_record.hook_run_id,
           'hook_exit_code', update_record.hook_exit_code,
           'hook_exit_signal', update_record.hook_exit_signal,
           'created_at', update_record.created_at,
           'updated_at', update_record.updated_at,
           'completed_at', update_record.completed_at,
           'instance_state', CASE instance.state
               WHEN 'active' THEN 'active'
               WHEN 'disabled' THEN 'disabled'
               WHEN 'update_draining' THEN 'update_draining'
               WHEN 'updating' THEN 'updating'
               WHEN 'update_rejected' THEN 'update_rejected'
               WHEN 'paused_unknown_state' THEN 'paused_unknown_state'
               WHEN 'paused_activation_recovery' THEN 'paused_activation_recovery'
               WHEN 'recovering' THEN 'recovering'
               WHEN 'removed' THEN 'removed'
               ELSE 'unknown'
           END,
           'instance_active_revision_id', instance.active_revision_id,
           'instance_run_gate_open', instance.run_gate_open,
           'hook_run_instance_id', hook_run.instance_id,
           'hook_run_revision_id', hook_run.instance_revision_id,
           'hook_run_kind', CASE hook_run.run_kind
               WHEN 'normal' THEN 'normal'
               WHEN 'update' THEN 'update'
               ELSE 'unknown'
           END,
           'hook_run_state', CASE hook_run.state
               WHEN 'queued' THEN 'queued'
               WHEN 'leasing_volume' THEN 'leasing_volume'
               WHEN 'provisioning' THEN 'provisioning'
               WHEN 'starting' THEN 'starting'
               WHEN 'running' THEN 'running'
               WHEN 'succeeded' THEN 'succeeded'
               WHEN 'failed' THEN 'failed'
               WHEN 'cancelled' THEN 'cancelled'
               WHEN 'cleaning_up' THEN 'cleaning_up'
               WHEN 'cleaned_up' THEN 'cleaned_up'
               ELSE 'unknown'
           END,
           'hook_run_outcome', CASE hook_run.outcome
               WHEN 'succeeded' THEN 'succeeded'
               WHEN 'failed' THEN 'failed'
               WHEN 'cancelled' THEN 'cancelled'
               ELSE 'unknown'
           END,
           'hook_run_exit_code', hook_run.exit_code,
           'hook_run_exit_signal', hook_run.exit_signal,
           'hook_run_created_at', hook_run.created_at,
           'hook_run_updated_at', hook_run.updated_at
       )::text
  FROM agent_updates AS update_record
  JOIN agent_instances AS instance ON instance.id = update_record.instance_id
  LEFT JOIN runs AS hook_run ON hook_run.id = update_record.hook_run_id
 ORDER BY update_record.updated_at DESC, update_record.id
 LIMIT 256;
WITH failed_runs AS (
    SELECT id
      FROM runs
     WHERE outcome = 'failed'
     ORDER BY updated_at DESC, id
     LIMIT 100
)
SELECT json_build_object(
           'surface', 'mailbox_delivery_attempt',
           'event_id', attempt.event_id,
           'run_id', attempt.run_id,
           'attempt_number', attempt.attempt_number,
           'state', attempt.state,
           'state_volume_id', attempt.state_volume_id,
           'lease_id', attempt.lease_id,
           'state_access_outcome', attempt.state_access_outcome,
           'disposition', delivery.disposition
       )::text
  FROM mailbox_delivery_attempts AS attempt
  JOIN failed_runs ON failed_runs.id = attempt.run_id
  JOIN mailbox_deliveries AS delivery ON delivery.event_id = attempt.event_id
 ORDER BY attempt.created_at, attempt.id
 LIMIT 256;
WITH failed_runs AS (
    SELECT id
      FROM runs
     WHERE outcome = 'failed'
     ORDER BY updated_at DESC, id
     LIMIT 100
), numbered_events AS (
    SELECT event.run_id, event.sequence, event.occurred_at,
           event.payload,
           row_number() OVER (
               PARTITION BY event.run_id ORDER BY event.sequence
           ) AS event_number
      FROM run_events AS event
      JOIN failed_runs ON failed_runs.id = event.run_id
     WHERE event.event_type = 'vm.log'
       AND jsonb_typeof(event.payload->'bytes') = 'array'
       AND jsonb_array_length(event.payload->'bytes') <= 65536
)
SELECT json_build_object(
           'surface', 'run_vm_log',
           'run_id', run_id,
           'sequence', sequence,
           'occurred_at', occurred_at,
           'stream', payload->>'stream',
           'bytes', payload->'bytes'
       )::text
  FROM numbered_events
 WHERE event_number <= 32
 ORDER BY run_id, payload->>'stream', sequence
 LIMIT 256;
SELECT json_build_object('surface', 'gateway_invocations', 'count', count(*))::text
  FROM gateway_invocations;
SELECT json_build_object(
           'surface', 'gateway_invocation', 'id', id, 'gateway_id', gateway_id,
           'gateway_revision_id', gateway_revision_id, 'gateway_route_id', gateway_route_id,
           'project_id', project_id, 'request_id', request_id, 'outcome', outcome,
           'accepted_at', accepted_at, 'completed_at', completed_at
       )::text
  FROM gateway_invocations
 ORDER BY accepted_at DESC, id
 LIMIT 100;
SELECT json_build_object(
           'surface', 'brokered_secret_audit_events',
           'count', count(*))::text
  FROM brokered_secret_audit_events;
SELECT json_build_object(
           'surface', 'brokered_secret_audit_event', 'id', id,
           'lease_snapshot_id', lease_snapshot_id, 'rule_id', rule_id,
           'runtime_session_id', runtime_session_id, 'run_id', run_id,
           'request_id', request_id, 'event_kind', event_kind,
           'decision', decision, 'outcome', outcome,
           'occurred_at', occurred_at, 'created_at', created_at
       )::text
  FROM brokered_secret_audit_events
 ORDER BY occurred_at DESC, id
 LIMIT 100;
SQL
    pipeline_status=("${PIPESTATUS[@]}")
    set -e
    if [[ "${pipeline_status[0]}" -ne 0 || "${pipeline_status[1]}" -ne 0 ]]; then
        if [[ "${retained}" == true ]]; then
            rm -f -- "${destination}"
        fi
        printf 'postgres lifecycle snapshot unavailable\n' >&2
        return 0
    fi

    if [[ "${retained}" == true ]]; then
        printf '  retained-postgres-snapshot=%s\n' "${destination}" >&2
    fi
}

failure_diagnostics() {
    if [[ -n "${diagnostics_dir}" ]]; then
        postgres_lifecycle_snapshot
        local report="${diagnostics_dir}/libkrun-failure-${PPID}.log"
        diagnostics_body | redact_diagnostics >"${report}" || true
        chmod 600 -- "${report}"
        cat -- "${report}" >&2
        printf '  retained-report=%s\n' "${report}" >&2
    else
        postgres_lifecycle_snapshot
        diagnostics_body | redact_diagnostics >&2 || true
    fi
}

cleanup() {
    local status=$?
    trap - EXIT INT TERM
    set +e
    if [[ "${status}" -ne 0 ]]; then
        failure_diagnostics
    fi
    cleanup_cgroup
    if [[ -n "${container_name}" ]]; then
        podman rm --force "${container_name}" >/dev/null 2>&1
    fi
    if [[ -n "${nats_container_name}" ]]; then
        podman rm --force "${nats_container_name}" >/dev/null 2>&1
    fi
    if [[ -n "${zot_container_name}" ]]; then
        podman rm --force "${zot_container_name}" >/dev/null 2>&1 || true
    fi
    if [[ -n "${postgres_container_name}" ]]; then
        podman rm --force "${postgres_container_name}" >/dev/null 2>&1
    fi
    if [[ "${builder_image_loaded}" == "true" ]]; then
        podman rmi "${builder_vm_image}" >/dev/null 2>&1 || true
    fi
    if [[ "${verifier_image_loaded}" == "true" ]]; then
        podman rmi "${verifier_vm_image}" >/dev/null 2>&1 || true
    fi
    if [[ -n "${fixture_root}" && -f "${fixture_root}/.hephaestus-integration-fixture" ]]; then
        chmod -R u+w "${fixture_root}"
        rm -rf -- "${fixture_root}"
    fi
    exit "${status}"
}
interrupt() {
    local signal="$1"
    trap - INT TERM
    if [[ "${signal}" == TERM ]]; then
        exit 143
    fi
    exit 130
}
trap cleanup EXIT
trap 'interrupt INT' INT
trap 'interrupt TERM' TERM

[[ "$(id -u)" -ne 0 ]] || die "the integration test must run as a non-root service account"
[[ "$(uname -m)" == "x86_64" ]] ||
    die "the pinned fixture currently supports x86_64 only"

for command in awk blkid cargo cat find grep head id install ldconfig mkfs.ext4 mktemp musl-gcc openssl podman python3 rustup sha256sum sort tar timeout truncate unshare; do
    require_command "${command}"
done
if [[ "$(id -u)" -ne 10001 || "$(id -g)" -ne 10001 ]]; then
    unshare --map-user 10001 --map-group 10001 true ||
        die "cannot map the integration process to guest UID/GID 10001"
fi
[[ -r /dev/kvm && -w /dev/kvm ]] || die "/dev/kvm is not readable and writable"
[[ -x /usr/bin/passt ]] || die "/usr/bin/passt is unavailable"
loader_cache="$(ldconfig -p)"
grep -q 'libkrun\.so\.1' <<<"${loader_cache}" ||
    die "libkrun.so.1 is unavailable to the loader"
grep -q 'libkrunfw\.so\.5' <<<"${loader_cache}" ||
    die "libkrunfw.so.5 is unavailable to the loader"
if [[ -n "${rust_builder_image}" ]]; then
    require_digest_reference 'Rust builder image' "${rust_builder_image}"
fi
if [[ "${HEPHAESTUS_APP_COOKING_BUILD_PROOF:-0}" == "1" ]]; then
    require_command skopeo
    require_command oras
    export HEPHAESTUS_SKOPEO="${HEPHAESTUS_SKOPEO:-$(command -v skopeo)}"
    export HEPHAESTUS_ORAS="${HEPHAESTUS_ORAS:-$(command -v oras)}"
    [[ -x "${HEPHAESTUS_SKOPEO}" && -x "${HEPHAESTUS_ORAS}" ]] ||
        die 'repository-image proof requires executable HEPHAESTUS_SKOPEO and HEPHAESTUS_ORAS'
fi

cgroup_parent="$(discover_cgroup_parent)"
readonly cgroup_parent

rustup target add "${GUEST_TARGET}"
cargo build \
    --manifest-path "${repo_root}/Cargo.toml" \
    --release \
    --package vm-libkrun \
    --bin heph-init \
    --bin heph-integration-check \
    --features integration-guest \
    --target "${GUEST_TARGET}"

# libkrun appends a UUID and supervisor.sock beneath this directory. Keep the
# generated prefix short enough for Linux's 108-byte Unix socket limit.
fixture_root="$(mktemp -d "${HEPHAESTUS_LIBKRUN_TMP_ROOT:-/tmp}/h.XXXXXX")"
touch "${fixture_root}/.hephaestus-integration-fixture"
chmod 0700 "${fixture_root}"
mkdir -p \
    "${fixture_root}/rootfs" \
    "${fixture_root}/image-root" \
    "${fixture_root}/runtime" \
    "${fixture_root}/disks" \
    "${fixture_root}/mounts/repository" \
    "${fixture_root}/mounts/workspace"
chmod 0700 "${fixture_root}/runtime"

materialize_image "${ubuntu_image}" "${fixture_root}/rootfs" fixture
prepare_guest_root "${fixture_root}/rootfs"
if [[ "${HEPHAESTUS_APP_COOKING_BUILD_PROOF:-0}" == "1" ]]; then
    load_repository_image_workflow
    builder_operation_root="${fixture_root}/image-root/oci-builder"
    verifier_operation_root="${fixture_root}/image-root/oci-verifier"
    materialize_layout_image \
        "${builder_vm_image}" "${builder_layout}" "${builder_operation_root}" oci-builder
    prepare_guest_root "${builder_operation_root}"
    materialize_layout_image \
        "${verifier_vm_image}" "${verifier_layout}" "${verifier_operation_root}" oci-verifier
    prepare_guest_root "${verifier_operation_root}"
    printf 'OCI worker roots prepared from reviewed layouts:\n  builder=%s\n  verifier=%s\n' \
        "${builder_vm_image}" "${verifier_vm_image}"
fi
if [[ -n "${rust_builder_image}" ]]; then
    rust_builder_root="${fixture_root}/image-root/rust-builder"
    materialize_image "${rust_builder_image}" "${rust_builder_root}" rust-builder
    prepare_guest_root "${rust_builder_root}"
    printf 'Rust builder root prepared at %s from %s\n' \
        "${rust_builder_root}" "${rust_builder_image}"
fi
printf 'repository\n' >"${fixture_root}/mounts/repository/integration-marker"
chmod 0777 "${fixture_root}/mounts/workspace"
truncate -s 128M "${fixture_root}/disks/sqlite.raw"
mkfs.ext4 -q -F "${fixture_root}/disks/sqlite.raw"
filesystem_uuid="$(blkid -s UUID -o value "${fixture_root}/disks/sqlite.raw")"
readonly filesystem_uuid

cgroup_root="${cgroup_parent}/hephaestus-integration-$$"
mkdir "${cgroup_root}"
printf '+cpu +io +memory +pids\n' >"${cgroup_root}/cgroup.subtree_control"

network_before="$(network_snapshot)"
readonly network_before
printf 'Host kernel: %s\n' "$(uname -r)"
printf 'passt: %s\n' "$(/usr/bin/passt --version | head -n 1)"
if command -v rpm >/dev/null 2>&1; then
    rpm -q libkrun libkrunfw
fi
if [[ "${HEPHAESTUS_APP_LIBKRUN_E2E:-0}" == "1" ]]; then
    printf 'Running daemon golden E2E with pinned image %s\n' "${ubuntu_image}"
    start_golden_services
    cargo build \
        --manifest-path "${repo_root}/Cargo.toml" \
        --package vm-libkrun \
        --bin hephaestus-vm-libkrun-worker
    run_as_guest_owner env \
        HEPHAESTUS_APP_LIBKRUN_E2E=1 \
        HEPHAESTUS_POSTGRES_TEST_URL="${postgres_url}" \
        HEPHAESTUS_NATS_TEST_URL="${nats_url}" \
        HEPHAESTUS_LIBKRUN_RUNTIME_ROOT="${fixture_root}/runtime" \
        HEPHAESTUS_LIBKRUN_IMAGE_ROOT="${fixture_root}" \
        HEPHAESTUS_LIBKRUN_RUST_BUILDER_ROOT="${rust_builder_root}" \
        HEPHAESTUS_LIBKRUN_ROOTFS="${fixture_root}/rootfs" \
        HEPHAESTUS_LIBKRUN_DISK_ROOT="${fixture_root}/disks" \
        HEPHAESTUS_LIBKRUN_MOUNT_ROOT="${fixture_root}/mounts" \
        HEPHAESTUS_LIBKRUN_CGROUP_ROOT="${cgroup_root}" \
        HEPHAESTUS_LIBKRUN_WORKER="${repo_root}/target/debug/hephaestus-vm-libkrun-worker" \
        HEPHAESTUS_TEST_OCI_BUILDER_VM_IMAGE="${builder_vm_image}" \
        HEPHAESTUS_TEST_OCI_VERIFIER_VM_IMAGE="${verifier_vm_image}" \
        HEPHAESTUS_TEST_OCI_BASE_LAYOUT_MANIFEST="${base_layout_manifest}" \
        HEPHAESTUS_TEST_OCI_BUILDER_LAYOUT="${builder_layout}" \
        HEPHAESTUS_TEST_OCI_VERIFIER_LAYOUT="${verifier_layout}" \
        HEPHAESTUS_TEST_OCI_BUILDER_ROOT="${builder_operation_root}" \
        HEPHAESTUS_TEST_OCI_VERIFIER_ROOT="${verifier_operation_root}" \
        HEPHAESTUS_TEST_OCI_ROOTFS_ROOT="${fixture_root}" \
        cargo test \
        --manifest-path "${repo_root}/Cargo.toml" \
        --package hephaestus-app \
        --test golden \
        -- --nocapture
    # Reuse the same disposable authority database and JetStream fixture for
    # the gateway publication persistence, RLS, and recovery proof. Keeping
    # it here makes the joined wrapper one complete operator command.
    run_as_guest_owner env \
        HEPHAESTUS_POSTGRES_TEST_URL="${postgres_url}" \
        HEPHAESTUS_NATS_TEST_URL="${nats_url}" \
        cargo test \
        --manifest-path "${repo_root}/Cargo.toml" \
        --package gateway-postgres \
        --test postgres \
        -- --nocapture
elif [[ "${HEPHAESTUS_PHASE1B_INTEGRATION:-0}" == "1" ]]; then
    printf 'Running Phase 1B persistence test with pinned image %s\n' "${ubuntu_image}"
    [[ -n "${HEPHAESTUS_POSTGRES_TEST_URL:-}" ]] ||
        die "HEPHAESTUS_POSTGRES_TEST_URL is required for the Phase 1B scenario"
    cargo build \
        --manifest-path "${repo_root}/Cargo.toml" \
        --package vm-libkrun \
        --bin hephaestus-vm-libkrun-worker
    run_as_guest_owner env \
        HEPHAESTUS_LIBKRUN_RUNTIME_ROOT="${fixture_root}/runtime" \
        HEPHAESTUS_LIBKRUN_IMAGE_ROOT="${fixture_root}" \
        HEPHAESTUS_LIBKRUN_RUST_BUILDER_ROOT="${rust_builder_root}" \
        HEPHAESTUS_LIBKRUN_ROOTFS="${fixture_root}/rootfs" \
        HEPHAESTUS_LIBKRUN_DISK_ROOT="${fixture_root}/disks" \
        HEPHAESTUS_LIBKRUN_MOUNT_ROOT="${fixture_root}/mounts" \
        HEPHAESTUS_LIBKRUN_CGROUP_ROOT="${cgroup_root}" \
        HEPHAESTUS_LIBKRUN_WORKER="${repo_root}/target/debug/hephaestus-vm-libkrun-worker" \
        cargo test \
        --manifest-path "${repo_root}/Cargo.toml" \
        --package run-postgres \
        --test phase1b_libkrun \
        -- --nocapture
else
    printf 'Running libkrun smoke test with pinned image %s\n' "${ubuntu_image}"
    run_as_guest_owner env \
        HEPHAESTUS_LIBKRUN_INTEGRATION=1 \
        HEPHAESTUS_LIBKRUN_RUNTIME_ROOT="${fixture_root}/runtime" \
        HEPHAESTUS_LIBKRUN_IMAGE_ROOT="${fixture_root}" \
        HEPHAESTUS_LIBKRUN_RUST_BUILDER_ROOT="${rust_builder_root}" \
        HEPHAESTUS_LIBKRUN_ROOTFS="${fixture_root}/rootfs" \
        HEPHAESTUS_LIBKRUN_DISK_ROOT="${fixture_root}/disks" \
        HEPHAESTUS_LIBKRUN_SQLITE_DISK="${fixture_root}/disks/sqlite.raw" \
        HEPHAESTUS_LIBKRUN_SQLITE_UUID="${filesystem_uuid}" \
        HEPHAESTUS_LIBKRUN_MOUNT_ROOT="${fixture_root}/mounts" \
        HEPHAESTUS_LIBKRUN_REPOSITORY="${fixture_root}/mounts/repository" \
        HEPHAESTUS_LIBKRUN_WORKSPACE="${fixture_root}/mounts/workspace" \
        HEPHAESTUS_LIBKRUN_CGROUP_ROOT="${cgroup_root}" \
        cargo test \
        --manifest-path "${repo_root}/Cargo.toml" \
        --package vm-libkrun \
        --test libkrun_integration \
        -- --nocapture
fi

if find "${fixture_root}/runtime" -mindepth 1 -print -quit | grep -q .; then
    die "runtime files leaked after the integration test"
fi
if find "${cgroup_root}" -mindepth 1 -maxdepth 1 -type d -print -quit | grep -q .; then
    die "per-VM cgroups leaked after the integration test"
fi
grep -q '^populated 0$' "${cgroup_root}/cgroup.events" ||
    die "the integration cgroup remains populated"
[[ "$(network_snapshot)" == "${network_before}" ]] ||
    die "host network interfaces or routes changed during the integration test"

if [[ "${HEPHAESTUS_APP_LIBKRUN_E2E:-0}" == "1" ]]; then
    printf 'daemon golden E2E passed; runtime and cgroup cleanup verified\n'
elif [[ "${HEPHAESTUS_PHASE1B_INTEGRATION:-0}" == "1" ]]; then
    printf 'Phase 1B persistence test passed; runtime and cgroup cleanup verified\n'
else
    printf 'libkrun integration smoke test passed; runtime and cgroup cleanup verified\n'
fi
