#!/usr/bin/env bash
set -euo pipefail
repository_root=$(git rev-parse --show-toplevel)
baseline="$repository_root/authz/history/private-volumes-v1-tuples.sql"
fragment="$repository_root/authz/volume-mount-grant-tuples.sql"
projection="$repository_root/authz/melange_tuples.sql"
migration="$repository_root/migrations/0105_private_volume_mount_grants.sql"
generated=$(mktemp)
embedded=$(mktemp)
trap 'rm -f "$generated" "$embedded"' EXIT
# This source authority helper is equivalent only to the frozen0102 model.
# A model transition must revisit the helper and its native parity matrix.
model_checksum=$(sha256sum "$repository_root/authz/hephaestus.fga" | cut -d ' ' -f 1)
if ! grep -F -q "source-model-sha256: $model_checksum" "$migration"; then
    printf 'typed volume source helper model pin differs; update the helper and native parity tests in a new migration\n' >&2
    exit 1
fi
{
    sed '$s/;[[:space:]]*$//' "$baseline"
    printf '\nUNION ALL\n'
    cat "$fragment"
    printf ';\n'
} > "$generated"
if [[ ${1:-} == --write ]]; then
    printf 'applied projections are frozen; draft a new migration when extending authority\n' >&2
    exit 1
fi
sed -n '/^-- BEGIN GENERATED VOLUME MOUNT PROJECTION$/,/^-- END GENERATED VOLUME MOUNT PROJECTION$/p' "$migration" | sed '1d;$d' > "$embedded"
if ! cmp --silent "$generated" "$projection" || ! cmp --silent "$generated" "$embedded"; then
    printf 'typed mount projection differs from frozen0102 baseline plus typed grant fragment\n' >&2
    diff --unified "$projection" "$generated" || true
    exit 1
fi
printf 'frozen0102 and additive0105 tuple projection verified\n'
