#!/usr/bin/env bash
set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
melange_binary=${MELANGE_BIN:-"$repository_root/.tools/melange/0.8.5/melange"}
schema="$repository_root/authz/hephaestus.fga"
committed="$repository_root/migrations/0003_melange_generated.sql"
tuple_source="$repository_root/authz/melange_tuples.sql"
tuple_migration="$repository_root/migrations/0002_melange_tuples.sql"

if [[ ! -x "$melange_binary" ]]; then
    printf 'Mélange v0.8.5 is required; run scripts/install-melange.sh\n' >&2
    exit 1
fi
if [[ "$("$melange_binary" version)" != melange\ 0.8.5* ]]; then
    printf 'expected Mélange v0.8.5 at %s\n' "$melange_binary" >&2
    exit 1
fi
if ! diff --unified \
    <(tail -n +3 "$tuple_source") \
    <(tail -n +2 "$tuple_migration"); then
    printf 'melange_tuples source and migration differ\n' >&2
    exit 1
fi

"$melange_binary" validate --schema "$schema" --no-update-check
generated=$(mktemp)
trap 'rm -f "$generated"' EXIT
"$melange_binary" generate migration \
    --schema "$schema" \
    --up \
    --no-update-check > "$generated"

if [[ ${1:-} == --write ]]; then
    cp "$generated" "$committed"
    printf 'wrote %s\n' "$committed"
    exit 0
fi

if ! cmp --silent "$generated" "$committed"; then
    diff --unified "$committed" "$generated" || true
    printf 'generated Mélange SQL differs; run scripts/generate-authz.sh --write\n' >&2
    exit 1
fi
printf 'Mélange generated migration is current\n'
