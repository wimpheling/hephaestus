#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tool_root="${HEPHAESTUS_PROTOBUF_TOOL_ROOT:-${repository_root}/.local/protobuf}"
scratch=$(mktemp -d)
trap 'rm -rf "${scratch}"' EXIT

cp -R "${repository_root}/proto" "${scratch}/proto"
cp "${repository_root}/buf.yaml" "${scratch}/buf.yaml"
cp "${repository_root}/buf.gen.rust.yaml" "${scratch}/buf.gen.rust.yaml"
cp "${repository_root}/buf.gen.elixir.yaml" "${scratch}/buf.gen.elixir.yaml"
mkdir -p "${scratch}/crates/rpc-proto/src/generated" \
  "${scratch}/web/lib/hephaestus_web/rpc/generated"

HEPHAESTUS_REPOSITORY_ROOT="${scratch}" \
HEPHAESTUS_PROTOBUF_TOOL_ROOT="${tool_root}" \
  "${repository_root}/scripts/generate-protobuf.sh"

diff --recursive --unified --new-file \
  "${repository_root}/crates/rpc-proto/src/generated" \
  "${scratch}/crates/rpc-proto/src/generated"
diff --recursive --unified --new-file \
  "${repository_root}/web/lib/hephaestus_web/rpc/generated" \
  "${scratch}/web/lib/hephaestus_web/rpc/generated"
