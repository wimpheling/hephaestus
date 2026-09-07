#!/usr/bin/env bash
# Smoke-test the mapped-namespace browser request bridge without KVM or a web
# service.  The fixed harness validates the deliberately incomplete fixture
# and returns status 1 after the host watcher dispatches it.
set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
bridge_dir="$(mktemp -d /tmp/heph-browser-bridge-smoke.XXXXXX)"
host_log="$(mktemp /tmp/heph-browser-bridge-smoke-host.XXXXXX.log)"
client_log="$(mktemp /tmp/heph-browser-bridge-smoke-client.XXXXXX.log)"
watcher_pid=""
cleanup() {
    local status="$?"
    if [[ -n "${watcher_pid}" ]]; then
        kill "${watcher_pid}" >/dev/null 2>&1 || true
        wait "${watcher_pid}" 2>/dev/null || true
    fi
    rm -rf -- "${bridge_dir}" "${host_log}" "${client_log}"
    exit "${status}"
}
trap cleanup EXIT INT TERM

printf '{}\n' >"${bridge_dir}/fixture.json"
chmod 700 -- "${bridge_dir}"
chmod 600 -- "${bridge_dir}/fixture.json"
deadline="$(( $(date +%s) + 30 ))"
"${script_dir}/run-ui-e2e-host-bridge.sh" "${bridge_dir}" "" "${deadline}" \
    >"${host_log}" 2>&1 &
watcher_pid="$!"

set +e
unshare --map-user 10001 --map-group 10001 env \
    HEPHAESTUS_COOKING_BROWSER_BRIDGE_DIR="${bridge_dir}" \
    HEPHAESTUS_COOKING_BRIDGE_DEADLINE_EPOCH="${deadline}" \
    HEPHAESTUS_E2E_COOKING_FIXTURE="${bridge_dir}/fixture.json" \
    HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL='postgres://bridge-smoke.invalid/test' \
    HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT='http://127.0.0.1:1' \
    HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET='bridge-smoke-secret-with-sufficient-entropy' \
    HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER='http://127.0.0.1:1' \
    HEPHAESTUS_E2E_EXTERNAL_WEB_PORT=4000 \
    HEPHAESTUS_E2E_COOKING_PHASE=post-operation \
    "${script_dir}/run-ui-e2e-external.sh" >"${client_log}" 2>&1
client_status="$?"
set -e
[[ "${client_status}" -eq 1 ]] || {
    printf 'expected fixed harness validation status 1, got %s\n' "${client_status}" >&2
    exit 1
}
[[ ! -s "${client_log}" && ! -s "${host_log}" ]] || {
    printf 'bridge smoke emitted unexpected output\n' >&2
    exit 1
}
if compgen -G "${bridge_dir}/request.*" >/dev/null || compgen -G "${bridge_dir}/response.*" >/dev/null; then
    printf 'bridge request artifacts were not consumed\n' >&2
    exit 1
fi
printf 'host browser bridge smoke passed\n'
