#!/usr/bin/env bash
# Root startup script for one disposable GCE integration VM.
# Smoke mode has no service account; diagnostic and Cooking modes use only the
# reviewed runtime identity for their private cache/evidence object operations.
# The VM fetches only this public repo at the exact SHA metadata value.
set -Eeuo pipefail
umask 077

# A baked image installs the shared pin/verifier library. Stock Ubuntu keeps
# the fallbacks below so the transition path remains usable.
runner_image_helper="${HEPH_GCP_IMAGE_BAKE_HELPER:-/usr/local/libexec/hephaestus/gcp-runner-image-provision.sh}"
if [[ -f "$runner_image_helper" && ! -L "$runner_image_helper" ]]; then
  # shellcheck source=/dev/null
  source "$runner_image_helper"
fi

readonly metadata_root='http://metadata.google.internal/computeMetadata/v1'
readonly repository_url='https://github.com/wimpheling/hephaestus.git'
readonly forge_uid=10001
readonly forge_gid=10001
readonly rust_version="${HEPH_IMAGE_RUST_VERSION:-1.88.0}"
readonly libkrun_tag="${HEPH_IMAGE_LIBKRUN_TAG:-v1.19.0}"
readonly libkrun_revision_pin="${HEPH_IMAGE_LIBKRUN_REVISION:-9932c4b59d8f891e60c6aba20d22ebb99ceaa8e2}"
readonly libkrunfw_tag="${HEPH_IMAGE_LIBKRUNFW_TAG:-v5.5.0}"
readonly passt_revision="${HEPH_IMAGE_PASST_REVISION:-386b5f5472b89769c025f5d5056348532a823b93}"
readonly passt_source_url='https://passt.top/passt'
readonly work_root="${HEPH_GCP_WORK_ROOT:-/srv/hephaestus}"
readonly checkout_root="${work_root}/checkout"
readonly source_root="${work_root}/src"
readonly temporary_root="${work_root}/tmp"
# Noble's packaged passt AppArmor profile permits owner writes below /tmp and
# HOME; keep its socket, pid, and log beneath this forge-owned subtree.
readonly smoke_temporary_root='/tmp/hephaestus-libkrun'
readonly evidence_root="${work_root}/evidence"
readonly passt_preflight_path='/run/hephaestus/gcp-passt-preflight.sh'
readonly passt_profile_path='/etc/apparmor.d/usr.bin.passt'
readonly passt_local_profile_path='/etc/apparmor.d/local/usr.bin.passt'
readonly passt_profile_overlay='/run/hephaestus/usr.bin.passt'
readonly guest_image='docker.io/library/ubuntu@sha256:52df9b1ee71626e0088f7d400d5c6b5f7bb916f8f0c82b474289a4ece6cf3faf'
readonly log_file="${HEPH_GCP_LOG_FILE:-/var/log/hephaestus/gcp-kvm-startup.log}"
readonly diagnostics_bucket='hephaestus-508000-cooking-diagnostics'
readonly diagnostics_max_archive_bytes=67108864
readonly diagnostics_metadata_root="${HEPH_GCP_DIAGNOSTICS_METADATA_ROOT:-/run/hephaestus/diagnostics}"

phase='initializing'
revision='unknown'
trial_deadline=0
collection_deadline=0
vm_start_epoch=0
trial_deadline_epoch=0
collection_deadline_epoch=0
test_mode='smoke'
diagnostics_collection_status=0
diagnostics_uploaded=false
diagnostics_enabled=false
diagnostics_object_metadata=''
diagnostics_journal_unit=''
diagnostic_timeout_log=''
diagnostic_probe_completed=false
diagnostic_quarantine_validated=false
diagnostics_token_json=''
diagnostics_header_file=''
smoke_output_log=''
runner_image_manifest_sha=''
runner_image_browser_lock_sha=''
runner_image_browser_version=''
runner_image_libkrun_revision=''
runner_image_libkrunfw_tag=''

die() { printf 'gcp-kvm-startup: %s\n' "$*" >&2; return 1; }

retain_passt_host_audit() {
  local audit_path="${evidence_root}/integration/passt-host-audit.log"
  install -d -m 0700 "${evidence_root}/integration" 2>/dev/null || return 0
  install -m 0600 /dev/null "$audit_path" 2>/dev/null || return 0
  {
    printf 'Bounded AppArmor audit lines captured on failure:\n'
    dmesg --color=never 2>/dev/null |
      awk '
        {
          line = tolower($0)
          if (line ~ /apparmor/ &&
              (line ~ /profile="[^"]*passt[^"]*"/ || line ~ /comm="passt"/) &&
              (line ~ /denied/ || line ~ /audit/))
            print
        }
      ' |
      tail -n 100 || true
  } >"$audit_path" 2>&1 || true
  chmod 0600 "$audit_path" 2>/dev/null || true
  printf 'HEPH_GCP_PASST_HOST_AUDIT_BEGIN path=%s\n' "$audit_path"
  sed -n '1,101p' "$audit_path" 2>/dev/null || true
  printf 'HEPH_GCP_PASST_HOST_AUDIT_END path=%s\n' "$audit_path"
}

if [[ "${HEPH_GCP_STARTUP_LIBRARY:-0}" != 1 ]]; then
  install -d -m 0700 "$(dirname -- "$log_file")"
  install -m 0600 /dev/null "$log_file"
  [[ -w /dev/ttyS0 ]] || die 'GCE serial console /dev/ttyS0 is unavailable'
  # The caller writes this immediately before the Compute create request. It is
  # conservative for startup delays and is the single deadline anchor.
  exec > >(tee -a "$log_file" /dev/ttyS0) 2>&1
fi

finish() {
  local status=$?
  trap - EXIT
  if ((status != 0)) && [[ "$test_mode" == smoke ]]; then
    retain_passt_host_audit
  fi
  if [[ "$diagnostics_enabled" == true || "$test_mode" == diagnostic || "$test_mode" == gcp-cooking ]]; then
    set +e
    collect_diagnostics "${status}"
    diagnostics_collection_status=$?
    set -e
    if ((diagnostics_collection_status != 0)) && ((status == 0)); then
      status=1
    fi
  fi
  [[ -z "$diagnostics_token_json" ]] || rm -f -- "$diagnostics_token_json"
  [[ -z "$diagnostics_header_file" ]] || rm -f -- "$diagnostics_header_file"
  if [[ "$test_mode" == diagnostic ]]; then
    local expected_fixture=false diagnostic_result
    if ((diagnostics_collection_status == 0)) &&
        [[ "$diagnostic_probe_completed" == true ]] &&
        [[ "$diagnostic_quarantine_validated" == true ]] &&
        ((status == 42)) && [[ "$phase" == diagnostic-synthetic ]]; then
      expected_fixture=true
    fi
    if [[ "$expected_fixture" == true ]]; then
      diagnostic_result=expected-failure
    elif ((status == 124)); then
      diagnostic_result=timed-out
    else
      diagnostic_result=failed
    fi
    printf 'HEPHAESTUS_GCP_DIAGNOSTIC: TEST-FAIL expected=%s phase=%s exit=%s\n' \
      "$expected_fixture" "$phase" "$status"
    if [[ "$expected_fixture" == true ]]; then
      printf 'HEPHAESTUS_GCP_DIAGNOSTIC: DIAGNOSTICS PASS test_result=expected-failure\n'
    else
      printf 'HEPHAESTUS_GCP_DIAGNOSTIC: DIAGNOSTICS FAIL test_result=%s\n' "$diagnostic_result"
    fi
    exit "$status"
  fi
  if ((status == 0)); then
    if [[ "$test_mode" == gcp-cooking ]]; then
      printf 'HEPHAESTUS_GCP_COOKING: PASS\n'
    else
      printf 'HEPHAESTUS_GCP_KVM_SMOKE: PASS\n'
    fi
  else
    if [[ "$test_mode" == gcp-cooking ]]; then
      printf 'HEPHAESTUS_GCP_COOKING: FAIL phase=%s exit=%s revision=%s\n' \
        "$phase" "$status" "$revision"
    else
      printf 'HEPHAESTUS_GCP_KVM_SMOKE: FAIL phase=%s exit=%s revision=%s\n' \
        "$phase" "$status" "$revision"
    fi
  fi
  exit "$status"
}
trap finish EXIT

marker() {
  printf 'HEPH_GCP_KVM_STARTUP event=%s phase=%s revision=%s\n' "$1" "$phase" "$revision"
}

phase_start() { phase="$1"; marker phase-start; }
phase_pass() { marker phase-pass; }

metadata_value() {
  if [[ "${HEPH_GCP_IMAGE_BAKE:-0}" == 1 ]]; then
    case "$1" in
      github-sha) printf '%s\n' "${HEPH_GCP_IMAGE_BAKE_REPO_SHA:-0000000000000000000000000000000000000000}" ;;
      test-mode) printf '%s\n' smoke ;;
      trial-start-epoch) date +%s ;;
      passt-preflight-script) cat "${HEPH_GCP_LOCAL_PASST_PREFLIGHT:?}" ;;
      *) die "image bake does not support metadata attribute: $1" ;;
    esac
    return 0
  fi
  curl --fail --silent --show-error -H 'Metadata-Flavor: Google' \
    "${metadata_root}/instance/attributes/$1"
}

metadata_optional_value() {
  curl --fail --silent --show-error -H 'Metadata-Flavor: Google' \
    "${metadata_root}/instance/attributes/$1" 2>/dev/null || true
}

