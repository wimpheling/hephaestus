#!/usr/bin/env bash
#
# Run the browser golden path against PostgreSQL, NATS, the Rust daemon,
# a local OIDC fixture, and Phoenix LiveView.

set -Eeuo pipefail

readonly POSTGRES_IMAGE="${HEPHAESTUS_POSTGRES_TEST_IMAGE:-docker.io/library/postgres:17-alpine}"
readonly NATS_IMAGE="${HEPHAESTUS_NATS_TEST_IMAGE:-docker.io/library/nats:2.11-alpine}"

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly script_dir
repo_root="$(cd -- "${script_dir}/.." && pwd -P)"
readonly repo_root
fixture_root="$(mktemp -d)"
readonly fixture_root
postgres_container="hephaestus-ui-postgres-$$"
nats_container="hephaestus-ui-nats-$$"
web_container="hephaestus-ui-web-$$"
oidc_pid=""
daemon_pid=""

cleanup() {
    local status="$?"
    if [[ "${status}" -ne 0 ]]; then
        if [[ -f "${fixture_root}/daemon.log" ]]; then
            tail -200 "${fixture_root}/daemon.log" >&2
        fi
        podman logs "${web_container}" >&2 2>/dev/null || true
    fi
    if [[ -n "${daemon_pid}" ]]; then
        kill "${daemon_pid}" 2>/dev/null || true
        wait "${daemon_pid}" 2>/dev/null || true
    fi
    if [[ -n "${oidc_pid}" ]]; then
        kill "${oidc_pid}" 2>/dev/null || true
        wait "${oidc_pid}" 2>/dev/null || true
    fi
    podman stop "${web_container}" "${nats_container}" "${postgres_container}" \
        >/dev/null 2>&1 || true
    rm -rf -- "${fixture_root}"
    return "${status}"
}
trap cleanup EXIT

wait_for_url() {
    local url="$1"
    local log="$2"
    for _attempt in {1..300}; do
        if curl --fail --silent "${url}" >/dev/null 2>&1; then
            return
        fi
        sleep 0.1
    done
    if [[ -f "${log}" ]]; then
        tail -200 "${log}" >&2
    fi
    printf 'timed out waiting for %s\n' "${url}" >&2
    exit 1
}

published_port() {
    local container="$1"
    local container_port="$2"
    podman port "${container}" "${container_port}/tcp" | awk -F: '{print $NF}'
}

mkdir -p \
    "${fixture_root}/repositories" \
    "${fixture_root}/volumes" \
    "${fixture_root}/workspaces" \
    "${fixture_root}/artifacts" \
    "${fixture_root}/runtime" \
    "${fixture_root}/root-image"

podman run --detach --rm \
    --name "${postgres_container}" \
    --env POSTGRES_PASSWORD=postgres \
    --env POSTGRES_DB=hephaestus \
    --publish 127.0.0.1::5432 \
    "${POSTGRES_IMAGE}" >/dev/null
podman run --detach --rm \
    --name "${nats_container}" \
    --publish 127.0.0.1::4222 \
    "${NATS_IMAGE}" \
    --jetstream --store_dir /tmp/nats >/dev/null

for _attempt in {1..300}; do
    if podman exec "${postgres_container}" \
        pg_isready --quiet --username postgres --dbname hephaestus; then
        break
    fi
    sleep 0.1
done

postgres_port="$(published_port "${postgres_container}" 5432)"
nats_port="$(published_port "${nats_container}" 4222)"
readonly postgres_port nats_port
database_url="postgres://postgres:postgres@127.0.0.1:${postgres_port}/hephaestus?sslmode=disable"
readonly database_url

cd "${repo_root}/e2e/playwright"
npm ci
if [[ "${HEPHAESTUS_PLAYWRIGHT_SKIP_BROWSER_INSTALL:-0}" != "1" ]]; then
    npx playwright install chromium
fi
node oidc-provider.mjs >"${fixture_root}/oidc.log" 2>&1 &
oidc_pid="$!"
wait_for_url "http://127.0.0.1:5556/.well-known/openid-configuration" \
    "${fixture_root}/oidc.log"

