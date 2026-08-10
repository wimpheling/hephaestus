#!/usr/bin/env bash
# Real PostgreSQL proof for transactional gateway lifecycle invalidations,
# compare-and-swap behavior, and the product-event outbox.
set -euo pipefail

readonly repo_root="$(git rev-parse --show-toplevel)"
readonly postgres_image="${HEPHAESTUS_POSTGRES_TEST_IMAGE:-docker.io/library/postgres:17-alpine}"
readonly postgres_container="hephaestus-gateway-postgres-${PPID}-${RANDOM}"

cleanup() {
    podman rm --force "${postgres_container}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

command -v cargo >/dev/null || { printf 'required command is unavailable: cargo\n' >&2; exit 1; }
command -v podman >/dev/null || { printf 'required command is unavailable: podman\n' >&2; exit 1; }

podman run --detach --rm \
    --name "${postgres_container}" \
    --env POSTGRES_PASSWORD=postgres \
    --env POSTGRES_DB=hephaestus \
    --publish 127.0.0.1::5432 \
    "${postgres_image}" >/dev/null

for attempt in $(seq 1 30); do
    if podman exec "${postgres_container}" pg_isready --quiet --username postgres --dbname hephaestus; then
        break
    fi
    if (( attempt == 30 )); then
        podman logs "${postgres_container}" >&2 || true
        printf 'PostgreSQL did not become ready\n' >&2
        exit 1
    fi
    sleep 1
done

readonly postgres_port="$(podman port "${postgres_container}" 5432/tcp | awk -F: 'NR == 1 { print $NF }')"
# Alpine PostgreSQL can report ready immediately before the TCP listener has
# completed its final handoff; a short bounded settle mirrors the other local
# integration harnesses and avoids a first-client reset.
sleep 1
HEPHAESTUS_POSTGRES_TEST_URL="postgres://postgres:postgres@127.0.0.1:${postgres_port}/hephaestus?sslmode=disable" \
cargo test --manifest-path "${repo_root}/Cargo.toml" \
    --package hephaestus-app --test event_durability \
    postgres_gateway_lifecycle_event_is_atomic_receipted_and_compare_and_swap_safe -- --nocapture

printf 'PostgreSQL gateway lifecycle/outbox integration passed\n'
