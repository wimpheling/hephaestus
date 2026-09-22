#!/usr/bin/env bash
# Run the installed UI browser smoke against an already running golden daemon.
# The caller owns the database, daemon, Caddy TLS edge, and OIDC identity mapping.
set -Eeuo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
gcp_failure_marker() {
    local failure_phase="${HEPH_GCP_FAILURE_PHASE:-${phase:-browser-setup}}"
    printf 'HEPH_GCP_FAILURE phase=%s command_id=%s exit_code=%s diagnostic_source=%s diagnostic_error=%s\n' \
        "${failure_phase}" "$1" "$2" "$3" "$4" >&2
}
if [[ "${HEPHAESTUS_INSTALLED_UI_PREREQUISITE_ONLY:-0}" == 1 ]]; then
    exec "${repo_root}/scripts/installed-ui-browser-image/smoke.sh"
fi
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
browser_grep="${HEPHAESTUS_INSTALLED_UI_BROWSER_GREP:-cooking installed UI TLS}"
case "${browser_grep}" in
    "cooking installed UI TLS") browser_fixture_mode="installed_reference_uis" ;;
    "cooking session-chat installed UI initializes and reconnects ordinary Git history") browser_fixture_mode="session_chat_ui" ;;
    "cooking new session chat creates and opens a real Git-backed browser session") browser_fixture_mode="session_chat_new" ;;
    "cooking concurrent session chat clients reconcile a stale Git push and preserve both turns") browser_fixture_mode="session_chat_concurrent" ;;
    "cooking forked session chat preserves inherited history and receives a fresh response") browser_fixture_mode="session_chat_fork" ;;
    *)
        printf 'unsupported installed UI browser selector\n' >&2
        exit 1
        ;;
esac
case "${phase}" in
    initial|recovery|concurrency|fork) true ;;
    *) printf 'installed UI smoke supports only the initial, recovery, concurrency, or fork phase\n' >&2; exit 1 ;;
esac
if [[ "${phase}" == concurrency && "${browser_fixture_mode}" != session_chat_concurrent ]] ||
    [[ "${browser_fixture_mode}" == session_chat_concurrent && "${phase}" != concurrency ]]; then
    printf 'session-chat concurrency selector and phase must be paired\n' >&2
    exit 1
fi
if [[ "${phase}" == fork && "${browser_fixture_mode}" != session_chat_fork ]] ||
    [[ "${browser_fixture_mode}" == session_chat_fork && "${phase}" != fork ]]; then
    printf 'session-chat fork selector and phase must be paired\n' >&2
    exit 1
fi
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

[[ "${fixture}" = /* ]] || {
    printf 'installed UI fixture path must be absolute\n' >&2
    exit 1
}
fixture_parent="$(cd -- "$(dirname -- "${fixture}")" && pwd -P)"
control_dir="${HEPHAESTUS_INSTALLED_UI_CONTROL_DIR:-${fixture_parent}/installed-ui-control}"
[[ "${control_dir}" = /* && -d "${control_dir}" && ! -L "${control_dir}" ]] || {
    printf 'installed UI control directory is invalid\n' >&2
    exit 1
}
control_real="$(cd -- "${control_dir}" && pwd -P)"
expected_control="${fixture_parent}/installed-ui-control"
[[ "${control_real}" == "${expected_control}" ]] || {
    printf 'installed UI control directory is outside fixture parent\n' >&2
    exit 1
}
[[ "$(stat -c '%a' -- "${control_real}")" == 700 ]] || {
    printf 'installed UI control directory mode is not 0700\n' >&2
    exit 1
}
export HEPHAESTUS_INSTALLED_UI_CONTROL_DIR="${control_real}"

if [[ -n "${HEPHAESTUS_COOKING_BROWSER_BRIDGE_DIR:-}" ]]; then
    export HEPHAESTUS_E2E_BROWSER_RUNNER=installed-ui
    exec "${repo_root}/scripts/run-ui-e2e-external.sh"
fi
# This is the reviewed local image built by scripts/installed-ui-browser-image/build.sh.
# Callers may provide a separately pinned reviewed image through the environment.
browser_image="${HEPHAESTUS_PLAYWRIGHT_IMAGE:-localhost/hephestus-playwright:1.62.0-certutil}"
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
# The outer Podman redirect owns browser-container.log. Keep nested npm output
# separate so opening the outer file cannot truncate a concurrently written
# inner log.
browser_container_log="${fixture_root}/browser-container.log"
install -m 600 /dev/null "${browser_container_log}"
browser_npm_log="${fixture_root}/browser-npm.log"
install -m 600 /dev/null "${browser_npm_log}"
readiness_error="${fixture_root}/readiness-curl.log"
install -m 600 /dev/null "${readiness_error}"
print_readiness_error() {
    if [[ -s "${readiness_error}" ]]; then
        head -c 1024 "${readiness_error}" >&2
        printf '\n' >&2
    fi
}

print_browser_container_error() {
    local diagnostic_log
    for diagnostic_log in "${browser_container_log}" "${browser_npm_log}" "${fixture_root}/playwright.log"; do
        if [[ -s "${diagnostic_log}" ]]; then
            printf '%s\n' "--- ${diagnostic_log##*/} ---" >&2
            head -c 4096 "${diagnostic_log}" >&2 || true
            printf '\n' >&2
        fi
    done
}

