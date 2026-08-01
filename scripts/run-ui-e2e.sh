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
secret_runtime_root="$(mktemp -d /dev/shm/hephaestus-ui-secrets.XXXXXX)"
readonly secret_runtime_root
postgres_container="hephaestus-ui-postgres-$$"
nats_container="hephaestus-ui-nats-$$"
web_container="hephaestus-ui-web-$$"
oidc_pid=""
daemon_pid=""
base_port=$((20000 + ($$ % 10000) * 3))
readonly oidc_port="${base_port}"
readonly daemon_port="$((base_port + 1))"
readonly web_port="$((base_port + 2))"
readonly oidc_url="http://127.0.0.1:${oidc_port}"
readonly daemon_url="http://127.0.0.1:${daemon_port}"
readonly web_url="http://127.0.0.1:${web_port}"
readonly secret_sentinel="HEPHAESTUS_BROWSER_SECRET_4d7ccf"

cleanup() {
    local status="$?"
    if [[ "${status}" -ne 0 ]]; then
        if [[ -f "${fixture_root}/daemon.log" ]]; then
            tail -200 "${fixture_root}/daemon.log" >&2
        fi
        podman exec "${postgres_container}" psql --username postgres \
            --dbname hephaestus --tuples-only --command \
            "SELECT id, name, organization_id, project_id, status FROM secrets ORDER BY created_at" \
            >&2 2>/dev/null || true
        podman exec "${postgres_container}" psql --username postgres \
            --dbname hephaestus --tuples-only --command \
            "SELECT id, \"alias\", target_kind, target_id, status FROM secret_imports ORDER BY accepted_at" \
            >&2 2>/dev/null || true
        podman exec "${postgres_container}" psql --username postgres \
            --dbname hephaestus --tuples-only --command \
            "SELECT id, target_kind, target_id, status FROM secret_grants ORDER BY created_at" \
            >&2 2>/dev/null || true
        podman exec "${postgres_container}" psql --username postgres \
            --dbname hephaestus --tuples-only --command \
            "SELECT project_id, user_id, role FROM project_secret_roles ORDER BY project_id, role" \
            >&2 2>/dev/null || true
        podman exec "${postgres_container}" psql --username postgres \
            --dbname hephaestus --tuples-only --command \
            "SELECT id, state, failure FROM runs ORDER BY created_at" \
            >&2 2>/dev/null || true
        podman exec "${postgres_container}" psql --username postgres \
            --dbname hephaestus --tuples-only --command \
            "SELECT run_id, sequence, event_type, payload FROM run_events ORDER BY run_id, sequence" \
            >&2 2>/dev/null || true
        podman exec "${postgres_container}" psql --username postgres \
            --dbname hephaestus --tuples-only --command \
            "SELECT scope_kind, scope_id, cursor, aggregate_type, aggregate_id, event_type, change_kind, safe_state FROM application_events ORDER BY occurred_at, cursor" \
            >&2 2>/dev/null || true
        podman exec "${postgres_container}" psql --username postgres \
            --dbname hephaestus --tuples-only --command \
            "SELECT event.scope_kind, event.scope_id, event.cursor, outbox.published_at IS NOT NULL AS published, outbox.dead_lettered_at IS NOT NULL AS dead_lettered, outbox.last_error FROM product_event_outbox outbox JOIN application_events event ON event.id = outbox.event_id ORDER BY event.occurred_at, event.cursor" \
            >&2 2>/dev/null || true
        podman logs "${web_container}" >&2 2>/dev/null || true
    fi
    if [[ "${HEPHAESTUS_E2E_KEEP_FIXTURES:-0}" == "1" ]]; then
        podman logs "${web_container}" >"${fixture_root}/web.log" 2>&1 || true
    fi
    if [[ -n "${daemon_pid}" ]]; then
        kill "${daemon_pid}" 2>/dev/null || true
        wait "${daemon_pid}" 2>/dev/null || true
    fi
    if [[ -n "${oidc_pid}" ]]; then
        kill "${oidc_pid}" 2>/dev/null || true
        wait "${oidc_pid}" 2>/dev/null || true
    fi
    podman stop "${web_container}" >/dev/null 2>&1 || true
    podman stop "${nats_container}" >/dev/null 2>&1 || true
    podman stop "${postgres_container}" >/dev/null 2>&1 || true
    if [[ "${HEPHAESTUS_E2E_KEEP_FIXTURES:-0}" == "1" ]]; then
        printf 'retained browser E2E fixtures at %s\n' "${fixture_root}" >&2
        printf 'retained secret runtime at %s\n' "${secret_runtime_root}" >&2
    else
        rm -rf -- "${fixture_root}"
        rm -rf -- "${secret_runtime_root}"
    fi
    return "${status}"
}
trap cleanup EXIT

