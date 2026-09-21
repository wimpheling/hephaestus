#!/usr/bin/env bash
# Focused regression for libkrun failure diagnostics and container cleanup.
set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
runner="${script_dir}/run-libkrun-integration.sh"
root="$(mktemp -d /tmp/heph-libkrun-cleanup-test.XXXXXX)"
trap 'rm -rf -- "${root}"' EXIT

extract_function() {
    local name="$1"
    awk -v function_name="${name}" '
        $0 == function_name "() {" { capture = 1 }
        capture { print }
        capture && $0 == "}" { exit }
    ' "${runner}"
}

diagnostics_body="${root}/diagnostics-body.sh"
{
    printf '%s\n' 'set -Eeuo pipefail'
    extract_function diagnostics_body
    cat <<'SCRIPT'
postgres_container_name=fake-service
nats_container_name=''
zot_container_name=''
fixture_root=''
diagnostics_body
SCRIPT
} >"${diagnostics_body}"

fake_bin="${root}/bin"
mkdir -- "${fake_bin}"
cat >"${fake_bin}/podman" <<'PODMAN'
#!/usr/bin/env bash
set -Eeuo pipefail
mark() {
    [[ -n "${HEPH_FAKE_MARKER:-}" ]] || return 0
    printf '%s\n' "$1" >>"${HEPH_FAKE_MARKER}"
}
case "${1:-}" in
    container)
        [[ "${2:-}" == exists ]] || exit 0
        mark exists-entered
        if [[ "${HEPH_FAKE_PODMAN_MODE:-}" == exists-hang ]]; then
            trap '' TERM
            sleep 30
        fi
        ;;
    inspect)
        mark inspect-entered
        printf '  container=fake-service state=running exit=0\n'
        ;;
    logs)
        mark logs-entered
        log_count=0
        if [[ -n "${HEPH_FAKE_MARKER:-}" ]]; then
            log_count="$(grep -c '^logs-entered$' "${HEPH_FAKE_MARKER}" || true)"
        fi
        if [[ "${HEPH_FAKE_PODMAN_MODE:-}" == logs-hang && "${log_count}" == 1 ]]; then
            trap '' TERM
            sleep 30
        fi
        ;;
    rm)
        printf '%s\n' "$*" >>"${HEPH_FAKE_RECORD:?}"
        ;;
    *)
        ;;
esac
PODMAN
chmod 700 -- "${fake_bin}/podman"

for mode in exists-hang logs-hang; do
    marker="${root}/${mode}.marker"
    set +e
    HEPH_FAKE_PODMAN_MODE="${mode}" HEPH_FAKE_MARKER="${marker}" PATH="${fake_bin}:${PATH}" \
        timeout --kill-after=1s 15s bash "${diagnostics_body}" \
        >"${root}/diagnostics-${mode}.log" 2>&1
    diagnostics_status="$?"
    set -e
    [[ "${diagnostics_status}" == 0 ]] || {
        cat -- "${root}/diagnostics-${mode}.log" >&2
        printf 'bounded diagnostics regression failed for %s with status %s\n' \
            "${mode}" "${diagnostics_status}" >&2
        exit 1
    }
    grep -Fxq exists-entered "${marker}" || {
        printf 'bounded diagnostics regression did not enter exists for %s\n' "${mode}" >&2
        exit 1
    }
    if [[ "${mode}" == logs-hang ]]; then
        grep -Fxq inspect-entered "${marker}" || {
            printf 'bounded diagnostics regression did not enter inspect\n' >&2
            exit 1
        }
        grep -Fxq logs-entered "${marker}" || {
            printf 'bounded diagnostics regression did not enter logs\n' >&2
            exit 1
        }
    fi
done

cleanup_function="${root}/cleanup-function.sh"
extract_function cleanup >"${cleanup_function}"
cleanup_script="${root}/cleanup.sh"
{
    printf '%s\n' 'set -Eeuo pipefail'
    extract_function redact_diagnostics
    extract_function diagnostics_body
    extract_function failure_diagnostics | sed '1s/^failure_diagnostics()/real_failure_diagnostics()/'
    cat -- "${cleanup_function}"
    cat <<'SCRIPT'
record="${HEPH_CLEANUP_RECORD:?}"
container_name=runtime
nats_container_name=nats
zot_container_name=zot
postgres_container_name=postgres
builder_image_loaded=false
verifier_image_loaded=false
fixture_root=''
diagnostics_dir=''
phase_timing_finish_open() { :; }
heph_shell_failure_on_exit() { :; }
heph_shell_failure_begin_cleanup() { :; }
cleanup_cgroup() { :; }
postgres_lifecycle_snapshot() { :; }
failure_diagnostics() {
    printf '%s\n' term-injected >>"${HEPH_FAKE_MARKER:?}"
    kill -TERM "$$"
    real_failure_diagnostics
}
trap cleanup EXIT
trap 'exit 143' TERM
kill -TERM "$$"
SCRIPT
} >"${cleanup_script}"

record="${root}/removed.txt"
set +e
HEPH_CLEANUP_RECORD="${record}" HEPH_FAKE_RECORD="${record}" \
    HEPH_FAKE_PODMAN_MODE=logs-hang HEPH_FAKE_MARKER="${root}/cleanup.marker" \
    PATH="${fake_bin}:${PATH}" \
    bash "${cleanup_script}" >"${root}/cleanup.log" 2>&1
cleanup_status="$?"
set -e
[[ "${cleanup_status}" == 143 ]] || {
    printf 'cleanup status changed from TERM: %s\n' "${cleanup_status}" >&2
    exit 1
}
for container in runtime nats zot postgres; do
    grep -Fxq "rm --force ${container}" "${record}" || {
        printf 'cleanup did not remove %s\n' "${container}" >&2
        exit 1
    }
done
grep -Fxq term-injected "${root}/cleanup.marker" || {
    printf 'cleanup regression did not inject TERM during diagnostics\n' >&2
    exit 1
}
grep -Fxq logs-entered "${root}/cleanup.marker" || {
    printf 'cleanup regression did not enter real diagnostics logs\n' >&2
    exit 1
}

printf 'libkrun cleanup regression passed\n'
