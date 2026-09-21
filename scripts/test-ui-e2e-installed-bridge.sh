#!/usr/bin/env bash
# Protocol-only bridge coverage for legacy and installed UI runner selection.
# It uses fake child runners and never starts a browser, web service, Cargo, or VM.
set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
root="$(mktemp -d /tmp/heph-installed-bridge-test.XXXXXX)"
bridge="${root}/bridge"
fake_repo="${root}/fake-repo"
fixture="${bridge}/fixture.json"
recovery_fixture="${bridge}/recovery.json"
concurrency_fixture="${bridge}/concurrency.json"
host_pid=""
cleanup() {
    local status="$?"
    if [[ -n "${host_pid}" ]]; then
        kill "${host_pid}" >/dev/null 2>&1 || true
        wait "${host_pid}" 2>/dev/null || true
    fi
    rm -rf -- "${root}"
    exit "${status}"
}
trap cleanup EXIT INT TERM

mkdir -p -- "${bridge}" "${fake_repo}/scripts"
chmod 700 -- "${root}" "${bridge}" "${fake_repo}" "${fake_repo}/scripts"
printf '{}\n' >"${fixture}"
printf '%s\n' '{"recovery":"fixture","session_chat_ui":{"project_id":"project","repository_id":"repository","installation_id":"installation","generation_id":"generation","actor_id":"actor","ui_path":"/session-chat/index.html","agent_response_text":"response","initial_transcript_count":"4","initial_agent_count":"2"}}' >"${recovery_fixture}"
printf '%s\n' '{"concurrency":"fixture","session_chat_concurrent":{"project_id":"project","repository_id":"repository","installation_id":"installation","generation_id":"generation","actor_id":"actor","ui_path":"/session-chat/index.html","agent_response_text":"response","initial_transcript_count":"6","initial_agent_count":"3"}}' >"${concurrency_fixture}"
mkdir -m 700 -- "${bridge}/installed-ui-control"
chmod 600 -- "${fixture}"
cp -- "${script_dir}/run-ui-e2e-host-bridge.sh" "${fake_repo}/scripts/"
cat >"${fake_repo}/scripts/run-ui-e2e-external.sh" <<'CHILD'
#!/usr/bin/env bash
set -Eeuo pipefail
[[ -z "${HEPHAESTUS_E2E_BROWSER_RUNNER:-}" ]]
exit 0
CHILD
cat >"${fake_repo}/scripts/run-installed-ui-e2e.sh" <<'CHILD'
#!/usr/bin/env bash
set -Eeuo pipefail
[[ "${HEPHAESTUS_E2E_BROWSER_RUNNER:-}" == installed-ui ]]
[[ "${HEPHAESTUS_PLATFORM_HTTPS_ORIGIN:-}" == https://platform.localhost:4443 ]]
[[ "${HEPHAESTUS_UI_NAMESPACE:-}" == ui.platform.localhost ]]
[[ "${HEPHAESTUS_UI_PORT:-}" == 4443 ]]
[[ -f "${HEPHAESTUS_CADDY_TEST_CA_CERT:-}" && ! -L "${HEPHAESTUS_CADDY_TEST_CA_CERT:-}" ]]
[[ "${HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT:-}" == http://bridge-test.invalid/rpc ]]
case "${HEPHAESTUS_INSTALLED_UI_BROWSER_GREP:-}" in
    "cooking installed UI TLS")
        [[ "${HEPHAESTUS_E2E_COOKING_PHASE:-}" == initial ]]
        ;;
    "cooking new session chat creates and opens a real Git-backed browser session")
        [[ "${HEPHAESTUS_E2E_COOKING_PHASE:-}" == initial ]]
        ;;
    "cooking session-chat installed UI initializes and reconnects ordinary Git history")
        [[ "${HEPHAESTUS_E2E_COOKING_PHASE:-}" == recovery ]]
        ;;
    "cooking concurrent session chat clients reconcile a stale Git push and preserve both turns")
        [[ "${HEPHAESTUS_E2E_COOKING_PHASE:-}" == concurrency ]]
        ;;
    *) exit 1 ;;
