#!/usr/bin/env bash
set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
melange_binary=${MELANGE_BIN:-"$repository_root/.tools/melange/0.8.5/melange"}
schema="$repository_root/authz/hephaestus.fga"
committed="$repository_root/migrations/0102_private_volume_authorization.sql"
tuple_source="$repository_root/authz/history/private-volumes-v1-tuples.sql"

# Applied migration 0006 is reproduced only from frozen historical inputs.
# A model update must never change its SQLx checksum or forward schema needs.
"$repository_root/scripts/check-authz-history.sh"
"$melange_binary" validate --schema "$schema" --no-update-check
generated=$(mktemp)
normalized=$(mktemp)
composed=$(mktemp)
trap 'rm -f "$generated" "$normalized" "$composed"' EXIT
"$melange_binary" generate migration --schema "$schema" --up --no-update-check > "$generated"
# Pinned Mélange 0.8.5 ReadSchemaContent reads a single .fga file unchanged;
# ComputeSchemaChecksum hashes those bytes (modular manifests differ).
model_checksum=$(sha256sum "$schema" | cut -d ' ' -f 1)
# Preserve the deployed dispatcher argument name. Generated CREATE OR REPLACE
# also resets its security attributes; restore them after generation below.
awk '
    /^CREATE OR REPLACE FUNCTION "public"."check_permission"\(/ { normalize = 1 }
    normalize { gsub(/p_relation/, "p_permission") }
    normalize && /^\$\$ LANGUAGE sql STABLE;/ { normalize = 0 }
    { print }
' "$generated" > "$normalized"
{
    printf '%s\n' '-- Current private-volume authorization model; historical migration 0006 is frozen.'
    cat "$tuple_source"
    printf '\n'
    cat "$normalized"
    cat <<'SQL'

-- Preserve the request authorization security boundary after regeneration.
ALTER FUNCTION check_permission(text, text, text, text, text) SECURITY DEFINER;
ALTER FUNCTION check_permission(text, text, text, text, text) SET search_path = pg_catalog, public;
ALTER FUNCTION check_permission(text, text, text, text, text) OWNER TO hephaestus_authz_owner;
REVOKE ALL ON FUNCTION check_permission(text, text, text, text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION check_permission(text, text, text, text, text) TO hephaestus_app, hephaestus_worker;
GRANT SELECT ON melange_tuples TO hephaestus_authz_owner;
SQL
    # Append current tracking; never replace the historical migration records.
    printf '\nINSERT INTO melange_migrations\n    (melange_version, schema_checksum, codegen_version, function_names)\nSELECT\n    '\''0.8.5'\'',\n    '\''%s'\'',\n    '\''0.8.5'\'',\n    array_agg(pg_proc.proname ORDER BY pg_proc.proname)\nFROM pg_proc\nJOIN pg_namespace ON pg_namespace.oid = pg_proc.pronamespace\nWHERE pg_namespace.nspname = '\''public'\''\n  AND pg_proc.proname ~ '\''^(check_|expand_|explain_|list_)'\'';\n' "$model_checksum"
} | sed -E -e 's/[[:space:]]+$//' -e 's/ +\t/\t/g' > "$composed"

if [[ ${1:-} == --write ]]; then
    printf 'migration0102 is applied and frozen; add a new authorization migration for model changes\n' >&2
    exit 1
fi
if ! cmp --silent "$composed" "$committed"; then
    diff --unified "$committed" "$composed" || true
    printf 'frozen authorization migration0102 differs; preserve applied history and inspect frozen inputs\n' >&2
    exit 1
fi
printf 'historical and current Mélange migrations are current\n'
if ! grep -F -q "\"melange-0.8.5:${model_checksum}\"" \
    "$repository_root/crates/heph-std/authorization/authz-postgres/src/lib.rs"; then
    printf 'current authorization audit version differs; update AUTHORIZATION_MODEL_VERSION to melange-0.8.5:%s without changing historical audit records\n' "$model_checksum" >&2
    exit 1
fi

"$repository_root/scripts/generate-volume-mount-tuples.sh"
