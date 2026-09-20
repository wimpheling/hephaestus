#!/usr/bin/env bash
# Run the installed UI browser smoke against an already running golden daemon.
# The caller owns the database, daemon, Caddy TLS edge, and OIDC identity mapping.
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
web_port="${HEPHAESTUS_E2E_EXTERNAL_WEB_PORT:?set HEPHAESTUS_E2E_EXTERNAL_WEB_PORT to the reserved Phoenix port}"
phase="${HEPHAESTUS_E2E_COOKING_PHASE:-initial}"
case "${phase}" in
    initial) true ;;
    *) printf 'installed UI smoke supports only the initial phase\n' >&2; exit 1 ;;
esac
platform_origin="${HEPHAESTUS_PLATFORM_HTTPS_ORIGIN:?set HEPHAESTUS_PLATFORM_HTTPS_ORIGIN}"
ca_cert="${HEPHAESTUS_CADDY_TEST_CA_CERT:?set HEPHAESTUS_CADDY_TEST_CA_CERT}"
ui_namespace="${HEPHAESTUS_UI_NAMESPACE:?set HEPHAESTUS_UI_NAMESPACE}"
[[ "${platform_origin}" =~ ^https://[^/:]+:[0-9]+$ ]] || {
    printf 'platform origin must be an explicit HTTPS host and port\n' >&2
    exit 1
}
ui_port="${platform_origin##*:}"
[[ "${ui_port}" =~ ^[1-9][0-9]{0,4}$ && "${ui_port}" -le 65535 ]] || {
    printf 'platform origin port is invalid\n' >&2
    exit 1
}
if [[ -n "${HEPHAESTUS_UI_PORT:-}" && "${HEPHAESTUS_UI_PORT}" != "${ui_port}" ]]; then
    printf 'HEPHAESTUS_UI_PORT must match the platform origin port\n' >&2
    exit 1
fi
export HEPHAESTUS_UI_PORT="${ui_port}"

if [[ -n "${HEPHAESTUS_COOKING_BROWSER_BRIDGE_DIR:-}" ]]; then
    export HEPHAESTUS_E2E_BROWSER_RUNNER=installed-ui
    exec "${repo_root}/scripts/run-ui-e2e-external.sh"
fi
browser_image="${HEPHAESTUS_PLAYWRIGHT_IMAGE:?set HEPHAESTUS_PLAYWRIGHT_IMAGE to a reviewed image containing Chromium and certutil}"
[[ -f "${ca_cert}" ]] || { printf 'Caddy CA certificate is unavailable\n' >&2; exit 1; }
platform_host="${platform_origin#https://}"
platform_host="${platform_host%%:*}"
command -v openssl >/dev/null || {
    printf 'installed UI smoke requires openssl for an ephemeral production cookie key\n' >&2
    exit 1
}
secret_key_base="$(openssl rand -hex 64)"
web_url="${platform_origin}"
web_container="hephaestus-ui-installed-web-$$"
browser_container="hephaestus-ui-installed-browser-$$"
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
install -m 600 /dev/null "${fixture_root}/playwright.log"
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
    podman rm --force "${browser_container}" >/dev/null 2>&1 || true
    if python3 "${repo_root}/scripts/check-browser-evidence.py" "${fixture_root}"; then
        :
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
required = ("installed_reference_uis",)
missing = [key for key in required if key not in value or value[key] in (None, "")]
if missing:
    raise SystemExit("fixture manifest missing installed UI fields: " + ", ".join(missing))
installed = value["installed_reference_uis"]
if not isinstance(installed, dict) or any(not isinstance(installed.get(key), str) or not installed[key] for key in ("project_id", "static_installation_id", "managed_installation_id")):
    raise SystemExit("fixture installed_reference_uis fields are invalid")
PY

# The Phoenix container mounts the host asset tree and must receive the
# locked JavaScript dependencies before compiling its production bundles.
npm ci --prefix "${repo_root}/web/assets" >/dev/null

podman run --detach \
    --name "${web_container}" \
    --network host \
    --volume "${repo_root}:/workspace:z" \
    --workdir /workspace/web \
    --env MIX_ENV=prod \
    --env PHX_SERVER=true \
    --env SECRET_KEY_BASE="${secret_key_base}" \
    --env PHX_HOST="${platform_host}" \
    --env PORT="${web_port}" \
    --env HEPHAESTUS_RPC_ENDPOINT="${rpc_endpoint}" \
    --env HEPHAESTUS_RPC_MEDIATOR_SECRET="${rpc_secret}" \
    --env HEPHAESTUS_BROWSER_OIDC_ISSUER="${oidc_issuer}" \
    --env HEPHAESTUS_BROWSER_OIDC_CLIENT_ID="${oidc_client_id}" \
    --env HEPHAESTUS_BROWSER_OIDC_CLIENT_SECRET="${oidc_client_secret}" \
    --env HEPHAESTUS_BROWSER_OIDC_REDIRECT_URI="${web_url}/auth/oidc/callback" \
    --env HEPHAESTUS_PLATFORM_HTTPS_ORIGIN="${platform_origin}" \
    --env HEPHAESTUS_UI_NAMESPACE="${ui_namespace}" \
    --env HEPHAESTUS_UI_PORT="${ui_port}" \
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
    if curl --fail --silent --show-error --cacert "${ca_cert}" \
        --connect-timeout "${readiness_timeout}" \
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

browser_project="${fixture_root}/playwright"
mkdir -p -- "${browser_project}/cooking-tests"
cp -- \
    "${repo_root}/e2e/playwright/package.json" \
    "${repo_root}/e2e/playwright/package-lock.json" \
    "${repo_root}/e2e/playwright/playwright.config.ts" \
    "${repo_root}/e2e/playwright/playwright.installed-ui.config.ts" \
    "${repo_root}/e2e/playwright/safe-installed-ui-reporter.mjs" \
    "${browser_project}/"
cp -- \
    "${repo_root}/e2e/playwright/cooking-tests/cooking-installed-ui.spec.ts" \
    "${browser_project}/cooking-tests/"
cp -- "${fixture}" "${fixture_root}/fixture.json"
cp -- "${ca_cert}" "${fixture_root}/caddy-ca.pem"
chmod 600 "${fixture_root}/fixture.json" "${fixture_root}/caddy-ca.pem"
# Run Chromium in a disposable container so the Caddy CA is imported into the
# container user's normal NSS store. No host trust store or certificate bypass
# is used, and the container is removed by the trap.
if podman run --rm --name "${browser_container}" \
    --network host \
    --volume "${fixture_root}:/run/heph-fixture:Z" \
    --env HEPHAESTUS_WEB_URL="${web_url}" \
    --env HEPHAESTUS_OIDC_URL="${oidc_issuer}" \
    --env HEPHAESTUS_COOKING_BROWSER_FIXTURE=/run/heph-fixture/fixture.json \
    --env HEPHAESTUS_E2E_EVIDENCE_DIR=/run/heph-fixture/playwright-results \
    --env HEPHAESTUS_SAFE_SCREENSHOT_DIR=/run/heph-fixture/playwright-results \
    "${browser_image}" \
    sh -euc '
        command -v certutil >/dev/null || { echo "browser image lacks certutil (libnss3-tools)" >&2; exit 78; }
        nss_dir="$HOME/.local/share/pki/nssdb"
        mkdir -p "$nss_dir"
        certutil -N -d "sql:$nss_dir" --empty-password >/dev/null 2>&1 || true
        certutil -A -d "sql:$nss_dir" -n heph-caddy-fixture -t "C,," -i /run/heph-fixture/caddy-ca.pem
        cd /run/heph-fixture/playwright
        npm ci --ignore-scripts >/dev/null
        HEPHAESTUS_WEB_URL="$HEPHAESTUS_WEB_URL" \
        HEPHAESTUS_OIDC_URL="$HEPHAESTUS_OIDC_URL" \
        HEPHAESTUS_COOKING_BROWSER_FIXTURE="$HEPHAESTUS_COOKING_BROWSER_FIXTURE" \
        HEPHAESTUS_E2E_EVIDENCE_DIR="$HEPHAESTUS_E2E_EVIDENCE_DIR" \
        ./node_modules/.bin/playwright test --config=playwright.installed-ui.config.ts --grep "cooking installed UI TLS" \
        >/run/heph-fixture/playwright.log 2>/dev/null
    ' >/dev/null 2>&1
then
    browser_status=0
else
    browser_status="$?"
fi
exit "${browser_status}"