esac
case "${HEPHAESTUS_E2E_COOKING_PHASE:-}" in
    initial)
        [[ "${HEPHAESTUS_E2E_COOKING_FIXTURE##*/}" == fixture.json ]]
        ;;
    recovery)
        [[ "${HEPHAESTUS_E2E_COOKING_FIXTURE##*/}" == recovery.json ]]
        grep -q '"initial_transcript_count":"4"' "${HEPHAESTUS_E2E_COOKING_FIXTURE}"
        grep -q '"initial_agent_count":"2"' "${HEPHAESTUS_E2E_COOKING_FIXTURE}"
        ;;
    concurrency)
        [[ "${HEPHAESTUS_E2E_COOKING_FIXTURE##*/}" == concurrency.json ]]
        grep -q '"initial_transcript_count":"6"' "${HEPHAESTUS_E2E_COOKING_FIXTURE}"
        grep -q '"initial_agent_count":"3"' "${HEPHAESTUS_E2E_COOKING_FIXTURE}"
        ;;
    *) exit 1 ;;
esac
exit 23
CHILD
chmod 700 "${fake_repo}/scripts/"*.sh
printf '%s\n' '-----BEGIN CERTIFICATE-----' 'bridge-test' '-----END CERTIFICATE-----' >"${root}/source-ca.pem"
chmod 600 -- "${root}/source-ca.pem"

deadline="$(( $(date +%s) + 30 ))"
"${fake_repo}/scripts/run-ui-e2e-host-bridge.sh" "${bridge}" "" "${deadline}" &
host_pid="$!"
for _attempt in {1..100}; do
    [[ -f "${bridge}/.ready" ]] && break
    sleep 0.05
done
[[ -f "${bridge}/.ready" ]]

common_env=(
    HEPHAESTUS_COOKING_BROWSER_BRIDGE_DIR="${bridge}"
    HEPHAESTUS_COOKING_BRIDGE_DEADLINE_EPOCH="${deadline}"
    HEPHAESTUS_E2E_COOKING_FIXTURE="${fixture}"
    HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL='postgres://bridge-test.invalid/test'
    HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT='http://bridge-test.invalid/rpc'
    HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET='bridge-test-secret-with-sufficient-entropy'
    HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER='http://bridge-test.invalid/oidc'
    HEPHAESTUS_E2E_EXTERNAL_WEB_PORT=4000
)

env "${common_env[@]}" HEPHAESTUS_E2E_COOKING_PHASE=initial "${script_dir}/run-ui-e2e-external.sh"

env "${common_env[@]}" HEPHAESTUS_E2E_COOKING_PHASE=post-operation "${script_dir}/run-ui-e2e-external.sh"

installed_status=0
env "${common_env[@]}" \
    HEPHAESTUS_E2E_COOKING_PHASE=initial \
    HEPHAESTUS_E2E_BROWSER_RUNNER=installed-ui \
    HEPHAESTUS_PLATFORM_HTTPS_ORIGIN='https://platform.localhost:4443' \
    HEPHAESTUS_UI_NAMESPACE='ui.platform.localhost' \
    HEPHAESTUS_UI_PORT=4443 \
    HEPHAESTUS_CADDY_TEST_CA_CERT="${root}/source-ca.pem" \
    "${script_dir}/run-ui-e2e-external.sh" || installed_status="$?"
[[ "${installed_status}" == 23 ]]
! compgen -G "${bridge}/ca.*.pem" >/dev/null

recovery_env=()
for value in "${common_env[@]}"; do
    [[ "${value}" == HEPHAESTUS_E2E_COOKING_FIXTURE=* ]] || recovery_env+=("${value}")
done
recovery_status=0
env "${recovery_env[@]}" \
    HEPHAESTUS_E2E_COOKING_FIXTURE="${recovery_fixture}" \
    HEPHAESTUS_E2E_COOKING_PHASE=recovery \
    HEPHAESTUS_E2E_BROWSER_RUNNER=installed-ui \
    HEPHAESTUS_INSTALLED_UI_BROWSER_GREP='cooking session-chat installed UI initializes and reconnects ordinary Git history' \
    HEPHAESTUS_PLATFORM_HTTPS_ORIGIN='https://platform.localhost:4443' \
    HEPHAESTUS_UI_NAMESPACE='ui.platform.localhost' \
    HEPHAESTUS_UI_PORT=4443 \
    HEPHAESTUS_CADDY_TEST_CA_CERT="${root}/source-ca.pem" \
    "${script_dir}/run-ui-e2e-external.sh" || recovery_status="$?"
