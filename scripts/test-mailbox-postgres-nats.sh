#!/usr/bin/env bash
# Real-service proof for the authoritative PostgreSQL mailbox and identifier-
# only JetStream command path. The Rust test owns assertions and migrations.
set -euo pipefail

readonly repo_root="$(git rev-parse --show-toplevel)"
readonly postgres_image="${HEPHAESTUS_POSTGRES_TEST_IMAGE:-docker.io/library/postgres:17-alpine}"
readonly nats_image="${HEPHAESTUS_NATS_TEST_IMAGE:-docker.io/library/nats:2.11-alpine}"
readonly postgres_container="hephaestus-mailbox-postgres-${PPID}-${RANDOM}"
readonly nats_container="hephaestus-mailbox-nats-${PPID}-${RANDOM}"

cleanup() {
    podman rm --force "${nats_container}" "${postgres_container}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

for command in cargo podman; do
    command -v "${command}" >/dev/null || {
        printf 'required command is unavailable: %s\n' "${command}" >&2
        exit 1
    }
done

published_port() {
    podman port "$1" "$2/tcp" | awk -F: 'NR == 1 { print $NF }'
}

podman run --detach --rm \
    --name "${postgres_container}" \
    --env POSTGRES_PASSWORD=postgres \
    --env POSTGRES_DB=hephaestus \
    --publish 127.0.0.1::5432 \
    "${postgres_image}" >/dev/null
podman run --detach --rm \
    --name "${nats_container}" \
    --publish 127.0.0.1::4222 \
    "${nats_image}" -js >/dev/null

for _attempt in $(seq 1 30); do
    if podman exec "${postgres_container}" pg_isready --quiet --username postgres --dbname hephaestus \
        && podman logs "${nats_container}" 2>&1 | grep -q 'Server is ready'; then
        break
    fi
    sleep 1
    if (( _attempt == 30 )); then
        podman logs "${postgres_container}" >&2 || true
        podman logs "${nats_container}" >&2 || true
        printf 'PostgreSQL or NATS did not become ready\n' >&2
        exit 1
    fi
done

readonly postgres_port="$(published_port "${postgres_container}" 5432)"
readonly nats_port="$(published_port "${nats_container}" 4222)"
HEPHAESTUS_POSTGRES_TEST_URL="postgres://postgres:postgres@127.0.0.1:${postgres_port}/hephaestus?sslmode=disable" \
HEPHAESTUS_NATS_TEST_URL="nats://127.0.0.1:${nats_port}" \
cargo test --manifest-path "${repo_root}/Cargo.toml" \
    --package mailbox-postgres --test postgres -- --nocapture

printf 'PostgreSQL/NATS durable mailbox integration passed\n'
