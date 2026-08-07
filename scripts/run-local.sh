#!/usr/bin/env bash
#
# Start a persistent local Hephaestus stack for manual browser and Git smoke
# testing. No automated tests are run.

set -Eeuo pipefail

readonly POSTGRES_IMAGE="${HEPHAESTUS_POSTGRES_IMAGE:-docker.io/library/postgres:17-alpine}"
readonly NATS_IMAGE="${HEPHAESTUS_NATS_IMAGE:-docker.io/library/nats:2.11-alpine}"
readonly ELIXIR_IMAGE="${HEPHAESTUS_ELIXIR_IMAGE:-docker.io/hexpm/elixir:1.18.4-erlang-27.3.4-debian-bookworm-20250428-slim}"
readonly DEFAULT_LOCAL_OCI_IMAGE="${HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE:-docker.io/library/ubuntu@sha256:52df9b1ee71626e0088f7d400d5c6b5f7bb916f8f0c82b474289a4ece6cf3faf}"
readonly local_oci_images="${HEPHAESTUS_LOCAL_OCI_IMAGES:-${DEFAULT_LOCAL_OCI_IMAGE}}"
readonly GUEST_TARGET="x86_64-unknown-linux-musl"
readonly REQUIRED_CONTROLLERS=(cpu io memory pids)

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly script_dir
repo_root="$(cd -- "${script_dir}/.." && pwd -P)"
readonly repo_root
local_root="${HEPHAESTUS_LOCAL_ROOT:-${repo_root}/.local/hephaestus}"
readonly local_root
readonly image_cache="${local_root}/oci-images"
readonly image_manifest="${image_cache}/manifest.json"
readonly runtime_root="${HEPHAESTUS_LOCAL_RUNTIME_ROOT:-/tmp/hephaestus-runtime-$(id -u)}"
readonly secret_runtime_root="${HEPHAESTUS_LOCAL_SECRET_RUNTIME_ROOT:-/dev/shm/hephaestus-secret-runtime-$(id -u)}"
readonly secret_key_directory="${local_root}/secret-keys"
readonly secret_key_reference="local-v1"
readonly internal_command_token="development-internal-command-token"

readonly local_namespace="${HEPHAESTUS_LOCAL_NAMESPACE:-hephaestus-local}"
if [[ ! "${local_namespace}" =~ ^[a-z0-9-]+$ ]]; then
    printf 'HEPHAESTUS_LOCAL_NAMESPACE must contain lowercase letters, digits, and hyphens\n' >&2
    exit 1
fi

declare -a oci_image_references=()
declare -A oci_image_digests=()
IFS=',' read -r -a configured_oci_images <<<"${local_oci_images}"
for reference in "${configured_oci_images[@]}"; do
    reference="${reference//[[:space:]]/}"
    if [[ ! "${reference}" =~ ^[A-Za-z0-9._/:+-]+@sha256:[0-9a-f]{64}$ ]]; then
        printf 'HEPHAESTUS_LOCAL_OCI_IMAGES entries must be immutable OCI SHA-256 references\n' >&2
        exit 1
    fi
    digest="${reference##*@sha256:}"
    if [[ -n "${oci_image_digests[${digest}]:-}" && "${oci_image_digests[${digest}]}" != "${reference}" ]]; then
        printf 'HEPHAESTUS_LOCAL_OCI_IMAGES cannot map one digest to multiple references\n' >&2
        exit 1
    fi
    if [[ -z "${oci_image_digests[${digest}]:-}" ]]; then
        oci_image_references+=("${reference}")
        oci_image_digests[${digest}]="${reference}"
    fi