cleanup() {
    local status="$?"
    # Stop the service before collecting its final logs, then check the same
    # retained evidence on successful tests and on startup/test failures.
    podman stop --time 5 "${web_container}" >/dev/null 2>&1 || true
    podman logs "${web_container}" >"${fixture_root}/web-service.log" 2>&1 || true
    podman rm --force "${web_container}" >/dev/null 2>&1 || true
    podman rm --force "${browser_container}" >/dev/null 2>&1 || true
    # npm ci creates .bin symlinks in this disposable browser-only tree. Do
    # not retain generated dependencies; the evidence scanner intentionally
    # rejects symlinks in retained artifacts.
    rm -rf -- "${fixture_root}/playwright/node_modules"
    evidence_status=0
    if ! python3 "${repo_root}/scripts/check-browser-evidence.py" "${fixture_root}"; then
        evidence_status=1
        status=1
    fi
    # Only expose retained browser output after the complete fixture has
    # passed credential scanning. Raw diagnostics must never bypass the
    # evidence gate on an early startup failure.
    if ((status != 0 && evidence_status == 0)); then
        print_browser_container_error
    fi
    if [[ -z "${diagnostics_dir}" ]]; then
        rm -rf -- "${fixture_root}"
    fi
    exit "${status}"
}
trap cleanup EXIT

[[ -f "${fixture}" ]] || { printf 'fixture manifest is missing: %s\n' "${fixture}" >&2; exit 1; }
[[ "${#browser_grep}" -le 256 && "${browser_grep}" != *$'\n'* && "${browser_grep}" != *$'\r'* ]] || {
    printf 'installed UI browser grep is invalid\n' >&2
    exit 1
}
python3 - "${fixture}" "${browser_fixture_mode}" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as stream:
    value = json.load(stream)
fixture_mode = sys.argv[2]
if fixture_mode == "session_chat_new":
    session = value.get("session_chat_new")
    required = ("project_id", "release_agent_id", "model_import_id", "agent_response_text", "ui_path")
    if not isinstance(session, dict) or any(not isinstance(session.get(key), str) or not session[key] for key in required):
        raise SystemExit("fixture session_chat_new fields are invalid")
elif fixture_mode == "session_chat_ui":
    session = value.get("session_chat_ui")
    required = ("project_id", "repository_id", "installation_id", "generation_id", "actor_id", "ui_path", "agent_response_text")
    if not isinstance(session, dict) or any(not isinstance(session.get(key), str) or not session[key] for key in required):
        raise SystemExit("fixture session_chat_ui fields are invalid")
elif fixture_mode == "session_chat_concurrent":
    session = value.get("session_chat_concurrent")
    required = ("project_id", "repository_id", "installation_id", "generation_id", "actor_id", "ui_path", "agent_response_text", "initial_transcript_count", "initial_agent_count")
    if not isinstance(session, dict) or any(not isinstance(session.get(key), str) or not session[key] for key in required):
        raise SystemExit("fixture session_chat_concurrent fields are invalid")
    if any(not session[key].isdigit() for key in ("initial_transcript_count", "initial_agent_count")):
        raise SystemExit("fixture session_chat_concurrent counts are invalid")
elif fixture_mode == "session_chat_fork":
    session = value.get("session_chat_fork")
    required = ("project_id", "repository_id", "installation_id", "generation_id", "actor_id", "ui_path", "agent_response_text", "initial_transcript_count", "initial_agent_count")
    if not isinstance(session, dict) or any(not isinstance(session.get(key), str) or not session[key] for key in required):
        raise SystemExit("fixture session_chat_fork fields are invalid")
    if any(not session[key].isdigit() for key in ("initial_transcript_count", "initial_agent_count")):
        raise SystemExit("fixture session_chat_fork counts are invalid")
elif fixture_mode == "installed_reference_uis":
    installed = value.get("installed_reference_uis")
    required = ("project_id", "static_installation_id", "managed_installation_id")
    if not isinstance(installed, dict) or any(not isinstance(installed.get(key), str) or not installed[key] for key in required):
        raise SystemExit("fixture installed_reference_uis fields are invalid")
else:
    raise SystemExit("unsupported installed UI fixture mode")
PY

# The Phoenix container mounts the host asset tree and must receive the
# locked JavaScript dependencies before compiling its production bundles.
if npm ci --prefix "${repo_root}/web/assets" >"${browser_npm_log}" 2>&1; then
    :
else
    npm_setup_status=$?
    gcp_failure_marker npm-install "${npm_setup_status}" npm-log npm-failed
    exit "${npm_setup_status}"