cd "${repo_root}"
cargo build -p hephaestus-app --bins
HEPHAESTUS_DATABASE_URL="${database_url}" \
HEPHAESTUS_REPOSITORY_ROOT="${fixture_root}/repositories" \
HEPHAESTUS_BROWSER_OIDC_ISSUER="http://127.0.0.1:5556" \
    "${repo_root}/target/debug/hephaestus-e2e-seed" \
    >"${fixture_root}/seed.json"

export HEPHAESTUS_DATABASE_URL="${database_url}"
export HEPHAESTUS_NATS_URL="nats://127.0.0.1:${nats_port}"
export HEPHAESTUS_HTTP_LISTEN="127.0.0.1:8080"
export HEPHAESTUS_REPOSITORY_ROOT="${fixture_root}/repositories"
export HEPHAESTUS_GIT_HTTP_BACKEND="$(git --exec-path)/git-http-backend"
export HEPHAESTUS_OIDC_ISSUER="http://127.0.0.1:5556"
export HEPHAESTUS_OIDC_AUDIENCE="hephaestus-git"
export HEPHAESTUS_OIDC_ALGORITHM="HS256"
export HEPHAESTUS_OIDC_HS256_SECRET="e2e-signing-secret-with-sufficient-entropy"
export HEPHAESTUS_VOLUME_ROOT="${fixture_root}/volumes"
export HEPHAESTUS_WORKSPACE_ROOT="${fixture_root}/workspaces"
export HEPHAESTUS_ARTIFACT_ROOT="${fixture_root}/artifacts"
export HEPHAESTUS_RUNTIME_ROOT="${fixture_root}/runtime"
export HEPHAESTUS_ROOT_IMAGE_PATH="${fixture_root}/root-image"
export HEPHAESTUS_ROOT_IMAGE_REFERENCE="fixture-root@sha256:e2e"
export HEPHAESTUS_VM_BACKEND="fixture"
export HEPHAESTUS_HOST_ID="browser-e2e"
export HEPHAESTUS_MKFS_EXT4="$(command -v mkfs.ext4)"
export RUST_LOG="hephaestus_app=debug,git_http=debug,forge_service=debug,run_orchestrator=debug,review_service=debug"

"${repo_root}/target/debug/hephaestusd" >"${fixture_root}/daemon.log" 2>&1 &
daemon_pid="$!"
wait_for_url "http://127.0.0.1:8080/healthz" "${fixture_root}/daemon.log"

web_database_url="ecto://hephaestus_web_e2e:hephaestus-web-e2e@127.0.0.1:${postgres_port}/hephaestus"
podman run --detach --rm \
    --name "${web_container}" \
    --network host \
    --volume "${repo_root}:/workspace:Z" \
    --volume "${fixture_root}:${fixture_root}:Z" \
    --workdir /workspace/web \
    --env MIX_ENV=dev \
    --env PHX_SERVER=true \
    --env PORT=4000 \
    --env DATABASE_URL="${web_database_url}" \
    --env HEPHAESTUS_ARTIFACT_ROOT="${fixture_root}/artifacts" \
    --env HEPHAESTUS_BROWSER_OIDC_ISSUER="http://127.0.0.1:5556" \
    --env HEPHAESTUS_BROWSER_OIDC_CLIENT_ID="hephaestus-web" \
    --env HEPHAESTUS_BROWSER_OIDC_CLIENT_SECRET="development-secret" \
    --env HEPHAESTUS_BROWSER_OIDC_REDIRECT_URI="http://127.0.0.1:4000/auth/oidc/callback" \
    docker.io/hexpm/elixir:1.18.4-erlang-27.3.4-debian-bookworm-20250428-slim \
    sh -lc 'mix local.hex --force >/dev/null && mix phx.server' \
    >"${fixture_root}/web-container-id"
wait_for_url "http://127.0.0.1:4000/" "${fixture_root}/web.log"

cd "${repo_root}/e2e/playwright"
HEPHAESTUS_E2E_DATABASE_URL="${database_url}" \
HEPHAESTUS_REPOSITORY_ROOT="${fixture_root}/repositories" \
HEPHAESTUS_GIT_URL="http://127.0.0.1:8080" \
HEPHAESTUS_WEB_URL="http://127.0.0.1:4000" \
HEPHAESTUS_OIDC_URL="http://127.0.0.1:5556" \
    npm test
