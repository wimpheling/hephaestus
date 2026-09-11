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
[[ "${PLAYWRIGHT_BROWSERS_PATH:-}" == "/tmp/heph-playwright-browsers" ]] || {
    printf '%s\n' 'browser path was not forwarded'
    exit 9
}
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
PLAYWRIGHT_BROWSERS_PATH=/tmp/heph-playwright-browsers \
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

# Exercise an early web readiness failure.  The external harness must retain
# the child web/service logs and the bridge must expose bounded, scanned
# excerpts alongside curl's actionable exit reason.
retained_bridge="${diagnostics_root}/retained-bridge"
retained_dir="${diagnostics_root}/retained-evidence"
mkdir -p -- "${retained_bridge}" "${retained_dir}"
printf '{}\n' >"${retained_bridge}/fixture.json"
chmod 700 -- "${retained_bridge}" "${retained_dir}"
chmod 600 -- "${retained_bridge}/fixture.json"
cat >"${fake_scripts}/run-ui-e2e-external.sh" <<'SH'
#!/usr/bin/env bash
set -Eeuo pipefail
failure_root="${HEPHAESTUS_COOKING_DIAGNOSTICS_DIR}/browser.synthetic"
mkdir -p -- "${failure_root}"
printf '%s\n' 'synthetic web startup failure' >"${failure_root}/web.log"
printf '%s\n' 'synthetic service bind failure' >"${failure_root}/web-service.log"
printf '%s\n' 'playwright was not started' >"${failure_root}/playwright.log"
curl --fail --silent --show-error http://127.0.0.1:1/ >/dev/null
SH
chmod 700 -- "${fake_scripts}/run-ui-e2e-external.sh"

retained_deadline="$(( $(date +%s) + 30 ))"
retained_host_log="${diagnostics_root}/retained-host.log"
retained_client_log="${diagnostics_root}/retained-client.log"
"${fake_scripts}/run-ui-e2e-host-bridge.sh" "${retained_bridge}" \
    "${retained_dir}" "${retained_deadline}" >"${retained_host_log}" 2>&1 &
diagnostic_watcher_pid="$!"
for _attempt in {1..100}; do
    [[ -f "${retained_bridge}/.ready" ]] && break
    sleep 0.05
done
[[ -f "${retained_bridge}/.ready" ]] || {
    printf 'retained-log bridge did not become ready\n' >&2
    exit 1
}
set +e
env \
    HEPHAESTUS_COOKING_BROWSER_BRIDGE_DIR="${retained_bridge}" \
    HEPHAESTUS_COOKING_BRIDGE_DEADLINE_EPOCH="${retained_deadline}" \
    HEPHAESTUS_E2E_COOKING_FIXTURE="${retained_bridge}/fixture.json" \
    HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL='postgres://bridge-smoke.invalid/test' \
    HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT='http://127.0.0.1:1' \
    HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET='bridge-smoke-secret-with-sufficient-entropy' \
    HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER='http://127.0.0.1:1' \
    HEPHAESTUS_E2E_EXTERNAL_WEB_PORT=4000 \
    HEPHAESTUS_E2E_COOKING_PHASE=initial \
    "${script_dir}/run-ui-e2e-external.sh" >"${retained_client_log}" 2>&1
retained_client_status="$?"
set -e
kill "${diagnostic_watcher_pid}" >/dev/null 2>&1 || true
wait "${diagnostic_watcher_pid}" 2>/dev/null || true
diagnostic_watcher_pid=""
[[ "${retained_client_status}" -eq 7 ]] || {
    printf 'retained-log bridge returned %s, expected 7\n' "${retained_client_status}" >&2
    exit 1
}
grep -q 'HEPH_BROWSER_BRIDGE event=external-failure status=7 diagnostics-scan=0' \
    "${retained_host_log}"
grep -q 'curl: (7)' "${retained_host_log}"
grep -q 'synthetic web startup failure' "${retained_host_log}"
grep -q 'synthetic service bind failure' "${retained_host_log}"
grep -q 'playwright was not started' "${retained_host_log}"
[[ ! -s "${retained_client_log}" ]]
printf 'host browser bridge retained-log diagnostics smoke passed\n'

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

# Exercise the external harness readiness contract with command mocks: a
# delayed service becomes ready, a dead service fails immediately, and a
# live-but-never-ready service obeys the propagated absolute deadline.
readiness_root="${diagnostics_root}/readiness"
readiness_repo="${readiness_root}/repo"
readiness_bin="${readiness_root}/bin"
readiness_diag="${readiness_root}/evidence"
mkdir -p -- "${readiness_repo}/scripts" "${readiness_repo}/e2e/playwright" \
    "${readiness_bin}" "${readiness_diag}"
