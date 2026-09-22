#!/usr/bin/env bash
# Regression coverage for retaining installed-browser container startup errors.
set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
root="$(mktemp -d /var/tmp/heph-installed-ui-startup-test.XXXXXX)"
cleanup() {
    local status="$?"
    rm -rf -- "${root}"
    exit "${status}"
}
trap cleanup EXIT

fake_repo="${root}/repo"
fake_bin="${root}/bin"
diagnostics="${root}/diagnostics"
fixture="${root}/fixture.json"
mkdir -p -- \
    "${fake_repo}/scripts" \
    "${fake_repo}/e2e/playwright/cooking-tests" \
    "${fake_repo}/web/assets" \
    "${fake_bin}" \
    "${diagnostics}"
chmod 700 -- "${root}" "${fake_repo}" "${fake_repo}/scripts" "${fake_bin}" "${diagnostics}"
cp -- "${script_dir}/run-installed-ui-e2e.sh" "${fake_repo}/scripts/"
cp -- "${script_dir}/check-browser-evidence.py" "${fake_repo}/scripts/"
for file in package.json package-lock.json playwright.config.ts playwright.installed-ui.config.ts safe-installed-ui-reporter.mjs; do
    : >"${fake_repo}/e2e/playwright/${file}"
done
for file in cooking-installed-ui.spec.ts session-chat-installed-ui.spec.ts session-chat-new-installed-ui.spec.ts session-chat-concurrent-installed-ui.spec.ts session-chat-concurrent-parser.mjs session-chat-fork-installed-ui.spec.ts; do
    : >"${fake_repo}/e2e/playwright/cooking-tests/${file}"
done
printf '%s\n' '{"installed_reference_uis":{"project_id":"project","static_installation_id":"static","managed_installation_id":"managed"}}' >"${fixture}"
chmod 600 -- "${fixture}"
mkdir -m 700 -- "${root}/installed-ui-control"
printf '%s\n' '-----BEGIN CERTIFICATE-----' 'startup-test' '-----END CERTIFICATE-----' >"${root}/caddy-ca.pem"
chmod 600 -- "${root}/caddy-ca.pem"

cat >"${fake_bin}/podman" <<'PODMAN'
#!/usr/bin/env bash
set -Eeuo pipefail
name=''
image=''
case "${1:-}" in
    run)
        shift
        while (($# > 0)); do
            if [[ "$1" == --name ]]; then
                name="$2"
                shift 2
            elif [[ "$1" == localhost/hephestus-playwright:* || "$1" == browser@sha256:* ]]; then
                image="$1"
                shift
            else
                shift
            fi
        done
        if [[ "${name}" == hephaestus-ui-installed-browser-* ]]; then
            if [[ "${HEPH_TEST_UNSAFE_BROWSER_OUTPUT:-}" == 1 ]]; then
                printf '%s\n' 'cooking-inbound-only-fixture-sentinel' >&2
            else
                printf 'fake browser startup failure: certutil unavailable image=%s\n' "${image}" >&2
            fi
            exit 1
        fi
        exit 0
        ;;
    logs)
        printf '%s\n' 'fake web service log'
        ;;
    stop|rm|inspect)
        ;;
    *)
        ;;
esac
PODMAN
cat >"${fake_bin}/curl" <<'CURL'
#!/usr/bin/env bash
exit 0
CURL
cat >"${fake_bin}/npm" <<'NPM'
#!/usr/bin/env bash
exit 0
NPM
cat >"${fake_bin}/openssl" <<'OPENSSL'
#!/usr/bin/env bash
printf '%0128d\n' 0
OPENSSL
chmod 700 -- "${fake_bin}/podman" "${fake_bin}/curl" "${fake_bin}/npm" "${fake_bin}/openssl"

