#!/usr/bin/env bash
set -euo pipefail

cargo build --locked --offline --release
install -D -m 0755 target/release/cooking-gateway /workspace/output/bin/cooking-gateway