fi

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
        gcp_failure_marker browser-setup 1 setup-log setup-failed
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
    "${repo_root}/e2e/playwright/cooking-tests/session-chat-installed-ui.spec.ts" \
    "${repo_root}/e2e/playwright/cooking-tests/session-chat-new-installed-ui.spec.ts" \
    "${repo_root}/e2e/playwright/cooking-tests/session-chat-concurrent-installed-ui.spec.ts" \
    "${repo_root}/e2e/playwright/cooking-tests/session-chat-concurrent-parser.mjs" \
    "${repo_root}/e2e/playwright/cooking-tests/session-chat-fork-installed-ui.spec.ts" \
    "${browser_project}/cooking-tests/"
cp -- "${fixture}" "${fixture_root}/fixture.json"
cp -- "${ca_cert}" "${fixture_root}/caddy-ca.pem"
chmod 600 "${fixture_root}/fixture.json" "${fixture_root}/caddy-ca.pem"
# Run Chromium in a disposable container so the Caddy CA is imported into the
# container user's normal NSS store. No host trust store or certificate bypass
# is used, and the container is removed by the trap.
if podman run --rm --name "${browser_container}" \
    --userns=keep-id \
    --user "$(id -u):$(id -g)" \
    --network host \
    --volume "${fixture_root}:/run/heph-fixture:Z" \
    --volume "${control_real}:/run/heph-control:Z" \
    --env HOME=/tmp \
    --env HEPHAESTUS_WEB_URL="${web_url}" \
    --env HEPHAESTUS_OIDC_URL="${oidc_issuer}" \
    --env HEPHAESTUS_UI_NAMESPACE="${ui_namespace}" \
    --env HEPHAESTUS_INSTALLED_UI_CONTROL_DIR=/run/heph-control \
    --env HEPHAESTUS_COOKING_BROWSER_FIXTURE=/run/heph-fixture/fixture.json \
    --env HEPHAESTUS_E2E_EVIDENCE_DIR=/run/heph-fixture/playwright-results \
    --env HEPHAESTUS_SAFE_SCREENSHOT_DIR=/run/heph-fixture/playwright-results \
    --env HEPHAESTUS_INSTALLED_UI_BROWSER_GREP="${browser_grep}" \
    "${browser_image}" \
    sh -euc '
        command -v certutil >/dev/null || {
            printf '%s\n' 78 >/run/heph-fixture/setup.status
            echo "browser image lacks certutil (libnss3-tools)" >&2
            exit 78
        }
        nss_dir="$HOME/.local/share/pki/nssdb"
        mkdir -p "$nss_dir"
        certutil -N -d "sql:$nss_dir" --empty-password >/dev/null 2>&1 || true
        if certutil -A -d "sql:$nss_dir" -n heph-caddy-fixture -t "C,," -i /run/heph-fixture/caddy-ca.pem; then
            :
        else
            certutil_status="$?"
            printf '%s\n' "$certutil_status" >/run/heph-fixture/setup.status
            exit "$certutil_status"
        fi
        cd /run/heph-fixture/playwright
        if npm ci --ignore-scripts >/run/heph-fixture/browser-npm.log 2>&1; then
            :
        else
            npm_status="$?"
            printf '%s\n' "$npm_status" >/run/heph-fixture/npm.status
            exit "$npm_status"
        fi
        if HEPHAESTUS_WEB_URL="$HEPHAESTUS_WEB_URL" \
        HEPHAESTUS_OIDC_URL="$HEPHAESTUS_OIDC_URL" \
        HEPHAESTUS_UI_NAMESPACE="$HEPHAESTUS_UI_NAMESPACE" \
        HEPHAESTUS_INSTALLED_UI_CONTROL_DIR="$HEPHAESTUS_INSTALLED_UI_CONTROL_DIR" \
        HEPHAESTUS_COOKING_BROWSER_FIXTURE="$HEPHAESTUS_COOKING_BROWSER_FIXTURE" \
        HEPHAESTUS_E2E_EVIDENCE_DIR="$HEPHAESTUS_E2E_EVIDENCE_DIR" \
        ./node_modules/.bin/playwright test --config=playwright.installed-ui.config.ts \
        --grep "${HEPHAESTUS_INSTALLED_UI_BROWSER_GREP}" \
        >/run/heph-fixture/playwright.log 2>&1; then
            :
        else
            playwright_status="$?"
            printf '%s\n' "$playwright_status" >/run/heph-fixture/playwright.status
            exit "$playwright_status"
        fi
    ' >"${browser_container_log}" 2>&1
then
    browser_status=0
else
    browser_status="$?"
    marker_status=1
    marker_command=browser-setup
    marker_source=setup-log
    marker_error=setup-failed
    for marker_file in playwright.status npm.status setup.status; do
        if [[ -s "${fixture_root}/${marker_file}" ]]; then
            marker_status="$(cat "${fixture_root}/${marker_file}")"
            [[ "${marker_status}" =~ ^[0-9]+$ ]] || marker_status=1
            case "${marker_file}" in
                playwright.status) marker_command=playwright-run; marker_source=playwright-log; marker_error=playwright-failed ;;
                npm.status) marker_command=npm-install; marker_source=npm-log; marker_error=npm-failed ;;
            esac
            break
        fi
    done
    gcp_failure_marker "${marker_command}" "${marker_status}" "${marker_source}" "${marker_error}"
fi
exit "${browser_status}"