[[ "${recovery_status}" == 23 ]]
! compgen -G "${bridge}/ca.*.pem" >/dev/null

concurrency_env=()
for value in "${common_env[@]}"; do
    [[ "${value}" == HEPHAESTUS_E2E_COOKING_FIXTURE=* ]] || concurrency_env+=("${value}")
done
concurrency_status=0
env "${concurrency_env[@]}" \
    HEPHAESTUS_E2E_COOKING_FIXTURE="${concurrency_fixture}" \
    HEPHAESTUS_E2E_COOKING_PHASE=concurrency \
    HEPHAESTUS_E2E_BROWSER_RUNNER=installed-ui \
    HEPHAESTUS_INSTALLED_UI_BROWSER_GREP='cooking concurrent session chat clients reconcile a stale Git push and preserve both turns' \
    HEPHAESTUS_PLATFORM_HTTPS_ORIGIN='https://platform.localhost:4443' \
    HEPHAESTUS_UI_NAMESPACE='ui.platform.localhost' \
    HEPHAESTUS_UI_PORT=4443 \
    HEPHAESTUS_CADDY_TEST_CA_CERT="${root}/source-ca.pem" \
    "${script_dir}/run-ui-e2e-external.sh" || concurrency_status="$?"
[[ "${concurrency_status}" == 23 ]]
! compgen -G "${bridge}/ca.*.pem" >/dev/null

rm -rf -- "${bridge}/installed-ui-control"
missing_control_status=0
env "${common_env[@]}" \
    HEPHAESTUS_E2E_COOKING_PHASE=initial \
    HEPHAESTUS_E2E_BROWSER_RUNNER=installed-ui \
    HEPHAESTUS_PLATFORM_HTTPS_ORIGIN='https://platform.localhost:4443' \
    HEPHAESTUS_UI_NAMESPACE='ui.platform.localhost' \
    HEPHAESTUS_UI_PORT=4443 \
    HEPHAESTUS_CADDY_TEST_CA_CERT="${root}/source-ca.pem" \
    "${script_dir}/run-ui-e2e-external.sh" >/dev/null 2>&1 || missing_control_status="$?"
[[ "${missing_control_status}" == 1 ]]
mkdir -m 700 -- "${bridge}/installed-ui-control"

rm -rf -- "${bridge}/installed-ui-control"
ln -s -- "${root}/source-ca.pem" "${bridge}/installed-ui-control"
symlink_control_status=0
env "${common_env[@]}" \
    HEPHAESTUS_E2E_COOKING_PHASE=initial \
    HEPHAESTUS_E2E_BROWSER_RUNNER=installed-ui \
    HEPHAESTUS_PLATFORM_HTTPS_ORIGIN='https://platform.localhost:4443' \
    HEPHAESTUS_UI_NAMESPACE='ui.platform.localhost' \
    HEPHAESTUS_UI_PORT=4443 \
    HEPHAESTUS_CADDY_TEST_CA_CERT="${root}/source-ca.pem" \
    "${script_dir}/run-ui-e2e-external.sh" >/dev/null 2>&1 || symlink_control_status="$?"
[[ "${symlink_control_status}" == 1 ]]
rm -f -- "${bridge}/installed-ui-control"
mkdir -m 700 -- "${bridge}/installed-ui-control"