wait_for_url() {
    local url="$1"
    local log="$2"
    for _attempt in {1..1200}; do
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

assert_web_isolation() {
    local actual_hephaestus_environment
    local expected_hephaestus_environment
    local mount_destinations

    actual_hephaestus_environment="$(
        podman exec "${web_container}" env |
            sed -n 's/^\(HEPHAESTUS_[^=]*\)=.*/\1/p' |
            sort
    )"
    expected_hephaestus_environment="$(
        printf '%s\n' \
            HEPHAESTUS_BROWSER_OIDC_CLIENT_ID \
            HEPHAESTUS_BROWSER_OIDC_CLIENT_SECRET \
            HEPHAESTUS_BROWSER_OIDC_ISSUER \
            HEPHAESTUS_BROWSER_OIDC_REDIRECT_URI \
            HEPHAESTUS_RPC_ENDPOINT \
            HEPHAESTUS_RPC_MEDIATOR_SECRET |
            sort
    )"
    if [[ "${actual_hephaestus_environment}" != "${expected_hephaestus_environment}" ]]; then
        printf 'Phoenix received unexpected HEPHAESTUS_* configuration:\n%s\n' \
            "${actual_hephaestus_environment}" >&2
        exit 1
    fi

    if podman exec "${web_container}" sh -c 'command -v git >/dev/null 2>&1'; then
        printf 'Phoenix container unexpectedly contains the Git CLI\n' >&2
        exit 1
    fi

    mount_destinations="$(
        podman inspect --format '{{range .Mounts}}{{println .Destination}}{{end}}' \
            "${web_container}" |
            sed '/^$/d' |
            sort
    )"
    if [[ "${mount_destinations}" != "/workspace" ]]; then
        printf 'Phoenix received unexpected mounts:\n%s\n' "${mount_destinations}" >&2
        exit 1
    fi

    printf 'Phoenix isolation verified: RPC/OIDC configuration only; no Git CLI or product-storage mounts\n'
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
    "${fixture_root}/root-image" \
    "${fixture_root}/secret-keys"
chmod 0700 "${fixture_root}/secret-keys" "${secret_runtime_root}"
umask 077
head -c 32 /dev/zero | tr '\0' '\127' >"${fixture_root}/secret-keys/e2e-v1"
chmod 0400 "${fixture_root}/secret-keys/e2e-v1"

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
HEPHAESTUS_E2E_OIDC_PORT="${oidc_port}" \
HEPHAESTUS_E2E_WEB_URL="${web_url}" \
    node oidc-provider.mjs >"${fixture_root}/oidc.log" 2>&1 &
oidc_pid="$!"
wait_for_url "${oidc_url}/.well-known/openid-configuration" \
    "${fixture_root}/oidc.log"

cd "${repo_root}"
cargo build -p hephaestus-app --bins
HEPHAESTUS_DATABASE_URL="${database_url}" \
HEPHAESTUS_REPOSITORY_ROOT="${fixture_root}/repositories" \
HEPHAESTUS_ARTIFACT_ROOT="${fixture_root}/artifacts" \
HEPHAESTUS_BROWSER_OIDC_ISSUER="${oidc_url}" \
    "${repo_root}/target/debug/hephaestus-e2e-seed" \
    >"${fixture_root}/seed.json"

