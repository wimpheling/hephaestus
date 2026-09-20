#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
parent_url="${HEPHAESTUS_POSTGRES_TEST_URL:?HEPHAESTUS_POSTGRES_TEST_URL is required}"
nats_url="${HEPHAESTUS_NATS_TEST_URL:?HEPHAESTUS_NATS_TEST_URL is required}"
run_identity="${GITHUB_RUN_ID:-local_${PPID}}"
run_attempt="${GITHUB_RUN_ATTEMPT:-1}"
run_identity="${run_identity//[^a-zA-Z0-9_]/_}"
run_attempt="${run_attempt//[^a-zA-Z0-9_]/_}"
database_name="hephaestus_ui_rpc_${run_identity}_${run_attempt}_${RANDOM}"

database_url_without_query="${parent_url%%\?*}"
query_suffix=""
if [[ "${parent_url}" == *\?* ]]; then
  query_suffix="?${parent_url#*\?}"
fi
database_authority="${database_url_without_query%/*}"
maintenance_url="${database_authority}/postgres${query_suffix}"
isolated_url="${database_authority}/${database_name}${query_suffix}"
test_output="${HEPHAESTUS_UI_RPC_LOG:-${TMPDIR:-/tmp}/heph-ui-installation-rpc.${database_name}.log}"

if command -v psql >/dev/null 2>&1; then
  run_psql() {
    psql --no-psqlrc --set ON_ERROR_STOP=1 --dbname "$1" --command "$2"
  }
else
  postgres_container="${HEPHAESTUS_POSTGRES_CONTAINER:-}"
  if [[ "${HEPHAESTUS_UI_RPC_POSTGRES_MODE:-}" != container ]] || \
    [[ -z "${postgres_container}" ]] || ! command -v podman >/dev/null 2>&1; then
    printf 'UI installation RPC requires host psql or explicit PostgreSQL container mode\n' >&2
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
  exit "${status}"
}
trap cleanup EXIT

mkdir -p -- "$(dirname -- "${test_output}")"
printf 'UI installation RPC disposable database: %s\n' "${database_name}"
run_psql "${maintenance_url}" "CREATE DATABASE \"${database_name}\"" >/dev/null
database_created=1

HEPHAESTUS_POSTGRES_TEST_URL="${isolated_url}" \
  HEPHAESTUS_NATS_TEST_URL="${nats_url}" \
  REAL_UI_INSTALLATION_RPC=1 \
  CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}" \
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" \
  timeout --kill-after=30s "${HEPHAESTUS_UI_RPC_TIMEOUT:-300s}" cargo +1.88.0 test \
    --manifest-path "${repo_root}/Cargo.toml" \
    --package hephaestus-app \
    --test ui_installation_rpc \
    --features test-fixtures \
    -- --ignored --exact ui_installation_rpc::ui_installation_transport::production_ui_installation_rpc_matrix --nocapture --test-threads=1 2>&1 | tee "${test_output}"

if ! grep -Fq 'REAL_UI_INSTALLATION_RPC=1' "${test_output}"; then
  printf 'UI installation RPC did not report a real completion marker; log: %s\n' "${test_output}" >&2
  exit 1
fi
printf 'UI installation RPC proof passed; log: %s\n' "${test_output}"