done
if (( ${#oci_image_references[@]} == 0 )); then
    printf 'HEPHAESTUS_LOCAL_OCI_IMAGES must contain at least one image\n' >&2
    exit 1
fi

image_digest() {
    local reference="$1"
    printf '%s\n' "${reference##*@sha256:}"
}

image_cache_path() {
    local reference="$1"
    printf '%s/sha256-%s\n' "${image_cache}" "$(image_digest "${reference}")"
}

image_container_name() {
    local reference="$1"
    local digest
    digest="$(image_digest "${reference}")"
    printf '%s-image-%s\n' "${local_namespace}" "${digest:0:12}"
}

readonly postgres_container="${local_namespace}-postgres"
readonly nats_container="${local_namespace}-nats"
readonly web_container="${local_namespace}-web"
readonly postgres_volume="${local_namespace}-postgres-data"
readonly nats_volume="${local_namespace}-nats-data"
readonly local_postgres_port="${HEPHAESTUS_LOCAL_POSTGRES_PORT:-55432}"
if [[ ! "${local_postgres_port}" =~ ^[0-9]+$ ]] || (( local_postgres_port < 1024 || local_postgres_port > 65535 )); then
    printf 'HEPHAESTUS_LOCAL_POSTGRES_PORT must be between 1024 and 65535\n' >&2
    exit 1
fi
readonly local_zot_port="${HEPHAESTUS_LOCAL_ZOT_PORT:-55000}"
if [[ ! "${local_zot_port}" =~ ^[0-9]+$ ]] || (( local_zot_port < 1024 || local_zot_port > 65535 || local_zot_port == local_postgres_port )); then
    printf 'HEPHAESTUS_LOCAL_ZOT_PORT must be between 1024 and 65535 and differ from PostgreSQL\n' >&2
    exit 1
fi
readonly registry_token_private_key="${local_root}/zot/secrets/registry-token-signing-key.pem"
readonly registry_notification_callback_file="${local_root}/zot/secrets/notification-callback-token"
if [[ ! -f "${registry_token_private_key}" ]]; then
    printf 'local registry signing key is missing; run cargo dev state init --zot\n' >&2
    exit 1
fi
if [[ ! -f "${registry_notification_callback_file}" ]]; then
    printf 'local registry callback credential is missing; run cargo dev state init --zot\n' >&2
    exit 1
fi
readonly supervisor_pid_file="${local_root}/run-local.pid"
readonly oidc_pid_file="${local_root}/oidc.pid"
readonly daemon_pid_file="${local_root}/daemon.pid"
readonly web_log_pid_file="${local_root}/web-log.pid"
readonly cgroup_path_file="${local_root}/cgroup.path"

oidc_pid=""
daemon_pid=""
web_log_pid=""
cgroup_root=""
daemon_restart_requested="false"

stop_pid_file() {
    local pid_file="$1"
    if [[ -f "${pid_file}" ]]; then
        local process_id
        process_id="$(<"${pid_file}")"
        if [[ "${process_id}" =~ ^[0-9]+$ ]]; then
            kill "${process_id}" 2>/dev/null || true
        fi
    fi
}

contains_word() {
    local words="$1"
    local expected="$2"
    [[ " ${words} " == *" ${expected} "* ]]
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
            {
                printf 'configured cgroup parent is not usable: %s\n' "${candidate}" >&2
                return 1
            }
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

    printf 'no writable cgroup parent delegates cpu, io, memory, and pids\n' >&2
    printf 'set HEPHAESTUS_LIBKRUN_CGROUP_PARENT to a suitable cgroup-v2 directory\n' >&2
    return 1
}

cleanup_cgroup_path() {
    local path="$1"
    [[ -n "${path}" && -d "${path}" ]] || return
    if [[ -w "${path}/cgroup.kill" ]]; then
        printf '1\n' >"${path}/cgroup.kill" 2>/dev/null || true
    fi
    for _attempt in {1..100}; do
        if ! grep -q '^populated 1$' "${path}/cgroup.events" 2>/dev/null; then
            break
        fi
        sleep 0.01
    done
    find "${path}" -mindepth 1 -depth -type d -exec rmdir -- {} + 2>/dev/null || true
    rmdir -- "${path}" 2>/dev/null || true
}

cleanup_recorded_cgroup() {
    if [[ -f "${cgroup_path_file}" ]]; then
        local recorded_path
        recorded_path="$(<"${cgroup_path_file}")"
        if [[ "${recorded_path}" == /sys/fs/cgroup/*/hephaestus-local-[0-9]* ]]; then
            cleanup_cgroup_path "${recorded_path}"
        fi
    fi
    rm -f -- "${cgroup_path_file}"
}

cleanup_secret_runtime() {
    if [[ -d "${secret_runtime_root}" ]]; then
        find "${secret_runtime_root}" -mindepth 1 -delete
        rmdir -- "${secret_runtime_root}" 2>/dev/null || true
    fi
}

if [[ "${1:-}" == "stop" ]]; then
    stop_pid_file "${supervisor_pid_file}"
    stop_pid_file "${daemon_pid_file}"
    stop_pid_file "${oidc_pid_file}"
    stop_pid_file "${web_log_pid_file}"
    podman rm --force \
        "${web_container}" "${nats_container}" "${postgres_container}" \
        >/dev/null 2>&1 || true
    cleanup_recorded_cgroup
    cleanup_secret_runtime
    rm -f -- \
        "${supervisor_pid_file}" "${daemon_pid_file}" "${oidc_pid_file}" "${web_log_pid_file}"
    printf 'Hephaestus local services stopped; persistent data was retained.\n'
    exit 0
fi

cleanup() {
    local status="$?"
    trap - EXIT INT TERM
    if [[ -n "${daemon_pid}" ]]; then
        kill "${daemon_pid}" 2>/dev/null || true
        wait "${daemon_pid}" 2>/dev/null || true
    fi
    if [[ -n "${oidc_pid}" ]]; then
        kill "${oidc_pid}" 2>/dev/null || true
        wait "${oidc_pid}" 2>/dev/null || true
    fi
    if [[ -n "${web_log_pid}" ]]; then
        kill "${web_log_pid}" 2>/dev/null || true
        wait "${web_log_pid}" 2>/dev/null || true
    fi
    podman rm --force \
        "${web_container}" "${nats_container}" "${postgres_container}" \
        >/dev/null 2>&1 || true
    cleanup_cgroup_path "${cgroup_root}"
    cleanup_secret_runtime
    rm -f -- \
        "${supervisor_pid_file}" "${daemon_pid_file}" "${oidc_pid_file}" "${web_log_pid_file}"
    rm -f -- "${cgroup_path_file}"
    printf '\nHephaestus stopped. Local data remains in %s and Podman volumes.\n' \
        "${local_root}"
    exit "${status}"
}
trap cleanup EXIT INT TERM

wait_for_url() {
    local url="$1"
    local log="$2"
    for _attempt in {1..600}; do
        if curl --fail --silent "${url}" >/dev/null 2>&1; then
            return
        fi
        sleep 0.1
    done
    if [[ -f "${log}" ]]; then
        tail -200 "${log}" >&2
    fi
    printf 'timed out waiting for %s\n' "${url}" >&2
    return 1
}

for command in awk cargo curl find git grep install ldconfig mkfs.ext4 musl-gcc node npm podman rustup tar unshare; do
    if ! command -v "${command}" >/dev/null 2>&1; then
        printf 'required command is missing: %s\n' "${command}" >&2
        exit 1
    fi
done

[[ "$(id -u)" -ne 0 ]] || {
    printf 'run-local.sh must run as a non-root user\n' >&2
    exit 1
}
[[ "$(uname -m)" == "x86_64" ]] || {
    printf 'the pinned local libkrun image currently supports x86_64 only\n' >&2
    exit 1
}
unshare --map-user 10001 --map-group 10001 true || {
    printf 'cannot map the local daemon to guest UID/GID 10001\n' >&2
    exit 1
}
[[ -r /dev/kvm && -w /dev/kvm ]] || {
    printf '/dev/kvm must be readable and writable\n' >&2
    exit 1
}
[[ -x /usr/bin/passt ]] || {
    printf '/usr/bin/passt is required for libkrun networking\n' >&2
    exit 1
}
loader_cache="$(ldconfig -p)"
grep -q 'libkrun\.so\.1' <<<"${loader_cache}" || {
    printf 'libkrun.so.1 is unavailable to the dynamic loader\n' >&2
    exit 1
}
grep -q 'libkrunfw\.so\.5' <<<"${loader_cache}" || {
    printf 'libkrunfw.so.5 is unavailable to the dynamic loader\n' >&2
    exit 1
}

mkdir -p \
    "${local_root}/repositories" \
    "${local_root}/volumes" \
    "${local_root}/workspaces" \
    "${local_root}/artifacts" \
    "${image_cache}" \
    "${secret_key_directory}" \
    "${local_root}/logs" \
    "${runtime_root}" \
    "${secret_runtime_root}"
chmod 0700 "${runtime_root}" "${secret_runtime_root}" "${secret_key_directory}"
[[ ! -L "${image_cache}" ]] || {
    printf 'OCI image cache must not be a symbolic link: %s\n' "${image_cache}" >&2
    exit 1
}
find "${secret_runtime_root}" -mindepth 1 -delete
if [[ ! -e "${secret_key_directory}/${secret_key_reference}" ]]; then
    umask 077
    dd if=/dev/urandom \
        of="${secret_key_directory}/${secret_key_reference}" \
        bs=32 count=1 status=none
    chmod 0400 "${secret_key_directory}/${secret_key_reference}"
fi
printf '%s\n' "$$" >"${supervisor_pid_file}"

# Remove containers left by an interrupted prior local session. Named volumes
# and host-side forge data are deliberately retained.
podman rm --force \
    "${web_container}" "${nats_container}" "${postgres_container}" \
    >/dev/null 2>&1 || true

rustup target add "${GUEST_TARGET}"
cargo build \
    --release \
    --package vm-libkrun \
    --bin heph-init \
    --target "${GUEST_TARGET}"
cargo build \
    --package vm-libkrun \
    --bin hephaestus-vm-libkrun-worker

materialize_oci_image() {
    local reference="$1"
    local destination
    local staging
    local container
    destination="$(image_cache_path "${reference}")"
    if [[ -f "${destination}/.hephaestus-image" ]]; then
        [[ -d "${destination}" && ! -L "${destination}" && ! -L "${destination}/.hephaestus-image" ]] || {
            printf 'OCI image cache entry is unsafe: %s\n' "${destination}" >&2
            return 1
        }
        [[ "$(<"${destination}/.hephaestus-image")" == "${reference}" ]] || {
            printf 'OCI image cache entry has an unexpected immutable reference: %s\n' "${destination}" >&2
            return 1
        }
        return
    fi
    [[ ! -e "${destination}" ]] || {
        printf 'OCI image cache destination already exists without trusted metadata: %s\n' "${destination}" >&2
        return 1
    }
    staging="$(mktemp -d "${image_cache}/.image.XXXXXX")"
    container="$(image_container_name "${reference}")"
    podman pull "${reference}"
    podman rm --force "${container}" >/dev/null 2>&1 || true
    if ! podman create --name "${container}" "${reference}" /bin/true >/dev/null \
        || ! podman export "${container}" | tar -C "${staging}" -xf -; then
        podman rm --force "${container}" >/dev/null 2>&1 || true
        rm -rf -- "${staging}"
        return 1
    fi
    podman rm "${container}" >/dev/null
    if grep -qE '(^|:)10001:' "${staging}/etc/passwd"; then
        printf 'the OCI image already assigns guest UID 10001: %s\n' "${reference}" >&2
        rm -rf -- "${staging}"
        return 1
    fi
    if grep -qE '(^|:)10001:' "${staging}/etc/group"; then
        printf 'the OCI image already assigns guest GID 10001: %s\n' "${reference}" >&2
        rm -rf -- "${staging}"
        return 1
    fi
    printf 'heph-agent:x:10001:10001:Hephaestus agent:/nonexistent:/sbin/nologin\n' \
        >>"${staging}/etc/passwd"
    printf 'heph-agent:x:10001:\n' >>"${staging}/etc/group"
    printf '%s\n' "${reference}" >"${staging}/.hephaestus-image"
    mv -- "${staging}" "${destination}"
}

for reference in "${oci_image_references[@]}"; do
    materialize_oci_image "${reference}"
    install -D -m 0755 \
        "${repo_root}/target/${GUEST_TARGET}/release/heph-init" \
        "$(image_cache_path "${reference}")/usr/libexec/hephaestus/heph-init"
done

manifest_temporary="${image_manifest}.$$"
{
    printf '{"version":1,"roots":{'
    separator=''
    for reference in "${oci_image_references[@]}"; do
        image_path="$(image_cache_path "${reference}")"
        escaped_image_path="${image_path//\\/\\\\}"
        escaped_image_path="${escaped_image_path//\"/\\\"}"
        printf '%s"%s":{"kind":"directory","path":"%s"}' \
            "${separator}" "${reference}" "${escaped_image_path}"
        separator=','
    done
    printf '}}\n'
} >"${manifest_temporary}"
mv -- "${manifest_temporary}" "${image_manifest}"

cgroup_parent="$(discover_cgroup_parent)"
readonly cgroup_parent
cgroup_root="${cgroup_parent}/hephaestus-local-$$"
mkdir "${cgroup_root}"
printf '+cpu +io +memory +pids\n' >"${cgroup_root}/cgroup.subtree_control"
printf '%s\n' "${cgroup_root}" >"${cgroup_path_file}"

podman run --detach --rm \
    --name "${postgres_container}" \
    --env POSTGRES_PASSWORD=postgres \
    --env POSTGRES_DB=hephaestus \
    --publish "127.0.0.1:${local_postgres_port}:5432" \
    --volume "${postgres_volume}:/var/lib/postgresql/data" \
    "${POSTGRES_IMAGE}" >/dev/null
podman run --detach --rm \
    --name "${nats_container}" \
    --publish 127.0.0.1:54222:4222 \
    --volume "${nats_volume}:/data" \
    "${NATS_IMAGE}" \
    --jetstream --store_dir /data >/dev/null

for _attempt in {1..600}; do
    if podman exec "${postgres_container}" \
        pg_isready --quiet --username postgres --dbname hephaestus; then
        break
    fi
    sleep 0.1
done

if [[ ! -d "${repo_root}/e2e/playwright/node_modules/jose" ]]; then
    npm ci --ignore-scripts --prefix "${repo_root}/e2e/playwright"
fi
node "${repo_root}/e2e/playwright/oidc-provider.mjs" \
    >"${local_root}/logs/oidc.log" 2>&1 &
oidc_pid="$!"
printf '%s\n' "${oidc_pid}" >"${oidc_pid_file}"
wait_for_url \
    "http://127.0.0.1:5556/.well-known/openid-configuration" \
    "${local_root}/logs/oidc.log"

cd "${repo_root}"
cargo build -p hephaestus-app --bins

readonly database_url="postgres://postgres:postgres@127.0.0.1:${local_postgres_port}/hephaestus?sslmode=disable"
HEPHAESTUS_DATABASE_URL="${database_url}" \
HEPHAESTUS_REPOSITORY_ROOT="${local_root}/repositories" \
HEPHAESTUS_ARTIFACT_ROOT="${local_root}/artifacts" \
HEPHAESTUS_BROWSER_OIDC_ISSUER="http://127.0.0.1:5556" \
    "${repo_root}/target/debug/hephaestus-e2e-seed" \
    >"${local_root}/seed.json"

export HEPHAESTUS_DATABASE_URL="${database_url}"
export HEPHAESTUS_NATS_URL="nats://127.0.0.1:54222"
export HEPHAESTUS_HTTP_LISTEN="127.0.0.1:8080"
export HEPHAESTUS_REPOSITORY_ROOT="${local_root}/repositories"
export HEPHAESTUS_GIT_HTTP_BACKEND="$(git --exec-path)/git-http-backend"
export HEPHAESTUS_OIDC_ISSUER="http://127.0.0.1:5556"
export HEPHAESTUS_OIDC_AUDIENCE="hephaestus-git"
export HEPHAESTUS_OIDC_ALGORITHM="HS256"
export HEPHAESTUS_OIDC_HS256_SECRET="e2e-signing-secret-with-sufficient-entropy"
export HEPHAESTUS_VOLUME_ROOT="${local_root}/volumes"
export HEPHAESTUS_WORKSPACE_ROOT="${local_root}/workspaces"
export HEPHAESTUS_ARTIFACT_ROOT="${local_root}/artifacts"
export HEPHAESTUS_RUNTIME_ROOT="${runtime_root}"
export HEPHAESTUS_SECRET_RUNTIME_ROOT="${secret_runtime_root}"
export HEPHAESTUS_SECRET_KEY_DIRECTORY="${secret_key_directory}"
export HEPHAESTUS_SECRET_KEY_REFERENCE="${secret_key_reference}"
export HEPHAESTUS_RPC_MEDIATOR_SECRET="${internal_command_token}"
export HEPHAESTUS_REGISTRY_TOKEN_PRIVATE_KEY="${registry_token_private_key}"
export HEPHAESTUS_REGISTRY_TOKEN_ISSUER="http://127.0.0.1:8080/v1/registry/token"
export HEPHAESTUS_REGISTRY_SERVICE="localhost:${local_zot_port}"
export HEPHAESTUS_REGISTRY_PRIVATE_ORIGIN="http://127.0.0.1:${local_zot_port}/"
export HEPHAESTUS_REGISTRY_TOKEN_KEY_ID="local-v1"
export HEPHAESTUS_REGISTRY_TOKEN_LIFETIME_SECONDS="300"
export HEPHAESTUS_REGISTRY_NOTIFICATION_CALLBACK_TOKEN_FILE="${registry_notification_callback_file}"
unset HEPHAESTUS_ROOT_IMAGE_PATH HEPHAESTUS_ROOT_IMAGE_REFERENCE
export HEPHAESTUS_ROOT_IMAGE_MANIFEST="${image_manifest}"
export HEPHAESTUS_VM_BACKEND="libkrun"
export HEPHAESTUS_LIBKRUN_WORKER="${repo_root}/target/debug/hephaestus-vm-libkrun-worker"
export HEPHAESTUS_CGROUP_ROOT="${cgroup_root}"
export HEPHAESTUS_HOST_ID="manual-local"
export HEPHAESTUS_MKFS_EXT4="$(command -v mkfs.ext4)"
export HEPHAESTUS_RUNTIME_POLICY_VERSION="manual-local/v1"
export HEPHAESTUS_RUNTIME_MAX_VCPUS="2"
export HEPHAESTUS_RUNTIME_MAX_MEMORY_MIB="1024"
export HEPHAESTUS_RUNTIME_ALLOW_BROKER_ONLY="true"
export HEPHAESTUS_RUNTIME_ALLOW_EGRESS="false"
export RUST_LOG="${RUST_LOG:-hephaestus_app=info,git_http=info,forge_service=info,run_orchestrator=info,review_service=info}"

start_daemon() {
    unshare --map-user 10001 --map-group 10001 \
        "${repo_root}/target/debug/hephaestusd" \
        >"${local_root}/logs/daemon.log" 2>&1 &
    daemon_pid="$!"
    printf '%s\n' "${daemon_pid}" >"${daemon_pid_file}"
    wait_for_url "http://127.0.0.1:8080/healthz" \
        "${local_root}/logs/daemon.log"
}

request_daemon_restart() {
    daemon_restart_requested="true"
}

trap request_daemon_restart USR1
start_daemon

# The repository is also mounted by browser-test containers, so use a shared
# SELinux label rather than letting either container revoke access.
podman run --detach --rm \
    --name "${web_container}" \
    --network host \
    --volume "${repo_root}:/workspace:z" \
    --workdir /workspace/web \
    --env MIX_ENV=dev \
    --env PHX_SERVER=true \
    --env PORT=4000 \
    --env HEPHAESTUS_RPC_MEDIATOR_SECRET="${internal_command_token}" \
    --env HEPHAESTUS_BROWSER_OIDC_ISSUER="http://127.0.0.1:5556" \
    --env HEPHAESTUS_BROWSER_OIDC_CLIENT_ID="hephaestus-web" \
    --env HEPHAESTUS_BROWSER_OIDC_CLIENT_SECRET="development-secret" \
    --env HEPHAESTUS_BROWSER_OIDC_REDIRECT_URI="http://127.0.0.1:4000/auth/oidc/callback" \
    "${ELIXIR_IMAGE}" \
    sh -lc \
    'apt-get update -qq && apt-get install -y -qq --no-install-recommends ca-certificates inotify-tools >/dev/null && rm -rf /var/lib/apt/lists/* && mix local.hex --force >/dev/null && mix deps.get && mix assets.setup && mix assets.build && mix phx.server' \
    >/dev/null
podman logs --follow "${web_container}" >"${local_root}/logs/web.log" 2>&1 &
web_log_pid="$!"
printf '%s\n' "${web_log_pid}" >"${web_log_pid_file}"
wait_for_url "http://127.0.0.1:4000/" \
    "${local_root}/logs/web.log"

repository_id="$(
    node -e \
        "process.stdout.write(JSON.parse(require('fs').readFileSync('${local_root}/seed.json')).repository_id)"
)"
readonly repository_id

printf '\nHephaestus is ready for manual smoke testing.\n\n'
printf '  Web UI:        http://127.0.0.1:4000\n'
printf '  Git endpoint:  http://127.0.0.1:8080/%s\n' "${repository_id}"
printf '  Login:         reviewer (Continue as Ada Reviewer)\n'
printf '  VM backend:    libkrun/KVM with %s materialized OCI image(s)\n' "${#oci_image_references[@]}"
printf '  OCI images:    %s\n' "${image_manifest}"
printf '  VM runtime:    %s\n' "${runtime_root}"
printf '  Data:          %s\n\n' "${local_root}"
printf 'Create a fresh Git bearer token with:\n\n'
printf '  export HEPHAESTUS_GIT_TOKEN="$(curl --fail --silent http://127.0.0.1:5556/test/git-token)"\n\n'
printf 'Then clone with:\n\n'
printf '  git -c http.extraHeader="Authorization: Bearer ${HEPHAESTUS_GIT_TOKEN}" clone http://127.0.0.1:8080/%s\n\n' \
    "${repository_id}"
printf 'Daemon log: %s\n' "${local_root}/logs/daemon.log"
printf 'Web log:    %s\n' "${local_root}/logs/web.log"
printf 'Press Ctrl-C here to stop services while retaining local data.\n\n'

while true; do
    daemon_status=0
    wait "${daemon_pid}" || daemon_status="$?"
    if [[ "${daemon_restart_requested}" == "true" ]]; then
        daemon_restart_requested="false"
        kill "${daemon_pid}" 2>/dev/null || true
        wait "${daemon_pid}" 2>/dev/null || true
        start_daemon
        continue
    fi
    exit "${daemon_status}"
done
