#!/usr/bin/env bash
set -euo pipefail

repository_root="${HEPHAESTUS_REPOSITORY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
tool_root="${HEPHAESTUS_PROTOBUF_TOOL_ROOT:-${repository_root}/.local/protobuf}"
bin_dir="${tool_root}/bin"
mix_dir="${tool_root}/mix"

buf_version=1.72.0
buf_sha256=8720830e26a733da55bb89bcd3cb44849c0965fc0c44fb5d691cccdc64dca5af
elixir_image=docker.io/hexpm/elixir:1.18.4-erlang-27.3.4-debian-bookworm-20250428-slim

mkdir -p "${bin_dir}" "${mix_dir}"

if [[ ! -x "${bin_dir}/buf" ]]; then
  curl --fail --location --proto '=https' --tlsv1.2 \
    "https://github.com/bufbuild/buf/releases/download/v${buf_version}/buf-Linux-x86_64" \
    --output "${bin_dir}/buf.download"
  printf '%s  %s\n' "${buf_sha256}" "${bin_dir}/buf.download" | sha256sum --check
  mv "${bin_dir}/buf.download" "${bin_dir}/buf"
  chmod +x "${bin_dir}/buf"
fi

if [[ "$("${bin_dir}/buf" --version)" != "${buf_version}" ]]; then
  printf 'expected Buf %s at %s\n' "${buf_version}" "${bin_dir}/buf" >&2
  exit 1
fi

install_rust_plugin() {
  local package=$1
  local version=$2
  local binary=$3

  if [[ ! -x "${bin_dir}/${binary}" ]]; then
    cargo install --locked --root "${tool_root}" --version "=${version}" "${package}"
  fi
}

install_rust_plugin protoc-gen-buffa 0.8.1 protoc-gen-buffa
install_rust_plugin protoc-gen-buffa-packaging 0.8.1 protoc-gen-buffa-packaging
install_rust_plugin connectrpc-codegen 0.8.0 protoc-gen-connect-rust

if [[ ! -x "${mix_dir}/escripts/protoc-gen-elixir" ]]; then
  podman run --rm \
    --volume "${mix_dir}:/root/.mix:z" \
    "${elixir_image}" \
    sh -lc 'apt-get update -qq && apt-get install -y -qq --no-install-recommends git ca-certificates >/dev/null && rm -rf /var/lib/apt/lists/* && mix local.hex --force >/dev/null && mix escript.install hex protobuf 0.17.0 --force'
fi

export PATH="${bin_dir}:${PATH}"
cd "${repository_root}"
buf format --write
buf lint
buf generate --template buf.gen.rust.yaml
# Cargo fmt follows declared modules but not every file reached through an
# include! in generated package shards. Format the two declared module roots.
rustup run 1.88.0 rustfmt --edition 2024 \
  crates/rpc-proto/src/generated/messages/mod.rs \
  crates/rpc-proto/src/generated/connect/mod.rs

podman run --rm \
  --volume "${repository_root}:/workspace:z" \
  --volume "${mix_dir}:/root/.mix:z" \
  --volume "${bin_dir}:/protobuf-tools:z,ro" \
  --workdir /workspace \
  "${elixir_image}" \
  sh -lc 'export PATH=/root/.mix/escripts:$PATH; /protobuf-tools/buf generate --template buf.gen.elixir.yaml; find web/lib/hephaestus_web/rpc/generated -type f -name "*.ex" -print0 | xargs -0 mix format'

buf build --output crates/rpc-proto/src/generated/descriptor.binpb
