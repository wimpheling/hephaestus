#!/usr/bin/env bash
# Run the deterministic cooking applications through the real joined fixture.
set -Eeuo pipefail

# GNU timeout intentionally places its command in a separate process group so
# the deadline reaches nested guests.  When this entrypoint is started from an
# interactive PTY, that group can otherwise be stopped by terminal job
# control. Detach once at the boundary while retaining the caller's streams;
# CI and redirected invocations do not need the re-exec.
if [[ -t 0 || -t 1 || -t 2 ]] &&
    [[ "${HEPHAESTUS_COOKING_SESSION_DETACHED:-0}" != "1" ]]; then
    command -v setsid >/dev/null 2>&1 || {
        printf 'Cooking PTY execution requires setsid.\n' >&2
        exit 1
    }
    export HEPHAESTUS_COOKING_SESSION_DETACHED=1
    exec setsid --fork --wait "${BASH_SOURCE[0]}" "$@"
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd -- "${script_dir}/../.." && pwd -P)"
cooking_root="${HEPHAESTUS_COOKING_SOURCE_ROOT:-${script_dir}}"
source "${repo_root}/scripts/shell-failure-diagnostics.sh"
heph_shell_failure_init cooking-run cooking
tmp_root="${TMPDIR:-/var/tmp}"
libkrun_tmp_root="${HEPHAESTUS_LIBKRUN_TMP_ROOT:-/var/tmp}"
for path in "${tmp_root}" "${libkrun_tmp_root}"; do
    [[ "${path}" = /* && ! -L "${path}" && -d "${path}" ]] || {
        printf 'Cooking temporary roots must be absolute, existing, non-symlink directories.\n' >&2
        exit 1
    }
done
export TMPDIR="${tmp_root}"
export HEPHAESTUS_LIBKRUN_TMP_ROOT="${libkrun_tmp_root}"
python_image="${HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE:?set an immutable python-ubuntu image reference}"
rust_builder_image="${HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE:?set an immutable rust-ubuntu builder image reference}"
[[ "${python_image}" == *@sha256:* ]] || {
    printf 'The cooking fixture requires a digest-pinned Python guest image.\n' >&2
    exit 1
}
[[ "${rust_builder_image}" == *@sha256:* ]] || {
    printf 'The cooking fixture requires a digest-pinned Rust builder guest image.\n' >&2
    exit 1
}
timeout_seconds="${HEPHAESTUS_COOKING_TIMEOUT_SECONDS:-900}"
[[ "${timeout_seconds}" =~ ^[1-9][0-9]*$ ]] || {
    printf 'HEPHAESTUS_COOKING_TIMEOUT_SECONDS must be a positive integer.\n' >&2
    exit 1
}
diagnostics_dir="${HEPHAESTUS_COOKING_DIAGNOSTICS_DIR:-}"
if [[ -n "${diagnostics_dir}" ]]; then
    [[ "${diagnostics_dir}" = /* && ! -L "${diagnostics_dir}" ]] || {
        printf 'HEPHAESTUS_COOKING_DIAGNOSTICS_DIR must be an absolute non-symlink path.\n' >&2
        exit 1
    }
    mkdir -p -- "${diagnostics_dir}"
    chmod 700 -- "${diagnostics_dir}"
fi
readonly script_dir repo_root cooking_root tmp_root libkrun_tmp_root python_image rust_builder_image timeout_seconds diagnostics_dir
phase_timing_path="${HEPH_GCP_PHASE_TIMING_PATH:-}"
phase_timing_source_sha="${HEPH_GCP_PHASE_TIMING_SOURCE_SHA:-}"
phase_timing_run_id="${HEPH_GCP_PHASE_TIMING_RUN_ID:-${GITHUB_RUN_ID:-manual}}"
phase_timing_attempt="${HEPH_GCP_PHASE_TIMING_ATTEMPT:-${GITHUB_RUN_ATTEMPT:-1}}"
if [[ -n "${phase_timing_path}" && -z "${phase_timing_source_sha}" ]]; then
    phase_timing_source_sha="$(git -C "${repo_root}" rev-parse HEAD)"
fi
phase_timing_open=''

phase_timing_start() {
    local name="$1"
    [[ -n "$phase_timing_path" ]] || return 0
    local source_sha="${HEPH_GCP_PHASE_TIMING_SOURCE_SHA:-}"
    [[ -n "$source_sha" ]] || source_sha="$(git -C "$repo_root" rev-parse HEAD)"
    python3 "$repo_root/scripts/gcp_phase_timing.py" start \
        --path "$phase_timing_path" --phase "$name" --trust workload \
        --clock-domain workload --run-id "$phase_timing_run_id" \
        --attempt "$phase_timing_attempt" --source-sha "$source_sha"
    phase_timing_open="$name"
}

phase_timing_end() {
    local name="$1" outcome="$2"
    [[ -n "$phase_timing_path" ]] || return 0
    local source_sha="${HEPH_GCP_PHASE_TIMING_SOURCE_SHA:-}"
    [[ -n "$source_sha" ]] || source_sha="$(git -C "$repo_root" rev-parse HEAD)"
    python3 "$repo_root/scripts/gcp_phase_timing.py" end \
        --path "$phase_timing_path" --phase "$name" --trust workload \
        --clock-domain workload --run-id "$phase_timing_run_id" \
        --attempt "$phase_timing_attempt" --source-sha "$source_sha" --outcome "$outcome"
    phase_timing_open=''
}

phase_timing_finish_open() {
    local status="$1" outcome='failed'
    [[ -n "$phase_timing_open" ]] || return 0
    if ((status == 124)); then
        outcome='timed-out'
    elif ((status == 130 || status == 143)); then
        outcome='cancelled'
    fi
    phase_timing_end "$phase_timing_open" "$outcome" || true
}

# Bash can run an EXIT trap with status zero after a direct signal. Convert
# TERM and INT to their conventional exit codes first so the
# timing record and the caller preserve cancellation as cancelled.
cooking_signal_exit() {
    case "$1" in
        TERM) exit 143 ;;
        INT) exit 130 ;;
        *) exit 1 ;;
    esac
}
trap 'cooking_signal_exit TERM' TERM
trap 'cooking_signal_exit INT' INT

# The deadline includes the browser fixture bootstrap as well as the joined
# cooking run.  This keeps dependency installation from consuming an
# unbounded amount of time before the VM timeout starts.
overall_deadline=$((SECONDS + timeout_seconds))
bridge_deadline_epoch=$(( $(date +%s) + timeout_seconds ))
run_timeout_seconds="${timeout_seconds}"

browser_oidc_pid=""
browser_fixture=""
browser_bridge_pid=""
browser_bridge_dir=""
browser_cleanup() {
    local status=$?
    phase_timing_finish_open "${status}"
    heph_shell_failure_on_exit "${status}" "${LINENO}"
    heph_shell_failure_begin_cleanup
    if [[ -n "${browser_oidc_pid}" ]]; then
        kill "${browser_oidc_pid}" >/dev/null 2>&1 || true
        wait "${browser_oidc_pid}" 2>/dev/null || true
    fi
    if [[ -n "${browser_bridge_pid}" ]]; then
        kill "${browser_bridge_pid}" >/dev/null 2>&1 || true
        wait "${browser_bridge_pid}" 2>/dev/null || true
    fi
    if [[ -n "${browser_fixture}" ]]; then
        if [[ -z "${diagnostics_dir}" ]]; then
            rm -f -- "${browser_fixture}" "${browser_fixture}.oidc.log"
        fi
    fi
    if [[ -n "${browser_bridge_dir}" && -z "${diagnostics_dir}" ]]; then
        rm -rf -- "${browser_bridge_dir}"
    fi
    return "${status}"
}
trap browser_cleanup EXIT

if [[ "${HEPHAESTUS_COOKING_BROWSER_E2E:-1}" == "1" ]]; then
    phase_timing_start browser-setup
    command -v node >/dev/null || {
        printf 'Cooking browser E2E requires node for the local OIDC fixture.\n' >&2
        exit 1
    }
    browser_oidc_port="$(python3 -c 'import socket; sock=socket.socket(); sock.bind(("127.0.0.1", 0)); print(sock.getsockname()[1]); sock.close()')"
    browser_web_port="$(python3 -c 'import socket; sock=socket.socket(); sock.bind(("127.0.0.1", 0)); print(sock.getsockname()[1]); sock.close()')"
    browser_oidc_url="http://127.0.0.1:${browser_oidc_port}"
    if [[ -n "${diagnostics_dir}" ]]; then
        browser_bridge_dir="$(mktemp -d "${diagnostics_dir}/browser-bridge.XXXXXX")"
        chmod 700 -- "${browser_bridge_dir}"
        browser_fixture="${browser_bridge_dir}/fixture.json"
        : >"${browser_fixture}"
        chmod 600 -- "${browser_fixture}"
    else
        browser_bridge_dir="$(mktemp -d "${tmp_root}/heph-cooking-browser-bridge.XXXXXX")"
        chmod 700 -- "${browser_bridge_dir}"
        browser_fixture="${browser_bridge_dir}/fixture.json"
        : >"${browser_fixture}"
        chmod 600 -- "${browser_fixture}"
    fi
    readonly browser_bridge_dir browser_fixture
    "${repo_root}/scripts/run-ui-e2e-host-bridge.sh" \
        "${browser_bridge_dir}" "${diagnostics_dir}" "${bridge_deadline_epoch}" &
    browser_bridge_pid="$!"
    for _attempt in {1..50}; do
        [[ -f "${browser_bridge_dir}/.ready" ]] && break
        kill -0 "${browser_bridge_pid}" >/dev/null 2>&1 || {
            printf 'Cooking browser host bridge exited during startup.\n' >&2
            exit 1
        }
        sleep 0.1
    done
    [[ -f "${browser_bridge_dir}/.ready" ]] || {
        printf 'Cooking browser host bridge did not become ready.\n' >&2
        exit 1
    }
    remaining_seconds=$((overall_deadline - SECONDS))
    (( remaining_seconds > 0 )) || {
        printf 'Cooking E2E deadline elapsed during browser setup.\n' >&2
        exit 124
    }
    (cd "${repo_root}/e2e/playwright" && \
        timeout --kill-after=30s "${remaining_seconds}s" npm ci >/dev/null)
    export HEPHAESTUS_COOKING_BROWSER_E2E=1
    export HEPHAESTUS_COOKING_BROWSER_OIDC_ISSUER="${browser_oidc_url}"
    export HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER="${browser_oidc_url}"
    export HEPHAESTUS_E2E_EXTERNAL_WEB_PORT="${browser_web_port}"
    export HEPHAESTUS_COOKING_BROWSER_FIXTURE_OUTPUT="${browser_fixture}"
    export HEPHAESTUS_COOKING_BROWSER_BRIDGE_DIR="${browser_bridge_dir}"
    export HEPHAESTUS_COOKING_BRIDGE_DEADLINE_EPOCH="${bridge_deadline_epoch}"
    HEPHAESTUS_E2E_OIDC_PORT="${browser_oidc_port}" \
    HEPHAESTUS_E2E_WEB_URL="http://127.0.0.1:${browser_web_port}" \
    HEPHAESTUS_E2E_OIDC_REVIEWER_SUBJECT="golden-subject" \
        node "${repo_root}/e2e/playwright/oidc-provider.mjs" \
        >"${browser_fixture}.oidc.log" 2>&1 &
    browser_oidc_pid="$!"
    for _attempt in {1..100}; do
        if curl --fail --silent --connect-timeout 1 --max-time 2 \
            "${browser_oidc_url}/.well-known/openid-configuration" >/dev/null 2>&1; then
            break
        fi
        sleep 0.1
    done
    curl --fail --silent --connect-timeout 1 --max-time 2 \
        "${browser_oidc_url}/.well-known/openid-configuration" >/dev/null
    run_timeout_seconds=$((overall_deadline - SECONDS))
    (( run_timeout_seconds > 0 )) || {
        printf 'Cooking E2E deadline elapsed during browser setup.\n' >&2
        exit 124
    }
    phase_timing_end browser-setup passed
fi

redact_diagnostics() {
    sed -u -E \
        -e 's/(authorization:[[:space:]]*Bearer[[:space:]]+)[^[:space:]]+/\1[REDACTED]/Ig' \
        -e 's/(x-telegram-bot-api-secret-token:[[:space:]]*)[^[:space:]]+/\1[REDACTED]/Ig' \
        -e 's/(HEPHAESTUS_[A-Z0-9_]*(SECRET|TOKEN|KEY)[A-Z0-9_]*=)[^[:space:]]+/\1[REDACTED]/g' \
        -e 's/golden-brokered-provider-sentinel-5d1a/[REDACTED]/g' \
        -e 's/cooking-inbound-only-fixture-sentinel/[REDACTED]/g' \
        -e 's/cooking-model-only-fixture-sentinel-724c/[REDACTED]/g' \
        -e 's/cooking-relay-only-fixture-sentinel-819e/[REDACTED]/g' \
        -e 's/cooking-model-rotated-fixture-sentinel-936f/[REDACTED]/g' \
        -e 's/cooking-inbound-rotated-fixture-sentinel-157a/[REDACTED]/g' \
        -e 's/cooking-relay-rotated-fixture-sentinel-482b/[REDACTED]/g'
}

diagnostics_body() {
    local status="$1"
    printf 'Cooking E2E failed (status %s); redacted host diagnostics:\n' "${status}"
    printf '  host=%s arch=%s uid=%s:%s timeout=%ss\n' \
        "$(uname -srm)" "$(uname -m)" "$(id -u)" "$(id -g)" "${timeout_seconds}"
    printf '  python-image=%s\n' "${python_image}"
    printf '  rust-builder-image=%s\n' "${rust_builder_image}"
    if command -v podman >/dev/null 2>&1; then
        for prefix in hephaestus-gateway-libkrun-caddy hephaestus-golden- hephaestus-ui-; do
            podman ps --all --filter "name=${prefix}" \
                --format '  container={{.Names}} state={{.State}} exit={{.ExitCode}}' 2>&1 || true
        done
    fi
}

failure_diagnostics() {
    local status="$1"
    if [[ -n "${diagnostics_dir}" ]]; then
        local report
        report="$(mktemp "${diagnostics_dir}/cooking-failure.XXXXXX.log")"
        diagnostics_body "${status}" | redact_diagnostics >"${report}" || true
        chmod 600 -- "${report}"
        cat -- "${report}" >&2
        printf '  retained-report=%s\n' "${report}" >&2
    else
        diagnostics_body "${status}" | redact_diagnostics >&2 || true
    fi
}

# Keep preflight, compilation and the VM process in one process group so the
# deadline covers all work and timeout can terminate nested guests together.
run_cooking() {
    timeout --kill-after=30s "${run_timeout_seconds}s" env \
    HEPHAESTUS_APP_COOKING_E2E=1 \
    HEPHAESTUS_APP_COOKING_BUILD_PROOF=1 \
    HEPHAESTUS_COOKING_UPDATE_E2E=1 \
    HEPHAESTUS_COOKING_OCI_BASE_IMPORT_DIAGNOSTIC=0 \
    HEPHAESTUS_APP_UPDATE_ADMISSION_E2E=0 \
    HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E=0 \
    HEPHAESTUS_COOKING_SOURCE_ROOT="${cooking_root}" \
    HEPHAESTUS_COOKING_GATEWAY_ARTIFACT="${cooking_root}/cooking-gateway/target/x86_64-unknown-linux-musl/release/cooking-gateway" \
    HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE="${python_image}" \
    HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE="${rust_builder_image}" \
    HEPH_GCP_PHASE_TIMING_PATH="${phase_timing_path}" \
    HEPH_GCP_PHASE_TIMING_SOURCE_SHA="${phase_timing_source_sha}" \
    HEPH_GCP_PHASE_TIMING_RUN_ID="${phase_timing_run_id}" \
    HEPH_GCP_PHASE_TIMING_ATTEMPT="${phase_timing_attempt}" \
        bash -Eeuo pipefail -c '
            timing_helper="$4"
            phase_timing_open=""
            workload_stage=initialization
            workload_detail_stage=workload-boundary
            workload_detail_active=0
            workload_failure_emitted=0
            workload_detail_stage_start() {
                case "$1" in
                    timing-helper-start|preflight-command-checks|rustup-target|cargo-build|timing-helper-end|gateway-invocation) ;;
                    *) return 1 ;;
                esac
                workload_detail_stage="$1"
                workload_detail_active=1
                printf "HEPH_GCP_COOKING event=workload-step operation=cooking-workload phase=cooking stage=%s status=start\\n" "$workload_detail_stage"
            }
            workload_detail_stage_pass() {
                local stage="$1"
                case "$stage" in
                    timing-helper-start|preflight-command-checks|rustup-target|cargo-build|timing-helper-end|gateway-invocation) ;;
                    *) return 1 ;;
                esac
                printf "HEPH_GCP_COOKING event=workload-step operation=cooking-workload phase=cooking stage=%s status=passed\\n" "$stage"
                workload_detail_active=0
            }
            workload_detail_stage_fail() {
                local status="$1"
                ((workload_detail_active == 1)) || return 0
                if ((status > 255)); then status=255; fi
                printf "HEPH_GCP_COOKING event=workload-step operation=cooking-workload phase=cooking stage=%s status=failed exit_code=%s\\n" "$workload_detail_stage" "$status"
                workload_detail_active=0
            }
            workload_step_start() {
                case "$1" in
                    dependency-setup|project-build|gateway-e2e) ;;
                    *) return 1 ;;
                esac
                workload_stage="$1"
                printf "HEPH_GCP_COOKING event=workload-step operation=cooking-workload phase=cooking stage=%s status=start\\n" "$workload_stage"
                if [[ "$workload_stage" != gateway-e2e ]]; then
                    workload_detail_stage_start timing-helper-start
                    phase_timing_start "$workload_stage"
                    workload_detail_stage_pass timing-helper-start
                fi
            }
            workload_step_pass() {
                local step="$1"
                if [[ "$step" != gateway-e2e ]]; then
                    workload_detail_stage_start timing-helper-end
                    phase_timing_end "$step" passed
                    workload_detail_stage_pass timing-helper-end
                fi
                printf "HEPH_GCP_COOKING event=workload-step operation=cooking-workload phase=cooking stage=%s status=passed\\n" "$step"
            }
            workload_failure() {
                local status="$1"
                ((status != 0)) || return 0
                ((workload_failure_emitted == 0)) || return 0
                workload_failure_emitted=1
                if ((status > 255)); then status=255; fi
                printf "HEPH_GCP_COOKING event=workload-step operation=cooking-workload phase=cooking stage=%s status=failed exit_code=%s\\n" "$workload_stage" "$status"
                workload_detail_stage_fail "$status"
            }
            phase_timing_start() {
                local name="$1"
                [[ -n "${HEPH_GCP_PHASE_TIMING_PATH:-}" ]] || return 0
                python3 "$timing_helper" start --path "$HEPH_GCP_PHASE_TIMING_PATH" \
                    --phase "$name" --trust workload --clock-domain workload \
                    --run-id "${HEPH_GCP_PHASE_TIMING_RUN_ID:-manual}" \
                    --attempt "${HEPH_GCP_PHASE_TIMING_ATTEMPT:-1}" \
                    --source-sha "$HEPH_GCP_PHASE_TIMING_SOURCE_SHA"
                phase_timing_open="$name"
            }
            phase_timing_end() {
                local name="$1" outcome="$2"
                [[ -n "${HEPH_GCP_PHASE_TIMING_PATH:-}" ]] || return 0
                python3 "$timing_helper" end --path "$HEPH_GCP_PHASE_TIMING_PATH" \
                    --phase "$name" --trust workload --clock-domain workload \
                    --run-id "${HEPH_GCP_PHASE_TIMING_RUN_ID:-manual}" \
                    --attempt "${HEPH_GCP_PHASE_TIMING_ATTEMPT:-1}" \
                    --source-sha "$HEPH_GCP_PHASE_TIMING_SOURCE_SHA" --outcome "$outcome"
                phase_timing_open=""
            }
            phase_timing_finish() {
                local status=$? outcome=failed
                trap - EXIT
                if [[ -n "$phase_timing_open" ]]; then
                    if ((status == 124)); then outcome=timed-out; fi
                    if ((status == 130 || status == 143)); then outcome=cancelled; fi
                    workload_detail_stage_start timing-helper-end
                    if phase_timing_end "$phase_timing_open" "$outcome"; then
                        workload_detail_stage_pass timing-helper-end
                    else
                        timing_status=$?
                        workload_detail_stage_fail "$timing_status"
                    fi
                fi
                exit "$status"
            }
            workload_signal_exit() {
                case "$1" in
                    TERM) exit 143 ;;
                    INT) exit 130 ;;
                    *) exit 1 ;;
                esac
            }
            trap '\''workload_signal_exit TERM'\'' TERM
            trap '\''workload_signal_exit INT'\'' INT
            trap '\''workload_failure "$?"'\'' ERR
            trap phase_timing_finish EXIT
            workload_step_start dependency-setup
            # Temporary reviewed acceptance fixture: remove after the lifecycle
            # run, or replace this one command with the bounded timeout variant
            # documented with the test evidence.  It intentionally takes no
            # workflow-controlled value.
            timeout --kill-after=1s 1s sleep 2
            workload_detail_stage_start preflight-command-checks
            "$1/preflight.sh"
            workload_detail_stage_pass preflight-command-checks
            workload_step_pass dependency-setup
            cd -- "$2/cooking-gateway"
            workload_step_start project-build
            workload_detail_stage_start rustup-target
            rust_target=x86_64-unknown-linux-musl
            installed_rust_targets="$(rustup target list --installed)"
            if ! grep -Fqx "$rust_target" <<<"$installed_rust_targets"; then
                rustup target add "$rust_target"
            fi
            workload_detail_stage_pass rustup-target
            workload_detail_stage_start cargo-build
            cargo build --locked --offline --release --target x86_64-unknown-linux-musl
            workload_detail_stage_pass cargo-build
            workload_step_pass project-build
            cd -- "$3/.."
            workload_step_start gateway-e2e
            workload_detail_stage_start gateway-invocation
            "$3/run-gateway-libkrun-e2e.sh"
            workload_detail_stage_pass gateway-invocation
            workload_step_pass gateway-e2e
        ' -- "${script_dir}" "${cooking_root}" "${repo_root}/scripts" \
        "${repo_root}/scripts/gcp_phase_timing.py"
}

run_with_diagnostics() {
    if [[ -n "${diagnostics_dir}" ]]; then
        local execution_log
        execution_log="$(mktemp "${diagnostics_dir}/cooking-execution.XXXXXX.log")"
        printf 'Cooking execution log: %s\n' "${execution_log}"
        # Scan the raw execution stream before display redaction.  The stream
        # scanner retains only a bounded chunk overlap, redacts all matched
        # fixture encodings, and returns failure at EOF when it observed one.
        run_cooking 2>&1 |
            python3 -u "${repo_root}/scripts/check-browser-evidence.py" --stream |
            redact_diagnostics | tee -- "${execution_log}"
    else
        run_cooking 2>&1 |
            python3 -u "${repo_root}/scripts/check-browser-evidence.py" --stream |
            redact_diagnostics
    fi
}

if run_with_diagnostics; then
    exit 0
else
    status="$?"
    heph_shell_failure_on_exit "${status}" "${LINENO}"
    if [[ "${status}" -eq 124 || "${status}" -eq 137 ]]; then
        printf 'Cooking E2E exceeded its %ss timeout.\n' "${timeout_seconds}" >&2
    fi
    failure_diagnostics "${status}"
    exit "${status}"
fi