chmod 0755 -- "${bridge}/installed-ui-control"
control_mode_status=0
env "${common_env[@]}" \
    HEPHAESTUS_E2E_COOKING_PHASE=initial \
    HEPHAESTUS_E2E_BROWSER_RUNNER=installed-ui \
    HEPHAESTUS_PLATFORM_HTTPS_ORIGIN='https://platform.localhost:4443' \
    HEPHAESTUS_UI_NAMESPACE='ui.platform.localhost' \
    HEPHAESTUS_UI_PORT=4443 \
    HEPHAESTUS_CADDY_TEST_CA_CERT="${root}/source-ca.pem" \
    "${script_dir}/run-ui-e2e-external.sh" >/dev/null 2>&1 || control_mode_status="$?"
[[ "${control_mode_status}" == 1 ]]
chmod 0700 -- "${bridge}/installed-ui-control"

outside_control="${root}/outside-control"
mkdir -m 700 -- "${outside_control}"
outside_control_status=0
env "${common_env[@]}" \
    HEPHAESTUS_E2E_COOKING_PHASE=initial \
    HEPHAESTUS_E2E_BROWSER_RUNNER=installed-ui \
    HEPHAESTUS_PLATFORM_HTTPS_ORIGIN='https://platform.localhost:4443' \
    HEPHAESTUS_UI_NAMESPACE='ui.platform.localhost' \
    HEPHAESTUS_UI_PORT=4443 \
    HEPHAESTUS_CADDY_TEST_CA_CERT="${root}/source-ca.pem" \
    HEPHAESTUS_INSTALLED_UI_CONTROL_DIR="${outside_control}" \
    "${script_dir}/run-installed-ui-e2e.sh" >/dev/null 2>&1 || outside_control_status="$?"
[[ "${outside_control_status}" == 1 ]]

client_mismatch_status=0
env "${common_env[@]}" \
    HEPHAESTUS_E2E_COOKING_PHASE=initial \
    HEPHAESTUS_E2E_BROWSER_RUNNER=installed-ui \
    HEPHAESTUS_PLATFORM_HTTPS_ORIGIN='https://platform.localhost:4443' \
    HEPHAESTUS_UI_NAMESPACE='ui.other.localhost' \
    HEPHAESTUS_UI_PORT=4443 \
    HEPHAESTUS_CADDY_TEST_CA_CERT="${root}/source-ca.pem" \
    "${script_dir}/run-ui-e2e-external.sh" >/dev/null 2>&1 || client_mismatch_status="$?"
[[ "${client_mismatch_status}" == 1 ]]
! compgen -G "${bridge}/ca.*.pem" >/dev/null

bad_request="${bridge}/request.invalid.json"
FIXTURE="${fixture}" python3 - "${bad_request}" <<'PY'
import json
import os
import sys

payload = {
    "runner": "installed-ui",
    "fixture": os.environ["FIXTURE"],
    "database_url": "postgres://bridge-test.invalid/test",
    "rpc_endpoint": "http://bridge-test.invalid/rpc",
    "rpc_secret": "bridge-test-secret-with-sufficient-entropy",
    "oidc_issuer": "http://bridge-test.invalid/oidc",
    "oidc_client_id": "hephaestus-web",
    "oidc_client_secret": "bridge-test-client-secret",
    "web_port": "4000",
    "phase": "initial",
    "platform_origin": "https://platform.localhost:4443",
    "ui_namespace": "ui.platform.localhost",
    "ui_port": "4443",
    "ca_cert": "../escape.pem",
}
with open(sys.argv[1], "x", encoding="utf-8") as stream:
    json.dump(payload, stream, separators=(",", ":"))
PY
response="${bridge}/response.invalid"
for _attempt in {1..100}; do
    [[ -f "${response}" ]] && break
    sleep 0.05
done
[[ "$(cat "${response}")" == 1 ]]

mismatched_phase_request="${bridge}/request.mismatched-phase.json"
printf '%s\n' '-----BEGIN CERTIFICATE-----' 'bridge-test' '-----END CERTIFICATE-----' >"${bridge}/ca.post.pem"
chmod 600 -- "${bridge}/ca.post.pem"
FIXTURE="${fixture}" python3 - "${mismatched_phase_request}" <<'PY'
import json
import os
import sys

