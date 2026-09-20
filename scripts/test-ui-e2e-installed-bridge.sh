#!/usr/bin/env bash
# Protocol-only bridge coverage for legacy and installed UI runner selection.
# It uses fake child runners and never starts a browser, web service, Cargo, or VM.
set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
root="$(mktemp -d /tmp/heph-installed-bridge-test.XXXXXX)"
bridge="${root}/bridge"
fake_repo="${root}/fake-repo"
fixture="${bridge}/fixture.json"
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
