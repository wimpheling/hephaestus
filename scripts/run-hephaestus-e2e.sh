#!/usr/bin/env bash
#
# Run the daemon-level golden path locally with the real libkrun/KVM backend.

set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly script_dir

export HEPHAESTUS_APP_LIBKRUN_E2E=1
exec "${script_dir}/run-libkrun-integration.sh"
