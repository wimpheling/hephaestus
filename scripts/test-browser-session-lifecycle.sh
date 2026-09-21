#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
parent_url="${HEPHAESTUS_POSTGRES_TEST_URL:?HEPHAESTUS_POSTGRES_TEST_URL is required}"
nats_url="${HEPHAESTUS_NATS_TEST_URL:?HEPHAESTUS_NATS_TEST_URL is required}"
run_identity="${GITHUB_RUN_ID:-local_${PPID}}"
run_attempt="${GITHUB_RUN_ATTEMPT:-1}"
run_identity="${run_identity//[^a-zA-Z0-9_]/_}"
run_attempt="${run_attempt//[^a-zA-Z0-9_]/_}"
database_name="hephaestus_browser_session_${run_identity}_${run_attempt}_${RANDOM}"

database_url_without_query="${parent_url%%\?*}"
query_suffix=""
if [[ "${parent_url}" == *\?* ]]; then
  query_suffix="?${parent_url#*\?}"
fi
database_authority="${database_url_without_query%/*}"
maintenance_url="${database_authority}/postgres${query_suffix}"
isolated_url="${database_authority}/${database_name}${query_suffix}"
test_output="$(mktemp "${TMPDIR:-/tmp}/hephaestus-browser-session-lifecycle.XXXXXX")"

if command -v psql >/dev/null 2>&1; then
  run_psql() {
    psql --no-psqlrc --set ON_ERROR_STOP=1 --dbname "$1" --command "$2"
  }
else
  postgres_container="${HEPHAESTUS_POSTGRES_CONTAINER:-}"
  if [[ "${HEPHAESTUS_BROWSER_SESSION_POSTGRES_MODE:-}" != container ]] || \
    [[ -z "${postgres_container}" ]] || ! command -v podman >/dev/null 2>&1; then
    printf 'browser-session lifecycle requires host psql or explicit PostgreSQL container mode\n' >&2
    exit 1
  fi
  run_psql() {
    podman exec "${postgres_container}" psql --no-psqlrc \
      --username postgres --dbname postgres --set ON_ERROR_STOP=1 --command "$2"
  }
fi

database_created=0
cleanup() {
  local status=$?
  trap - EXIT
  if ((database_created)); then
    set +e
    run_psql "${maintenance_url}" "DROP DATABASE \"${database_name}\"" >/dev/null
    local drop_status=$?
    if ((status == 0 && drop_status != 0)); then
      status=${drop_status}
    fi
  fi
  rm -f -- "${test_output}"
  exit "${status}"
}
trap cleanup EXIT

printf 'browser-session lifecycle database: %s\n' "${database_name}"
run_psql "${maintenance_url}" "CREATE DATABASE \"${database_name}\"" >/dev/null
database_created=1

HEPHAESTUS_POSTGRES_TEST_URL="${isolated_url}" \
  HEPHAESTUS_NATS_TEST_URL="${nats_url}" \
  REAL_APP_BROWSER_SESSION_RPC=1 \
  CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}" \
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" \
  timeout --kill-after=30s 180s cargo +1.88.0 test \
    --manifest-path "${repo_root}/Cargo.toml" \
    --package hephaestus-app \
    --test browser_session_lifecycle \
    --features test-fixtures \
    -- --nocapture --test-threads=1 2>&1 | tee "${test_output}"

if ! grep -Fq 'REAL_BROWSER_SESSION_LIFECYCLE=1' "${test_output}"; then
  printf 'browser-session lifecycle did not report completion\n' >&2
  exit 1
fi
