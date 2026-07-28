#!/usr/bin/env bash
set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
fga_binary=${FGA_BIN:-"$repository_root/.tools/openfga-cli/0.7.19/fga"}
if [[ ! -x "$fga_binary" ]]; then
    printf 'OpenFGA CLI v0.7.19 is required; run scripts/install-openfga-cli.sh\n' >&2
    exit 1
fi
"$fga_binary" model test \
    --tests "$repository_root/authz/hephaestus.fga.yaml"
