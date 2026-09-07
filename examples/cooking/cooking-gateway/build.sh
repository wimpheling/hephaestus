#!/usr/bin/env bash
set -euo pipefail

# Isolated builds mount the exact source read-only; compiler output belongs in
# the guest's temporary filesystem.
build_root="$(mktemp -d /tmp/cooking-gateway-build.XXXXXX)"
trap 'rm -rf -- "${build_root}"' EXIT
# The libkrun rootfs carries files from the reviewed Rust image but does not
# carry OCI `ENV` metadata. Keep the reviewed toolchain contract explicit.
export CARGO_HOME=/opt/cargo
export LD_LIBRARY_PATH=/opt/rust/lib
export PATH=/opt/cargo/bin:/opt/rust/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
export RUSTUP_HOME=/opt/rustup
cargo build --locked --offline --release --target-dir "${build_root}"
install -D -m 0755 "${build_root}/release/cooking-gateway" /workspace/output/bin/cooking-gateway