export HEPHAESTUS_DATABASE_URL="${database_url}"
export HEPHAESTUS_NATS_URL="nats://127.0.0.1:${nats_port}"
export HEPHAESTUS_HTTP_LISTEN="127.0.0.1:${daemon_port}"
export HEPHAESTUS_REPOSITORY_ROOT="${fixture_root}/repositories"
export HEPHAESTUS_GIT_HTTP_BACKEND="$(git --exec-path)/git-http-backend"
export HEPHAESTUS_OIDC_ISSUER="${oidc_url}"
export HEPHAESTUS_OIDC_AUDIENCE="hephaestus-git"
export HEPHAESTUS_OIDC_ALGORITHM="HS256"
export HEPHAESTUS_OIDC_HS256_SECRET="e2e-signing-secret-with-sufficient-entropy"
export HEPHAESTUS_VOLUME_ROOT="${fixture_root}/volumes"
export HEPHAESTUS_WORKSPACE_ROOT="${fixture_root}/workspaces"
export HEPHAESTUS_ARTIFACT_ROOT="${fixture_root}/artifacts"
export HEPHAESTUS_RUNTIME_ROOT="${fixture_root}/runtime"
export HEPHAESTUS_SECRET_RUNTIME_ROOT="${secret_runtime_root}"
export HEPHAESTUS_SECRET_KEY_DIRECTORY="${fixture_root}/secret-keys"
export HEPHAESTUS_SECRET_KEY_REFERENCE="e2e-v1"
export HEPHAESTUS_RPC_MEDIATOR_SECRET="e2e-rpc-mediator-secret-with-sufficient-entropy"
export HEPHAESTUS_ROOT_IMAGE_PATH="${fixture_root}/root-image"
export HEPHAESTUS_ROOT_IMAGE_REFERENCE="fixture-root@sha256:e2e"
export HEPHAESTUS_VM_BACKEND="fixture"
export HEPHAESTUS_HOST_ID="browser-e2e"
export HEPHAESTUS_MKFS_EXT4="$(command -v mkfs.ext4)"
export HEPHAESTUS_RUNTIME_POLICY_VERSION="browser-e2e/v1"
export HEPHAESTUS_RUNTIME_MAX_VCPUS="2"
export HEPHAESTUS_RUNTIME_MAX_MEMORY_MIB="1024"
export HEPHAESTUS_RUNTIME_ALLOW_BROKER_ONLY="true"
export HEPHAESTUS_RUNTIME_ALLOW_EGRESS="false"
export RUST_LOG="hephaestus_app=debug,git_http=debug,forge_service=debug,run_orchestrator=debug,review_service=debug"

"${repo_root}/target/debug/hephaestusd" >"${fixture_root}/daemon.log" 2>&1 &
daemon_pid="$!"
wait_for_url "${daemon_url}/healthz" "${fixture_root}/daemon.log"

# The repository may also be mounted by the persistent local server, so keep
# one shared SELinux label across development containers.
podman run --detach --rm \
    --name "${web_container}" \
    --network host \
    --volume "${repo_root}:/workspace:z" \
    --workdir /workspace/web \
    --env MIX_ENV=dev \
    --env PHX_SERVER=true \
    --env PORT="${web_port}" \
    --env HEPHAESTUS_RPC_ENDPOINT="127.0.0.1:${daemon_port}" \
    --env HEPHAESTUS_RPC_MEDIATOR_SECRET="${HEPHAESTUS_RPC_MEDIATOR_SECRET}" \
    --env HEPHAESTUS_BROWSER_OIDC_ISSUER="${oidc_url}" \
    --env HEPHAESTUS_BROWSER_OIDC_CLIENT_ID="hephaestus-web" \
    --env HEPHAESTUS_BROWSER_OIDC_CLIENT_SECRET="development-secret" \
    --env HEPHAESTUS_BROWSER_OIDC_REDIRECT_URI="${web_url}/auth/oidc/callback" \
    docker.io/hexpm/elixir:1.18.4-erlang-27.3.4-debian-bookworm-20250428-slim \
    sh -lc 'mix local.hex --force >/dev/null && mix clean && mix phx.server' \
    >"${fixture_root}/web-container-id"
wait_for_url "${web_url}/" "${fixture_root}/web.log"
assert_web_isolation

cd "${repo_root}/e2e/playwright"
HEPHAESTUS_E2E_DATABASE_URL="${database_url}" \
HEPHAESTUS_REPOSITORY_ROOT="${fixture_root}/repositories" \
HEPHAESTUS_GIT_URL="${daemon_url}" \
HEPHAESTUS_WEB_URL="${web_url}" \
HEPHAESTUS_OIDC_URL="${oidc_url}" \
    npm test

podman logs "${web_container}" >"${fixture_root}/web.log" 2>&1
podman exec "${postgres_container}" pg_dump --username postgres --dbname hephaestus \
    >"${fixture_root}/postgres.sql"

mapfile -t sentinel_files < <(
    grep --recursive --binary-files=text --fixed-strings --files-with-matches \
        "${secret_sentinel}" "${fixture_root}" || true
)
if [[ "${#sentinel_files[@]}" -ne 0 ]]; then
    printf 'secret sentinel escaped into host evidence file: %s\n' \
        "${sentinel_files[0]#"${fixture_root}/"}" >&2
    exit 1
fi
if podman exec "${nats_container}" \
    grep -R -a -F -q "${secret_sentinel}" /tmp/nats; then
    printf 'secret sentinel escaped into JetStream storage\n' >&2
    exit 1
fi
if find "${secret_runtime_root}" -mindepth 1 -print -quit | grep --quiet .; then
    printf 'ephemeral secret runtime objects remain after guest cleanup\n' >&2
    exit 1
fi
