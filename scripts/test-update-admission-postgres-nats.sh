#!/usr/bin/env bash
# Rerunnable PostgreSQL/NATS proof for the durable update-admission race.
# The golden test uses the deterministic result guest; no cooking or libkrun
# resources are enabled by this wrapper.
set -euo pipefail

readonly repo_root="$(git rev-parse --show-toplevel)"
readonly container_engine="${CONTAINER_ENGINE:-podman}"
readonly default_postgres_image="docker.io/library/postgres@sha256:af194ccf3e2d7fe367012c7b88ce8b816c5c889b18a5b316799a1f0d7eac746a"
readonly default_nats_image="docker.io/library/nats@sha256:e4bf19f15fd3218814a4e3c9e0064e1334bd8aa20d5984b9f1a0afd084f8cc00"
readonly postgres_image="${HEPHAESTUS_POSTGRES_TEST_IMAGE:-${default_postgres_image}}"
readonly nats_image="${HEPHAESTUS_NATS_TEST_IMAGE:-${default_nats_image}}"
readonly postgres_container="hephaestus-update-admission-postgres-${PPID}-${RANDOM}"
readonly nats_container="hephaestus-update-admission-nats-${PPID}-${RANDOM}"
readonly test_timeout_seconds="${HEPHAESTUS_UPDATE_ADMISSION_TIMEOUT_SECONDS:-180}"
no_feature_log=''

cleanup() {
    "${container_engine}" rm --force "${nats_container}" "${postgres_container}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

for command in cargo "${container_engine}" timeout mktemp; do
    command -v "${command}" >/dev/null || {
        printf 'required command is unavailable: %s\n' "${command}" >&2
        exit 1
    }
done
[[ "${test_timeout_seconds}" =~ ^[1-9][0-9]*$ ]] || {
    printf 'test timeout must be a positive integer number of seconds\n' >&2
    exit 1
}

for image in "${postgres_image}" "${nats_image}"; do
    [[ "${image}" =~ @sha256:[0-9a-f]{64}$ ]] || {
        printf 'test service image must be an immutable digest reference: %s\n' "${image}" >&2
        exit 1
    }
done

published_port() {
    "${container_engine}" port "$1" "$2/tcp" | awk -F: 'NR == 1 { print $NF }'
}

"${container_engine}" run --detach --rm \
    --name "${postgres_container}" \
    --env POSTGRES_PASSWORD=postgres \
    --env POSTGRES_DB=hephaestus \
    --publish 127.0.0.1::5432 \
    "${postgres_image}" >/dev/null
"${container_engine}" run --detach --rm \
    --name "${nats_container}" \
    --publish 127.0.0.1::4222 \
    "${nats_image}" -js >/dev/null

for _attempt in $(seq 1 30); do
    if "${container_engine}" exec "${postgres_container}" pg_isready --quiet --username postgres --dbname hephaestus \
        && "${container_engine}" logs "${nats_container}" 2>&1 | grep -q 'Server is ready'; then
        break
    fi
    sleep 1
    if (( _attempt == 30 )); then
        "${container_engine}" logs "${postgres_container}" >&2 || true
        "${container_engine}" logs "${nats_container}" >&2 || true
        printf 'PostgreSQL or NATS did not become ready\n' >&2
        exit 1
    fi
done

readonly postgres_port="$(published_port "${postgres_container}" 5432)"
readonly nats_port="$(published_port "${nats_container}" 4222)"
export HEPHAESTUS_POSTGRES_TEST_URL="postgres://postgres:postgres@127.0.0.1:${postgres_port}/hephaestus?sslmode=disable"
export HEPHAESTUS_NATS_TEST_URL="nats://127.0.0.1:${nats_port}"
export HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E=1
export HEPHAESTUS_APP_COOKING_E2E=0
export HEPHAESTUS_APP_LIBKRUN_E2E=0
export HEPHAESTUS_APP_COOKING_BUILD_PROOF=0
export HEPHAESTUS_APP_GATEWAY_CADDY_E2E=0

timeout --kill-after=30s "${test_timeout_seconds}s" cargo test --manifest-path "${repo_root}/Cargo.toml" \
    --package hephaestus-app --test golden --features test-fixtures \
    bearer_push_starts_run_through_production_bootstrap -- --nocapture

# Use a fresh database for the guard branch. The golden fixture intentionally
# creates durable identities with fixed issuer/subject pairs, so reusing the
# first database would test fixture idempotency instead of the feature guard.
"${container_engine}" exec "${postgres_container}" psql -U postgres -d postgres -v ON_ERROR_STOP=1 \
    -c 'CREATE DATABASE hephaestus_admission' >/dev/null
readonly ordinary_url="postgres://postgres:postgres@127.0.0.1:${postgres_port}/hephaestus_admission?sslmode=disable"
unset HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E
HEPHAESTUS_POSTGRES_TEST_URL="${ordinary_url}" \
HEPHAESTUS_APP_UPDATE_ADMISSION_E2E=1 \
timeout --kill-after=30s "${test_timeout_seconds}s" cargo test --manifest-path "${repo_root}/Cargo.toml" \
    --package hephaestus-app --test golden \
    bearer_push_starts_run_through_production_bootstrap -- --nocapture

"${container_engine}" exec "${postgres_container}" psql -U postgres -d postgres -v ON_ERROR_STOP=1 \
    -c 'CREATE DATABASE hephaestus_no_feature' >/dev/null
readonly no_feature_url="postgres://postgres:postgres@127.0.0.1:${postgres_port}/hephaestus_no_feature?sslmode=disable"
no_feature_log="$(mktemp "${TMPDIR:-/tmp}/hephaestus-update-admission-no-feature.XXXXXX.log")"
set +e
HEPHAESTUS_POSTGRES_TEST_URL="${no_feature_url}" HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E=1 timeout --kill-after=30s "${test_timeout_seconds}s" cargo test --manifest-path "${repo_root}/Cargo.toml" \
    --package hephaestus-app --test golden \
    bearer_push_starts_run_through_production_bootstrap -- --nocapture \
    >"${no_feature_log}" 2>&1
status=$?
set -e
if (( status != 101 )) || ! grep -Fq 'HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E requires --features hephaestus-app/test-fixtures' \
    "${no_feature_log}"; then
    cat "${no_feature_log}" >&2
    printf 'no-feature race invocation did not fail with the required diagnostic\n' >&2
    exit 1
fi

printf 'PostgreSQL/NATS durable update-admission race integration passed\n'
printf 'No-feature guard diagnostic retained at %s\n' "${no_feature_log}"
