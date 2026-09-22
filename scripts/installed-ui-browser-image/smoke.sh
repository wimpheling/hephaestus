#!/usr/bin/env bash
# Verify the reviewed installed-UI browser image can trust the live Caddy CA
# and render the bounded prerequisite page. This does not start Phoenix.
set -Eeuo pipefail

image="${HEPHAESTUS_PLAYWRIGHT_IMAGE:?set HEPHAESTUS_PLAYWRIGHT_IMAGE}"
ca_cert="${HEPHAESTUS_CADDY_TEST_CA_CERT:?set HEPHAESTUS_CADDY_TEST_CA_CERT}"
public_url="${HEPHAESTUS_CADDY_TEST_PUBLIC_URL:?set HEPHAESTUS_CADDY_TEST_PUBLIC_URL}"
public_port="${HEPHAESTUS_CADDY_TEST_PUBLIC_PORT:?set HEPHAESTUS_CADDY_TEST_PUBLIC_PORT}"
require_marker="${HEPHAESTUS_CADDY_TEST_SMOKE_PAGE:-0}"
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
diagnostics_dir="${HEPHAESTUS_COOKING_DIAGNOSTICS_DIR:-}"
scanner="${HEPH_GCP_DIAGNOSTICS_SCANNER_SCRIPT:-${repo_root}/scripts/check-browser-evidence.py}"
gcp_failure_marker() {
    printf 'HEPH_GCP_FAILURE phase=browser-setup command_id=%s exit_code=%s diagnostic_source=%s diagnostic_error=%s\n' \
        "$1" "$2" "$3" "$4" >&2
}
[[ "${public_url}" =~ ^https://[^/:]+:${public_port}$ ]] || {
    gcp_failure_marker browser-setup 1 setup-log setup-failed
    printf 'installed UI prerequisite requires an explicit HTTPS Caddy URL\n' >&2
    exit 1
}
[[ -f "${ca_cert}" && ! -L "${ca_cert}" ]] || {
    gcp_failure_marker browser-setup 1 setup-log setup-failed
    printf 'installed UI prerequisite Caddy CA is unavailable\n' >&2
    exit 1
}
for command in openssl podman; do
    command -v "${command}" >/dev/null || {
        gcp_failure_marker browser-setup 1 setup-log setup-failed
        printf 'installed UI prerequisite requires %s\n' "${command}" >&2
        exit 1
    }
done
podman image exists "${image}" || {
    gcp_failure_marker installed-ui-image-build 1 image-build-log image-build-failed
    printf 'installed UI prerequisite browser image is unavailable: %s\n' "${image}" >&2
    exit 1
}

if [[ -n "${diagnostics_dir}" ]]; then
    [[ "${diagnostics_dir}" = /* && ! -L "${diagnostics_dir}" ]] || {
        printf 'installed UI prerequisite diagnostics path is unsafe\n' >&2
        exit 1
    }
    mkdir -p -- "${diagnostics_dir}"
    chmod 700 -- "${diagnostics_dir}"
    fixture_root="$(mktemp -d "${diagnostics_dir}/browser-prerequisite.XXXXXX")"
    retain_fixture=1
else
    fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/heph-installed-ui-prerequisite.XXXXXX")"
    retain_fixture=0
fi
cleanup() {
    local status=$?
    if ((retain_fixture == 0)); then
        rm -rf -- "${fixture_root}"
    fi
    exit "${status}"
}
trap cleanup EXIT
install -m 0644 "${ca_cert}" "${fixture_root}/caddy-ca.pem"
install -m 0600 /dev/null "${fixture_root}/browser.log"
browser_user="$(id -u):$(id -g)"

print_probe_failure_context() {
    local expected="$1" diagnostic_log
    for diagnostic_log in "${fixture_root}/probe-${expected}.log" "${fixture_root}/browser.log"; do
        if [[ -s "${diagnostic_log}" ]]; then
            printf '%s\n' "--- ${diagnostic_log##*/} ---" >&2
            head -c 4096 "${diagnostic_log}" >&2 || true
            printf '\n' >&2
        fi
    done
}

run_browser_probe() {
    local trust_ca="$1" expected="$2"
    local probe_log="${fixture_root}/probe-${expected}.log" probe_status
    install -m 600 /dev/null "${probe_log}"
    if podman run --rm --userns=keep-id --user "${browser_user}" --network host \
        --volume "${fixture_root}:/run/heph-prerequisite:Z" \
        --env HOME=/tmp \
        --env HEPH_PREREQUISITE_CA="${trust_ca}" \
        --env HEPH_PREREQUISITE_URL="${public_url}/" \
        --env HEPH_PREREQUISITE_REQUIRE_MARKER="${require_marker}" \
        --env HEPH_PREREQUISITE_EXPECTED="${expected}" \
        "${image}" bash -Eeuo pipefail -c '
            command -v certutil >/dev/null
            command -v timeout >/dev/null
            browser="$(find /ms-playwright -type f \( -name chrome -o -name chrome-headless-shell \) -perm -0100 -print -quit)"
            test -n "$browser"
            "$browser" --version
            export HOME=/run/heph-prerequisite/home
            nss_dir="$HOME/.local/share/pki/nssdb"
            mkdir -p "$nss_dir"
            trap '\''rm -rf "$HOME"'\'' EXIT
            certutil -N -d "sql:$nss_dir" --empty-password >/dev/null 2>&1
            certutil -A -d "sql:$nss_dir" -n heph-caddy-fixture -t "C,," -i "$HEPH_PREREQUISITE_CA"
            output=/run/heph-prerequisite/browser.html
            if timeout --kill-after=2s 15s "$browser" --headless=new --no-sandbox \
                --disable-gpu --disable-background-networking \
                --dump-dom "$HEPH_PREREQUISITE_URL" \
                >"$output" 2>/run/heph-prerequisite/browser.log; then
                if [[ "$HEPH_PREREQUISITE_EXPECTED" == pass ]]; then
                    if [[ "$HEPH_PREREQUISITE_REQUIRE_MARKER" == 1 ]]; then
                        grep -Fq heph-installed-ui-prerequisite "$output"
                    else
                        test -s "$output"
                        ! grep -Fq ERR_CERT_AUTHORITY_INVALID "$output"
                    fi
                else
                    grep -Fq ERR_CERT_AUTHORITY_INVALID "$output"
                fi
            else
                false
            fi
        ' >"${probe_log}" 2>&1; then
        probe_status=0
    else
        probe_status=$?
    fi
    return "${probe_status}"
}

if run_browser_probe /run/heph-prerequisite/caddy-ca.pem pass; then
    :
else
    probe_status=$?
    printf 'HEPH_GCP_COOKING event=installed-ui-prerequisite status=failed reason=browser-ca-or-page\n' >&2
    if python3 "${scanner}" "${fixture_root}" >/dev/null 2>&1; then
        print_probe_failure_context pass
    else
        retain_fixture=0
        printf 'Installed UI prerequisite diagnostics were withheld by the credential scanner.\n' >&2
    fi
    gcp_failure_marker playwright-run "${probe_status}" playwright-log playwright-failed
    exit "${probe_status}"
fi

if ! openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
    -subj /CN=heph-installed-ui-wrong-ca \
    -keyout "${fixture_root}/wrong-ca.key" -out "${fixture_root}/wrong-ca.pem" \
    >/dev/null 2>&1; then
    gcp_failure_marker browser-setup 1 setup-log setup-failed
    printf 'installed UI prerequisite could not create wrong-CA probe certificate\n' >&2
    exit 1
fi
chmod 0600 "${fixture_root}/wrong-ca.key" "${fixture_root}/wrong-ca.pem"
rm -f -- "${fixture_root}/wrong-ca.key"
if run_browser_probe /run/heph-prerequisite/wrong-ca.pem fail; then
    :
else
    probe_status=$?
    if ! grep -Fq ERR_CERT_AUTHORITY_INVALID "${fixture_root}/browser.html" 2>/dev/null; then
        gcp_failure_marker playwright-run "${probe_status}" playwright-log playwright-failed
    fi
    printf 'HEPH_GCP_COOKING event=installed-ui-prerequisite status=failed reason=wrong-ca-probe-failed\n' >&2
    if python3 "${scanner}" "${fixture_root}" >/dev/null 2>&1; then
        print_probe_failure_context fail
    else
        retain_fixture=0
        printf 'Installed UI prerequisite diagnostics were withheld by the credential scanner.\n' >&2
    fi
    exit "${probe_status}"
fi
if ! python3 "${scanner}" "${fixture_root}" >/dev/null 2>&1; then
    retain_fixture=0
    gcp_failure_marker playwright-run 1 playwright-log playwright-failed
    printf 'Installed UI prerequisite diagnostics were withheld by the credential scanner.\n' >&2
    exit 1
fi
printf 'HEPH_GCP_COOKING event=installed-ui-prerequisite status=pass image=%s port=%s\n' \
    "${image}" "${public_port}"
