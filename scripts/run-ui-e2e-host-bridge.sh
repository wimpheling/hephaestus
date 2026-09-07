#!/usr/bin/env bash
# Execute the browser harness in the caller's host namespace.
#
# The cooking golden test runs in a mapped UID namespace.  Rootless Podman
# cannot join its host user's pause namespace from there, so this watcher is
# started before that mapping and receives only fixture-scoped requests.
set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd -- "${script_dir}/.." && pwd -P)"
bridge_dir="${1:?bridge directory is required}"
diagnostics_dir="${2:-}"
deadline_epoch="${3:-0}"
external_script="${repo_root}/scripts/run-ui-e2e-external.sh"

[[ "${bridge_dir}" = /* && -d "${bridge_dir}" && ! -L "${bridge_dir}" ]] || {
    printf 'browser bridge directory is invalid\n' >&2
    exit 1
}
bridge_real="$(cd -- "${bridge_dir}" && pwd -P)"
[[ -x "${external_script}" ]] || {
    printf 'browser external harness is unavailable\n' >&2
    exit 1
}
command -v setsid >/dev/null 2>&1 || {
    printf 'browser host bridge requires setsid\n' >&2
    exit 1
}
if [[ -n "${diagnostics_dir}" ]]; then
    [[ "${diagnostics_dir}" = /* && -d "${diagnostics_dir}" && ! -L "${diagnostics_dir}" ]] || {
        printf 'browser diagnostics directory is invalid\n' >&2
        exit 1
    }
    diagnostics_real="$(cd -- "${diagnostics_dir}" && pwd -P)"
else
    diagnostics_real=""
fi
[[ "${deadline_epoch}" =~ ^[1-9][0-9]*$ ]] || {
    printf 'browser bridge deadline is invalid\n' >&2
    exit 1
}

child_pid=""
cleanup() {
    local status="$?"
    trap - EXIT INT TERM
    if [[ -n "${child_pid}" ]]; then
        kill -TERM -- "-${child_pid}" >/dev/null 2>&1 || true
        for _attempt in {1..50}; do
            kill -0 -- "${child_pid}" >/dev/null 2>&1 || break
            sleep 0.1
        done
        kill -KILL -- "-${child_pid}" >/dev/null 2>&1 || true
        wait "${child_pid}" 2>/dev/null || true
    fi
    rm -f -- "${bridge_real}"/request.*.json "${bridge_real}"/request.*.json.pending \
        "${bridge_real}"/response.* "${bridge_real}"/values.* \
        "${bridge_real}/.ready" "${bridge_real}/.ready.tmp."* 2>/dev/null || true
    exit "${status}"
}
trap cleanup EXIT INT TERM

ready_tmp="${bridge_real}/.ready.tmp.$$"
printf 'ready\n' >"${ready_tmp}"
chmod 600 -- "${ready_tmp}"
mv -- "${ready_tmp}" "${bridge_real}/.ready"

while :; do
    now="$(date +%s)"
    if (( now >= deadline_epoch )); then
        exit 124
    fi
    request=""
    for candidate in "${bridge_real}"/request.*.json; do
        [[ -f "${candidate}" && ! -L "${candidate}" ]] || continue
        request="${candidate}"
        break
    done
    if [[ -z "${request}" ]]; then
        sleep 0.05
        continue
    fi

    response="${bridge_real}/response.${request##*/request.}"
    response="${response%.json}"
    values_file="$(mktemp "${bridge_real}/values.XXXXXX")"
    chmod 600 -- "${values_file}"
    if ! python3 - "${request}" "${bridge_real}" "${diagnostics_real}" >"${values_file}" <<'PY'
import json
import pathlib
import sys

request = pathlib.Path(sys.argv[1])
bridge = pathlib.Path(sys.argv[2]).resolve()
diagnostics = sys.argv[3]
try:
    payload = json.loads(request.read_text(encoding="utf-8"))
except (OSError, ValueError) as error:
    raise SystemExit(f"invalid browser bridge request: {error}")
required = {
    "fixture", "database_url", "rpc_endpoint", "rpc_secret", "oidc_issuer",
    "oidc_client_id", "oidc_client_secret", "web_port", "phase",
}
if set(payload) != required or any(
    not isinstance(payload[key], str) or not payload[key] for key in required
):
    raise SystemExit("browser bridge request fields are invalid")
fixture = pathlib.Path(payload["fixture"])
try:
    fixture.resolve().relative_to(bridge)
except ValueError:
    raise SystemExit("browser fixture escapes bridge directory")
if fixture.is_symlink() or not fixture.is_file():
    raise SystemExit("browser fixture is not a regular file")
if diagnostics:
    diag = pathlib.Path(diagnostics)
    if not diag.is_absolute() or diag.is_symlink() or not diag.is_dir():
        raise SystemExit("browser diagnostics directory is invalid")
if payload["phase"] not in {"initial", "post-operation"}:
    raise SystemExit("browser phase is invalid")
if not payload["web_port"].isdigit() or not 1 <= int(payload["web_port"]) <= 65535:
    raise SystemExit("browser web port is invalid")
for key in ("database_url", "rpc_endpoint", "oidc_issuer"):
    if any(character in payload[key] for character in "\r\n"):
        raise SystemExit("browser endpoint contains a newline")
if len(payload["rpc_secret"]) < 32:
    raise SystemExit("browser RPC mediator secret is too short")
if any("\x00" in payload[key] for key in required):
    raise SystemExit("browser bridge values cannot contain NUL")
for key in required:
    print(f"{key}\0{payload[key]}", end="\0")
PY
    then
        rm -f -- "${request}" "${values_file}"
        response_tmp="${response}.tmp.$$"
        printf '1' >"${response_tmp}"
        chmod 600 -- "${response_tmp}"
        mv -- "${response_tmp}" "${response}"
        continue
    fi

    declare -A request_values=()
    while IFS= read -r -d '' key && IFS= read -r -d '' value; do
        request_values["${key}"]="${value}"
    done <"${values_file}"
    rm -f -- "${request}" "${values_file}"

    remaining=$((deadline_epoch - $(date +%s)))
    if (( remaining < 1 )); then
        response_tmp="${response}.tmp.$$"
        printf '124' >"${response_tmp}"
        chmod 600 -- "${response_tmp}"
        mv -- "${response_tmp}" "${response}"
        continue
    fi
    set +e
    base_env=(
        "PATH=${PATH}"
        "HOME=${HOME:-/tmp}"
        "USER=${USER:-$(id -un)}"
        "LOGNAME=${LOGNAME:-${USER:-}}"
        "XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
        "TMPDIR=${TMPDIR:-/tmp}"
        "HEPHAESTUS_E2E_COOKING_FIXTURE=${request_values[fixture]}"
        "HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL=${request_values[database_url]}"
        "HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT=${request_values[rpc_endpoint]}"
        "HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET=${request_values[rpc_secret]}"
        "HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER=${request_values[oidc_issuer]}"
        "HEPHAESTUS_E2E_EXTERNAL_OIDC_CLIENT_ID=${request_values[oidc_client_id]}"
        "HEPHAESTUS_E2E_EXTERNAL_OIDC_CLIENT_SECRET=${request_values[oidc_client_secret]}"
        "HEPHAESTUS_E2E_EXTERNAL_WEB_PORT=${request_values[web_port]}"
        "HEPHAESTUS_E2E_COOKING_PHASE=${request_values[phase]}"
    )
    if [[ -n "${diagnostics_real}" ]]; then
        base_env+=("HEPHAESTUS_COOKING_DIAGNOSTICS_DIR=${diagnostics_real}")
    fi
    setsid timeout --kill-after=30s "${remaining}s" env -i "${base_env[@]}" \
        "${external_script}" >/dev/null 2>&1 &
    child_pid="$!"
    wait "${child_pid}"
    status="$?"
    child_pid=""
    set -e
    response_tmp="${response}.tmp.$$"
    printf '%s' "${status}" >"${response_tmp}"
    chmod 600 -- "${response_tmp}"
    mv -- "${response_tmp}" "${response}"
done