payload = {
    "runner": "installed-ui",
    "fixture": os.environ["FIXTURE"],
    "database_url": "postgres://bridge-test.invalid/test",
    "rpc_endpoint": "http://bridge-test.invalid/rpc",
    "rpc_secret": "bridge-test-secret-with-sufficient-entropy",
    "oidc_issuer": "http://bridge-test.invalid/oidc",
    "oidc_client_id": "hephaestus-web",
    "oidc_client_secret": "bridge-test-client-secret",
    "web_port": "4000",
    "phase": "post-operation",
    "platform_origin": "https://platform.localhost:4443",
    "ui_namespace": "ui.platform.localhost",
    "ui_port": "4443",
    "ca_cert": "ca.post.pem",
    "installed_ui_browser_grep": "cooking installed UI TLS",
}
with open(sys.argv[1], "x", encoding="utf-8") as stream:
    json.dump(payload, stream, separators=(",", ":"))
PY
mismatched_phase_response="${bridge}/response.mismatched-phase"
for _attempt in {1..100}; do
    [[ -f "${mismatched_phase_response}" ]] && break
    sleep 0.05
done
[[ "$(cat "${mismatched_phase_response}")" == 1 ]]

printf '%s\n' '-----BEGIN CERTIFICATE-----' 'bridge-test' '-----END CERTIFICATE-----' >"${bridge}/ca.selector.pem"
chmod 600 -- "${bridge}/ca.selector.pem"
invalid_selector_request="${bridge}/request.invalid-selector.json"
FIXTURE="${fixture}" python3 - "${invalid_selector_request}" <<'PY'
import json
import os
import sys

payload = {
    "runner": "installed-ui",
    "fixture": os.environ["FIXTURE"],
    "database_url": "postgres://bridge-test.invalid/test",
    "rpc_endpoint": "http://bridge-test.invalid/rpc",
    "rpc_secret": "bridge-test-secret-with-sufficient-entropy",
    "oidc_issuer": "http://bridge-test.invalid/oidc",
    "oidc_client_id": "hephaestus-web",
    "oidc_client_secret": "bridge-test-client-secret",
    "web_port": "4000",
    "phase": "initial",
    "platform_origin": "https://platform.localhost:4443",
    "ui_namespace": "ui.platform.localhost",
    "ui_port": "4443",
    "ca_cert": "ca.selector.pem",
    "installed_ui_browser_grep": "cooking arbitrary selector",
}
with open(sys.argv[1], "x", encoding="utf-8") as stream:
    json.dump(payload, stream, separators=(",", ":"))
PY
invalid_selector_response="${bridge}/response.invalid-selector"
for _attempt in {1..100}; do
    [[ -f "${invalid_selector_response}" ]] && break
    sleep 0.05
done
[[ "$(cat "${invalid_selector_response}")" == 1 ]]

for mismatch in phase-selector selector-phase; do
    mismatch_request="${bridge}/request.mismatched-${mismatch}.json"
    mismatch_ca="ca.concurrency-${mismatch}.pem"
    printf '%s\n' '-----BEGIN CERTIFICATE-----' 'bridge-test' '-----END CERTIFICATE-----' >"${bridge}/${mismatch_ca}"
    chmod 600 -- "${bridge}/${mismatch_ca}"
    if [[ "${mismatch}" == phase-selector ]]; then
        mismatch_phase=concurrency
        mismatch_selector='cooking installed UI TLS'
    else
        mismatch_phase=initial
        mismatch_selector='cooking concurrent session chat clients reconcile a stale Git push and preserve both turns'
    fi
    MISMATCH_PHASE="${mismatch_phase}" MISMATCH_SELECTOR="${mismatch_selector}" MISMATCH_CA="${mismatch_ca}" FIXTURE="${fixture}" python3 - "${mismatch_request}" <<'PY'
import json
import os
import sys

