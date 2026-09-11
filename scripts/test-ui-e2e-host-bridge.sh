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
diagnostics_root=""
diagnostic_watcher_pid=""
cleanup() {
    local status="$?"
    if [[ -n "${watcher_pid}" ]]; then
        kill "${watcher_pid}" >/dev/null 2>&1 || true
        wait "${watcher_pid}" 2>/dev/null || true
    fi
    if [[ -n "${diagnostic_watcher_pid}" ]]; then
        kill "${diagnostic_watcher_pid}" >/dev/null 2>&1 || true
        wait "${diagnostic_watcher_pid}" 2>/dev/null || true
    fi
    if [[ -n "${diagnostics_root}" ]]; then
        rm -rf -- "${diagnostics_root}"
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

# Exercise the failure path with diagnostics enabled.  The fake external
# child emits one safe line and two credential-shaped lines; the bridge must
# preserve status 1, expose the safe line, and suppress the sensitive lines.
diagnostics_root="$(mktemp -d /tmp/heph-browser-bridge-diagnostics.XXXXXX)"
fake_repo="${diagnostics_root}/repo"
fake_scripts="${fake_repo}/scripts"
diagnostics_bridge="${diagnostics_root}/bridge"
diagnostics_dir="${diagnostics_root}/evidence"
mkdir -p -- "${fake_scripts}" "${diagnostics_bridge}" "${diagnostics_dir}"
cp -- "${script_dir}/run-ui-e2e-host-bridge.sh" "${fake_scripts}/run-ui-e2e-host-bridge.sh"
cp -- "${script_dir}/check-browser-evidence.py" "${fake_scripts}/check-browser-evidence.py"
cat >"${fake_scripts}/run-ui-e2e-external.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' 'safe browser failure diagnostic'
printf '%s\n' 'Authorization: Bearer definitely-secret'
printf '%s\n' 'password=definitely-password'
exit 1
SH
chmod 700 -- "${fake_scripts}/run-ui-e2e-host-bridge.sh" "${fake_scripts}/run-ui-e2e-external.sh"
printf '{}\n' >"${diagnostics_bridge}/fixture.json"
chmod 700 -- "${diagnostics_bridge}" "${diagnostics_dir}"
chmod 600 -- "${diagnostics_bridge}/fixture.json"
diagnostics_deadline="$(( $(date +%s) + 30 ))"
diagnostics_host_log="${diagnostics_root}/host.log"
diagnostics_client_log="${diagnostics_root}/client.log"
"${fake_scripts}/run-ui-e2e-host-bridge.sh" \
    "${diagnostics_bridge}" "${diagnostics_dir}" "${diagnostics_deadline}" \
    >"${diagnostics_host_log}" 2>&1 &
diagnostic_watcher_pid="$!"
for _attempt in {1..100}; do
    [[ -f "${diagnostics_bridge}/.ready" ]] && break
    sleep 0.05
done
[[ -f "${diagnostics_bridge}/.ready" ]] || {
    printf 'diagnostic bridge did not become ready\n' >&2
    exit 1
}
set +e
HEPHAESTUS_COOKING_BROWSER_BRIDGE_DIR="${diagnostics_bridge}" \
HEPHAESTUS_COOKING_BRIDGE_DEADLINE_EPOCH="${diagnostics_deadline}" \
HEPHAESTUS_E2E_COOKING_FIXTURE="${diagnostics_bridge}/fixture.json" \
HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL='postgres://bridge-smoke.invalid/test' \
HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT='http://127.0.0.1:1' \
HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET='bridge-smoke-secret-with-sufficient-entropy' \
HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER='http://127.0.0.1:1' \
HEPHAESTUS_E2E_EXTERNAL_WEB_PORT=4000 \
HEPHAESTUS_E2E_COOKING_PHASE=post-operation \
    "${script_dir}/run-ui-e2e-external.sh" >"${diagnostics_client_log}" 2>&1
diagnostics_client_status="$?"
set -e
kill "${diagnostic_watcher_pid}" >/dev/null 2>&1 || true
wait "${diagnostic_watcher_pid}" 2>/dev/null || true
diagnostic_watcher_pid=""
[[ "${diagnostics_client_status}" -eq 1 ]] || {
    printf 'diagnostic bridge returned %s, expected 1\n' "${diagnostics_client_status}" >&2
    exit 1
}
grep -q 'HEPH_BROWSER_BRIDGE event=external-failure status=1 diagnostics-scan=0' \
    "${diagnostics_host_log}"
grep -q 'safe browser failure diagnostic' "${diagnostics_host_log}"
if grep -q 'definitely-secret\|definitely-password\|Authorization: Bearer' \
    "${diagnostics_host_log}"; then
    printf 'diagnostic bridge leaked credential-shaped output\n' >&2
    exit 1
fi
[[ ! -s "${diagnostics_client_log}" ]]
printf 'host browser bridge failure diagnostics smoke passed\n'

# The fail-closed branch must suppress the entire excerpt when the stream
# scanner sees a known fixture credential, while preserving the child status.
scanner_bridge="${diagnostics_root}/scanner-bridge"
scanner_dir="${diagnostics_root}/scanner-evidence"
mkdir -p -- "${scanner_bridge}" "${scanner_dir}"
printf '{}\n' >"${scanner_bridge}/fixture.json"
chmod 700 -- "${scanner_bridge}" "${scanner_dir}"
chmod 600 -- "${scanner_bridge}/fixture.json"
cat >"${fake_scripts}/run-ui-e2e-external.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' 'safe line must be withheld after scanner failure'
printf '%s\n' 'cooking-inbound-only-fixture-sentinel'
exit 1
SH
chmod 700 -- "${fake_scripts}/run-ui-e2e-external.sh"
scanner_deadline="$(( $(date +%s) + 30 ))"
scanner_host_log="${diagnostics_root}/scanner-host.log"
scanner_client_log="${diagnostics_root}/scanner-client.log"
"${fake_scripts}/run-ui-e2e-host-bridge.sh" \
    "${scanner_bridge}" "${scanner_dir}" "${scanner_deadline}" \
    >"${scanner_host_log}" 2>&1 &
diagnostic_watcher_pid="$!"
for _attempt in {1..100}; do
    [[ -f "${scanner_bridge}/.ready" ]] && break
    sleep 0.05
done
[[ -f "${scanner_bridge}/.ready" ]] || {
    printf 'scanner bridge did not become ready\n' >&2
    exit 1
}
set +e
HEPHAESTUS_COOKING_BROWSER_BRIDGE_DIR="${scanner_bridge}" \
HEPHAESTUS_COOKING_BRIDGE_DEADLINE_EPOCH="${scanner_deadline}" \
HEPHAESTUS_E2E_COOKING_FIXTURE="${scanner_bridge}/fixture.json" \
HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL='postgres://bridge-smoke.invalid/test' \
HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT='http://127.0.0.1:1' \
HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET='bridge-smoke-secret-with-sufficient-entropy' \
HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER='http://127.0.0.1:1' \
HEPHAESTUS_E2E_EXTERNAL_WEB_PORT=4000 \
HEPHAESTUS_E2E_COOKING_PHASE=post-operation \
    "${script_dir}/run-ui-e2e-external.sh" >"${scanner_client_log}" 2>&1
scanner_client_status="$?"
set -e
kill "${diagnostic_watcher_pid}" >/dev/null 2>&1 || true
wait "${diagnostic_watcher_pid}" 2>/dev/null || true
diagnostic_watcher_pid=""
[[ "${scanner_client_status}" -eq 1 ]] || {
    printf 'scanner bridge returned %s, expected 1\n' "${scanner_client_status}" >&2
    exit 1
}
grep -q 'HEPH_BROWSER_BRIDGE event=external-failure status=1 diagnostics-scan=1' \
    "${scanner_host_log}"
grep -q 'HEPH_BROWSER_BRIDGE diagnostics-withheld credential-scan-failed' \
    "${scanner_host_log}"
if grep -q 'safe line must be withheld\|cooking-inbound-only-fixture-sentinel' \
    "${scanner_host_log}"; then
    printf 'scanner bridge emitted output after credential scan failure\n' >&2
    exit 1
fi
[[ ! -s "${scanner_client_log}" ]]
printf 'host browser bridge fail-closed diagnostics smoke passed\n'
