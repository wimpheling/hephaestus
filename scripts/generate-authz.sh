#!/usr/bin/env bash
set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
melange_binary=${MELANGE_BIN:-"$repository_root/.tools/melange/0.8.5/melange"}
schema="$repository_root/authz/hephaestus.fga"
committed="$repository_root/migrations/0006_melange_releases_and_secrets.sql"
tuple_source="$repository_root/authz/melange_tuples.sql"

if [[ ! -x "$melange_binary" ]]; then
    printf 'Mélange v0.8.5 is required; run scripts/install-melange.sh\n' >&2
    exit 1
fi
if [[ "$("$melange_binary" version)" != melange\ 0.8.5* ]]; then
    printf 'expected Mélange v0.8.5 at %s\n' "$melange_binary" >&2
    exit 1
fi
"$melange_binary" validate --schema "$schema" --no-update-check
generated=$(mktemp)
normalized=$(mktemp)
composed=$(mktemp)
trap 'rm -f "$generated" "$normalized" "$composed"' EXIT
"$melange_binary" generate migration \
    --schema "$schema" \
    --up \
    --no-update-check > "$generated"
# Migration 0004 wraps the dispatcher with a SECURITY DEFINER function whose
# third argument is named p_permission. PostgreSQL requires CREATE OR REPLACE
# to retain input parameter names, so normalize that one generated wrapper.
awk '
    /^CREATE OR REPLACE FUNCTION "public"."check_permission"\(/ {
        normalize = 1
    }
    normalize {
        gsub(/p_relation/, "p_permission")
    }
    normalize && /^\$\$ LANGUAGE sql STABLE;/ {
        normalize = 0
    }
    { print }
' "$generated" > "$normalized"
{
    printf '%s\n' '-- Authoritative tuple projection for authorization model v2.'
    tail -n +4 "$tuple_source"
    printf '\n'
    # Initial development intentionally removes the superseded hollow-agent
    # type. Full-mode generation does not infer deleted functions, so record
    # the exact one-time cleanup in the committed migration.
    for function_name in \
        check_agent_can_execute check_agent_can_execute_nw \
        check_agent_can_manage check_agent_can_manage_nw \
        check_agent_can_read check_agent_can_read_nw \
        check_agent_project check_agent_project_nw \
        check_run_agent check_run_agent_nw \
        check_state_volume_agent check_state_volume_agent_nw \
        expand_agent_can_execute expand_agent_can_manage \
        expand_agent_can_read expand_agent_project expand_run_agent \
        expand_state_volume_agent explain_agent_can_execute \
        explain_agent_can_manage explain_agent_can_read \
        explain_agent_project explain_run_agent explain_state_volume_agent \
        list_agent_can_execute_obj list_agent_can_execute_sub \
        list_agent_can_manage_obj list_agent_can_manage_sub \
        list_agent_can_read_obj list_agent_can_read_sub \
        list_agent_project_obj list_agent_project_sub \
        list_run_agent_obj list_run_agent_sub \
        list_state_volume_agent_obj list_state_volume_agent_sub
    do
        printf 'DROP FUNCTION IF EXISTS "public"."%s" CASCADE;\n' "$function_name"
    done
    printf '\n'
    cat "$normalized"
} > "$composed"

if [[ ${1:-} == --write ]]; then
    cp "$composed" "$committed"
    printf 'wrote %s\n' "$committed"
    exit 0
fi

if ! cmp --silent "$composed" "$committed"; then
    diff --unified "$committed" "$composed" || true
    printf 'generated Mélange SQL differs; run scripts/generate-authz.sh --write\n' >&2
    exit 1
fi
printf 'Mélange generated migration is current\n'