require_command() { command -v "$1" >/dev/null 2>&1 || die "missing command: $1"; }

stage_diagnostics_metadata() {
  [[ "$diagnostics_enabled" == true || "$test_mode" == diagnostic || "$test_mode" == gcp-cooking ]] || return 0
  # Fetch these before image verification so finish() can collect an early
  # custom-image failure through the metadata-provided private pipeline.
  install -d -m 0700 "$diagnostics_metadata_root"
  metadata_value diagnostics-collector-script >"$diagnostics_metadata_root/collect-cooking-diagnostics.py"
  metadata_value diagnostics-scanner-script >"$diagnostics_metadata_root/check-browser-evidence.py"
  chmod 0700 "$diagnostics_metadata_root" "$diagnostics_metadata_root"/*.py
}

diagnostics_object() {
  local object="${HEPH_GCP_DIAGNOSTICS_OBJECT:-$diagnostics_object_metadata}"
  [[ -n "$object" && "$object" != *..* && "$object" =~ ^cooking/runs/[0-9]+/[0-9]+/[0-9a-f]{40}\.tar\.gz$ ]] ||
    die 'diagnostics object metadata is missing or unsafe'
  printf '%s\n' "$object"
}

bounded_copy() {
  local source="$1" destination="$2"
  [[ -f "$source" && ! -L "$source" ]] || return 1
  # The collector's per-source budget is 16 MiB. Keep the newest bounded tail
  # so a large serial log cannot consume the bundle budget.
  tail -c 8388608 -- "$source" >"$destination"
  chmod 0600 "$destination"
}

bounded_copy_status() {
  local source="$1" label="$2" destination="$3" bytes retained
  if [[ ! -f "$source" || -L "$source" ]]; then
    printf 'HEPH_GCP_DIAGNOSTICS source=%s status=missing\n' "$label"
    return 1
  fi
  bytes="$(stat -c '%s' -- "$source")" || return 1
  if bounded_copy "$source" "$destination"; then
    retained="$(stat -c '%s' -- "$destination")" || return 1
    if ((bytes > retained)); then
      printf 'HEPH_GCP_DIAGNOSTICS source=%s status=truncated original_bytes=%s retained_bytes=%s\n' \
        "$label" "$bytes" "$retained"
    else
      printf 'HEPH_GCP_DIAGNOSTICS source=%s status=retained bytes=%s\n' "$label" "$retained"
    fi
    return 0
  fi
  printf 'HEPH_GCP_DIAGNOSTICS source=%s status=unavailable\n' "$label"
  return 1
}

collect_diagnostics() {
  local test_status="$1" collector scanner output_root input_root archive snapshot snapshot_input snapshot_status_path status_json cooking_evidence_root
  local browser_summary_source journal_unit copy_status serial_copy_status
  local object token encoded_object upload_status
  object="$(diagnostics_object)" || return 1
  collector="$checkout_root/scripts/collect-cooking-diagnostics.py"
  scanner="$checkout_root/scripts/check-browser-evidence.py"
  if [[ ! -f "$collector" || -L "$collector" ]]; then
    collector="$diagnostics_metadata_root/collect-cooking-diagnostics.py"
    scanner="$diagnostics_metadata_root/check-browser-evidence.py"
  fi
  [[ -f "$collector" && ! -L "$collector" && -f "$scanner" && ! -L "$scanner" ]] || {
    printf 'HEPH_GCP_DIAGNOSTICS event=collection status=fail reason=collector-unavailable\n'
    return 1
  }
  output_root="${evidence_root}/cooking-diagnostics"
  input_root="${temporary_root}/diagnostics-input"
  archive="${temporary_root}/cooking-diagnostics.tar.gz"
  status_json="${input_root}/runtime-structured.json"
  rm -rf -- "$output_root" "$input_root" "$archive"
  # Collection also runs in diagnostic mode, whose image intentionally has no
  # forge account.  Root owns this staging area; the collector only reads it.
  install -d -m 0700 "$input_root" "$evidence_root"
  printf 'HEPH_GCP_DIAGNOSTICS event=collection status=start object=%s\n' "$object"
  printf 'HEPH_GCP_DIAGNOSTICS test_result=%s diagnostics_result=collection_pending phase=%s revision=%s\n' \
    "$test_status" "$phase" "$revision" >"$status_json"
  if serial_copy_status="$(bounded_copy_status "$log_file" serial "$input_root/serial.log")"; then
    printf '%s\n' "$serial_copy_status" >>"$status_json"
  else
    printf 'HEPH_GCP_DIAGNOSTICS source=serial status=missing\n' >>"$status_json"
    printf 'HEPH_GCP_DIAGNOSTICS source=serial status=missing\n' >"$input_root/serial.log"
  fi
  journal_unit="${diagnostics_journal_unit:-}"
  if [[ -z "$journal_unit" ]]; then
    if [[ "$test_mode" == smoke ]]; then
      journal_unit="heph-gcp-kvm-smoke-${HEPH_GCP_RUN_ID:-manual}"
    else
      journal_unit="heph-gcp-cooking-${HEPH_GCP_RUN_ID:-manual}"
    fi
  fi
  if command -v journalctl >/dev/null 2>&1; then
    if ! journalctl --no-pager --quiet --output=short-iso --unit="$journal_unit" --lines=200 \
      >"$input_root/host-journal.log" 2>"$input_root/host-journal.err"; then
      printf 'HEPH_GCP_DIAGNOSTICS source=host-journal status=unavailable unit=%s\n' "$journal_unit" \
        >"$input_root/host-journal.log"
      printf 'HEPH_GCP_DIAGNOSTICS source=host-journal status=unavailable unit=%s\n' "$journal_unit" >>"$status_json"
    else
      printf 'HEPH_GCP_DIAGNOSTICS source=host-journal status=retained bytes=%s unit=%s\n' \
        "$(stat -c '%s' -- "$input_root/host-journal.log")" "$journal_unit" >>"$status_json"
    fi
    rm -f -- "$input_root/host-journal.err"
  else
    printf 'HEPH_GCP_DIAGNOSTICS source=host-journal status=missing unit=%s\n' "$journal_unit" \
      >"$input_root/host-journal.log"
    printf 'HEPH_GCP_DIAGNOSTICS source=host-journal status=missing unit=%s\n' "$journal_unit" >>"$status_json"
  fi
  cooking_evidence_root="${evidence_root}/cooking"
  snapshot_input="${cooking_evidence_root}/cooking-lineage.jsonl"
  snapshot_status_path="${cooking_evidence_root}/cooking-lineage-status.json"
  if [[ ! -f "$snapshot_input" || -L "$snapshot_input" ]]; then
    printf 'HEPH_GCP_DIAGNOSTICS source=cooking-lineage status=missing path=%s\n' "$snapshot_input" >>"$status_json"
  fi
  if [[ ! -f "$snapshot_status_path" || -L "$snapshot_status_path" ]]; then
    printf 'HEPH_GCP_DIAGNOSTICS source=cooking-lineage-status status=missing path=%s\n' "$snapshot_status_path" >>"$status_json"
  fi
  local collector_args=(
    --output-dir "$output_root"
    --source "serial=$input_root/serial.log"
    --source "host-journal=$input_root/host-journal.log"
    --source "runtime-structured=$status_json"
  )
  local snapshot_args=()
  if [[ "$test_mode" == diagnostic ]]; then
    # Diagnostic mode deliberately supplies synthetic evidence and labels it
    # as such.  Full Cooking mode must use only the producer's real files.
    snapshot="${input_root}/diagnostic-lineage.jsonl"
    printf '{"attempt_id":"00000000-0000-4000-8000-000000000001","attempt_number":1,"attempt_state":"failed","run_state":"failed","run_outcome":"failed"}\n' >"$snapshot"
    snapshot_args+=(--snapshot-jsonl "$snapshot")
  elif [[ -n "$snapshot_input" && -f "$snapshot_input" && ! -L "$snapshot_input" ]]; then
    snapshot_args+=(--snapshot-jsonl "$snapshot_input")
  else
    # The collector records a missing optional source in manifest.json.  Do
    # not manufacture an ID or state row when the producer did not emit one.
    snapshot_args+=(--missing-source "lineage=$snapshot_input")
  fi
  if [[ "$test_mode" == diagnostic ]]; then
    :
  elif [[ -f "$snapshot_status_path" && ! -L "$snapshot_status_path" ]]; then
    snapshot_args+=(--snapshot-status "$snapshot_status_path")
  else
    # This also gives the manifest an explicit missing record for the status
    # sidecar while preserving a real runtime log when one exists.
    snapshot_args+=(--missing-source "lineage-status=$snapshot_status_path")
  fi
  if [[ "$test_mode" == diagnostic ]]; then
    printf '{"status":"failed","phase":"browser","test":"diagnostic-synthetic","exit_code":42}\n' \
      >"$input_root/browser-summary.json"
    # Load one known fixture value from the checked-out scanner into an
    # isolated input file.  It is never sent to serial output or an argument,
    # and the producer must quarantine it by content rather than filename.
    diagnostic_unsafe_source="$input_root/diagnostic-input.log"
    python3 - "$scanner" "$diagnostic_unsafe_source" <<'PY'
import importlib.util
from pathlib import Path
import sys

spec = importlib.util.spec_from_file_location("diagnostic_scanner", sys.argv[1])
if spec is None or spec.loader is None:
    raise SystemExit("diagnostic scanner cannot be loaded")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
Path(sys.argv[2]).write_bytes(module.VALUES[0] + b"\n")
PY
    chmod 0600 "$diagnostic_unsafe_source"
    collector_args+=(--source "runtime-log=$diagnostic_unsafe_source")
    if [[ -n "$diagnostic_timeout_log" && -f "$diagnostic_timeout_log" && ! -L "$diagnostic_timeout_log" ]]; then
      bounded_copy "$diagnostic_timeout_log" "$input_root/test-output.log"
    else
      printf 'HEPH_GCP_DIAGNOSTIC test result: %s expected failure is intentional\n' \
        "$test_status" >"$input_root/test-output.log"
    fi
    collector_args+=(
      --source "browser-summary=$input_root/browser-summary.json"
      --source "test-output=$input_root/test-output.log"
    )
  else
    if [[ "$test_mode" == smoke && -n "${smoke_output_log:-}" &&
      -f "$smoke_output_log" && ! -L "$smoke_output_log" ]]; then
      copy_status="$(bounded_copy_status "$smoke_output_log" runtime-log "$input_root/runtime-log")" ||
        copy_status='HEPH_GCP_DIAGNOSTICS source=runtime-log status=unavailable'
      printf '%s\n' "$copy_status" >>"$status_json"
      [[ -f "$input_root/runtime-log" ]] && collector_args+=(--source "runtime-log=$input_root/runtime-log")
    elif [[ -f /var/log/hephaestus/gcp-cooking-run.log && ! -L /var/log/hephaestus/gcp-cooking-run.log ]]; then
      copy_status="$(bounded_copy_status /var/log/hephaestus/gcp-cooking-run.log runtime-log "$input_root/runtime-log")" || copy_status='HEPH_GCP_DIAGNOSTICS source=runtime-log status=unavailable'
      printf '%s\n' "$copy_status" >>"$status_json"
      [[ -f "$input_root/runtime-log" ]] && collector_args+=(--source "runtime-log=$input_root/runtime-log")
      copy_status="$(bounded_copy_status /var/log/hephaestus/gcp-cooking-run.log test-output "$input_root/test-output.log")" || copy_status='HEPH_GCP_DIAGNOSTICS source=test-output status=unavailable'
      printf '%s\n' "$copy_status" >>"$status_json"
      [[ -f "$input_root/test-output.log" ]] && collector_args+=(--source "test-output=$input_root/test-output.log")
    fi
    # Browser request/response logs are intentionally excluded. The runner or
    # the typed collector may provide one allowlisted structured summary.
    browser_summary_source="${HEPH_GCP_BROWSER_SUMMARY:-${cooking_evidence_root}/browser-summary.json}"
    if [[ -f "$browser_summary_source" && ! -L "$browser_summary_source" ]]; then
      cp -- "$browser_summary_source" "$input_root/browser-summary.json" || true
    fi
    if [[ ! -s "$input_root/browser-summary.json" ]]; then
      printf '{"status":"failed","suite":"cooking-playwright","test":"browser-report","phase":"browser","exit_code":%s,"error":"browser summary missing"}\n' \
        "$test_status" >"$input_root/browser-summary.json"
    fi
    collector_args+=(--source "browser-summary=$input_root/browser-summary.json")
  fi
  chmod 0600 "$input_root"/*
  local collector_status=0
  run_with_collection_deadline python3 "$collector" "${collector_args[@]}" \
    "${snapshot_args[@]}" --archive "$archive" || collector_status=$?
  ((collector_status == 0)) || {
    printf 'HEPH_GCP_DIAGNOSTICS event=collection status=fail exit=%s\n' "$collector_status"
    return "$collector_status"
  }
  if [[ "$test_mode" == diagnostic ]]; then
    if ! python3 - "$output_root/manifest.json" <<'PY'
import json
from pathlib import Path
import sys

manifest = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
expected = [{"label": "runtime-log", "reason": "credential-scan-rejected", "status": "rejected"}]
if manifest.get("collectionStatus") != "partial":
    raise SystemExit("diagnostic quarantine did not produce a partial collection")
if manifest.get("rejectedSources") != expected:
    raise SystemExit("diagnostic quarantine record does not match the expected source policy rejection")
PY
    then
      printf 'HEPH_GCP_DIAGNOSTICS event=collection status=fail reason=quarantine-contract\n'
      return 1
    fi
    diagnostic_quarantine_validated=true
    printf 'HEPH_GCP_DIAGNOSTICS event=collection status=partial rejected=runtime-log reason=credential-scan-rejected\n'
  fi
  printf 'HEPH_GCP_DIAGNOSTICS event=collection status=pass\n'
  run_with_collection_deadline python3 "$scanner" "$output_root" || {
    printf 'HEPH_GCP_DIAGNOSTICS event=scan status=fail\n'
    return 1
  }
  [[ -f "$archive" && ! -L "$archive" ]] || {
    printf 'HEPH_GCP_DIAGNOSTICS event=scan status=fail reason=archive-missing\n'
    return 1
  }
  [[ "$(stat -c '%s' "$archive")" -le "$diagnostics_max_archive_bytes" ]] || {
    printf 'HEPH_GCP_DIAGNOSTICS event=scan status=fail reason=archive-too-large\n'
    return 1
  }
  printf 'HEPH_GCP_DIAGNOSTICS event=scan status=pass bytes=%s\n' "$(stat -c '%s' "$archive")"
  diagnostics_token_json="${temporary_root}/diagnostics-token.json"
  diagnostics_header_file="${temporary_root}/diagnostics-curl.conf"
  run_with_collection_deadline curl --fail --silent --show-error -H 'Metadata-Flavor: Google' \
    "$metadata_root/instance/service-accounts/default/token" >"$diagnostics_token_json"
  chmod 0600 "$diagnostics_token_json"
  token="$(python3 -c 'import json,sys; value=json.load(open(sys.argv[1])).get("access_token"); raise SystemExit("missing access token") if not isinstance(value,str) or not value else print(value)' "$diagnostics_token_json")"
  # Keep the bearer token out of the curl process argument list and remove it
  # in finish(), including token parse and upload failure paths.
  printf 'header = "Authorization: Bearer %s"\n' "$token" >"$diagnostics_header_file"
  chmod 0600 "$diagnostics_header_file"
  encoded_object="$(python3 -c 'from urllib.parse import quote; import sys; print(quote(sys.argv[1], safe=""))' "$object")"
  upload_status=0
  run_with_collection_deadline curl --fail --silent --show-error --retry 2 --retry-all-errors \
    --config "$diagnostics_header_file" -H 'Content-Type: application/gzip' \
    --data-binary "@$archive" \
    "https://storage.googleapis.com/upload/storage/v1/b/${diagnostics_bucket}/o?uploadType=media&name=${encoded_object}&ifGenerationMatch=0" \
    >/dev/null || upload_status=$?
  if ((upload_status != 0)); then
    printf 'HEPH_GCP_DIAGNOSTICS event=upload status=fail exit=%s object=%s\n' "$upload_status" "$object"
    return "$upload_status"
  fi
  diagnostics_uploaded=true
  printf 'HEPH_GCP_DIAGNOSTICS event=upload status=pass object=gs://%s/%s bytes=%s\n' \
    "$diagnostics_bucket" "$object" "$(stat -c '%s' "$archive")"
}

range_is_free() {
  local start="$1" end file
  end=$((start + 65536))
  for file in /etc/subuid /etc/subgid; do
    awk -F: -v start="$start" -v end="$end" \
      '$1 != "forge" && $2 ~ /^[0-9]+$/ && $3 ~ /^[0-9]+$/ {
         range_end = $2 + $3
         if (start < range_end && end > $2) { conflict = 1 }
       }
       END { exit conflict ? 1 : 0 }' "$file" || return 1
  done
}

ensure_subordinate_range() {
  local uid_range gid_range start
  uid_range="$(awk -F: '$1 == "forge" { print $2 ":" $3 }' /etc/subuid)"
  gid_range="$(awk -F: '$1 == "forge" { print $2 ":" $3 }' /etc/subgid)"
  if [[ -n "$uid_range" || -n "$gid_range" ]]; then
    [[ "$uid_range" == "$gid_range" ]] || die 'forge subuid/subgid ranges differ'
    [[ "$uid_range" =~ ^[0-9]+:65536$ ]] || die 'forge subordinate range must contain 65536 IDs'
    start="${uid_range%%:*}"
    range_is_free "$start" || die 'forge subordinate range overlaps an existing account'
    return 0
  fi
  start=100000
  while ! range_is_free "$start"; do
    start=$((start + 65536))
  done
  printf 'forge:%s:65536\n' "$start" >>/etc/subuid
  printf 'forge:%s:65536\n' "$start" >>/etc/subgid
}

remaining_seconds() {
  local remaining
  if ((trial_deadline_epoch > 0)); then
    remaining=$((trial_deadline_epoch - $(date +%s)))
  else
    remaining=$((trial_deadline - SECONDS))
  fi
  ((remaining > 0)) || die 'internal test deadline elapsed'
  printf '%s\n' "$remaining"
}

run_with_deadline() {
  local remaining
  remaining="$(remaining_seconds)"
  timeout --kill-after=30s "${remaining}s" "$@"
}

collection_remaining_seconds() {
  local remaining
  if ((collection_deadline_epoch > 0)); then
    remaining=$((collection_deadline_epoch - $(date +%s)))
  else
    remaining=$((collection_deadline - SECONDS))
  fi
  ((remaining > 0)) || die 'diagnostics collection reserve elapsed'
  printf '%s\n' "$remaining"
}

run_with_collection_deadline() {
  local remaining
  remaining="$(collection_remaining_seconds)"
  timeout --kill-after=30s "${remaining}s" "$@"
}

run_logged_forge_command() {
  local log_path="$1" label="$2" status
  shift 2
  if run_with_deadline "${forge_env[@]}" bash -Eeuo pipefail -c '
    log_path="$1"
    label="$2"
    shift 2
    set +e
    "$@" 2>&1 | tee "$log_path"
    command_status="${PIPESTATUS[0]}"
    set -e
    if ((command_status != 0)); then
      first_error="$(grep -i -m1 -E \
        "fatal error:|error:|no rule to make target|no such file or directory|command not found|cannot find|undefined reference|Error [0-9]+" \
        "$log_path" || true)"
      printf "HEPH_GCP_KVM_BUILD_ERROR phase=%s status=%s log=%s\n" \
        "$label" "$command_status" "$log_path"
      printf "HEPH_GCP_KVM_FIRST_ERROR %s\n" "$first_error"
    fi
    exit "$command_status"
  ' -- "$log_path" "$label" "$@"
  then
    return 0
  else
    status=$?
    return "$status"
  fi
}

forge_env=(
  runuser -u forge -- env
  HOME=/home/forge
  XDG_RUNTIME_DIR=/run/user/10001
  RUSTUP_HOME=/home/forge/.rustup
  CARGO_HOME=/home/forge/.cargo
  PATH=/home/forge/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
  GIT_TERMINAL_PROMPT=0
)

runner_image_runtime_ready() {
  local root_prefix="${1:-}" image_browser_root="${2:-$work_root/playwright-browsers}"
  local expected_node="${HEPH_IMAGE_NODE_VERSION:-v24.16.0}"
  local expected_oras="${HEPH_IMAGE_ORAS_VERSION:-1.3.3}"
  local node_output rust_output oras_output oras_actual browser_executable browser_output
  local -a runtime_env=(
    runuser -u forge -- env
    HOME="${root_prefix}/home/forge"
    XDG_RUNTIME_DIR=/run/user/10001
    RUSTUP_HOME="${root_prefix}/home/forge/.rustup"
    CARGO_HOME="${root_prefix}/home/forge/.cargo"
    RUSTUP_TOOLCHAIN="$rust_version"
    PATH="${root_prefix}/home/forge/.cargo/bin:${root_prefix}/opt/hephaestus/node-${expected_node}/bin:${root_prefix}/usr/local/bin:${root_prefix}/usr/bin:${root_prefix}/bin:/usr/bin:/bin"
  )

  run_runner_image_tool() {
    # Metadata startup scripts inherit a private working directory.  Run
    # rustup-backed probes from forge's neutral HOME and pin the toolchain so
    # an incidental rust-toolchain file cannot change the image contract.
    run_with_deadline "${runtime_env[@]}" bash -Eeuo pipefail -c \
      'cd "$HOME" && exec "$@"' -- "$@"
  }

  node_output="$(run_runner_image_tool "${root_prefix}/usr/local/bin/node" --version)" ||
    die 'custom runner image Node executable cannot run as forge'
  [[ "$node_output" == "$expected_node" ]] ||
    die 'custom runner image Node executable version does not match its pin'

  rust_output="$(run_runner_image_tool rustc --version)" ||
    die 'custom runner image Rust executable cannot run as forge'
  [[ "$(awk '$1 == "rustc" { print $2; exit }' <<<"$rust_output")" == "${rust_version}" ]] ||
    die 'custom runner image Rust executable version does not match its pin'

  oras_output="$(run_runner_image_tool "${root_prefix}/usr/local/bin/oras" version)" ||
    die 'custom runner image ORAS executable cannot run as forge'
  oras_actual="$(awk '$1 == "Version:" { print $2; exit }' <<<"$oras_output")"
  [[ "$oras_actual" == "$expected_oras" ]] ||
    die 'custom runner image ORAS executable version does not match its pin'

  browser_executable="$(find -P "$image_browser_root" -type f \( -name chrome-headless-shell -o -name chrome \) -perm -0100 -print -quit 2>/dev/null)"
  [[ -n "$browser_executable" ]] || die 'custom runner image Chromium executable is missing'
  browser_output="$(run_runner_image_tool "$browser_executable" --version)" ||
    die 'custom runner image Chromium executable cannot run as forge'
  [[ -n "${runner_image_browser_version:-}" && "$browser_output" == "$runner_image_browser_version" ]] ||
    die 'custom runner image Chromium executable version does not match its manifest'
  printf 'HEPH_GCP_RUNNER_IMAGE runtime=pass node=%s rust=%s oras=%s chromium=%s\n' \
    "$node_output" "$(awk '$1 == "rustc" { print $2; exit }' <<<"$rust_output")" \
    "$oras_actual" "$browser_output"
}

emit_kvm_evidence() {
  if [[ "$runner_image_ready" == true ]]; then
    printf 'HEPH_GCP_KVM_EVIDENCE revision=%s diagnostics=%s libkrun_revision=%s libkrunfw_tag=%s\n' \
      "$revision" "$smoke_log_dir" "$runner_image_libkrun_revision" "$runner_image_libkrunfw_tag"
  else
    printf 'HEPH_GCP_KVM_EVIDENCE revision=%s diagnostics=%s libkrun_revision=%s libkrunfw_revision=%s\n' \
      "$revision" "$smoke_log_dir" "$libkrun_revision" "$libkrunfw_revision"
  fi
}

if [[ "${HEPH_GCP_STARTUP_LIBRARY:-0}" == 1 ]]; then
  if [[ "${HEPH_GCP_RUNNER_IMAGE_RUNTIME_TEST:-0}" == 1 ]]; then
    trial_deadline=$((SECONDS + 30))
    runner_image_browser_version="${HEPH_GCP_RUNNER_IMAGE_TEST_BROWSER_VERSION:-Chromium 1.2.3}"
    runner_image_runtime_ready "${HEPH_GCP_RUNNER_IMAGE_TEST_ROOT:?}" \
      "${HEPH_GCP_RUNNER_IMAGE_TEST_BROWSER_ROOT:?}"
  fi
  return 0
fi

phase_start metadata
require_command curl
  revision="${HEPHAESTUS_GCP_REVISION:-$(metadata_value github-sha)}"
[[ "$revision" =~ ^[0-9a-f]{40}$ ]] || die 'github-sha must be an exact lowercase 40-character commit SHA'
test_mode="$(metadata_value test-mode)"
case "$test_mode" in
  smoke|gcp-cooking|diagnostic) ;;
  *) die 'test-mode must be smoke, diagnostic, or gcp-cooking' ;;
esac
vm_start_epoch="$(metadata_value trial-start-epoch)"
[[ "$vm_start_epoch" =~ ^[0-9]+$ ]] || die 'trial-start-epoch metadata must be an epoch integer'
if [[ "$test_mode" == diagnostic || "$test_mode" == gcp-cooking ]]; then
  diagnostics_object_metadata="$(metadata_value diagnostics-object)"
  diagnostics_enabled=true
else
  diagnostics_object_metadata="$(metadata_optional_value diagnostics-object)"
  if [[ -n "$diagnostics_object_metadata" ]]; then
    diagnostics_journal_unit="$(metadata_optional_value diagnostics-journal-unit)"
    diagnostics_enabled=true
  fi
fi
stage_diagnostics_metadata
if [[ "$test_mode" == diagnostic ]]; then
  trial_deadline=$((SECONDS + 180))
  collection_deadline=$((SECONDS + 480))
  trial_deadline_epoch=$((vm_start_epoch + 180))
  collection_deadline_epoch=$((vm_start_epoch + 480))
else
  # Reserve five minutes inside the provider's 45-minute lifetime for
  # collection, scans, upload, and shutdown evidence.
  trial_deadline=$((SECONDS + 2100))
  collection_deadline=$((SECONDS + 2400))
  trial_deadline_epoch=$((vm_start_epoch + 2100))
  collection_deadline_epoch=$((vm_start_epoch + 2400))
fi
marker "ready mode=$test_mode"

runner_image_ready=false
runner_image_manifest_path="${HEPH_IMAGE_MANIFEST:-/usr/share/hephaestus/runner-image-manifest.json}"
runner_image_selection=''
if [[ "${HEPH_GCP_IMAGE_BAKE:-0}" == 1 ]]; then
  runner_image_selection=stock
else
  runner_image_selection="${HEPH_GCP_RUNNER_IMAGE_SELECTION:-$(metadata_optional_value runner-image-selection)}"
  case "$runner_image_selection" in
    custom|stock) ;;
    '') die 'runner-image-selection metadata must explicitly be custom or stock' ;;
    *) die 'runner-image-selection metadata is invalid' ;;
  esac
fi
if [[ "$runner_image_selection" == custom ]]; then
  [[ -f /etc/hephaestus/runner-image-required ]] ||
    die 'custom runner image was requested but its selection marker is missing'
  if declare -F runner_image_verify >/dev/null 2>&1 && runner_image_verify; then
    runner_image_ready=true
    manifest_field() {
      python3 - "$runner_image_manifest_path" "$1" <<'PY'
import json
import sys
from pathlib import Path

value = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
field = sys.argv[2]
item = value.get(field)
if not isinstance(item, str) or not item:
    raise SystemExit(f"runner image manifest field is missing: {field}")
print(item)
PY
    }
    manifest_pin() {
      python3 - "$runner_image_manifest_path" "$1" <<'PY'
import json
import sys
from pathlib import Path

document = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
pins = document.get("pins")
field = sys.argv[2]
item = pins.get(field) if isinstance(pins, dict) else None
if not isinstance(item, str) or not item:
    raise SystemExit(f"runner image manifest pin is missing: {field}")
print(item)
PY
    }
    runner_image_manifest_sha="$(manifest_field manifest_sha256)"
    expected_manifest_sha="$(metadata_optional_value runner-image-manifest-sha256)"
    [[ "$expected_manifest_sha" =~ ^[0-9a-f]{64}$ ]] ||
      die 'custom runner image manifest anchor is missing or invalid'
    [[ "$runner_image_manifest_sha" == "$expected_manifest_sha" ]] ||
      die 'custom runner image manifest does not match the external anchor'
    for image_hash_field in recipe_sha256 verifier_sha256 startup_sha256; do
      image_hash="$(manifest_field "$image_hash_field")"
      expected_hash="$(metadata_optional_value "runner-image-${image_hash_field%_sha256}-sha256")"
      [[ "$expected_hash" =~ ^[0-9a-f]{64}$ ]] ||
        die "external runner image ${image_hash_field} anchor is missing or invalid"
      [[ "$image_hash" == "$expected_hash" ]] ||
        die "runner image ${image_hash_field} does not match the external anchor"
    done
    runner_image_browser_lock_sha="$(manifest_field browser_lock_sha256)"
    [[ "$runner_image_browser_lock_sha" =~ ^[0-9a-f]{64}$ ]] ||
      die 'runner image browser lock fingerprint is invalid'
    runner_image_browser_version="$(manifest_field browser_version)"
    runner_image_libkrun_revision="$(manifest_pin libkrun_revision)"
    runner_image_libkrunfw_tag="$(manifest_pin libkrunfw_tag)"
    # Use the same llvm-config/tree lookup as the stock provisioning path.
    # Ubuntu's SONAME may be libclang-N.so.N, so ldconfig's unversioned
    # libclang.so pattern is insufficient.
    llvm_prefix="$(llvm-config --prefix)" || die 'baked runner image has no llvm-config'
    libclang_so="$(find "$llvm_prefix" -maxdepth 3 \( -type f -o -type l \) \
      -name 'libclang.so*' -print -quit 2>/dev/null)"
    if [[ -z "$libclang_so" ]]; then
      clang_path="$(readlink -f "$(command -v clang)")"
      clang_prefix="$(dirname "$(dirname "$clang_path")")"
      libclang_so="$(find "$clang_prefix" -maxdepth 3 \( -type f -o -type l \) \
        -name 'libclang.so*' -print -quit 2>/dev/null)"
    fi
    [[ -n "$libclang_so" && -r "$libclang_so" ]] ||
      die 'baked runner image has no usable libclang shared library'
    libclang_dir="$(dirname -- "$libclang_so")"
    printf 'HEPH_GCP_RUNNER_IMAGE mode=prebuilt manifest=%s\n' "$runner_image_manifest_path"
  else
    die 'runner image marker exists but the immutable manifest did not verify'
  fi
elif [[ -f /etc/hephaestus/runner-image-required ]]; then
  die 'stock runner image was requested but a baked-image marker is present'
fi

if [[ "$runner_image_ready" == true ]]; then
  phase_start runner-image-runtime
  runner_image_runtime_ready
  phase_pass
fi

if [[ "$test_mode" == diagnostic ]]; then
  phase_start diagnostic-bootstrap
  run_with_deadline apt-get update -qq
  run_with_deadline env DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
    ca-certificates curl git python3 tar gzip
  require_command git
  install -d -m 0700 "$work_root" "$temporary_root" "$evidence_root"
  run_with_deadline git clone --filter=blob:none --no-checkout "$repository_url" "$checkout_root"
  run_with_deadline git -C "$checkout_root" fetch --depth 1 origin "$revision"
  run_with_deadline git -C "$checkout_root" checkout --detach "$revision"
  [[ "$(git -C "$checkout_root" rev-parse HEAD)" == "$revision" ]] || die 'diagnostic checkout SHA mismatch'
  phase_pass
  phase_start diagnostic-synthetic
  printf 'HEPH_GCP_DIAGNOSTIC synthetic browser report; no request, response, cookie, trace, or credential data\n' >&2
  diagnostic_timeout_log="${temporary_root}/diagnostic-timeout.log"
  set +e
  timeout --kill-after=1s 3s bash -Eeuo pipefail -c 'sleep 30' >"$diagnostic_timeout_log" 2>&1
  diagnostic_timeout_status=$?
  set -e
  ((diagnostic_timeout_status == 124)) || die "diagnostic timeout probe returned ${diagnostic_timeout_status}"
  diagnostic_probe_completed=true
  printf 'HEPH_GCP_DIAGNOSTIC timeout-probe status=124 limit=3s\n' >>"$diagnostic_timeout_log"
  chmod 0600 "$diagnostic_timeout_log"
  phase_pass
  # The test result is intentionally unsuccessful; finish() turns collection
  # and upload into the authoritative CI result and retains both dimensions.
  exit 42
fi

phase_start host-packages
if [[ "$runner_image_ready" == true ]]; then
  printf 'HEPH_GCP_RUNNER_IMAGE phase=host-packages status=prebuilt\n'
else
run_with_deadline apt-get update -qq
run_with_deadline env DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
  apparmor bc bison build-essential ca-certificates clang cpio curl dwarves e2fsprogs flex \
  fuse-overlayfs git libcap-ng-dev libclang-dev libelf-dev libfdt-dev libglib2.0-dev \
  libncurses-dev libpixman-1-dev libseccomp-dev libslirp-dev libssl-dev \
  libzstd-dev llvm-dev lld make musl-tools nftables openssl patch patchelf perl podman \
  python3 python3-pyelftools rsync rustup skopeo slirp4netns passt tar uidmap xz-utils zstd
require_command llvm-config
llvm_config_version="$(llvm-config --version)" || die 'llvm-config cannot report its version'
llvm_prefix="$(llvm-config --prefix)" || die 'llvm-config cannot report its prefix'
libclang_so="$(find "$llvm_prefix" -maxdepth 3 \( -type f -o -type l \) \
  -name 'libclang.so*' -print -quit 2>/dev/null)"
if [[ -z "$libclang_so" ]]; then
  clang_path="$(readlink -f "$(command -v clang)")"
  clang_prefix="$(dirname "$(dirname "$clang_path")")"
  libclang_so="$(find "$clang_prefix" -maxdepth 3 \( -type f -o -type l \) \
    -name 'libclang.so*' -print -quit 2>/dev/null)"
fi
[[ -n "$libclang_so" && -r "$libclang_so" ]] || die 'libclang shared library is unavailable'
libclang_dir="$(dirname "$libclang_so")"
forge_env+=("LIBCLANG_PATH=$libclang_dir")
printf 'HEPH_GCP_KVM_LLVM llvm-config=%s version=%s libclang=%s\n' \
  "$(command -v llvm-config)" "$llvm_config_version" "$libclang_so"
fi
phase_pass
phase_start accounts
[[ "$(uname -m)" == x86_64 ]] || die 'host must be x86_64'
if [[ "${HEPH_GCP_IMAGE_BAKE:-0}" != 1 ]]; then
  [[ -r /dev/kvm && -w /dev/kvm ]] || die '/dev/kvm is not readable and writable'
fi
[[ -f /sys/fs/cgroup/cgroup.controllers ]] || die 'host must use cgroup v2'

if getent group forge >/dev/null; then
  [[ "$(getent group forge | cut -d: -f3)" == "$forge_gid" ]] || die 'forge group has wrong GID'
else
  [[ -z "$(getent group "$forge_gid" || true)" ]] || die 'GID 10001 is already in use'
  groupadd --gid "$forge_gid" forge
fi
if getent passwd forge >/dev/null; then
  [[ "$(id -u forge)" == "$forge_uid" && "$(id -g forge)" == "$forge_gid" ]] || die 'forge account has wrong UID/GID'
else
  [[ -z "$(getent passwd "$forge_uid" || true)" ]] || die 'UID 10001 is already in use'
  useradd --uid "$forge_uid" --gid "$forge_gid" --create-home --home-dir /home/forge --shell /usr/sbin/nologin forge
fi
if [[ "${HEPH_GCP_IMAGE_BAKE:-0}" != 1 ]]; then
  kvm_group="$(stat --format='%G' /dev/kvm)"
  [[ -n "$kvm_group" && "$kvm_group" != UNKNOWN ]] || die 'KVM device has no usable group'
  getent group "$kvm_group" >/dev/null || die "KVM group unavailable: $kvm_group"
  usermod --append --groups "$kvm_group" forge
fi
for file in /etc/subuid /etc/subgid; do
  [[ -e "$file" ]] || install -m 0644 /dev/null "$file"
done
ensure_subordinate_range
install -d -m 0700 -o forge -g forge "$work_root" "$temporary_root" "$evidence_root" \
  "$smoke_temporary_root" /run/user/10001 /home/forge/.cargo /home/forge/.rustup
phase_pass

phase_start passt-compat
if [[ "$runner_image_ready" == true ]]; then
  printf 'HEPH_GCP_RUNNER_IMAGE phase=passt-compat status=prebuilt\n'
else
# Ubuntu Noble's passt predates the DHCP broadcast fix needed by libkrun's
# minimal DHCP client. Build the reviewed upstream commit before installing
# the AppArmor profile so the replacement keeps the packaged executable's
# /usr/bin/passt attachment path. The AVX2 companion must be replaced too:
# upstream passt dispatches to it from the generic x86_64 binary. Both distro
# files remain available under their dpkg-divert names for rollback/audit.
passt_source_path="$source_root/passt"
install -d -m 0700 -o forge -g forge "$source_root"
if [[ -e "$passt_source_path" ]]; then
  [[ ! -L "$passt_source_path" && -d "$passt_source_path/.git" ]] ||
    die 'passt source path is not an owned git checkout'
else
  run_with_deadline "${forge_env[@]}" git init "$passt_source_path"
  run_with_deadline "${forge_env[@]}" git -C "$passt_source_path" remote add origin "$passt_source_url"
fi
[[ "$(stat --format='%u' "$passt_source_path")" == "$forge_uid" ]] ||
  die 'passt source checkout is not owned by forge'
if ! "${forge_env[@]}" git -C "$passt_source_path" remote get-url origin >/dev/null 2>&1; then
  run_with_deadline "${forge_env[@]}" git -C "$passt_source_path" remote add origin "$passt_source_url"
fi
[[ "$("${forge_env[@]}" git -C "$passt_source_path" remote get-url origin)" == "$passt_source_url" ]] ||
  die 'passt source remote is not the official upstream'
run_with_deadline "${forge_env[@]}" git -C "$passt_source_path" fetch --depth 1 origin "$passt_revision"
run_with_deadline "${forge_env[@]}" git -C "$passt_source_path" checkout --detach "$passt_revision"
[[ "$("${forge_env[@]}" git -C "$passt_source_path" rev-parse HEAD)" == "$passt_revision" ]] ||
  die 'passt source revision verification failed'
# Keep this diagnostic deterministic across reruns: the source tree is a
# dedicated disposable build tree, so remove a prior generated working-tree
# edit before applying the exact one-site instrumentation below.
run_with_deadline "${forge_env[@]}" git -C "$passt_source_path" reset --hard "$passt_revision"
run_with_deadline "${forge_env[@]}" git -C "$passt_source_path" clean -ffd
python3 - "$passt_source_path/epoll_ctl.c" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
source = path.read_text()
old_include = '#include <errno.h>\n\n#include "epoll_ctl.h"\n'
new_include = '#include <errno.h>\n#include <fcntl.h>\n\n#include "epoll_ctl.h"\n'
old_body = '''\tif (ret == -1) {
\t\tret = -errno;
\t\terr("Failed to add fd to epoll: %s", strerror_(-ret));
\t}
'''
new_body = '''\tif (ret == -1) {
\t\tint epoll_errno = errno;
\t\tint epollfd_flags = fcntl(epollfd, F_GETFD);
\t\tint epollfd_errno = epollfd_flags < 0 ? errno : 0;
\t\tint targetfd_flags = fcntl(ref.fd, F_GETFD);
\t\tint targetfd_errno = targetfd_flags < 0 ? errno : 0;

\t\tret = -epoll_errno;
\t\terrno = epoll_errno;
\t\terr("Failed to add fd to epoll: %s (epollfd=%d fcntl=%d/%d, targetfd=%d fcntl=%d/%d errno=%d)",
\t\t     strerror_(-ret), epollfd, epollfd_flags, epollfd_errno,
\t\t     ref.fd, targetfd_flags, targetfd_errno, epoll_errno);
\t}
'''
if source.count(old_include) != 1 or source.count(old_body) != 1:
    raise SystemExit('expected pinned epoll_ctl.c diagnostic sites were not unique')
source = source.replace(old_include, new_include).replace(old_body, new_body)
path.write_text(source)

tap_path = path.with_name('tap.c')
tap_source = tap_path.read_text()
old_accept = '''\tc->fd_tap = accept4(c->fd_tap_listen, NULL, NULL, 0);

\tif (!getsockopt(c->fd_tap, SOL_SOCKET, SO_PEERCRED, &ucred, &len))
'''
new_accept = '''\tc->fd_tap = accept4(c->fd_tap_listen, NULL, NULL, SOCK_CLOEXEC);
\tif (c->fd_tap < 0) {
\t\tstatic bool accept_error_reported;
\t\tint accept_errno = errno;
\t\tif (!accept_error_reported) {
\t\t\terr("Error accepting tap client: %s", strerror_(accept_errno));
\t\t\taccept_error_reported = true;
\t\t}
\t\treturn;
\t}

\tif (!getsockopt(c->fd_tap, SOL_SOCKET, SO_PEERCRED, &ucred, &len))
'''
if tap_source.count(old_accept) != 1:
    raise SystemExit('expected pinned tap accept site was not unique')
tap_path.write_text(tap_source.replace(old_accept, new_accept))
PY
run_logged_forge_command "${temporary_root}/passt-build.log" passt \
  make --no-print-directory -C "$passt_source_path" VERSION="$passt_revision" passt passt.avx2
for passt_build_binary in passt passt.avx2; do
  passt_build_path="$passt_source_path/$passt_build_binary"
  [[ -f "$passt_build_path" && -x "$passt_build_path" ]] || die "built $passt_build_binary is missing"
  readelf -h "$passt_build_path" | grep -qE 'Magic:[[:space:]]+7f 45 4c 46' ||
    die "built $passt_build_binary is not an ELF executable"
done

ensure_passt_diversion() {
  local active_path="$1" diverted_path="$2"
  if dpkg-divert --list "$active_path" | grep -Fq "to $diverted_path"; then
    [[ -e "$diverted_path" ]] || die "passt diversion target is missing: $diverted_path"
  else
    [[ -e "$active_path" ]] || die "packaged passt executable is missing: $active_path"
    run_with_deadline dpkg-divert --local --rename --add \
      --divert "$diverted_path" "$active_path"
  fi
}

ensure_passt_diversion /usr/bin/passt /usr/bin/passt.distrib
ensure_passt_diversion /usr/bin/passt.avx2 /usr/bin/passt.avx2.distrib
install -o root -g root -m 0755 "$passt_source_path/passt" /usr/bin/passt
install -o root -g root -m 0755 "$passt_source_path/passt.avx2" /usr/bin/passt.avx2
readelf -h /usr/bin/passt | grep -qE 'Magic:[[:space:]]+7f 45 4c 46' ||
  die 'installed passt is not an ELF executable'
readelf -h /usr/bin/passt.avx2 | grep -qE 'Magic:[[:space:]]+7f 45 4c 46' ||
  die 'installed passt.avx2 is not an ELF executable'
passt_version_output="$(/usr/bin/passt --version 2>&1)" || die 'installed passt cannot report its version'
grep -Fq "$passt_revision" <<<"$passt_version_output" ||
  die 'installed passt version does not match the pinned revision'
passt_version="${passt_version_output%%$'\n'*}"
printf 'HEPH_GCP_PASST_COMPAT revision=%s version=%s binary=/usr/bin/passt avx2=/usr/bin/passt.avx2 distro=/usr/bin/passt.distrib,/usr/bin/passt.avx2.distrib\n' \
  "$passt_revision" "$passt_version"
fi
phase_pass
phase_start cgroup-podman
if [[ "${HEPH_GCP_IMAGE_BAKE:-0}" == 1 ]]; then
  printf 'HEPH_GCP_RUNNER_IMAGE phase=cgroup-podman status=deferred-to-boot\n'
else
run_with_deadline systemd-run --unit="heph-gcp-kvm-preflight-${GITHUB_RUN_ID:-manual}" \
  --expand-environment=no \
  --service-type=oneshot --wait --pipe --collect --property=Delegate=yes \
  --property=TasksMax=infinity --property=LimitNOFILE=65536 \
  --property=CPUAccounting=yes --property=MemoryAccounting=yes \
  --property=TasksAccounting=yes --property=IOAccounting=yes \
  --uid="$forge_uid" --gid="$forge_gid" \
  --setenv=HOME=/home/forge --setenv=XDG_RUNTIME_DIR=/run/user/10001 \
  --setenv=PATH=/home/forge/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  /bin/bash -Eeuo pipefail -c '
    candidate="/sys/fs/cgroup$(awk -F: '\''$1 == "0" { print $3 }'\'' /proc/self/cgroup)"
    test -d "$candidate" -a -w "$candidate" -a -w "$candidate/cgroup.subtree_control"
    cgroup_type="$(<"$candidate/cgroup.type")"
    printf "HEPH_GCP_KVM_CGROUP parent=%s type=%s\n" "$candidate" "$cgroup_type"
    [[ "$cgroup_type" == domain ]]
    manager="$candidate/heph-bootstrap-manager"
    mkdir "$manager"
    # The shell remains in this delegated child until it exits. systemd owns
    # the transient unit and removes the now-empty child; moving back after
    # enabling domain controllers violates the cgroup v2 no-internal-process rule.
    pid="$BASHPID"
    printf "%s\n" "$pid" >"$manager/cgroup.procs"
    available="$(<"$candidate/cgroup.controllers")"
    for controller in cpu io memory pids; do
      [[ " $available " == *" $controller "* ]]
    done
    printf "+cpu +io +memory +pids\n" >"$candidate/cgroup.subtree_control"
    enabled="$(<"$candidate/cgroup.subtree_control")"
    for controller in cpu io memory pids; do
      [[ " $enabled " == *" $controller "* ]]
    done
    [[ "$(podman info --format "{{.Host.Security.Rootless}}")" == true ]]
    [[ -x /usr/bin/passt && -r /dev/kvm && -w /dev/kvm ]]
    forge_subuid="$(awk -F: '\''$1 == "forge" { print $2; exit }'\'' /etc/subuid)"
    forge_subgid="$(awk -F: '\''$1 == "forge" { print $2; exit }'\'' /etc/subgid)"
    [[ "$forge_subuid" =~ ^[0-9]+$ && "$forge_subgid" =~ ^[0-9]+$ ]]
    uid_map="$(podman unshare cat /proc/self/uid_map)"
    gid_map="$(podman unshare cat /proc/self/gid_map)"
    awk -v uid=10001 -v subuid="$forge_subuid" '\''
      $1 == 0 && $2 == uid && $3 == 1 { identity = 1 }
      $1 == 1 && $2 == subuid && $3 >= 65536 { subordinate = 1 }
      END { exit !(identity && subordinate) }
    '\'' <<<"$uid_map"
    awk -v gid=10001 -v subgid="$forge_subgid" '\''
      $1 == 0 && $2 == gid && $3 == 1 { identity = 1 }
      $1 == 1 && $2 == subgid && $3 >= 65536 { subordinate = 1 }
      END { exit !(identity && subordinate) }
    '\'' <<<"$gid_map"
  '
fi
phase_pass
phase_start passt-apparmor
require_command apparmor_parser
[[ -f "$passt_profile_path" && ! -L "$passt_profile_path" ]] ||
  die 'packaged passt AppArmor profile is unavailable or symlinked'
install -d -m 0755 /etc/apparmor.d/local /run/hephaestus
cat >"$passt_local_profile_path" <<'EOF'
# Restrict passt owner read/write access to the dedicated libkrun runtime tree.
owner /tmp/hephaestus-libkrun/** rw,
EOF
# Parse an overlay even when the packaged profile already has a local include:
# this enables the disconnected-socket diagnostic for the dedicated runtime
# tree while retaining every packaged rule and every existing profile flag.
awk '
  function with_required_flags(line, match_start, match_length, inside) {
    if (line !~ /^[[:space:]]*profile[[:space:]]+passt([[:space:]]|$)/)
      return line
    profile_seen = 1
    match_start = match(line, /flags=\([^)]*\)/)
    if (match_start) {
      match_length = RLENGTH
      inside = substr(line, match_start + 7, match_length - 8)
      if (inside !~ /(^|,)[[:space:]]*attach_disconnected([.]path)?([[:space:]]|=|,|$)/)
        inside = inside ",attach_disconnected.path=/tmp/hephaestus-libkrun"
      if (inside !~ /(^|,)[[:space:]]*audit([[:space:]]|,|$)/)
        inside = inside ",audit"
      line = substr(line, 1, match_start - 1) "flags=(" inside ")" \
        substr(line, match_start + match_length)
    } else {
      sub(/[[:space:]]*\{[[:space:]]*$/, \
          " flags=(attach_disconnected.path=/tmp/hephaestus-libkrun,audit) {", line)
    }
    return line
  }
  {
    lines[NR] = with_required_flags($0)
  }
  /^[[:space:]]*#include( if exists)?[[:space:]]+<local\/usr\.bin\.passt>[[:space:]]*$/ {
    local_include_seen = 1
  }
  /^[[:space:]]*}[[:space:]]*$/ { closing = NR }
  END {
    if (!profile_seen || !closing) exit 1
    for (line = 1; line <= NR; line++) {
      if (line == closing && !local_include_seen)
        print "#include <local/usr.bin.passt>"
      print lines[line]
    }
  }
' "$passt_profile_path" >"$passt_profile_overlay" ||
  die 'could not construct the passt AppArmor overlay'
apparmor_parser -r -I /etc/apparmor.d "$passt_profile_overlay"
phase_pass

phase_start passt-preflight
install -d -m 0700 /run/hephaestus
install -m 0700 /dev/null "$passt_preflight_path"
metadata_value passt-preflight-script >"$passt_preflight_path"
[[ -s "$passt_preflight_path" && ! -L "$passt_preflight_path" ]] ||
  die 'passthrough preflight script is unavailable or symlinked'
bash -n "$passt_preflight_path" || die 'passthrough preflight script has invalid shell syntax'
run_with_deadline bash "$passt_preflight_path"
phase_pass

phase_start rust-toolchain
if [[ "$runner_image_ready" == true ]]; then
  printf 'HEPH_GCP_RUNNER_IMAGE phase=rust-toolchain status=prebuilt\n'
else
run_with_deadline "${forge_env[@]}" rustup toolchain install "$rust_version" --profile minimal --no-self-update
run_with_deadline "${forge_env[@]}" rustup default "$rust_version"
run_with_deadline "${forge_env[@]}" rustup target add x86_64-unknown-linux-musl
fi
phase_pass
phase_start libkrunfw
if [[ "$runner_image_ready" == true ]]; then
  printf 'HEPH_GCP_RUNNER_IMAGE phase=libkrunfw status=prebuilt\n'
else
install -d -m 0700 -o forge -g forge "$source_root"
run_with_deadline "${forge_env[@]}" git clone --depth 1 --branch "$libkrunfw_tag" \
  https://github.com/libkrun/libkrunfw.git "$source_root/libkrunfw"
libkrunfw_revision="$("${forge_env[@]}" git -C "$source_root/libkrunfw" rev-parse HEAD)"
run_logged_forge_command "${temporary_root}/libkrunfw-build.log" libkrunfw \
  make --no-print-directory -C "$source_root/libkrunfw" -j8
run_with_deadline make --no-print-directory -C "$source_root/libkrunfw" PREFIX=/usr/local install
printf '/usr/local/lib64\n' >/etc/ld.so.conf.d/hephaestus-libkrun.conf
run_with_deadline ldconfig
libkrunfw_so="$(find /usr/local/lib64 -maxdepth 1 -type f -name 'libkrunfw.so.5*' -print -quit)"
[[ -n "$libkrunfw_so" ]] || die 'libkrunfw install artifact is missing'
readelf -d "$libkrunfw_so" | grep -q 'SONAME.*libkrunfw\.so\.5' || die 'libkrunfw SONAME is incompatible'
fi
phase_pass
phase_start libkrun
if [[ "$runner_image_ready" == true ]]; then
  printf 'HEPH_GCP_RUNNER_IMAGE phase=libkrun status=prebuilt\n'
else
run_with_deadline "${forge_env[@]}" git clone --depth 1 --branch "$libkrun_tag" \
  https://github.com/libkrun/libkrun.git "$source_root/libkrun"
[[ "$("${forge_env[@]}" git -C "$source_root/libkrun" rev-parse HEAD)" == "$libkrun_revision_pin" ]] ||
  die 'libkrun source revision verification failed'
libkrun_revision="$("${forge_env[@]}" git -C "$source_root/libkrun" rev-parse HEAD)"
python3 - "$source_root/libkrun/init/dhcp.c" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
source = path.read_text()

old_signature = '''static int mod_route4(int nl_sock, int iface_index, int cmd, struct in_addr gw)
'''
new_signature = '''/* Add a route with an optional directly-connected gateway. */
static int mod_route4(int nl_sock, int iface_index, int cmd,
                      struct in_addr dst, unsigned char prefix_len,
                      struct in_addr gw)
'''
old_setup = '''    struct rtmsg *rtm;
    struct in_addr dst = {.s_addr = INADDR_ANY};
'''
new_setup = '''    struct rtmsg *rtm;
'''
old_route_fields = '''    rtm->rtm_dst_len = 0;
    rtm->rtm_src_len = 0;
    rtm->rtm_tos = 0;
    rtm->rtm_table = RT_TABLE_MAIN;
    rtm->rtm_protocol = RTPROT_BOOT;
    rtm->rtm_scope = RT_SCOPE_UNIVERSE;
'''
new_route_fields = '''    rtm->rtm_dst_len = prefix_len;
    rtm->rtm_src_len = 0;
    rtm->rtm_tos = 0;
    rtm->rtm_table = RT_TABLE_MAIN;
    rtm->rtm_protocol = RTPROT_BOOT;
    rtm->rtm_scope = gw.s_addr == INADDR_ANY ? RT_SCOPE_LINK : RT_SCOPE_UNIVERSE;
'''
old_attributes = '''    add_rtattr(nlh, RTA_OIF, &iface_index, sizeof(iface_index));
    add_rtattr(nlh, RTA_DST, &dst, sizeof(dst));
    add_rtattr(nlh, RTA_GATEWAY, &gw, sizeof(gw));
'''
new_attributes = '''    add_rtattr(nlh, RTA_OIF, &iface_index, sizeof(iface_index));
    add_rtattr(nlh, RTA_DST, &dst, sizeof(dst));
    if (gw.s_addr != INADDR_ANY)
        add_rtattr(nlh, RTA_GATEWAY, &gw, sizeof(gw));
'''
old_call = '''    if (mod_route4(nl_sock, iface_index, RTM_NEWROUTE, router) != 0) {
        printf("couldn't add the default route provided by the DHCP server\\n");
        return -1;
    }
'''
new_call = '''    /* GCE presents the host as /32, so its off-subnet DHCP gateway needs
     * an explicit link-scoped host route before installing the default. */
    struct in_addr no_gateway = {.s_addr = INADDR_ANY};
    if (router.s_addr != INADDR_ANY &&
        (addr.s_addr & netmask.s_addr) != (router.s_addr & netmask.s_addr) &&
        mod_route4(nl_sock, iface_index, RTM_NEWROUTE, router, 32,
                   no_gateway) != 0) {
        printf("couldn't add the DHCP gateway host route\\n");
        return -1;
    }
    if (mod_route4(nl_sock, iface_index, RTM_NEWROUTE, no_gateway, 0,
                   router) != 0) {
        printf("couldn't add the default route provided by the DHCP server\\n");
        return -1;
    }
'''
for old, new in (
    (old_signature, new_signature),
    (old_setup, new_setup),
    (old_route_fields, new_route_fields),
    (old_attributes, new_attributes),
    (old_call, new_call),
):
    count = source.count(old)
    if count != 1:
        raise SystemExit(f'expected pinned libkrun DHCP patch site was not unique: {count}')
    source = source.replace(old, new)
path.write_text(source)
PY
run_with_deadline "${forge_env[@]}" make --no-print-directory -C "$source_root/libkrun" BLK=1 NET=1 -j8
run_with_deadline make --no-print-directory -C "$source_root/libkrun" BLK=1 NET=1 PREFIX=/usr/local install
run_with_deadline ldconfig
libkrun_so="$(find /usr/local/lib64 -maxdepth 1 -type f -name 'libkrun.so.1*' -print -quit)"
[[ -n "$libkrun_so" ]] || die 'libkrun install artifact is missing'
readelf -d "$libkrun_so" | grep -q 'SONAME.*libkrun\.so\.1' || die 'libkrun SONAME is incompatible'
# Read the complete cache before matching: with pipefail, grep -q can close
# early and make ldconfig report SIGPIPE on hosts with a large cache.
ldconfig -p | grep 'libkrun\.so\.1' >/dev/null || die 'libkrun.so.1 missing from loader cache'
ldconfig -p | grep 'libkrunfw\.so\.5' >/dev/null || die 'libkrunfw.so.5 missing from loader cache'
printf 'HEPH_GCP_KVM_LIBS libkrun_tag=%s commit=%s libkrunfw_tag=%s commit=%s features=blk,net\n' \
  "$libkrun_tag" "$libkrun_revision" "$libkrunfw_tag" "$libkrunfw_revision"
fi
phase_pass
if [[ "${HEPH_GCP_IMAGE_BAKE:-0}" == 1 ]]; then
  phase_start image-bake-ready
  phase_pass
  exit 0
fi

phase_start checkout
run_with_deadline "${forge_env[@]}" git clone --filter=blob:none --no-checkout "$repository_url" "$checkout_root"
run_with_deadline "${forge_env[@]}" git -C "$checkout_root" fetch --depth 1 origin "$revision"
run_with_deadline "${forge_env[@]}" git -C "$checkout_root" checkout --detach "$revision"
[[ "$("${forge_env[@]}" git -C "$checkout_root" rev-parse HEAD)" == "$revision" ]] || die 'checkout SHA mismatch'
phase_pass

if [[ "$test_mode" == gcp-cooking ]]; then
  phase_start gcp-cooking
  cooking_deadline_epoch=$(( $(date +%s) + $(remaining_seconds) ))
  run_with_deadline env \
    HEPH_GCP_COOKING_DEADLINE_EPOCH="$cooking_deadline_epoch" \
    HEPH_GCP_RUN_ID="${GITHUB_RUN_ID:-manual}" \
    HEPH_GCP_RUNNER_IMAGE_VERIFIED="$runner_image_ready" \
    HEPH_GCP_RUNNER_IMAGE_MANIFEST_SHA256="$runner_image_manifest_sha" \
    HEPH_GCP_RUNNER_IMAGE_BROWSER_LOCK_SHA256="$runner_image_browser_lock_sha" \
    HEPH_GCP_RUNNER_IMAGE_BROWSER_VERSION="$runner_image_browser_version" \
    HEPH_GCP_RUNNER_IMAGE_NODE_VERSION="${HEPH_IMAGE_NODE_VERSION:-v24.16.0}" \
    "$checkout_root/scripts/gcp-cooking-run.sh"
  phase_pass
else

phase_start real-libkrun-smoke
smoke_unit="heph-gcp-kvm-smoke-${GITHUB_RUN_ID:-manual}"
diagnostics_journal_unit="$smoke_unit"
smoke_log_dir="${evidence_root}/integration"
install -d -m 0700 -o forge -g forge "$smoke_log_dir"
smoke_output_log="${smoke_log_dir}/smoke-unit.log"
smoke_remaining="$(remaining_seconds)"
set +e
run_with_deadline systemd-run --unit="$smoke_unit" --service-type=oneshot --wait --pipe --collect \
  --expand-environment=no \
  --property=Delegate=yes --property=RuntimeMaxSec="${smoke_remaining}s" \
  --property=TimeoutStartSec="${smoke_remaining}s" --property=TimeoutStopSec=15s \
  --property=TasksMax=infinity --property=LimitNOFILE=65536 \
  --property=CPUAccounting=yes --property=MemoryAccounting=yes --property=TasksAccounting=yes \
  --property=IOAccounting=yes --uid="$forge_uid" --gid="$forge_gid" \
  --working-directory="$checkout_root" --setenv=HOME=/home/forge \
  --setenv=XDG_RUNTIME_DIR=/run/user/10001 --setenv=RUSTUP_HOME=/home/forge/.rustup \
  --setenv=CARGO_HOME=/home/forge/.cargo \
  --setenv=LIBCLANG_PATH="$libclang_dir" \
  --setenv=PATH=/home/forge/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  --setenv=HEPH_GCP_SMOKE_SCRIPT="$checkout_root/scripts/run-libkrun-integration.sh" \
  --setenv=HEPH_GCP_SMOKE_IMAGE="$guest_image" \
  --setenv=HEPH_GCP_SMOKE_TMP="$smoke_temporary_root" \
  --setenv=HEPH_GCP_SMOKE_DIAGNOSTICS="$smoke_log_dir" \
  /bin/bash -Eeuo pipefail -c '
    candidate="/sys/fs/cgroup$(awk -F: '\''$1 == "0" { print $3 }'\'' /proc/self/cgroup)"
    test -d "$candidate" -a -w "$candidate" -a -w "$candidate/cgroup.subtree_control"
    cgroup_type="$(<"$candidate/cgroup.type")"
    printf "HEPH_GCP_KVM_CGROUP parent=%s type=%s\n" "$candidate" "$cgroup_type"
    [[ "$cgroup_type" == domain ]]
    manager="$candidate/heph-smoke-manager"
    mkdir "$manager"
    # Leave the shell in this child until exit; systemd removes the empty
    # delegated child with the transient unit after the smoke returns.
    pid="$BASHPID"
    printf "%s\n" "$pid" >"$manager/cgroup.procs"
    available="$(<"$candidate/cgroup.controllers")"
    for controller in cpu io memory pids; do
      [[ " $available " == *" $controller "* ]]
    done
    printf "+cpu +io +memory +pids\n" >"$candidate/cgroup.subtree_control"
    enabled="$(<"$candidate/cgroup.subtree_control")"
    for controller in cpu io memory pids; do
      [[ " $enabled " == *" $controller "* ]]
    done
    /usr/bin/env HEPHAESTUS_LIBKRUN_INTEGRATION=1 \
      HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE="$HEPH_GCP_SMOKE_IMAGE" \
      HEPHAESTUS_LIBKRUN_TMP_ROOT="$HEPH_GCP_SMOKE_TMP" \
      HEPHAESTUS_LIBKRUN_DIAGNOSTICS_DIR="$HEPH_GCP_SMOKE_DIAGNOSTICS" \
      "$HEPH_GCP_SMOKE_SCRIPT"
  ' 2>&1 | tee "$smoke_output_log"
pipeline_status=("${PIPESTATUS[@]}")
set -e
smoke_status="${pipeline_status[0]}"
tee_status="${pipeline_status[1]}"
if ((tee_status != 0)); then
  printf 'HEPH_GCP_KVM_SMOKE output_capture=fail exit=%s path=%s\n' "$tee_status" "$smoke_output_log"
  if ((smoke_status == 0)); then
    smoke_status=1
  fi
fi
stop_smoke_unit() {
  local stop_status=0 kill_status=0
  command -v systemctl >/dev/null 2>&1 || {
    printf 'HEPH_GCP_KVM_SMOKE cleanup=fail reason=systemctl-missing\n'
    return 1
  }
  if systemctl is-active --quiet "$smoke_unit"; then
    timeout --kill-after=5s 15s systemctl stop "$smoke_unit" || stop_status=$?
    if ((stop_status != 0)) || systemctl is-active --quiet "$smoke_unit"; then
      timeout --kill-after=2s 5s systemctl kill --kill-who=all "$smoke_unit" || kill_status=$?
      timeout --kill-after=5s 15s systemctl stop "$smoke_unit" || true
    fi
  fi
  if systemctl is-active --quiet "$smoke_unit"; then
    printf 'HEPH_GCP_KVM_SMOKE cleanup=fail stop=%s kill=%s\n' "$stop_status" "$kill_status"
    return 1
  fi
  printf 'HEPH_GCP_KVM_SMOKE cleanup=pass stop=%s kill=%s\n' "$stop_status" "$kill_status"
}
if ((smoke_status != 0)); then
  # systemd normally reaps a oneshot before --wait returns.  Explicitly stop
  # and, if needed, kill the unit so failed clients cannot outlive collection.
  stop_smoke_unit || true
fi
((smoke_status == 0)) || exit "$smoke_status"
phase_pass
fi
if [[ "$test_mode" == gcp-cooking ]]; then
  printf 'HEPH_GCP_COOKING_EVIDENCE revision=%s diagnostics=%s\n' \
    "$revision" "${evidence_root}/cooking"
else
  emit_kvm_evidence
fi
