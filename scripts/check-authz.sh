#!/usr/bin/env bash
set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
melange_binary=${MELANGE_BIN:-"$repository_root/.tools/melange/0.8.5/melange"}
database_url=${HEPHAESTUS_POSTGRES_TEST_URL:?set HEPHAESTUS_POSTGRES_TEST_URL}

"$repository_root/scripts/generate-authz.sh"
"$melange_binary" doctor \
    --db "$database_url" \
    --schema "$repository_root/authz/hephaestus.fga" \
    --skip-performance \
    --no-update-check
