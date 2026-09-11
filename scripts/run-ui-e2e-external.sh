#!/usr/bin/env bash
# Run the browser client against an already running golden daemon/database.
# The caller owns the database, daemon, and OIDC identity mapping.
set -Eeuo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
fixture="${HEPHAESTUS_E2E_COOKING_FIXTURE:?set HEPHAESTUS_E2E_COOKING_FIXTURE}"
database_url="${HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL:?set HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL}"
rpc_endpoint="${HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT:?set HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT}"
rpc_secret="${HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET:?set HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET}"
if (( ${#rpc_secret} < 32 )); then
    printf 'browser E2E RPC mediator secret must be at least 32 characters.\n' >&2
    exit 1
fi
oidc_issuer="${HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER:?set HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER}"
oidc_client_id="${HEPHAESTUS_E2E_EXTERNAL_OIDC_CLIENT_ID:-hephaestus-web}"
oidc_client_secret="${HEPHAESTUS_E2E_EXTERNAL_OIDC_CLIENT_SECRET:-development-secret}"
web_port="${HEPHAESTUS_E2E_EXTERNAL_WEB_PORT:-4000}"
phase="${HEPHAESTUS_E2E_COOKING_PHASE:-initial}"
case "${phase}" in
    initial) test_grep='cooking release install' ;;
    post-operation) test_grep='cooking post-operation controls' ;;
    *) printf 'unsupported cooking browser phase: %s\n' "${phase}" >&2; exit 1 ;;
esac

