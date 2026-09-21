#!/usr/bin/env bash
set -euo pipefail

build_root="$(mktemp -d /tmp/cooking-service-build.XXXXXX)"
trap 'rm -rf -- "${build_root}"' EXIT
export CARGO_HOME=/opt/cargo
export LD_LIBRARY_PATH=/opt/rust/lib
export PATH=/opt/cargo/bin:/opt/rust/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
export RUSTUP_HOME=/opt/rustup
cargo build --locked --offline --release --target-dir "${build_root}"
install -D -m 0755 "${build_root}/release/cooking-service" \
    /workspace/output/bin/cooking-service
