#!/usr/bin/env bash
# Regression coverage for the installed UI prerequisite probe diagnostics.
set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
root="$(mktemp -d /var/tmp/heph-installed-ui-browser-smoke-test.XXXXXX)"
cleanup() {
    local status="$?"
    rm -rf -- "${root}"
    exit "${status}"
}
trap cleanup EXIT

fake_bin="${root}/bin"
diagnostics="${root}/diagnostics"
mkdir -p -- "${fake_bin}" "${diagnostics}"
chmod 700 -- "${root}" "${fake_bin}" "${diagnostics}"
printf '%s\n' '-----BEGIN CERTIFICATE-----' 'smoke-test' '-----END CERTIFICATE-----' >"${root}/caddy-ca.pem"
chmod 600 -- "${root}/caddy-ca.pem"

cat >"${fake_bin}/openssl" <<'OPENSSL'
#!/usr/bin/env bash
key=''
cert=''
while (($# > 0)); do
    case "$1" in
        -keyout) key="$2"; shift 2 ;;
        -out) cert="$2"; shift 2 ;;
        *) shift ;;
    esac
done
printf '%s\n' fake-key >"${key}"
printf '%s\n' fake-certificate >"${cert}"
exit 0
OPENSSL
cat >"${fake_bin}/podman" <<'PODMAN'
#!/usr/bin/env bash
set -Eeuo pipefail
if [[ "${1:-}" == image && "${2:-}" == exists ]]; then
    exit 0
fi
if [[ "${1:-}" != run ]]; then
    exit 0
fi
expected=''
fixture_mount=''
while (($# > 0)); do
    case "$1" in
        --env)
            case "$2" in
                HEPH_PREREQUISITE_EXPECTED=*) expected="${2#*=}" ;;
            esac
            shift 2
            ;;
        --volume)
            fixture_mount="${2%:/run/heph-prerequisite:Z}"
            shift 2
            ;;
        *) shift ;;
    esac
done
if [[ "${expected}" == pass ]]; then
    if [[ "${HEPHAESTUS_TEST_SMOKE_MODE:-}" == safe-failure || "${HEPHAESTUS_TEST_SMOKE_MODE:-}" == secret-failure ]]; then
        if [[ "${HEPHAESTUS_TEST_SMOKE_MODE}" == secret-failure ]]; then
            printf '%s\n' 'cooking-inbound-only-fixture-sentinel' >&2
        else
            printf '%s\n' 'injected browser startup failure: certutil unavailable' >&2
        fi
        exit "${HEPHAESTUS_TEST_SMOKE_STATUS:-17}"
    fi
    printf '%s\n' 'heph-installed-ui-prerequisite' >"${fixture_mount}/browser.html"
    exit 0
fi
if [[ "${HEPHAESTUS_TEST_SMOKE_MODE:-}" == wrong-ca-failure ]]; then
    printf '%s\n' 'injected wrong-CA browser startup failure' >&2
    exit "${HEPHAESTUS_TEST_SMOKE_STATUS:-17}"
fi
printf '%s\n' ERR_CERT_AUTHORITY_INVALID >"${fixture_mount}/browser.html"
exit 0
PODMAN
chmod 700 -- "${fake_bin}/openssl" "${fake_bin}/podman"

run_smoke() {
    local mode="$1" output="$2" status
    set +e
    env PATH="${fake_bin}:${PATH}" \
        HEPHAESTUS_PLAYWRIGHT_IMAGE='localhost/hephestus-playwright:1.62.0-certutil' \
        HEPHAESTUS_CADDY_TEST_CA_CERT="${root}/caddy-ca.pem" \
        HEPHAESTUS_CADDY_TEST_PUBLIC_URL='https://127.0.0.1:4443' \
        HEPHAESTUS_CADDY_TEST_PUBLIC_PORT=4443 \
        HEPHAESTUS_COOKING_DIAGNOSTICS_DIR="${diagnostics}" \
        HEPHAESTUS_TEST_SMOKE_MODE="${mode}" \
        bash "${script_dir}/installed-ui-browser-image/smoke.sh" >"${output}" 2>&1
    status="$?"
    set -e
    printf '%s\n' "${status}"
}

positive_output="${root}/positive.log"
positive_status="$(run_smoke success "${positive_output}")"
[[ "${positive_status}" == 0 ]] || { cat "${positive_output}" >&2; exit 1; }
grep -Fq -- 'HEPH_GCP_COOKING event=installed-ui-prerequisite status=pass' "${positive_output}"

safe_output="${root}/safe.log"
safe_status="$(run_smoke safe-failure "${safe_output}")"
[[ "${safe_status}" == 17 ]] || { cat "${safe_output}" >&2; exit 1; }
grep -Fq -- '--- probe-pass.log ---' "${safe_output}"
grep -Fq -- 'injected browser startup failure: certutil unavailable' "${safe_output}"
grep -Fq -- 'HEPH_GCP_FAILURE phase=browser-setup command_id=playwright-run exit_code=17' "${safe_output}"

wrong_ca_output="${root}/wrong-ca.log"
wrong_ca_status="$(run_smoke wrong-ca-failure "${wrong_ca_output}")"
[[ "${wrong_ca_status}" == 17 ]] || { cat "${wrong_ca_output}" >&2; exit 1; }
grep -Fq -- '--- probe-fail.log ---' "${wrong_ca_output}"
grep -Fq -- 'injected wrong-CA browser startup failure' "${wrong_ca_output}"

secret_output="${root}/secret.log"
secret_status="$(run_smoke secret-failure "${secret_output}")"
[[ "${secret_status}" == 17 ]] || { cat "${secret_output}" >&2; exit 1; }
grep -Fq -- 'Installed UI prerequisite diagnostics were withheld by the credential scanner.' "${secret_output}"
if grep -Fq -- 'cooking-inbound-only-fixture-sentinel' "${secret_output}"; then
    cat "${secret_output}" >&2
    printf '%s\n' 'credential-bearing probe output escaped the scanner' >&2
    exit 1
fi

printf '%s\n' 'installed UI browser smoke diagnostics passed'
