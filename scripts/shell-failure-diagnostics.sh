#!/usr/bin/env bash
# Emit a small, allowlisted failure record from the disposable shell runners.
# The record deliberately excludes BASH_COMMAND, arguments, paths, and error
# text so it remains safe for the bounded diagnostics collector.

heph_shell_failure_init() {
    local script="$1"
    local component="$2"
    case "${script}:${component}" in
        cooking-run:cooking|gateway-libkrun-e2e:gateway|libkrun-integration:libkrun) ;;
        *) return 2 ;;
    esac
    HEPH_SHELL_FAILURE_SCRIPT="${script}"
    HEPH_SHELL_FAILURE_COMPONENT="${component}"
    HEPH_SHELL_FAILURE_EMITTED=0
    HEPH_SHELL_FAILURE_DISABLED=0
    trap 'heph_shell_failure_err "$?" "${LINENO:-0}"' ERR
}

heph_shell_failure_emit() {
    local operation="$1"
    local reason="$2"
    local status="$3"
    local line="$4"
    [[ "${status}" =~ ^[0-9]+$ && "${status}" -gt 0 ]] || return 0
    [[ "${HEPH_SHELL_FAILURE_DISABLED:-0}" == 0 ]] || return 0
    [[ "${HEPH_SHELL_FAILURE_EMITTED:-0}" == 0 ]] || return 0
    case "${operation}" in
        preflight|command|cgroup-events|runtime-cleanup|cgroup-cleanup|network-integrity|gateway-cleanup|cooking-cleanup) ;;
        *) operation=command; reason=command-failed ;;
    esac
    case "${reason}" in
        command-failed|missing-input|assertion-mismatch|network-mismatch|read-failed|signal|process-exit|timeout) ;;
        *) reason=command-failed ;;
    esac
    [[ "${status}" =~ ^[0-9]+$ && "${status}" -le 255 ]] || status=1
    [[ "${line}" =~ ^[1-9][0-9]*$ ]] || line=1
    HEPH_SHELL_FAILURE_EMITTED=1
    printf 'HEPH_GCP_SHELL_FAILURE script=%s component=%s operation=%s reason=%s exit_code=%s line=%s\n' \
        "${HEPH_SHELL_FAILURE_SCRIPT}" "${HEPH_SHELL_FAILURE_COMPONENT}" \
        "${operation}" "${reason}" "${status}" "${line}" >&2
}

heph_shell_failure_err() {
    local status="$1"
    local line="$2"
    (( status != 0 )) || return 0
    heph_shell_failure_emit command command-failed "${status}" "${line}"
}

heph_shell_failure_die() {
    local operation="$1"
    local reason="$2"
    local status="${3:-1}"
    local line="${4:-${BASH_LINENO[0]:-1}}"
    heph_shell_failure_emit "${operation}" "${reason}" "${status}" "${line}"
}

heph_shell_failure_on_exit() {
    local status="$1"
    local line="${2:-${BASH_LINENO[0]:-1}}"
    (( status == 0 )) && return 0
    if (( status == 124 || status == 137 )); then
        heph_shell_failure_emit command timeout "${status}" "${line}"
    elif (( status == 130 || status == 143 )); then
        heph_shell_failure_emit command signal "${status}" "${line}"
    else
        heph_shell_failure_emit command process-exit "${status}" "${line}"
    fi
}

heph_shell_failure_begin_cleanup() {
    # Cleanup is best effort after the original status has been captured. Its
    # own ERR events must never replace the primary failure classification.
    HEPH_SHELL_FAILURE_DISABLED=1
}