if [[ -n "${HEPHAESTUS_COOKING_BROWSER_BRIDGE_DIR:-}" ]]; then
    bridge_dir="${HEPHAESTUS_COOKING_BROWSER_BRIDGE_DIR}"
    [[ "${bridge_dir}" = /* && -d "${bridge_dir}" && ! -L "${bridge_dir}" ]] || {
        printf 'browser bridge directory is invalid\n' >&2
        exit 1
    }
    [[ "${fixture}" = /* ]] || {
        printf 'browser fixture path must be absolute\n' >&2
        exit 1
    }
    fixture_parent="$(cd -- "$(dirname -- "${fixture}")" && pwd -P)"
    bridge_real="$(cd -- "${bridge_dir}" && pwd -P)"
    [[ "${fixture_parent}" == "${bridge_real}" ]] || {
        printf 'browser fixture is outside the bridge directory\n' >&2
        exit 1
    }
    request_id="$(date +%s%N)-$$"
    request="${bridge_dir}/request.${request_id}.json"
    pending="${request}.pending"
    response="${bridge_dir}/response.${request_id}"
    python3 - "${pending}" <<'PY'
import json
import os
import sys

payload = {
    "fixture": os.environ["HEPHAESTUS_E2E_COOKING_FIXTURE"],
    "database_url": os.environ["HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL"],
    "rpc_endpoint": os.environ["HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT"],
    "rpc_secret": os.environ["HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET"],
    "oidc_issuer": os.environ["HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER"],
    "oidc_client_id": os.environ.get("HEPHAESTUS_E2E_EXTERNAL_OIDC_CLIENT_ID", "hephaestus-web"),
    "oidc_client_secret": os.environ.get("HEPHAESTUS_E2E_EXTERNAL_OIDC_CLIENT_SECRET", "development-secret"),
    "web_port": os.environ.get("HEPHAESTUS_E2E_EXTERNAL_WEB_PORT", "4000"),
    "phase": os.environ.get("HEPHAESTUS_E2E_COOKING_PHASE", "initial"),
}
with open(sys.argv[1], "x", opener=lambda path, flags: os.open(path, flags, 0o600), encoding="utf-8") as stream:
    json.dump(payload, stream, separators=(",", ":"))
    stream.write("\n")
PY
    mv -- "${pending}" "${request}"
    bridge_deadline="${HEPHAESTUS_COOKING_BRIDGE_DEADLINE_EPOCH:-0}"
    [[ "${bridge_deadline}" =~ ^[1-9][0-9]*$ ]] || {
        printf 'browser host bridge deadline is missing or invalid\n' >&2
        exit 1
    }
    while [[ ! -f "${response}" ]]; do
        if [[ "${bridge_deadline}" =~ ^[1-9][0-9]*$ && "$(date +%s)" -ge "${bridge_deadline}" ]]; then
            printf 'browser host bridge deadline elapsed\n' >&2
            rm -f -- "${request}" "${pending}" "${response}"
            exit 124
        fi
        sleep 0.1
    done
    status="$(cat -- "${response}")"
    rm -f -- "${response}"
    [[ "${status}" =~ ^[0-9]+$ ]] || {
        printf 'browser host bridge returned an invalid status\n' >&2
        exit 1
    }
    exit "${status}"
fi
web_url="http://127.0.0.1:${web_port}"
web_container="hephaestus-ui-external-web-$$"
diagnostics_dir="${HEPHAESTUS_COOKING_DIAGNOSTICS_DIR:-}"
if [[ -n "${diagnostics_dir}" ]]; then
    [[ "${diagnostics_dir}" = /* && ! -L "${diagnostics_dir}" ]] || {
        printf 'HEPHAESTUS_COOKING_DIAGNOSTICS_DIR must be absolute and non-symlinked.\n' >&2
        exit 1
    }
    mkdir -p -- "${diagnostics_dir}"
    chmod 700 -- "${diagnostics_dir}"
    fixture_root="$(mktemp -d "${diagnostics_dir}/browser.XXXXXX")"
else
    fixture_root="$(mktemp -d)"
fi
evidence_dir="${fixture_root}/playwright-results"
mkdir -p -- "${evidence_dir}"
chmod 700 -- "${fixture_root}" "${evidence_dir}"
readiness_error="${fixture_root}/readiness-curl.log"
install -m 600 /dev/null "${readiness_error}"
print_readiness_error() {
    if [[ -s "${readiness_error}" ]]; then
        head -c 1024 "${readiness_error}" >&2
        printf '\n' >&2
    fi
}

cleanup() {
    local status="$?"
    # Stop the service before collecting its final logs, then check the same
    # retained evidence on successful tests and on startup/test failures.
    podman stop --time 5 "${web_container}" >/dev/null 2>&1 || true
    podman logs "${web_container}" >"${fixture_root}/web-service.log" 2>&1 || true
    podman rm --force "${web_container}" >/dev/null 2>&1 || true
    if python3 "${repo_root}/scripts/check-browser-evidence.py" "${fixture_root}"; then
        if [[ "${status}" -ne 0 && -f "${fixture_root}/playwright.log" ]]; then
            cat "${fixture_root}/playwright.log" >&2
        fi
    else
        status=1
    fi
    if [[ -z "${diagnostics_dir}" ]]; then
        rm -rf -- "${fixture_root}"
    fi
    exit "${status}"
}
trap cleanup EXIT

[[ -f "${fixture}" ]] || { printf 'fixture manifest is missing: %s\n' "${fixture}" >&2; exit 1; }
python3 - "${fixture}" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as stream:
    value = json.load(stream)
required = ("project_id", "release_id", "release_agent_id", "instance_id", "mailbox_id", "gateway_id")
missing = [key for key in required if not isinstance(value.get(key), str) or not value[key]]
if missing:
    raise SystemExit("fixture manifest missing IDs: " + ", ".join(missing))
PY

# The Phoenix container mounts the host asset tree and must receive the
# locked JavaScript dependencies before compiling its development bundles.
npm ci --prefix "${repo_root}/web/assets" >/dev/null

podman run --detach \
    --name "${web_container}" \
    --network host \
    --volume "${repo_root}:/workspace:z" \
    --workdir /workspace/web \
    --env MIX_ENV=dev \
    --env PHX_SERVER=true \
    --env PORT="${web_port}" \
    --env HEPHAESTUS_RPC_ENDPOINT="${rpc_endpoint}" \
    --env HEPHAESTUS_RPC_MEDIATOR_SECRET="${rpc_secret}" \
    --env HEPHAESTUS_BROWSER_OIDC_ISSUER="${oidc_issuer}" \
    --env HEPHAESTUS_BROWSER_OIDC_CLIENT_ID="${oidc_client_id}" \
    --env HEPHAESTUS_BROWSER_OIDC_CLIENT_SECRET="${oidc_client_secret}" \
    --env HEPHAESTUS_BROWSER_OIDC_REDIRECT_URI="${web_url}/auth/oidc/callback" \
    docker.io/hexpm/elixir@sha256:4d96e2b972aafea313822843efc1076b33fa1a2633c4972c6118eb603120d464 \
    sh -lc 'mix local.hex --force >/dev/null && mix deps.get >/dev/null && mix assets.setup && mix assets.build && mix phx.server' \
    >"${fixture_root}/web.log" 2>&1

readiness_deadline="${HEPHAESTUS_COOKING_BROWSER_DEADLINE_EPOCH:-}"
if [[ -z "${readiness_deadline}" ]]; then
    readiness_deadline="$(( $(date +%s) + 60 ))"
fi
[[ "${readiness_deadline}" =~ ^[1-9][0-9]*$ ]] || {
    printf 'browser readiness deadline is invalid\n' >&2
    exit 1
}
while :; do
    readiness_remaining=$((readiness_deadline - $(date +%s)))
    if (( readiness_remaining < 1 )); then
        printf 'browser web readiness deadline elapsed port=%s\n' "${web_port}" >&2
        print_readiness_error
        exit 124
    fi
    readiness_timeout=$((readiness_remaining < 2 ? readiness_remaining : 2))
    if curl --fail --silent --connect-timeout "${readiness_timeout}" \
        --max-time "${readiness_timeout}" "${web_url}/" >/dev/null 2>"${readiness_error}"; then
        break
    else
        web_curl_status="$?"
    fi
    if (( $(date +%s) >= readiness_deadline )); then
        printf 'browser web readiness deadline elapsed port=%s status=%s\n' \
            "${web_port}" "${web_curl_status}" >&2
        print_readiness_error
        exit 124
    fi
    web_container_state="$(podman inspect --format '{{.State.Status}} {{.State.ExitCode}}' \
        "${web_container}" 2>/dev/null || true)"
    if [[ "${web_container_state}" != running* ]]; then
        printf 'browser web container stopped state=%s status=%s\n' \
            "${web_container_state:-unavailable}" "${web_curl_status}" >&2
        print_readiness_error
        exit 1
    fi
    sleep 1
done

cd "${repo_root}/e2e/playwright"
npm ci >/dev/null
# Keep the human line report while writing the structured report to the private
# phase directory. PLAYWRIGHT_JSON_OUTPUT_FILE prevents the JSON reporter from
# writing report contents to the captured process log.
if HEPHAESTUS_E2E_DATABASE_URL="${database_url}" \
    HEPHAESTUS_WEB_URL="${web_url}" \
    HEPHAESTUS_OIDC_URL="${oidc_issuer}" \
    HEPHAESTUS_COOKING_BROWSER_FIXTURE="${fixture}" \
    HEPHAESTUS_E2E_EVIDENCE_DIR="${evidence_dir}" \
    PLAYWRIGHT_JSON_OUTPUT_FILE="${fixture_root}/playwright-report.json" \
    npx playwright test --config=playwright.cooking.config.ts --grep "${test_grep}" --reporter=line,json \
    >"${fixture_root}/playwright.log" 2>&1
then
    browser_status=0
else
    browser_status="$?"
fi
exit "${browser_status}"
