#!/usr/bin/env bash
#
# Start a persistent local Hephaestus stack for manual browser and Git smoke
# testing. No automated tests are run.

set -Eeuo pipefail

readonly POSTGRES_IMAGE="${HEPHAESTUS_POSTGRES_IMAGE:-docker.io/library/postgres:17-alpine}"
readonly NATS_IMAGE="${HEPHAESTUS_NATS_IMAGE:-docker.io/library/nats:2.11-alpine}"
readonly ELIXIR_IMAGE="${HEPHAESTUS_ELIXIR_IMAGE:-docker.io/hexpm/elixir:1.18.4-erlang-27.3.4-debian-bookworm-20250428-slim}"
readonly FEDORA_IMAGE="${HEPHAESTUS_LIBKRUN_FEDORA_IMAGE:-registry.fedoraproject.org/fedora-minimal@sha256:8f42d200f04990b41081322d1c260ddf23b124b3b92538665ef4cc3064537249}"
readonly ROOT_IMAGE_REFERENCE="fedora-minimal@sha256:8f42d200f04990b41081322d1c260ddf23b124b3b92538665ef4cc3064537249"
readonly GUEST_TARGET="x86_64-unknown-linux-musl"
readonly REQUIRED_CONTROLLERS=(cpu io memory pids)

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly script_dir
repo_root="$(cd -- "${script_dir}/.." && pwd -P)"
readonly repo_root
local_root="${HEPHAESTUS_LOCAL_ROOT:-${repo_root}/.local/hephaestus}"
readonly local_root
readonly root_image_path="${local_root}/root-images/fedora-minimal-8f42d200"
readonly runtime_root="${HEPHAESTUS_LOCAL_RUNTIME_ROOT:-/tmp/hephaestus-runtime-$(id -u)}"
readonly secret_runtime_root="${HEPHAESTUS_LOCAL_SECRET_RUNTIME_ROOT:-/dev/shm/hephaestus-secret-runtime-$(id -u)}"
readonly secret_key_directory="${local_root}/secret-keys"
readonly secret_key_reference="local-v1"
readonly internal_command_token="development-internal-command-token"

readonly postgres_container="hephaestus-local-postgres"
readonly nats_container="hephaestus-local-nats"
readonly web_container="hephaestus-local-web"
readonly rootfs_container="hephaestus-local-rootfs"
readonly postgres_volume="hephaestus-local-postgres-data"
readonly nats_volume="hephaestus-local-nats-data"
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
        "${web_container}" "${nats_container}" "${postgres_container}" "${rootfs_container}" \
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
        "${web_container}" "${nats_container}" "${postgres_container}" "${rootfs_container}" \
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
    "${local_root}/root-images" \
    "${secret_key_directory}" \
    "${local_root}/logs" \
    "${runtime_root}" \
    "${secret_runtime_root}"
chmod 0700 "${runtime_root}" "${secret_runtime_root}" "${secret_key_directory}"
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
    "${web_container}" "${nats_container}" "${postgres_container}" "${rootfs_container}" \
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

if [[ ! -f "${root_image_path}/.hephaestus-image" ]]; then
    root_image_staging="$(mktemp -d "${local_root}/root-images/.fedora-minimal.XXXXXX")"
    podman pull "${FEDORA_IMAGE}"
    podman create --name "${rootfs_container}" "${FEDORA_IMAGE}" /bin/true >/dev/null
    podman export "${rootfs_container}" | tar -C "${root_image_staging}" -xf -
    podman rm "${rootfs_container}" >/dev/null
    if grep -qE '(^|:)10001:' "${root_image_staging}/etc/passwd"; then
        printf 'the pinned image already assigns guest UID 10001\n' >&2
        exit 1
    fi
    if grep -qE '(^|:)10001:' "${root_image_staging}/etc/group"; then
        printf 'the pinned image already assigns guest GID 10001\n' >&2
        exit 1
    fi
    printf 'heph-agent:x:10001:10001:Hephaestus agent:/nonexistent:/sbin/nologin\n' \
        >>"${root_image_staging}/etc/passwd"
    printf 'heph-agent:x:10001:\n' >>"${root_image_staging}/etc/group"
    printf '%s\n' "${FEDORA_IMAGE}" >"${root_image_staging}/.hephaestus-image"
    mv -- "${root_image_staging}" "${root_image_path}"
fi
install -D -m 0755 \
    "${repo_root}/target/${GUEST_TARGET}/release/heph-init" \
    "${root_image_path}/usr/libexec/hephaestus/heph-init"

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
    --publish 127.0.0.1:55432:5432 \
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

readonly database_url="postgres://postgres:postgres@127.0.0.1:55432/hephaestus?sslmode=disable"
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
export HEPHAESTUS_ROOT_IMAGE_PATH="${root_image_path}"
export HEPHAESTUS_ROOT_IMAGE_REFERENCE="${ROOT_IMAGE_REFERENCE}"
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
printf '  VM backend:    libkrun/KVM with pinned Fedora 44\n'
printf '  Root image:    %s\n' "${ROOT_IMAGE_REFERENCE}"
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
