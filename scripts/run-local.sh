#!/usr/bin/env bash
#
# Start a persistent local Hephaestus stack for manual browser and Git smoke
# testing. No automated tests are run.

set -Eeuo pipefail

readonly POSTGRES_IMAGE="${HEPHAESTUS_POSTGRES_IMAGE:-docker.io/library/postgres:17-alpine}"
readonly NATS_IMAGE="${HEPHAESTUS_NATS_IMAGE:-docker.io/library/nats:2.11-alpine}"
readonly ELIXIR_IMAGE="${HEPHAESTUS_ELIXIR_IMAGE:-docker.io/hexpm/elixir:1.18.4-erlang-27.3.4-debian-bookworm-20250428-slim}"

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly script_dir
repo_root="$(cd -- "${script_dir}/.." && pwd -P)"
readonly repo_root
local_root="${HEPHAESTUS_LOCAL_ROOT:-${repo_root}/.local/hephaestus}"
readonly local_root

readonly postgres_container="hephaestus-local-postgres"
readonly nats_container="hephaestus-local-nats"
readonly web_container="hephaestus-local-web"
readonly postgres_volume="hephaestus-local-postgres-data"
readonly nats_volume="hephaestus-local-nats-data"
readonly supervisor_pid_file="${local_root}/run-local.pid"
readonly oidc_pid_file="${local_root}/oidc.pid"
readonly daemon_pid_file="${local_root}/daemon.pid"

oidc_pid=""
daemon_pid=""

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

if [[ "${1:-}" == "stop" ]]; then
    stop_pid_file "${supervisor_pid_file}"
    stop_pid_file "${daemon_pid_file}"
    stop_pid_file "${oidc_pid_file}"
    podman rm --force \
        "${web_container}" "${nats_container}" "${postgres_container}" \
        >/dev/null 2>&1 || true
    rm -f -- "${supervisor_pid_file}" "${daemon_pid_file}" "${oidc_pid_file}"
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
    podman rm --force \
        "${web_container}" "${nats_container}" "${postgres_container}" \
        >/dev/null 2>&1 || true
    rm -f -- "${supervisor_pid_file}" "${daemon_pid_file}" "${oidc_pid_file}"
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

for command in cargo curl git mkfs.ext4 node npm podman; do
    if ! command -v "${command}" >/dev/null 2>&1; then
        printf 'required command is missing: %s\n' "${command}" >&2
        exit 1
    fi
done

mkdir -p \
    "${local_root}/repositories" \
    "${local_root}/volumes" \
    "${local_root}/workspaces" \
    "${local_root}/artifacts" \
    "${local_root}/runtime" \
    "${local_root}/root-image" \
    "${local_root}/logs"
printf '%s\n' "$$" >"${supervisor_pid_file}"

# Remove containers left by an interrupted prior local session. Named volumes
# and host-side forge data are deliberately retained.
podman rm --force \
    "${web_container}" "${nats_container}" "${postgres_container}" \
    >/dev/null 2>&1 || true

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
export HEPHAESTUS_RUNTIME_ROOT="${local_root}/runtime"
export HEPHAESTUS_ROOT_IMAGE_PATH="${local_root}/root-image"
export HEPHAESTUS_ROOT_IMAGE_REFERENCE="fixture-root@sha256:local"
export HEPHAESTUS_VM_BACKEND="fixture"
export HEPHAESTUS_HOST_ID="manual-local"
export HEPHAESTUS_MKFS_EXT4="$(command -v mkfs.ext4)"
export RUST_LOG="${RUST_LOG:-hephaestus_app=info,git_http=info,forge_service=info,run_orchestrator=info,review_service=info}"

"${repo_root}/target/debug/hephaestusd" \
    >"${local_root}/logs/daemon.log" 2>&1 &
daemon_pid="$!"
printf '%s\n' "${daemon_pid}" >"${daemon_pid_file}"
wait_for_url "http://127.0.0.1:8080/healthz" \
    "${local_root}/logs/daemon.log"

readonly web_database_url="ecto://hephaestus_web_e2e:hephaestus-web-e2e@127.0.0.1:55432/hephaestus"
podman run --detach --rm \
    --name "${web_container}" \
    --network host \
    --volume "${repo_root}:/workspace:Z" \
    --volume "${local_root}:${local_root}:Z" \
    --workdir /workspace/web \
    --env MIX_ENV=dev \
    --env PHX_SERVER=true \
    --env PORT=4000 \
    --env DATABASE_URL="${web_database_url}" \
    --env HEPHAESTUS_ARTIFACT_ROOT="${local_root}/artifacts" \
    --env HEPHAESTUS_BROWSER_OIDC_ISSUER="http://127.0.0.1:5556" \
    --env HEPHAESTUS_BROWSER_OIDC_CLIENT_ID="hephaestus-web" \
    --env HEPHAESTUS_BROWSER_OIDC_CLIENT_SECRET="development-secret" \
    --env HEPHAESTUS_BROWSER_OIDC_REDIRECT_URI="http://127.0.0.1:4000/auth/oidc/callback" \
    "${ELIXIR_IMAGE}" \
    sh -lc \
    'mix local.hex --force >/dev/null && mix deps.get && mix assets.setup && mix assets.build && mix phx.server' \
    >/dev/null
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
printf '  Data:          %s\n\n' "${local_root}"
printf 'Create a fresh Git bearer token with:\n\n'
printf '  export HEPHAESTUS_GIT_TOKEN="$(curl --fail --silent http://127.0.0.1:5556/test/git-token)"\n\n'
printf 'Then clone with:\n\n'
printf '  git -c http.extraHeader="Authorization: Bearer ${HEPHAESTUS_GIT_TOKEN}" clone http://127.0.0.1:8080/%s\n\n' \
    "${repository_id}"
printf 'Daemon log: %s\n' "${local_root}/logs/daemon.log"
printf 'Web log:    podman logs -f %s\n' "${web_container}"
printf 'Press Ctrl-C here to stop services while retaining local data.\n\n'

wait "${daemon_pid}"