result_log="${root}/result.log"
set +e
env \
    PATH="${fake_bin}:${PATH}" \
    HOME="${root}/home" \
    HEPHAESTUS_E2E_COOKING_FIXTURE="${fixture}" \
    HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL='postgres://startup-test.invalid/test' \
    HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT='http://startup-test.invalid/rpc' \
    HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET='startup-test-secret-with-sufficient-entropy' \
    HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER='http://startup-test.invalid/oidc' \
    HEPHAESTUS_E2E_EXTERNAL_WEB_PORT=4444 \
    HEPHAESTUS_PLATFORM_HTTPS_ORIGIN='https://platform.localhost:4443' \
    HEPHAESTUS_UI_NAMESPACE='ui.platform.localhost' \
    HEPHAESTUS_CADDY_TEST_CA_CERT="${root}/caddy-ca.pem" \
    HEPHAESTUS_COOKING_DIAGNOSTICS_DIR="${diagnostics}" \
    bash "${fake_repo}/scripts/run-installed-ui-e2e.sh" >"${result_log}" 2>&1
status="$?"
set -e

[[ "${status}" == 1 ]] || {
    cat -- "${result_log}" >&2
    printf 'installed UI startup fixture returned %s, expected 1\n' "${status}" >&2
    exit 1
}
cat -- "${result_log}"
grep -Fq 'fake browser startup failure: certutil unavailable image=localhost/hephestus-playwright:1.62.0-certutil' "${result_log}"
browser_log="$(find "${diagnostics}" -type f -name browser-container.log -print -quit)"
[[ -n "${browser_log}" && ! -L "${browser_log}" ]] || {
    printf 'retained browser startup log is missing\n' >&2
    exit 1
}
grep -Fq 'fake browser startup failure: certutil unavailable image=localhost/hephestus-playwright:1.62.0-certutil' "${browser_log}"
browser_npm_log="$(find "${diagnostics}" -type f -name browser-npm.log -print -quit)"
[[ -n "${browser_npm_log}" && ! -L "${browser_npm_log}" ]] || {
    printf 'retained browser npm log is missing\n' >&2
    exit 1
}
[[ "$(stat -c '%i' -- "${browser_log}")" != "$(stat -c '%i' -- "${browser_npm_log}")" ]] || {
    printf 'browser container and npm logs unexpectedly share an inode\n' >&2
    exit 1
}
printf 'installed UI startup diagnostics smoke passed log=%s\n' "${browser_log}"

# The launcher must scan retained startup output before exposing it. Exercise
# the failure path with a known fixture credential and ensure it stays out of
# the caller-visible stream even though the retained log is rejected.
unsafe_result_log="${root}/unsafe-result.log"
set +e
env \
    PATH="${fake_bin}:${PATH}" \
    HOME="${root}/home" \
    HEPH_TEST_UNSAFE_BROWSER_OUTPUT=1 \
    HEPHAESTUS_E2E_COOKING_FIXTURE="${fixture}" \
    HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL='postgres://startup-test.invalid/test' \
    HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT='http://startup-test.invalid/rpc' \
    HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET='startup-test-secret-with-sufficient-entropy' \
    HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER='http://startup-test.invalid/oidc' \
    HEPHAESTUS_E2E_EXTERNAL_WEB_PORT=4444 \
    HEPHAESTUS_PLATFORM_HTTPS_ORIGIN='https://platform.localhost:4443' \
    HEPHAESTUS_UI_NAMESPACE='ui.platform.localhost' \
    HEPHAESTUS_CADDY_TEST_CA_CERT="${root}/caddy-ca.pem" \
    HEPHAESTUS_COOKING_DIAGNOSTICS_DIR="${diagnostics}" \
    bash "${fake_repo}/scripts/run-installed-ui-e2e.sh" >"${unsafe_result_log}" 2>&1
unsafe_status="$?"
set -e
[[ "${unsafe_status}" == 1 ]] || {
    cat -- "${unsafe_result_log}" >&2
    printf 'unsafe browser startup fixture returned %s, expected 1\n' "${unsafe_status}" >&2
    exit 1
}
if grep -Fq 'cooking-inbound-only-fixture-sentinel' "${unsafe_result_log}"; then
    cat -- "${unsafe_result_log}" >&2
    printf 'unsafe browser output escaped the evidence scanner\n' >&2
    exit 1
fi
printf 'installed UI startup credential-output suppression passed\n'
