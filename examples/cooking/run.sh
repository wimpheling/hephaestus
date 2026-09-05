#!/usr/bin/env bash
# Run the deterministic cooking applications through the real joined fixture.
set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd -- "${script_dir}/../.." && pwd -P)"
cooking_root="${HEPHAESTUS_COOKING_SOURCE_ROOT:-${script_dir}}"
python_image="${HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE:?set an immutable python-ubuntu image reference}"
[[ "${python_image}" == *@sha256:* ]] || {
    printf 'The cooking fixture requires a digest-pinned Python guest image.\n' >&2
    exit 1
}
readonly script_dir repo_root cooking_root python_image

# Run inside the example so Cargo loads its vendored-source configuration.
(
    cd -- "${cooking_root}/cooking-gateway"
    cargo build --locked --offline --release --target x86_64-unknown-linux-musl
)

HEPHAESTUS_APP_COOKING_E2E=1 \
HEPHAESTUS_COOKING_SOURCE_ROOT="${cooking_root}" \
HEPHAESTUS_COOKING_GATEWAY_ARTIFACT="${cooking_root}/cooking-gateway/target/x86_64-unknown-linux-musl/release/cooking-gateway" \
HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE="${python_image}" \
    "${repo_root}/scripts/run-gateway-libkrun-e2e.sh"