payload = {
    "runner": "installed-ui",
    "fixture": os.environ["FIXTURE"],
    "database_url": "postgres://bridge-test.invalid/test",
    "rpc_endpoint": "http://bridge-test.invalid/rpc",
    "rpc_secret": "bridge-test-secret-with-sufficient-entropy",
    "oidc_issuer": "http://bridge-test.invalid/oidc",
    "oidc_client_id": "hephaestus-web",
    "oidc_client_secret": "bridge-test-client-secret",
    "web_port": "4000",
    "phase": os.environ["MISMATCH_PHASE"],
    "platform_origin": "https://platform.localhost:4443",
    "ui_namespace": "ui.platform.localhost",
    "ui_port": "4443",
    "ca_cert": os.environ["MISMATCH_CA"],
    "installed_ui_browser_grep": os.environ["MISMATCH_SELECTOR"],
}
with open(sys.argv[1], "x", encoding="utf-8") as stream:
    json.dump(payload, stream, separators=(",", ":"))
PY
    mismatch_response="${bridge}/response.mismatched-${mismatch}"
    for _attempt in {1..100}; do
        [[ -f "${mismatch_response}" ]] && break
        sleep 0.05
    done
    [[ "$(cat "${mismatch_response}")" == 1 ]]
done

invalid_mode_request="${bridge}/request.invalid-mode.json"
FIXTURE="${fixture}" python3 - "${invalid_mode_request}" <<'PY'
import json
import os
import sys

payload = {
    "runner": "unsupported",
    "fixture": os.environ["FIXTURE"],
    "database_url": "postgres://bridge-test.invalid/test",
    "rpc_endpoint": "http://bridge-test.invalid/rpc",
    "rpc_secret": "bridge-test-secret-with-sufficient-entropy",
    "oidc_issuer": "http://bridge-test.invalid/oidc",
    "oidc_client_id": "hephaestus-web",
    "oidc_client_secret": "bridge-test-client-secret",
    "web_port": "4000",
    "phase": "initial",
    "platform_origin": "https://platform.localhost:4443",
    "ui_namespace": "ui.platform.localhost",
    "ui_port": "4443",
    "ca_cert": "ca.post.pem",
    "installed_ui_browser_grep": "cooking installed UI TLS",
}
with open(sys.argv[1], "x", encoding="utf-8") as stream:
    json.dump(payload, stream, separators=(",", ":"))
PY
invalid_mode_response="${bridge}/response.invalid-mode"
for _attempt in {1..100}; do
    [[ -f "${invalid_mode_response}" ]] && break
    sleep 0.05
done
[[ "$(cat "${invalid_mode_response}")" == 1 ]]

mismatched_namespace_request="${bridge}/request.mismatched-namespace.json"
FIXTURE="${fixture}" python3 - "${mismatched_namespace_request}" <<'PY'
import json
import os
import sys

payload = {
    "runner": "installed-ui",
    "fixture": os.environ["FIXTURE"],
    "database_url": "postgres://bridge-test.invalid/test",
    "rpc_endpoint": "http://bridge-test.invalid/rpc",
    "rpc_secret": "bridge-test-secret-with-sufficient-entropy",
    "oidc_issuer": "http://bridge-test.invalid/oidc",
    "oidc_client_id": "hephaestus-web",
    "oidc_client_secret": "bridge-test-client-secret",
    "web_port": "4000",
    "phase": "initial",
    "platform_origin": "https://platform.localhost:4443",
    "ui_namespace": "ui.other.localhost",
    "ui_port": "4443",
    "ca_cert": "ca.mismatched.pem",
}
with open(sys.argv[1], "x", encoding="utf-8") as stream:
    json.dump(payload, stream, separators=(",", ":"))
PY
mismatched_response="${bridge}/response.mismatched-namespace"
for _attempt in {1..100}; do
    [[ -f "${mismatched_response}" ]] && break
    sleep 0.05
done
[[ "$(cat "${mismatched_response}")" == 1 ]]

kill "${host_pid}"
wait "${host_pid}" 2>/dev/null || true
host_pid=""
if compgen -G "${bridge}/request.*" >/dev/null ||
    compgen -G "${bridge}/response.*" >/dev/null ||
    compgen -G "${bridge}/values.*" >/dev/null ||
    compgen -G "${bridge}/ca.*.pem" >/dev/null ||
    [[ -e "${bridge}/.ready" ]]; then
    printf 'installed bridge cleanup left protocol artifacts\n' >&2
    exit 1
fi
printf 'installed UI bridge selector/path/cleanup/legacy compatibility passed\n'