cp -- "${script_dir}/run-ui-e2e-external.sh" "${readiness_repo}/scripts/"
cp -- "${script_dir}/check-browser-evidence.py" "${readiness_repo}/scripts/"
cat >"${readiness_root}/fixture.json" <<'JSON'
{"project_id":"project","release_id":"release","release_agent_id":"agent","instance_id":"instance","mailbox_id":"mailbox","gateway_id":"gateway"}
JSON
chmod 700 -- "${readiness_root}" "${readiness_repo}" "${readiness_repo}/scripts" \
    "${readiness_bin}" "${readiness_diag}"
chmod 600 -- "${readiness_root}/fixture.json"
cat >"${readiness_bin}/podman" <<'SH'
#!/usr/bin/env bash
set -Eeuo pipefail
case "${1:-}" in
    inspect) printf '%s 0\n' "${MOCK_CONTAINER_STATE:-running}" ;;
    logs) printf '%s\n' 'mock web service log' ;;
    run|stop|rm) : ;;
    *) : ;;
esac
SH
cat >"${readiness_bin}/curl" <<'SH'
#!/usr/bin/env bash
set -Eeuo pipefail
count_file="${MOCK_CURL_COUNT_FILE:?missing mock curl count file}"
printf '%s\n' "$*" >>"${MOCK_CURL_ARG_LOG:?missing mock curl argument log}"
count="$(<"${count_file}")"
count=$((count + 1))
printf '%s\n' "${count}" >"${count_file}"
if [[ "${MOCK_CURL_MODE:-fail}" == delayed && "${count}" -ge 4 ]]; then
    exit 0
fi
printf '%s\n' 'curl: (7) mock connection refused' >&2
exit 7
SH
cat >"${readiness_bin}/npm" <<'SH'
#!/usr/bin/env bash
exit 0
SH
cat >"${readiness_bin}/npx" <<'SH'
#!/usr/bin/env bash
exit 0
SH
chmod 700 -- "${readiness_bin}/podman" "${readiness_bin}/curl" \
    "${readiness_bin}/npm" "${readiness_bin}/npx"
readiness_external="${readiness_repo}/scripts/run-ui-e2e-external.sh"
run_readiness_case() {
    local name="$1" mode="$2" container_state="$3" deadline="$4" expected="$5"
    local count_file="${readiness_root}/${name}.count" output_file="${readiness_root}/${name}.log"
    local arg_file="${readiness_root}/${name}.args"
    printf '0\n' >"${count_file}"
    : >"${arg_file}"
    set +e
    PATH="${readiness_bin}:${PATH}" \
    MOCK_CURL_MODE="${mode}" MOCK_CONTAINER_STATE="${container_state}" \
    MOCK_CURL_COUNT_FILE="${count_file}" MOCK_CURL_ARG_LOG="${arg_file}" \
    HEPHAESTUS_E2E_COOKING_FIXTURE="${readiness_root}/fixture.json" \
    HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL='postgres://bridge-smoke.invalid/test' \
    HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT='http://127.0.0.1:1' \
    HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET='bridge-smoke-secret-with-sufficient-entropy' \
    HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER='http://127.0.0.1:1' \
    HEPHAESTUS_E2E_EXTERNAL_WEB_PORT=4000 \
    HEPHAESTUS_E2E_COOKING_PHASE=initial \
    HEPHAESTUS_COOKING_DIAGNOSTICS_DIR="${readiness_diag}" \
    HEPHAESTUS_COOKING_BROWSER_DEADLINE_EPOCH="${deadline}" \
        "${readiness_external}" >"${output_file}" 2>&1
    local actual="$?"
    set -e
    [[ "${actual}" -eq "${expected}" ]] || {
        printf '%s readiness returned %s, expected %s\n' "${name}" "${actual}" "${expected}" >&2
        return 1
    }
}
run_readiness_case delayed delayed running "$(( $(date +%s) + 10 ))" 0
[[ "$(<"${readiness_root}/delayed.count")" -eq 4 ]]
grep -q -- '--connect-timeout 2 --max-time 2' "${readiness_root}/delayed.args"
run_readiness_case dead fail exited "$(( $(date +%s) + 10 ))" 1
grep -q 'browser web container stopped state=exited 0 status=7' "${readiness_root}/dead.log"
run_readiness_case timeout fail running "$(( $(date +%s) + 1 ))" 124
grep -q 'browser web readiness deadline elapsed' "${readiness_root}/timeout.log"
grep -q -- '--connect-timeout 1 --max-time 1' "${readiness_root}/timeout.args"
printf 'external browser readiness deadline/liveness smoke passed\n'
