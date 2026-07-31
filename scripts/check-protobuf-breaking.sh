#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
buf="${HEPHAESTUS_BUF:-${repository_root}/.local/protobuf/bin/buf}"

if [[ ! -x "${buf}" ]]; then
  printf 'Buf is not installed; run scripts/generate-protobuf.sh first\n' >&2
  exit 1
fi

if [[ -n "${HEPHAESTUS_BUF_BREAKING_AGAINST:-}" ]]; then
  exec "${buf}" breaking --against "${HEPHAESTUS_BUF_BREAKING_AGAINST}"
fi

if git -C "${repository_root}" rev-parse --verify --quiet origin/main >/dev/null; then
  if git -C "${repository_root}" cat-file -e origin/main:proto 2>/dev/null; then
    exec "${buf}" breaking --against '.git#ref=origin/main,subdir=proto'
  fi

  printf '%s\n' 'PASS Buf breaking bootstrap: origin/main has no proto tree; this change establishes the reviewed v1 baseline'
  exit 0
fi

printf '%s\n' 'SKIP Buf breaking: set HEPHAESTUS_BUF_BREAKING_AGAINST to the reviewed baseline in CI'
