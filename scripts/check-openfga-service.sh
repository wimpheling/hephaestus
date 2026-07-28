#!/usr/bin/env bash
set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
fga_binary=${FGA_BIN:-"$repository_root/.tools/openfga-cli/0.7.19/fga"}
container_engine=${CONTAINER_ENGINE:-podman}
image='docker.io/openfga/openfga@sha256:b0eaf46ae75bf329d17a346a4a905c229694c998d3be2a36ec569ca7bebf6a5b'
container_name="hephaestus-openfga-${PPID}-${RANDOM}"

cleanup() {
    "$container_engine" stop "$container_name" >/dev/null 2>&1 || true
}
trap cleanup EXIT

"$container_engine" run --rm --detach \
    --name "$container_name" \
    --publish 127.0.0.1::8080 \
    "$image" run >/dev/null
port=$("$container_engine" port "$container_name" 8080/tcp | sed 's/.*://')
for _attempt in $(seq 1 30); do
    if curl --fail --silent "http://127.0.0.1:${port}/healthz" >/dev/null; then
        break
    fi
    sleep 1
done
curl --fail --silent "http://127.0.0.1:${port}/healthz" >/dev/null

imported=$(
    "$fga_binary" store import \
        --api-url "http://127.0.0.1:${port}" \
        --file "$repository_root/authz/hephaestus.fga.yaml"
)
store_id=$(sed -n 's/.*"id":"\([^"]*\)".*/\1/p' <<<"$imported")
model_id=$(sed -n 's/.*"authorization_model_id":"\([^"]*\)".*/\1/p' <<<"$imported")
test -n "$store_id"
test -n "$model_id"

"$fga_binary" model test \
    --api-url "http://127.0.0.1:${port}" \
    --store-id "$store_id" \
    --model-id "$model_id" \
    --tests "$repository_root/authz/hephaestus.fga.yaml"
