#!/usr/bin/env bash
# Read the regional quota or create one disposable nested-KVM smoke VM.
# Smoke and Cooking modes use only the reviewed runtime identity for private
# diagnostics; preflight and image operations remain credential-free.
set -Eeuo pipefail

readonly PROJECT_ID="hephaestus-508000"
readonly REGION="europe-west1"
readonly STARTUP_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/gcp-kvm-startup.sh"
readonly PASST_PREFLIGHT_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/gcp-passt-preflight.sh"
readonly DIAGNOSTICS_COLLECTOR_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/collect-cooking-diagnostics.py"
readonly DIAGNOSTICS_SCANNER_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/check-browser-evidence.py"
readonly DIAGNOSTICS_TRIAGE_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/summarize-cooking-diagnostics.py"
readonly COOKING_GATE_RESULTS_HELPER_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/cooking-gate-results.py"
readonly COOKING_RUNTIME_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/gcp-cooking-run.sh"
readonly COOKING_BROWSER_SUMMARY_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/project-playwright-browser-summary.py"
readonly PHASE_TIMING_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/gcp_phase_timing.py"
readonly PR_PROVENANCE_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/verify-gcp-pr-provenance.py"
readonly RUNNER_IMAGE_COMPATIBILITY_VALIDATOR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/gcp-runner-image-compatibility.py"
readonly RUNNER_IMAGE_COMPATIBILITY_RECORDS="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/gcp-runner-image-compatibility.json"
readonly MACHINE_TYPE="n2-standard-8"
readonly DISK_SIZE="150GB"
readonly CACHE_BUCKET="hephaestus-508000-cooking-cache"
readonly CACHE_OBJECT="cooking/heph-gcp-cooking-cache.tar.zst"
# These values come from the reviewed local cache archive uploaded for Cooking.
# The preflight checks object metadata before any VM is created; the helper still
# verifies the archive SHA-256 after download.
readonly CACHE_SIZE_BYTES="1783474345"
readonly CACHE_MD5_BASE64="di95x0b0Yqqt4RTyVUvb6A=="
readonly DIAGNOSTICS_BUCKET="hephaestus-508000-cooking-diagnostics"
readonly DIAGNOSTICS_OBJECT_PREFIX="cooking/runs"
readonly DIAGNOSTIC_MACHINE_TYPE="e2-small"
readonly DIAGNOSTIC_DISK_SIZE="20GB"
smoke_created=false
smoke_name=""
smoke_zone=""
smoke_log="${GCP_SMOKE_LOG:-}"
diagnostics_download_error='download-failed'
diagnostics_cleanup_state='unverified'
diagnostics_upload_state='unknown'
diagnostics_download_state='failed'
diagnostics_scan_state='not-run'
diagnostics_triage_state='not-run'
diagnostics_source_mode=false
diagnostics_source_run_id=''
diagnostics_source_attempt=''
diagnostics_source_sha=''
diagnostics_controller_sha=''
diagnostics_source_zone=''
validated_source_sha=''
pr_workload_mode=false
cache_generation=''
gcloud_json_output=''
gcloud_json_stderr=''
controller_phase_timing_path="${HEPH_GCP_CONTROLLER_PHASE_TIMING_PATH:-${RUNNER_TEMP:-/tmp}/gcp-phase-timing-controller-${GITHUB_RUN_ID:-manual}-${GITHUB_RUN_ATTEMPT:-1}-$(date +%s%N)-$$.jsonl}"
controller_phase_timing_open=''

die() { printf 'gcp-kvm-smoke: %s\n' "$*" >&2; exit 1; }

controller_phase_timing_start() {
  local name="$1" source_sha="${GCP_WORKLOAD_SHA:-${validated_source_sha:-${GITHUB_SHA:-}}}"
  [[ "$source_sha" =~ ^[0-9a-f]{40}$ ]] || return 0
  python3 -B "$PHASE_TIMING_SCRIPT" start \
    --path "$controller_phase_timing_path" --phase "$name" --trust supervisor \
    --clock-domain controller --run-id "${GITHUB_RUN_ID:-manual}" \
    --attempt "${GITHUB_RUN_ATTEMPT:-1}" --source-sha "$source_sha"
  controller_phase_timing_open="$name"
}

controller_phase_timing_end() {
  local outcome="$1" source_sha="${GCP_WORKLOAD_SHA:-${validated_source_sha:-${GITHUB_SHA:-}}}"
  [[ -n "$controller_phase_timing_open" ]] || return 0
  python3 -B "$PHASE_TIMING_SCRIPT" end \
    --path "$controller_phase_timing_path" --phase "$controller_phase_timing_open" \
    --trust supervisor --clock-domain controller --run-id "${GITHUB_RUN_ID:-manual}" \
    --attempt "${GITHUB_RUN_ATTEMPT:-1}" --source-sha "$source_sha" --outcome "$outcome"
  controller_phase_timing_open=''
}

controller_phase_timing_finish_open() {
  local status="$1" outcome='failed'
  [[ -n "$controller_phase_timing_open" ]] || return 0
  if ((status == 124)); then outcome='timed-out'; fi
  if ((status == 130 || status == 143)); then outcome='cancelled'; fi
  controller_phase_timing_end "$outcome" || true
}

timed_controller_phase() {
  local name="$1"; shift
  controller_phase_timing_start "$name"
  set +e
  "$@"
  local status=$?
  set -e
  if ((status == 0)); then
    controller_phase_timing_end passed
  else
    controller_phase_timing_finish_open "$status"
  fi
  return "$status"
}

run_json_gcloud() {
  local stderr_file rc
  stderr_file="$(mktemp "${TMPDIR:-/tmp}/gcp-kvm-smoke-gcloud-stderr.XXXXXX")"
  if gcloud_json_output="$(gcloud "$@" 2>"$stderr_file")"; then
    rc=0
  else
    rc=$?
  fi
  gcloud_json_stderr="$(cat "$stderr_file")"
  rm -f -- "$stderr_file"
  return "$rc"
}

report_gcloud_json_stderr() {
  if [[ -n "$gcloud_json_stderr" ]]; then
    printf '%s\n' "$gcloud_json_stderr" >&2
  fi
  return 0
}

require_commands() {
  command -v gcloud >/dev/null 2>&1 || die 'gcloud is unavailable'
  command -v python3 >/dev/null 2>&1 || die 'python3 is unavailable'
  command -v timeout >/dev/null 2>&1 || die 'timeout is unavailable'
  command -v sha256sum >/dev/null 2>&1 || die 'sha256sum is unavailable'
}

quota_preflight() {
  local mode="${1:-full}" quota_json
  if run_json_gcloud compute regions describe "$REGION" --project="$PROJECT_ID" --format=json; then
    quota_json="$gcloud_json_output"
    report_gcloud_json_stderr
  else
    printf '%s\n' "$gcloud_json_stderr" >&2
    die "cannot read regional quota for $REGION"
  fi
  python3 -c 'import json,sys
quotas={q.get("metric"):q for q in json.load(sys.stdin).get("quotas",[])}
required={"INSTANCES":1}
if sys.argv[1] == "full": required["N2_CPUS"] = 8
for metric,need in required.items():
    q=quotas.get(metric)
    if not q: raise SystemExit(f"missing quota metric {metric}")
    available=float(q.get("limit",0))-float(q.get("usage",0))
    print("{}: usage={} limit={} available={:g}".format(metric, q.get("usage",0), q.get("limit",0), available))
    if available < need: raise SystemExit(f"{metric} available quota {available:g} is below required {need}")' "$mode" <<<"$quota_json" ||
    die 'regional quota is insufficient for the disposable smoke VM'
  if [[ "$mode" == full ]]; then
    printf 'Quota preflight passed for %s in %s; capacity is not guaranteed.\n' "$MACHINE_TYPE" "$REGION"
  else
    printf 'Diagnostic VM quota preflight passed for %s in %s; capacity is not guaranteed.\n' "$DIAGNOSTIC_MACHINE_TYPE" "$REGION"
  fi
}

cache_preflight() {
  local object_uri="gs://${CACHE_BUCKET}/${CACHE_OBJECT}"
  local describe_output describe_status list_output list_status metadata_output
  if run_json_gcloud storage objects describe "$object_uri" \
      --project="$PROJECT_ID" --billing-project="$PROJECT_ID" \
      --raw --format='json(name,size,md5Hash,generation)'; then
    describe_output="$gcloud_json_output"
    report_gcloud_json_stderr
    if ! metadata_output="$(python3 -c '
import json
import sys

object_name, expected_size, expected_md5 = sys.argv[1:]
data = json.load(sys.stdin)
if not isinstance(data, dict):
    raise SystemExit("object metadata is not a JSON object")
required = ("name", "size", "md5Hash", "generation")
if any(key not in data for key in required):
    raise SystemExit("object metadata is missing name, size, md5Hash, or generation")
try:
    size = int(data["size"])
    generation = int(data["generation"])
except (TypeError, ValueError):
    raise SystemExit("object metadata size or generation is not an integer")
md5_hash = data["md5Hash"]
name = data["name"]
if not isinstance(name, str) or not isinstance(md5_hash, str):
    raise SystemExit("object metadata name or md5Hash is not a string")
if name != object_name:
    raise SystemExit("object metadata name does not match the reviewed object")
if size != int(expected_size):
    raise SystemExit(f"cache size mismatch: expected {expected_size}, got {size}")
if md5_hash != expected_md5:
    raise SystemExit(f"cache MD5 mismatch: expected {expected_md5}, got {md5_hash}")
print(f"Private Cooking cache metadata: name={name} size={size} md5Hash={md5_hash} generation={generation}")
' "$CACHE_OBJECT" "$CACHE_SIZE_BYTES" "$CACHE_MD5_BASE64" <<<"$describe_output")"; then
      printf 'Cache object metadata validation failed for %s:\n' "$object_uri" >&2
      sed -n '1,20p' <<<"$metadata_output" >&2
      die 'private Cooking cache metadata does not match the reviewed local archive'
    fi
    cache_generation="$(sed -n 's/.* generation=\([0-9][0-9]*\)$/\1/p' <<<"$metadata_output")"
    [[ "$cache_generation" =~ ^[1-9][0-9]*$ ]] ||
      die 'private Cooking cache generation is missing or invalid'
    printf '%s\n' "$metadata_output"
    return 0
  else
    describe_status=$?
    describe_output="$gcloud_json_stderr"
  fi
  printf 'Cache object lookup failed (exit=%s) for %s:\n' \
    "$describe_status" "$object_uri" >&2
  sed -n '1,20p' <<<"$describe_output" >&2
  if list_output="$(gcloud storage objects list "gs://${CACHE_BUCKET}" \
      --project="$PROJECT_ID" --billing-project="$PROJECT_ID" \
      --format='value(name)' --limit=30 2>&1)"; then
    printf 'Objects visible in the designated cache bucket (up to 30):\n' >&2
    sed -n '1,30p' <<<"$list_output" >&2
  else
    list_status=$?
    printf 'Designated cache bucket listing failed (exit=%s):\n' "$list_status" >&2
    sed -n '1,20p' <<<"$list_output" >&2
  fi
  die "required private Cooking cache object is unavailable: $object_uri"
}

validate_workload_provenance() {
  local mode="${1:-gcp-cooking}"
  local pull_number="${GCP_PR_NUMBER:-}" repository_id="${GCP_PR_REPOSITORY_ID:-}"
  local head_sha="${GCP_PR_HEAD_SHA:-}" any=false response auth_file api_status
  [[ -f "$PR_PROVENANCE_SCRIPT" && ! -L "$PR_PROVENANCE_SCRIPT" ]] ||
    die 'PR provenance validator is unavailable or symlinked'
  for value in "$pull_number" "$repository_id" "$head_sha"; do
    [[ -z "$value" ]] || any=true
  done
  [[ "$any" == false || "$mode" == gcp-cooking ]] ||
    die 'PR provenance inputs are supported only for gcp-cooking'
  if [[ "$any" == false ]]; then
    validated_source_sha="${GCP_WORKLOAD_SHA:-${GITHUB_SHA:-}}"
    [[ "$validated_source_sha" =~ ^[0-9a-f]{40}$ ]] ||
      die 'GCP_WORKLOAD_SHA or GITHUB_SHA must be an exact 40-character commit SHA'
    pr_workload_mode=false
    return 0
  fi
  [[ -n "$pull_number" && -n "$repository_id" && -n "$head_sha" ]] ||
    die 'PR provenance inputs must be supplied together'
  [[ -n "${GH_TOKEN:-}" ]] || die 'GH_TOKEN is required to validate PR provenance'
  auth_file="$(mktemp "${TMPDIR:-/tmp}/gcp-pr-auth.XXXXXX")" || die 'cannot create PR auth file'
  response="$(mktemp "${TMPDIR:-/tmp}/gcp-pr-response.XXXXXX")" || {
    rm -f -- "$auth_file"
    die 'cannot create PR response file'
  }
  chmod 0600 "$auth_file" "$response"
  printf 'header = "Authorization: Bearer %s"\n' "$GH_TOKEN" >"$auth_file"
  set +e
  curl --fail --silent --show-error --connect-timeout 5 --max-time 20 \
    --max-filesize 1048576 --config "$auth_file" \
    -H 'Accept: application/vnd.github+json' \
    "https://api.github.com/repos/wimpheling/hephaestus/pulls/${pull_number}" \
    >"$response"
  api_status=$?
  set -e
  rm -f -- "$auth_file"
  if ((api_status != 0)); then
    rm -f -- "$response"
    die 'GitHub PR provenance lookup failed'
  fi
  if ! python3 "$PR_PROVENANCE_SCRIPT" --response-file "$response" \
      --pull-number "$pull_number" --repository-id "$repository_id" --head-sha "$head_sha"; then
    rm -f -- "$response"
    die 'GitHub PR provenance validation failed'
  fi
  rm -f -- "$response"
  validated_source_sha="$head_sha"
  pr_workload_mode=true
}

configure_diagnostics_source() {
  local historical="${1:-false}" explicit=false name value
  if [[ "$historical" == true ]]; then
    for name in GCP_DIAGNOSTICS_SOURCE_RUN_ID GCP_DIAGNOSTICS_SOURCE_ATTEMPT \
      GCP_DIAGNOSTICS_SOURCE_SHA GCP_DIAGNOSTICS_SOURCE_ZONE; do
      value="${!name:-}"
      [[ -z "$value" ]] || explicit=true
    done
  fi
  if [[ "$historical" == true && ("$explicit" == true || "${GCP_DIAGNOSTICS_REQUIRE_SOURCE:-false}" == true) ]]; then
    [[ -n "${GCP_DIAGNOSTICS_SOURCE_RUN_ID:-}" &&
      -n "${GCP_DIAGNOSTICS_SOURCE_ATTEMPT:-}" &&
      -n "${GCP_DIAGNOSTICS_SOURCE_SHA:-}" ]] || {
      diagnostics_download_error='source-identity-incomplete'
      return 1
    }
    diagnostics_source_mode=true
    diagnostics_source_run_id="$GCP_DIAGNOSTICS_SOURCE_RUN_ID"
    diagnostics_source_attempt="$GCP_DIAGNOSTICS_SOURCE_ATTEMPT"
    diagnostics_source_sha="$GCP_DIAGNOSTICS_SOURCE_SHA"
    diagnostics_controller_sha="${GCP_DIAGNOSTICS_CONTROLLER_SHA:-$diagnostics_source_sha}"
    diagnostics_source_zone="${GCP_DIAGNOSTICS_SOURCE_ZONE:-${GCP_ZONE:-}}"
  else
    diagnostics_source_run_id="${GITHUB_RUN_ID:-}"
    diagnostics_source_attempt="${GITHUB_RUN_ATTEMPT:-}"
    diagnostics_source_sha="${validated_source_sha:-${GCP_WORKLOAD_SHA:-${GITHUB_SHA:-}}}"
    diagnostics_controller_sha="${GITHUB_SHA:-}"
    diagnostics_source_zone="${GCP_ZONE:-}"
  fi
  [[ "$diagnostics_source_run_id" =~ ^[0-9]+$ &&
    "$diagnostics_source_attempt" =~ ^[0-9]+$ &&
    "$diagnostics_source_sha" =~ ^[0-9a-f]{40}$ ]] || {
    diagnostics_download_error='source-identity-invalid'
    return 1
  }
  [[ "$diagnostics_controller_sha" =~ ^[0-9a-f]{40}$ ]] || {
    diagnostics_download_error='controller-identity-invalid'
    return 1
  }
  if [[ "$diagnostics_source_mode" == true && "$diagnostics_source_zone" != "$REGION"-* ]]; then
    diagnostics_download_error='source-zone-invalid'
    return 1
  fi
}

diagnostics_object_for_run() {
  configure_diagnostics_source || return 1
  printf '%s/%s/%s/%s.tar.gz\n' "$DIAGNOSTICS_OBJECT_PREFIX" \
    "$diagnostics_source_run_id" "$diagnostics_source_attempt" "$diagnostics_source_sha"
}

verify_github_source_run() {
  [[ "$diagnostics_source_mode" == true ]] || return 0
  local token="${GH_TOKEN:-${GITHUB_TOKEN:-}}" repository="${GITHUB_REPOSITORY:-}"
  local auth_file api_output
  [[ -n "$token" ]] || {
    diagnostics_download_error='source-run-auth-missing'
    return 1
  }
  [[ "$repository" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || {
    diagnostics_download_error='source-repository-invalid'
    return 1
  }
  auth_file="$(mktemp "${TMPDIR:-/tmp}/gcp-diagnostics-gh-auth.XXXXXX")" || {
    diagnostics_download_error='source-run-auth-file-failed'
    return 1
  }
  chmod 0600 "$auth_file"
  printf 'header = "Authorization: Bearer %s"\n' "$token" >"$auth_file"
  if ! api_output="$(curl --fail --silent --show-error --connect-timeout 5 --max-time 20 \
      --max-filesize 1048576 \
      --config "$auth_file" -H 'Accept: application/vnd.github+json' \
      "https://api.github.com/repos/${repository}/actions/runs/${diagnostics_source_run_id}/attempts/${diagnostics_source_attempt}" \
      2>/dev/null)"; then
    rm -f -- "$auth_file"
    diagnostics_download_error='source-run-lookup-failed'
    return 1
  fi
  rm -f -- "$auth_file"
  if ! python3 - "$api_output" "$diagnostics_controller_sha" "$diagnostics_source_attempt" <<'PY'
import json
import sys

value = json.loads(sys.argv[1])
expected_controller_sha, expected_attempt = sys.argv[2:]
if not isinstance(value, dict):
    raise SystemExit("source run response is not an object")
if value.get("head_sha") != expected_controller_sha:
    raise SystemExit("controller run SHA does not match")
if value.get("path") != ".github/workflows/cooking-e2e.yml":
    raise SystemExit("source run workflow does not match")
if value.get("head_branch") != "main":
    raise SystemExit("source run branch does not match")
if str(value.get("run_attempt")) != expected_attempt:
    raise SystemExit("source run attempt does not match")
PY
  then
    diagnostics_download_error='source-run-identity-mismatch'
    return 1
  fi
}

verify_disposable_vm_absent() {
  local zone="$diagnostics_source_zone" name="heph-kvm-smoke-${diagnostics_source_run_id}-${diagnostics_source_attempt}" describe_output
  diagnostics_cleanup_state='unverified'
  if [[ "$diagnostics_source_mode" == true ]]; then
    if ! run_json_gcloud compute instances list --project="$PROJECT_ID" \
        --filter="name=${name}" --format='json(name)'; then
      report_gcloud_json_stderr
      return 1
    fi
    local list_state
    if ! list_state="$(python3 -c '
import json
import sys

target = sys.argv[1]
value = json.load(sys.stdin)
if not isinstance(value, list):
    raise SystemExit("instance list response is not an array")
print("present" if any(isinstance(row, dict) and row.get("name") == target for row in value) else "absent")
' "$name" <<<"$gcloud_json_output")"; then
      return 1
    fi
    if [[ "$list_state" == present ]]; then
      printf 'cleanup verification found historical disposable VM still present: %s\n' "$name" >&2
      return 1
    fi
    diagnostics_cleanup_state='verified-absent'
    printf 'cleanup verified historical VM absent project-wide: %s\n' "$name"
    return 0
  fi
  [[ "$zone" == "$REGION"-* ]] || { printf 'cleanup zone is outside %s: %s\n' "$REGION" "$zone" >&2; return 1; }
  if describe_output="$(gcloud compute instances describe "$name" --project="$PROJECT_ID" --zone="$zone" 2>&1)"; then
    printf 'cleanup verification found disposable VM still present: %s\n' "$name" >&2
    return 1
  fi
  if grep -Eqi "instances/${name}[^[:alnum:]_].*was not found" <<<"$describe_output"; then
    diagnostics_cleanup_state='verified-absent'
    printf 'cleanup verified VM absent: %s (%s)\n' "$name" "$zone"
    return 0
  fi
  printf 'cleanup verification was inconclusive for %s:\n%s\n' "$name" "$describe_output" >&2
  return 1
}

_download_diagnostics() {
  local object="${DIAGNOSTICS_OBJECT_PREFIX}/${diagnostics_source_run_id}/${diagnostics_source_attempt}/${diagnostics_source_sha}.tar.gz"
  local destination="${GCP_DIAGNOSTICS_ARCHIVE:-${RUNNER_TEMP:-/tmp}/gcp-diagnostics.tar.gz}"
  local status_path="${GCP_DIAGNOSTICS_STATUS:-${RUNNER_TEMP:-/tmp}/gcp-diagnostics-status.json}"
  local extract_root="${destination}.extract" output digest archive_bytes download_timeout
  local phase_timing_status='not-applicable' phase_timing_object phase_timing_destination
  local expected_mode="${GCP_DIAGNOSTICS_EXPECT_MODE:-}"
  local expected_gate_script_sha256="${GCP_DIAGNOSTICS_EXPECT_GATE_SCRIPT_SHA256:-}"
  local expectation_file="${RUNNER_TEMP:-/tmp}/gcp-diagnostics-gate-expectation"
  if [[ -z "$expected_mode" && -f "$expectation_file" && ! -L "$expectation_file" ]]; then
    read -r expected_mode expected_gate_script_sha256 <"$expectation_file" || {
      diagnostics_download_error='gate-expectation-invalid'
      return 1
    }
  fi
  download_timeout="${GCP_DIAGNOSTICS_DOWNLOAD_TIMEOUT_SECONDS:-60}"
  [[ "$download_timeout" =~ ^[1-9][0-9]*$ ]] || { diagnostics_download_error='download-timeout-invalid'; return 1; }
  if ! verify_github_source_run; then
    return 1
  fi
  controller_phase_timing_start cleanup-verification
  if ! verify_disposable_vm_absent; then
    diagnostics_download_error='cleanup-unverified'
    controller_phase_timing_finish_open 1
    return 1
  fi
  controller_phase_timing_end passed
  mkdir -p -- "$(dirname -- "$destination")" "$(dirname -- "$status_path")"
  rm -rf -- "$extract_root"
  mkdir -m 700 -- "$extract_root"
  printf 'HEPH_GCP_DIAGNOSTICS event=download status=start object=gs://%s/%s\n' "$DIAGNOSTICS_BUCKET" "$object"
  controller_phase_timing_start post-delete-download
  if ! output="$(timeout --kill-after=5s "${download_timeout}s" gcloud storage cp "gs://${DIAGNOSTICS_BUCKET}/${object}" "$destination" \
      --project="$PROJECT_ID" --billing-project="$PROJECT_ID" --quiet 2>&1)"; then
    diagnostics_download_error='provider-download-failed'
    controller_phase_timing_finish_open 1
    printf '{"schema":1,"object":"gs://%s/%s","download":"failed","error":"provider download failed"}\n' \
      "$DIAGNOSTICS_BUCKET" "$object" >"$status_path"
    printf '%s\n' "$output" >&2
    return 1
  fi
  controller_phase_timing_end passed
  [[ -f "$destination" && ! -L "$destination" ]] || {
    diagnostics_download_error='archive-missing'
    printf '{"schema":1,"object":"gs://%s/%s","download":"failed","error":"archive missing"}\n' \
      "$DIAGNOSTICS_BUCKET" "$object" >"$status_path"
    return 1
  }
  diagnostics_upload_state='verified-by-download'
  diagnostics_download_state='passed'
  archive_bytes="$(stat -c '%s' "$destination")"
  ((archive_bytes <= 67108864)) || {
    diagnostics_download_error='archive-too-large'
    printf '{"schema":1,"object":"gs://%s/%s","download":"failed","error":"archive too large"}\n' \
      "$DIAGNOSTICS_BUCKET" "$object" >"$status_path"
    return 1
  }
  digest="$(sha256sum "$destination" | awk '{print $1}')"
  tar -tzf "$destination" >/dev/null || {
    diagnostics_download_error='archive-format-invalid'
    diagnostics_scan_state='failed'
    printf '{"schema":1,"object":"gs://%s/%s","download":"failed","error":"archive scan failed"}\n' \
      "$DIAGNOSTICS_BUCKET" "$object" >"$status_path"
    return 1
  }
  if ! python3 - "$destination" <<'PY'
import pathlib
import sys
import tarfile
archive = pathlib.Path(sys.argv[1])
with tarfile.open(archive, mode="r:gz") as target:
    members = target.getmembers()
    if not members or len(members) > 1000:
        raise SystemExit("diagnostics archive member budget exceeded")
    for member in members:
        path = pathlib.PurePosixPath(member.name)
        if path.is_absolute() or ".." in path.parts or not member.name.startswith("cooking-diagnostics/"):
            raise SystemExit("diagnostics archive path is unsafe")
        if not member.isfile() or member.size > 16 * 1024 * 1024:
            raise SystemExit("diagnostics archive member is not a bounded regular file")
PY
  then
    diagnostics_download_error='archive-path-invalid'
    diagnostics_scan_state='failed'
    return 1
  fi
  if ! tar -xzf "$destination" -C "$extract_root" --no-same-owner --no-same-permissions; then
    diagnostics_download_error='archive-extraction-failed'
    diagnostics_scan_state='failed'
    return 1
  fi
  if ! python3 - "$extract_root/cooking-diagnostics" <<'PY'
import hashlib
import json
import pathlib
import re
import sys
root = pathlib.Path(sys.argv[1])
if not root.is_dir() or root.is_symlink(): raise SystemExit("diagnostics root missing")
manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
if manifest.get("credentialScan") != "passed": raise SystemExit("collector scan was not passed")
for record in manifest.get("sources", []):
    path = root / record["path"]
    if not path.is_file() or path.is_symlink() or ".." in pathlib.PurePosixPath(record["path"]).parts:
        raise SystemExit("manifest source path is unsafe")
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    if digest != record.get("sha256"): raise SystemExit("manifest source checksum mismatch")
for path in root.rglob("*"):
    if path.is_symlink(): raise SystemExit("diagnostics archive contains a symlink")
PY
  then
    diagnostics_download_error='manifest-validation-failed'
    diagnostics_scan_state='failed'
    return 1
  fi
  # Current Cooking/diagnostic workflows opt into a post-delete gate contract.
  # Structural gate failures are recorded and evaluated after the safe archive
  # scan/triage, so a missing or stale sidecar cannot erase useful evidence.
  local gate_validation_status=0 gate_validation='not-applicable'
  if [[ -n "$expected_mode" || -f "$extract_root/cooking-diagnostics/sources/gate-results" ]]; then
    gate_validation='passed'
    python3 - "$extract_root/cooking-diagnostics" "$diagnostics_source_sha" "$expected_mode" "$expected_gate_script_sha256" <<'PYGATE' || gate_validation_status=$?
import re
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
expected_revision = sys.argv[2]
expected_mode = sys.argv[3]
expected_script_sha256 = sys.argv[4]
if expected_script_sha256 and re.fullmatch(r"[0-9a-f]{64}", expected_script_sha256) is None:
    raise SystemExit("requested gate helper hash anchor is invalid")
manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
records = {record.get("label"): record for record in manifest.get("sources", [])}
record = records.get("gate-results")
if not isinstance(record, dict):
    raise SystemExit("current run is missing finalized gate results")
path = root / record.get("path", "")
if not path.is_file() or path.is_symlink():
    raise SystemExit("finalized gate result source is unavailable")
value = json.loads(path.read_text(encoding="utf-8"))
expected_fields = {"schema", "revision", "script_sha256", "test_mode", "overall_exit_code", "supervisor_exit_code", "finalized", "gates"}
if set(value) != expected_fields:
    raise SystemExit("finalized gate result fields are invalid")
if value["schema"] != 1 or value["revision"] != expected_revision:
    raise SystemExit("finalized gate result provenance does not match the selected run")
if value["test_mode"] not in {"gcp-cooking", "diagnostic"}:
    raise SystemExit("finalized gate result mode is invalid")
if expected_mode and value["test_mode"] != expected_mode:
    raise SystemExit("finalized gate result mode does not match the requested mode")
if not isinstance(value["script_sha256"], str) or len(value["script_sha256"]) != 64:
    raise SystemExit("finalized gate result helper hash is invalid")
if expected_script_sha256 and value["script_sha256"] != expected_script_sha256:
    raise SystemExit("finalized gate result helper hash does not match the requested checkout")
if value["finalized"] is not True:
    raise SystemExit("gate results are not finalized")
gates = value["gates"]
if not isinstance(gates, dict) or set(gates) != {"workload", "evidence-scan", "browser-validation"}:
    raise SystemExit("finalized gate result set is incomplete")
for gate in gates.values():
    if not isinstance(gate, dict) or set(gate) != {"state", "exit_code", "reason_class"}:
        raise SystemExit("finalized gate result entry is invalid")
    if gate["state"] not in {"passed", "failed", "timed-out", "unknown"}:
        raise SystemExit("finalized gate result state is invalid")
    if gate["exit_code"] is not None and not isinstance(gate["exit_code"], int):
        raise SystemExit("finalized gate result exit code is invalid")
PYGATE
    if ((gate_validation_status != 0)); then
      gate_validation='failed'
    fi
  fi
  if [[ ! -f "$DIAGNOSTICS_SCANNER_SCRIPT" ]]; then
    diagnostics_download_error='scanner-unavailable'
    diagnostics_scan_state='failed'
    return 1
  fi
  if ! python3 -B "$DIAGNOSTICS_SCANNER_SCRIPT" "$extract_root/cooking-diagnostics" >/dev/null; then
    diagnostics_download_error='credential-scan-failed'
    diagnostics_scan_state='failed'
    return 1
  fi
  local triage_path="$extract_root/cooking-diagnostics/triage.json"
  if ! python3 -B "$DIAGNOSTICS_TRIAGE_SCRIPT" "$extract_root/cooking-diagnostics" >"$triage_path"; then
    diagnostics_download_error='triage-projection-failed'
    diagnostics_scan_state='passed'
    diagnostics_triage_state='failed'
    printf '{"schema":1,"object":"gs://%s/%s","cleanup":"verified-absent","upload":"verified-by-download","download":"passed","scan":"passed","triage":"failed","error":"triage-projection-failed","archiveBytes":%s,"archiveSha256":"%s","manifest":"%s"}\n' \
      "$DIAGNOSTICS_BUCKET" "$object" "$archive_bytes" "$digest" "$extract_root/cooking-diagnostics/manifest.json" >"$status_path"
    return 1
  fi
  if [[ "$expected_mode" == gcp-cooking && "${GCP_EXPECT_PHASE_TIMING:-false}" == true ]]; then
    phase_timing_object="${object%.tar.gz}.phase-timing.json"
    phase_timing_destination="${GCP_PHASE_TIMING_PROJECTION:-${RUNNER_TEMP:-/tmp}/gcp-cooking-phase-timing.json}"
    rm -f -- "$phase_timing_destination"
    if ! timeout --kill-after=5s "${download_timeout}s" gcloud storage cp \
        "gs://${DIAGNOSTICS_BUCKET}/${phase_timing_object}" "$phase_timing_destination" \
        --project="$PROJECT_ID" --billing-project="$PROJECT_ID" --quiet >/dev/null 2>&1; then
      diagnostics_download_error='phase-timing-download-failed'
      return 1
    fi
    if [[ ! -f "$phase_timing_destination" || -L "$phase_timing_destination" ]] ||
        (( $(stat -c '%s' "$phase_timing_destination") > 65536 )); then
      diagnostics_download_error='phase-timing-too-large'
      return 1
    fi
    chmod 0600 "$phase_timing_destination"
    if ! python3 -B "$PHASE_TIMING_SCRIPT" validate-projection \
        --path "$phase_timing_destination" \
        --expected-run-id "$diagnostics_source_run_id" \
        --expected-attempt "$diagnostics_source_attempt" \
        --expected-source-sha "$diagnostics_source_sha" \
        --require-supervisor-phase archive --require-supervisor-phase evidence-scan \
        --require-supervisor-phase upload \
        --require-workload-phase dependency-setup --require-workload-phase project-build \
        --require-workload-phase browser-setup --require-workload-phase runtime-guest-build \
        --require-workload-phase runtime-worker-build --require-workload-phase oci-image-materialization \
        --require-workload-phase gateway-edge-ready --require-workload-phase gateway-services-ready \
        --require-workload-phase gateway-readiness --require-workload-phase oci-builder \
        --require-workload-phase oci-verifier --require-workload-phase golden-tests \
        --require-workload-phase database-tests --require-workload-phase browser-initial \
        --require-workload-phase browser-post-operation >/dev/null; then
      diagnostics_download_error='phase-timing-invalid'
      return 1
    fi
    phase_timing_status='passed'
  fi
  if ! python3 - "$status_path" "$triage_path" "$DIAGNOSTICS_BUCKET" "$object" "$archive_bytes" "$digest" "$extract_root/cooking-diagnostics/manifest.json" "$phase_timing_status" <<'PY'
import json
from pathlib import Path
import sys

status_path, triage_path, bucket, object_name, archive_bytes, digest, manifest, phase_timing = sys.argv[1:]
triage = json.loads(Path(triage_path).read_text(encoding="utf-8"))
if not isinstance(triage, dict) or triage.get("schema") != 1:
    raise SystemExit("triage projection has an invalid schema")
status = {
    "schema": 1,
    "object": f"gs://{bucket}/{object_name}",
    "cleanup": "verified-absent",
    "upload": "verified-by-download",
    "download": "passed",
    "scan": "passed",
    "triage": triage,
    "archiveBytes": int(archive_bytes),
    "archiveSha256": digest,
    "manifest": manifest,
}
if phase_timing != "not-applicable":
    status["phaseTiming"] = phase_timing
Path(status_path).write_text(json.dumps(status, separators=(",", ":")) + "\n", encoding="utf-8")
PY
  then
    diagnostics_download_error='triage-status-write-failed'
    diagnostics_triage_state='failed'
    return 1
  fi
  if [[ "$gate_validation" != not-applicable || -f "$extract_root/cooking-diagnostics/sources/gate-results" ]]; then
    local gate_acceptance_status=0
    python3 - "$status_path" "$extract_root/cooking-diagnostics" "$expected_mode" "$gate_validation" <<'PYGATE_ACCEPT' || gate_acceptance_status=$?
import json
from pathlib import Path
import sys

status_path, root_name, requested_mode, gate_validation = sys.argv[1:]
root = Path(root_name)
status = json.loads(Path(status_path).read_text(encoding="utf-8"))
gate_path = root / "sources" / "gate-results"
accepted = False
if gate_validation == "passed" and gate_path.is_file():
    value = json.loads(gate_path.read_text(encoding="utf-8"))
    mode = value["test_mode"]
    if requested_mode and mode != requested_mode:
        raise SystemExit("gate result mode acceptance mismatch")
    if mode == "gcp-cooking":
        accepted = (
            value["overall_exit_code"] == 0
            and value["supervisor_exit_code"] == 0
            and all(gate["state"] == "passed" for gate in value["gates"].values())
        )
    elif mode == "diagnostic":
        manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
        evidence = json.loads((root / "sources" / "evidence-scan").read_text(encoding="utf-8"))
        evidence_gate = value["gates"]["evidence-scan"]
        accepted = (
            value["overall_exit_code"] == 42
            and value["supervisor_exit_code"] == 42
            and evidence_gate["state"] == "failed"
            and evidence_gate["exit_code"] == 1
            and evidence_gate["reason_class"] == "evidence-scan-failed"
            and evidence.get("status") == "failed"
            and evidence.get("rule") == "browser-secret-org"
            and any(
                item.get("label") == "runtime-log" and item.get("reason") == "credential-scan-rejected"
                for item in manifest.get("rejectedSources", [])
                if isinstance(item, dict)
            )
        )
    else:
        raise SystemExit("unknown gate result mode")
status["gateValidation"] = gate_validation
status["gateAcceptance"] = "passed" if accepted else "failed"
if not accepted:
    status["error"] = "gate-results-acceptance-failed"
Path(status_path).write_text(json.dumps(status, separators=(",", ":")) + "\n", encoding="utf-8")
raise SystemExit(0 if accepted else 1)
PYGATE_ACCEPT
    if ((gate_acceptance_status != 0)); then
      diagnostics_download_error='gate-results-acceptance-failed'
      return 1
    fi
  fi
  diagnostics_triage_state='passed'
  printf 'HEPH_GCP_DIAGNOSTICS event=triage status=pass\n'
  diagnostics_scan_state='passed'
  printf 'HEPH_GCP_DIAGNOSTICS event=download status=pass bytes=%s sha256=%s manifest=%s\n' \
    "$archive_bytes" "$digest" "$extract_root/cooking-diagnostics/manifest.json"
  printf 'Authenticated download: gcloud storage cp gs://%s/%s ./gcp-diagnostics.tar.gz\n' "$DIAGNOSTICS_BUCKET" "$object"
}

write_download_failure_status() {
  local status_path="$1" object="$2"
  python3 - "$status_path" "$object" "$diagnostics_cleanup_state" \
    "$diagnostics_upload_state" "$diagnostics_download_state" "$diagnostics_scan_state" "$diagnostics_triage_state" \
    "$diagnostics_download_error" <<'PY'
import json
from pathlib import Path
import sys

path, obj, cleanup, upload, download, scan, triage, error = sys.argv[1:]
status = {
    "schema": 1,
    "object": obj,
    "cleanup": cleanup,
    "upload": upload,
    "download": download,
    "scan": scan,
    "triage": triage,
    "error": error,
}
Path(path).write_text(json.dumps(status, separators=(",", ":")) + "\n", encoding="utf-8")
PY
}

download_diagnostics() {
  local status_path="${GCP_DIAGNOSTICS_STATUS:-${RUNNER_TEMP:-/tmp}/gcp-diagnostics-status.json}"
  local status object='private-diagnostics'
  mkdir -p -- "$(dirname -- "$status_path")"
  diagnostics_download_error='download-failed'
  diagnostics_cleanup_state='unverified'
  diagnostics_upload_state='unknown'
  diagnostics_download_state='failed'
  diagnostics_scan_state='not-run'
  diagnostics_triage_state='not-run'
  set +e
  if configure_diagnostics_source true; then
    object="gs://${DIAGNOSTICS_BUCKET}/${DIAGNOSTICS_OBJECT_PREFIX}/${diagnostics_source_run_id}/${diagnostics_source_attempt}/${diagnostics_source_sha}.tar.gz"
    _download_diagnostics
    status=$?
  else
    status=1
  fi
  set -e
  if ((status != 0)); then
    # A valid archive may fail the post-download acceptance gate after scan
    # and triage have already produced useful evidence. Preserve that detailed
    # status; only synthesize the compact failure record when no detailed
    # status was safely written.
    if [[ ! -s "$status_path" ]] || ! python3 - "$status_path" <<'PY'
import json
import os
import re
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    value = json.load(stream)
if not isinstance(value, dict):
    raise SystemExit(1)
if value.get("cleanup") != "verified-absent":
    raise SystemExit(1)
if value.get("upload") != "verified-by-download":
    raise SystemExit(1)
if value.get("download") != "passed" or value.get("scan") != "passed":
    raise SystemExit(1)
if os.environ.get("GCP_EXPECT_PHASE_TIMING") == "true" and value.get("phaseTiming") != "passed":
    raise SystemExit(1)
triage = value.get("triage")
if isinstance(triage, dict):
    if triage.get("schema") != 1:
        raise SystemExit(1)
elif triage != "failed" or value.get("error") != "triage-projection-failed":
    raise SystemExit(1)
archive_bytes = value.get("archiveBytes")
if type(archive_bytes) is not int or not 0 <= archive_bytes <= 67108864:
    raise SystemExit(1)
digest = value.get("archiveSha256")
if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
    raise SystemExit(1)
PY
    then
      write_download_failure_status "$status_path" "$object"
    fi
  fi
  return "$status"
}

record_serial() {
  local value="$1"
  if [[ -n "$smoke_log" ]]; then
    printf '%s\n' "$value" >"$smoke_log"
    chmod 600 "$smoke_log"
  fi
}

report_serial_failure_context() {
  local value="$1" context_file context_output context_status
  context_file="$(mktemp "${TMPDIR:-/tmp}/gcp-kvm-smoke-context.XXXXXX")" || {
    printf 'Bounded typed serial failure context unavailable (temporary file creation failed)\n' >&2
    return 0
  }
  chmod 600 "$context_file"
  printf '%s\n' "$value" >"$context_file"
  set +e
  context_output="$(python3 - "$context_file" "$(dirname -- "${BASH_SOURCE[0]}")/check-browser-evidence.py" "$(dirname -- "${BASH_SOURCE[0]}")/collect-cooking-diagnostics.py" <<'PY'
import importlib.util
import pathlib
import re
import sys

serial_path, scanner_path, collector_path = sys.argv[1:]
spec = importlib.util.spec_from_file_location("serial_evidence", scanner_path)
if spec is None or spec.loader is None:
    raise SystemExit("serial evidence scanner is unavailable")
scanner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scanner)
collector_spec = importlib.util.spec_from_file_location("diagnostic_collector", collector_path)
if collector_spec is None or collector_spec.loader is None:
    raise SystemExit("diagnostic collector is unavailable")
collector = importlib.util.module_from_spec(collector_spec)
collector_spec.loader.exec_module(collector)

prefix = re.compile(r"^\[[^]]+\] google_metadata_script_runner\[\d+\]:\s*")
safe_key = re.compile(r"^(?:event|phase|revision|exit|exit_code|expected|test_result|status|error|stage|duration_ms|remaining_seconds|reserve_seconds|terminal|state|result|code|test|location|errno|operation|reason_class)$")
safe_value = re.compile(r"^[A-Za-z0-9._:/=-]+$")
safe_head = re.compile(r"^HE(?:PH|PHAESTUS)_[A-Z0-9_-]+:?$")
interesting_keys = {"event", "phase", "terminal", "error", "stage", "status", "state", "result"}
interesting_words = {"ERROR", "FAIL", "FAILED", "TERMINAL"}
rust_panic = re.compile(r"^thread '[^']{1,96}' panicked at (?P<location>[A-Za-z0-9_./:-]+:\d+(?::\d+)?)(?:$|:)")
rust_test_failure = re.compile(r"^test (?P<test>[A-Za-z0-9_./:-]+) \.\.\. FAILED$")
unbound_variable = re.compile(
    r"^(?:[A-Za-z0-9._/-]+/)?(?P<source>[A-Za-z0-9._-]+): line (?P<line>[0-9]+): "
    r"(?:(?P<variable>[A-Za-z_][A-Za-z0-9_]*): )?unbound variable$"
)
errno_markers = (
    ("Permission denied", "EACCES", "permission-denied"),
    ("Operation not permitted", "EPERM", "operation-not-permitted"),
    ("No such file or directory", "ENOENT", "not-found"),
    ("Invalid argument", "EINVAL", "invalid-argument"),
    ("Connection refused", "ECONNREFUSED", "connection-refused"),
    ("Connection reset by peer", "ECONNRESET", "connection-reset"),
    ("Address already in use", "EADDRINUSE", "address-in-use"),
)


def normalize(raw: str) -> str:
    line = prefix.sub("", raw.strip())
    if "startup-script:" in line:
        line = line.split("startup-script:", 1)[1].lstrip()
    return line


def project(raw: str) -> str | None:
    line = normalize(raw)
    readiness = collector.classify_readiness_error(line)
    if readiness is not None:
        return readiness
    encoded = line.encode("utf-8", errors="surrogateescape")
    if any(pattern in encoded for pattern in scanner.STREAM_PATTERNS):
        return None
    panic = rust_panic.match(line)
    if panic is not None:
        return f"HEPH_GCP_TEST test=rust-panic location={panic.group('location')}"
    test_failure = rust_test_failure.fullmatch(line)
    if test_failure is not None:
        return f"HEPH_GCP_TEST test={test_failure.group('test')} status=failed"
    unbound = unbound_variable.fullmatch(line)
    if unbound is not None:
        fields = [
            "HEPH_GCP_SHELL",
            "error=unbound-variable",
            f"source={unbound.group('source')}",
            f"line={unbound.group('line')}",
        ]
        if unbound.group("variable") is not None:
            fields.append(f"variable={unbound.group('variable')}")
        return " ".join(fields)
    for phrase, errno, error_class in errno_markers:
        if phrase in line:
            return f"HEPH_GCP_RUNTIME error={error_class} errno={errno}"
    if not line.startswith(("HEPH_", "HEPHAESTUS_")):
        return None
    head, separator, body = line.partition(" ")
    if not separator or not safe_head.fullmatch(head):
        return None
    output = []
    interesting = False
    for token in body.split():
        if "=" in token:
            key, value = token.split("=", 1)
            if not safe_key.fullmatch(key) or not safe_value.fullmatch(value):
                return None
            output.append(f"{key}={value}")
            interesting = interesting or key in interesting_keys
        else:
            if not re.fullmatch(r"[A-Za-z0-9_-]+", token):
                return None
            output.append(token)
            interesting = interesting or token.upper() in interesting_words
    if not interesting or not output:
        return None
    return f"{head} {' '.join(output)}"


lines = []
seen = set()
for raw in pathlib.Path(serial_path).read_text(encoding="utf-8", errors="replace").splitlines():
    safe_line = project(raw)
    if safe_line is not None and safe_line not in seen:
        seen.add(safe_line)
        lines.append(safe_line)
    if len(lines) >= 40:
        break
if lines:
    print("Bounded typed serial failure context:")
    print("\n".join(lines))
else:
    print("Bounded typed serial failure context unavailable (no safe HEPH error/phase/terminal records)")
PY
)"
  context_status=$?
  set -e
  rm -f -- "$context_file"
  if ((context_status != 0)); then
    printf 'Bounded typed serial failure context unavailable (scanner failed)\n' >&2
  else
    printf '%s\n' "$context_output" >&2
  fi
  return 0
}

cleanup_vm() {
  local zone="${GCP_ZONE:-europe-west1-b}"
  local name="heph-kvm-smoke-${GITHUB_RUN_ID:-manual}-${GITHUB_RUN_ATTEMPT:-1}"
  [[ "$zone" == "$REGION"-* ]] || die "zone must be in $REGION: $zone"
  local data describe_error=''
  if run_json_gcloud compute instances describe "$name" --project="$PROJECT_ID" --zone="$zone" --format=json; then
    data="$gcloud_json_output"
    report_gcloud_json_stderr
    python3 -c 'import json,sys
d=json.load(sys.stdin); labels=d.get("labels",{})
expected={"purpose":"hephaestus-kvm-smoke","run_id":sys.argv[1],"run_attempt":sys.argv[2],"sha":sys.argv[3]}
if any(labels.get(k)!=v for k,v in expected.items()): raise SystemExit("ownership labels do not match this workflow run")' \
      "${GITHUB_RUN_ID:-manual}" "${GITHUB_RUN_ATTEMPT:-1}" "${GCP_WORKLOAD_SHA:-${GITHUB_SHA:-}}" <<<"$data" || { printf 'gcp-kvm-smoke: refusing to delete an unowned VM: %s\n' "$name" >&2; return 1; }
    local delete_output delete_status inspect
    for delete_attempt in {1..3}; do
      if delete_output="$(gcloud compute instances delete "$name" --project="$PROJECT_ID" --zone="$zone" --quiet 2>&1)"; then
        delete_status=0
      else
        delete_status=$?
        printf 'gcp-kvm-smoke: delete attempt %s failed for %s: %s\n' \
          "$delete_attempt" "$name" "$delete_output" >&2
      fi

      # A failed delete response is inconclusive. Independently inspect the
      # exact resource before deciding whether cleanup succeeded.
      if inspect="$(gcloud compute instances describe "$name" --project="$PROJECT_ID" --zone="$zone" 2>&1)"; then
        if ((delete_status != 0)); then
          if ((delete_attempt < 3)); then
            printf 'gcp-kvm-smoke: owned VM still exists after failed delete; retrying cleanup\n' >&2
            sleep 2
            continue
          fi
          printf 'gcp-kvm-smoke: delete failed and owned VM still exists: %s\n' "$name" >&2
          return 1
        fi
        for _attempt in {1..30}; do
          if inspect="$(gcloud compute instances describe "$name" --project="$PROJECT_ID" --zone="$zone" 2>&1)"; then
            sleep 2
          elif grep -Eqi "instances/${name}[^[:alnum:]_].*was not found" <<<"$inspect"; then
            printf 'Confirmed disposable VM absent (verified by describe): %s\n' "$name"
            return 0
          else
            printf '%s\n' "$inspect" >&2
            return 1
          fi
        done
        printf 'gcp-kvm-smoke: VM still exists after delete request: %s\n' "$name" >&2
        return 1
      elif grep -Eqi "instances/${name}[^[:alnum:]_].*was not found" <<<"$inspect"; then
        if ((delete_status == 0)); then
          printf 'Confirmed disposable VM absent (verified by describe): %s\n' "$name"
        else
          printf 'Delete returned an error, but independent describe verified VM absent: %s\n' "$name"
        fi
        return 0
      else
        printf '%s\n' "$inspect" >&2
        printf 'gcp-kvm-smoke: cannot verify cleanup state for %s\n' "$name" >&2
        return 1
      fi
    done
    printf 'gcp-kvm-smoke: cleanup retries exhausted for owned VM: %s\n' "$name" >&2
    return 1
  fi
  describe_error="$gcloud_json_stderr"
  if grep -Eqi "instances/${name}[^[:alnum:]_].*was not found" <<<"$describe_error"; then
    printf 'Disposable VM already absent (verified by describe): %s\n' "$name"
    return 0
  fi
  printf '%s\n' "$describe_error" >&2
  printf 'gcp-kvm-smoke: cannot inspect disposable VM for cleanup: %s\n' "$name" >&2
  return 1
}

smoke() {
  local mode="${1:-smoke}"
  case "$mode" in
    smoke|gcp-cooking|diagnostic) ;;
    *) die "unsupported test mode: $mode" ;;
  esac
  [[ -f "$STARTUP_SCRIPT" && ! -L "$STARTUP_SCRIPT" ]] || die "startup script is unavailable or symlinked: $STARTUP_SCRIPT"
  [[ -f "$PASST_PREFLIGHT_SCRIPT" && ! -L "$PASST_PREFLIGHT_SCRIPT" ]] || die "passthrough preflight script is unavailable or symlinked: $PASST_PREFLIGHT_SCRIPT"
  if [[ "$mode" == smoke || "$mode" == gcp-cooking || "$mode" == diagnostic ]]; then
    [[ -f "$DIAGNOSTICS_COLLECTOR_SCRIPT" && ! -L "$DIAGNOSTICS_COLLECTOR_SCRIPT" ]] ||
      die "diagnostics collector is unavailable or symlinked: $DIAGNOSTICS_COLLECTOR_SCRIPT"
    [[ -f "$DIAGNOSTICS_SCANNER_SCRIPT" && ! -L "$DIAGNOSTICS_SCANNER_SCRIPT" ]] ||
      die "diagnostics scanner is unavailable or symlinked: $DIAGNOSTICS_SCANNER_SCRIPT"
    [[ -f "$DIAGNOSTICS_TRIAGE_SCRIPT" && ! -L "$DIAGNOSTICS_TRIAGE_SCRIPT" ]] ||
      die "diagnostics triage projector is unavailable or symlinked: $DIAGNOSTICS_TRIAGE_SCRIPT"
    [[ -f "$COOKING_GATE_RESULTS_HELPER_SCRIPT" && ! -L "$COOKING_GATE_RESULTS_HELPER_SCRIPT" ]] ||
      die "Cooking gate sidecar helper is unavailable or symlinked: $COOKING_GATE_RESULTS_HELPER_SCRIPT"
  fi
  [[ "${GITHUB_SHA:-}" =~ ^[0-9a-f]{40}$ ]] || die 'GITHUB_SHA must be the exact 40-character workflow commit SHA'
  validate_workload_provenance "$mode"
  smoke_zone="${GCP_ZONE:-europe-west1-b}"
  smoke_name="heph-kvm-smoke-${GITHUB_RUN_ID:-manual}-${GITHUB_RUN_ATTEMPT:-1}"
  [[ "$smoke_zone" == "$REGION"-* ]] || die "zone must be in $REGION: $smoke_zone"
  smoke_created=false
  cleanup() {
    controller_phase_timing_finish_open 1
    if [[ "$smoke_created" == true ]]; then
      controller_phase_timing_start vm-delete
      if cleanup_vm; then
        controller_phase_timing_end passed
      else
        controller_phase_timing_finish_open 1
        printf 'warning: cleanup will be retried by the workflow cleanup step\n' >&2
      fi
    fi
  }
  on_signal() {
    local signal="$1"
    trap - EXIT INT TERM
    cleanup || printf 'warning: cleanup failed after signal %s\n' "$signal" >&2
    exit "$((128 + signal))"
  }
  trap cleanup EXIT
  trap 'on_signal 2' INT
  trap 'on_signal 15' TERM
  local inspect
  if run_json_gcloud compute instances describe "$smoke_name" --project="$PROJECT_ID" --zone="$smoke_zone" --format=json; then
    inspect="$gcloud_json_output"
    report_gcloud_json_stderr
    die "refusing to reuse existing smoke VM: $smoke_name"
  elif ! grep -Eqi "instances/${smoke_name}[^[:alnum:]_].*was not found" <<<"$gcloud_json_stderr"; then
    printf '%s\n' "$gcloud_json_stderr" >&2
    die "cannot establish that smoke VM name is absent: $smoke_name"
  fi
  local identity_args=(--no-service-account --no-scopes)
  local image_args=(--image-family=ubuntu-2404-lts-amd64 --image-project=ubuntu-os-cloud)
  local runner_image_metadata='runner-image-selection=stock'
  if [[ -n "${GCP_RUNNER_IMAGE:-}" ]]; then
    [[ "$mode" == diagnostic || "$mode" == smoke || "$mode" == gcp-cooking ]] || die 'custom runner images are supported only for diagnostic, smoke, and gcp-cooking modes'
    [[ "$GCP_RUNNER_IMAGE" =~ ^[a-z][a-z0-9-]{0,62}$ ]] || die 'GCP_RUNNER_IMAGE is not a valid immutable image name'
    local runner_image_data image_recipe_sha image_verifier_sha image_startup_sha
    local compatibility_validator_sha compatibility_records_sha
    image_recipe_sha="$(sha256sum "$(dirname -- "$STARTUP_SCRIPT")/gcp-runner-image-provision.sh" | awk '{print $1}')"
    image_verifier_sha="$(sha256sum "$(dirname -- "$STARTUP_SCRIPT")/gcp-runner-image-verify.py" | awk '{print $1}')"
    image_startup_sha="$(sha256sum "$STARTUP_SCRIPT" | awk '{print $1}')"
    [[ -f "$RUNNER_IMAGE_COMPATIBILITY_VALIDATOR" && ! -L "$RUNNER_IMAGE_COMPATIBILITY_VALIDATOR" ]] ||
      die 'runner image compatibility validator is unavailable or symlinked'
    [[ -f "$RUNNER_IMAGE_COMPATIBILITY_RECORDS" && ! -L "$RUNNER_IMAGE_COMPATIBILITY_RECORDS" ]] ||
      die 'runner image compatibility records are unavailable or symlinked'
    compatibility_validator_sha="$(sha256sum "$RUNNER_IMAGE_COMPATIBILITY_VALIDATOR" | awk '{print $1}')"
    compatibility_records_sha="$(sha256sum "$RUNNER_IMAGE_COMPATIBILITY_RECORDS" | awk '{print $1}')"
    run_json_gcloud compute images describe "$GCP_RUNNER_IMAGE" --project="$PROJECT_ID" --format='json(name,status,labels,description)' || {
      printf '%s\n' "$gcloud_json_stderr" >&2
      die "custom runner image cannot be described: $GCP_RUNNER_IMAGE"
    }
    runner_image_data="$gcloud_json_output"
    report_gcloud_json_stderr
    runner_image_metadata="$(python3 "$RUNNER_IMAGE_COMPATIBILITY_VALIDATOR" controller \
      --records-file "$RUNNER_IMAGE_COMPATIBILITY_RECORDS" \
      --metadata-json "$runner_image_data" --image-name "$GCP_RUNNER_IMAGE" \
      --expected-recipe "$image_recipe_sha" --expected-verifier "$image_verifier_sha" \
      --runtime-startup "$image_startup_sha")" ||
      die "custom runner image failed immutable-label validation: $GCP_RUNNER_IMAGE"
    runner_image_metadata="runner-image-selection=custom,${runner_image_metadata}"
    runner_image_metadata+=",runner-image-compatibility-validator-sha256=${compatibility_validator_sha}"
    runner_image_metadata+=",runner-image-compatibility-records-sha256=${compatibility_records_sha}"
    image_args=(--image="$GCP_RUNNER_IMAGE" --image-project="$PROJECT_ID")
    printf 'Using validated immutable runner image: %s\n' "$GCP_RUNNER_IMAGE"
  fi
  local machine_type="$MACHINE_TYPE" disk_size="$DISK_SIZE" nested_args=(--enable-nested-virtualization) \
    max_run_duration='45m' maintenance_args=(--maintenance-policy=TERMINATE)
  local diagnostics_metadata=( )
  if [[ "$mode" == smoke || "$mode" == gcp-cooking || "$mode" == diagnostic ]]; then
    identity_args=(
      --service-account="hephaestus-cooking-runtime@${PROJECT_ID}.iam.gserviceaccount.com"
      --scopes=storage-rw
    )
    diagnostics_metadata=(
      "diagnostics-bucket=${DIAGNOSTICS_BUCKET}"
      "diagnostics-object=$(diagnostics_object_for_run)"
    )
  fi
  if [[ "$mode" == diagnostic ]]; then
    machine_type="$DIAGNOSTIC_MACHINE_TYPE"
    disk_size="$DIAGNOSTIC_DISK_SIZE"
    nested_args=()
    max_run_duration='10m'
    maintenance_args=(--maintenance-policy=MIGRATE)
    # A custom image is captured from a 150 GB builder disk; Compute Engine
    # cannot boot that image from the diagnostic mode's 20 GB disk.
    [[ -z "${GCP_RUNNER_IMAGE:-}" ]] || disk_size='150GB'
  fi
  # Capture the conservative VM-start anchor immediately before the create
  # request.  Startup derives both workload and collection deadlines from it.
  local trial_start_epoch="$(date +%s)"
  local cooking_gate_script_sha256 cooking_runtime_script_sha256 cooking_browser_summary_script_sha256 gate_expectation_hash
  cooking_gate_script_sha256="$(sha256sum "$COOKING_GATE_RESULTS_HELPER_SCRIPT" | awk '{print $1}')"
  cooking_runtime_script_sha256="$(sha256sum "$COOKING_RUNTIME_SCRIPT" | awk '{print $1}')"
  cooking_browser_summary_script_sha256="$(sha256sum "$COOKING_BROWSER_SUMMARY_SCRIPT" | awk '{print $1}')"
  local phase_timing_script_sha256
  phase_timing_script_sha256="$(sha256sum "$PHASE_TIMING_SCRIPT" | awk '{print $1}')"
  if [[ "$mode" == diagnostic || "$mode" == gcp-cooking ]]; then
    local gate_expectation_file="${RUNNER_TEMP:-/tmp}/gcp-diagnostics-gate-expectation"
    install -m 0600 /dev/null "$gate_expectation_file"
    gate_expectation_hash="$cooking_gate_script_sha256"
    [[ "$mode" == diagnostic ]] || gate_expectation_hash="$cooking_runtime_script_sha256"
    printf '%s %s\n' "$mode" "$gate_expectation_hash" >"$gate_expectation_file"
  fi
  local metadata_values="test-mode=${mode},github-sha=$validated_source_sha,run-id=${GITHUB_RUN_ID:-manual},run-attempt=${GITHUB_RUN_ATTEMPT:-1},trial-start-epoch=${trial_start_epoch},cooking-gate-results-script-sha256=${cooking_gate_script_sha256},${runner_image_metadata}"
  if [[ "$mode" == gcp-cooking ]]; then
    [[ "$cache_generation" =~ ^[1-9][0-9]*$ ]] ||
      die 'cache preflight generation is unavailable for gcp-cooking'
    metadata_values+=",cooking-runtime-script-sha256=${cooking_runtime_script_sha256},cooking-browser-summary-script-sha256=${cooking_browser_summary_script_sha256},phase-timing-script-sha256=${phase_timing_script_sha256}"
    metadata_values+=",cache-generation=${cache_generation}"
    if [[ "$pr_workload_mode" == true ]]; then
      metadata_values+=",workload-trust=untrusted-pr"
    fi
  fi
  local metadata_value_item
  local metadata_file_values="startup-script=$STARTUP_SCRIPT,passt-preflight-script=$PASST_PREFLIGHT_SCRIPT"
  for metadata_value_item in "${diagnostics_metadata[@]}"; do
    metadata_values+=",${metadata_value_item}"
  done
  if [[ "$mode" == smoke || "$mode" == gcp-cooking || "$mode" == diagnostic ]]; then
    metadata_file_values+=",diagnostics-collector-script=$DIAGNOSTICS_COLLECTOR_SCRIPT,diagnostics-scanner-script=$DIAGNOSTICS_SCANNER_SCRIPT,cooking-gate-results-helper=$COOKING_GATE_RESULTS_HELPER_SCRIPT"
  fi
  if [[ "$mode" == gcp-cooking ]]; then
    metadata_file_values+=",cooking-runtime-script=$COOKING_RUNTIME_SCRIPT,cooking-browser-summary-script=$COOKING_BROWSER_SUMMARY_SCRIPT,phase-timing-script=$PHASE_TIMING_SCRIPT"
  fi
  if [[ -n "${GCP_RUNNER_IMAGE:-}" ]]; then
    metadata_file_values+=",runner-image-compatibility-validator=$RUNNER_IMAGE_COMPATIBILITY_VALIDATOR,runner-image-compatibility-records=$RUNNER_IMAGE_COMPATIBILITY_RECORDS"
  fi
  controller_phase_timing_start vm-create
  gcloud compute instances create "$smoke_name" \
    --project="$PROJECT_ID" --zone="$smoke_zone" --machine-type="$machine_type" \
    --network-interface=network=default,network-tier=PREMIUM \
    "${image_args[@]}" \
    --boot-disk-size="$disk_size" --boot-disk-type=pd-balanced \
    --boot-disk-auto-delete "${nested_args[@]}" \
    --max-run-duration="$max_run_duration" --instance-termination-action=DELETE \
    "${maintenance_args[@]}" "${identity_args[@]}" \
    --labels="purpose=hephaestus-kvm-smoke,run_id=${GITHUB_RUN_ID:-manual},run_attempt=${GITHUB_RUN_ATTEMPT:-1},sha=$validated_source_sha" \
    --metadata="$metadata_values" \
    --metadata-from-file="$metadata_file_values" || {
      controller_phase_timing_finish_open 1
      gcloud compute instances describe "$smoke_name" --project="$PROJECT_ID" --zone="$smoke_zone" >/dev/null 2>&1 && smoke_created=true
      die 'instance creation failed'
    }
  controller_phase_timing_end passed
  smoke_created=true
  local poll_deadline_epoch serial='' last_serial='' describe_output
  if [[ "$mode" == diagnostic ]]; then
    # Leave one minute inside the provider's ten-minute lifetime for cleanup.
    poll_deadline_epoch=$((trial_start_epoch + 540))
  else
    # Full mode stops before the provider's 45-minute deletion deadline so
    # collection retains its five-minute reserve.
    poll_deadline_epoch=$((trial_start_epoch + 2400))
  fi
  local pass_marker='HEPHAESTUS_GCP_KVM_SMOKE: PASS'
  local fail_marker='HEPHAESTUS_GCP_KVM_SMOKE: FAIL .*'
  # gcp-cooking-run emits a provisional FAIL before the startup EXIT trap
  # collects and uploads diagnostics.  The outer startup final failure marker
  # includes the verified revision, so it is the authoritative terminal
  # result and cannot be confused with the workload's provisional marker.
  local cooking_final_fail_marker=''
  if [[ "$mode" == gcp-cooking ]]; then
    pass_marker='HEPHAESTUS_GCP_COOKING: PASS'
    cooking_final_fail_marker='HEPHAESTUS_GCP_COOKING: FAIL phase=[A-Za-z0-9_-]+ exit=[0-9]+ revision=[0-9a-f]{40}'
  fi
  marker_matches() {
    local body="$1" output="$2"
    grep -Eq "^${body}$|^\\[[[:space:][:digit:].]+\\] google_metadata_script_runner\\[[[:digit:]]+\\]: startup-script: ${body}$" <<<"$output"
  }
  vm_absent_before_terminal() {
    local output
    if output="$(gcloud compute instances describe "$smoke_name" --project="$PROJECT_ID" --zone="$smoke_zone" 2>&1)"; then
      return 1
    fi
    grep -Eqi "instances/${smoke_name}[^[:alnum:]_].*was not found" <<<"$output"
  }
  controller_phase_timing_start vm-wait
  while (( $(date +%s) < poll_deadline_epoch )); do
    if serial="$(gcloud compute instances get-serial-port-output "$smoke_name" --project="$PROJECT_ID" --zone="$smoke_zone" --port=1 2>&1)"; then
      serial="${serial//$'\r'/}"
      last_serial="$serial"
      record_serial "$serial"
    else
      record_serial "serial retrieval error (bounded retry): $serial"
      [[ -z "$last_serial" ]] || record_serial "$last_serial"
      if vm_absent_before_terminal; then
        die "disposable VM disappeared before a terminal marker: $smoke_name"
      fi
      sleep 10
      continue
    fi
    if marker_matches "$pass_marker" "$serial"; then
      controller_phase_timing_end passed
      printf 'GCE %s passed for %s (%s); cleanup is automatic.\n' "$mode" "$smoke_name" "$GITHUB_SHA"
      return 0
    fi
    if [[ "$mode" == diagnostic ]] &&
        marker_matches 'HEPHAESTUS_GCP_DIAGNOSTIC: TEST-FAIL expected=true .*' "$serial" &&
        marker_matches 'HEPHAESTUS_GCP_DIAGNOSTIC: DIAGNOSTICS PASS .*' "$serial"; then
      controller_phase_timing_end passed
      printf 'GCE diagnostic produced the expected test failure and passed its evidence gate for %s.\n' "$smoke_name"
      return 0
    fi
    if [[ "$mode" == diagnostic ]] &&
        marker_matches 'HEPHAESTUS_GCP_DIAGNOSTIC: DIAGNOSTICS FAIL .*' "$serial"; then
      report_serial_failure_context "$serial"
      die 'diagnostic evidence pipeline failed'
    fi
    if [[ "$mode" == gcp-cooking ]]; then
      if marker_matches "$cooking_final_fail_marker" "$serial"; then
        report_serial_failure_context "$serial"
        die 'startup smoke reported failure'
      fi
    elif marker_matches "$fail_marker" "$serial"; then
      report_serial_failure_context "$serial"
      die 'startup smoke reported failure'
    fi
    if [[ "$mode" == gcp-cooking ]] &&
        marker_matches 'HEPHAESTUS_GCP_KVM_SMOKE: FAIL .*' "$serial"; then
      report_serial_failure_context "$serial"
      die 'common startup reported failure before Cooking helper'
    fi
    sleep 10
  done
  if vm_absent_before_terminal; then
    die "disposable VM disappeared before a terminal marker: $smoke_name"
  fi
  report_serial_failure_context "$last_serial"
  die "timed out waiting for the startup ${mode} marker before its cleanup reserve"
}

require_commands
case "${1:-preflight}" in
  preflight) timed_controller_phase preflight-quota quota_preflight full ;;
  cache-preflight) timed_controller_phase preflight-quota quota_preflight full; timed_controller_phase preflight-cache cache_preflight ;;
  diagnostic) timed_controller_phase preflight-quota quota_preflight diagnostic; smoke diagnostic ;;
  smoke) timed_controller_phase preflight-quota quota_preflight full; smoke smoke ;;
  gcp-cooking) timed_controller_phase preflight-quota quota_preflight full; timed_controller_phase preflight-cache cache_preflight; smoke gcp-cooking ;;
  download-diagnostics) download_diagnostics ;;
  cleanup) cleanup_vm ;;
  *) die 'usage: scripts/gcp-kvm-smoke.sh [preflight|cache-preflight|diagnostic|smoke|gcp-cooking|download-diagnostics|cleanup]' ;;
esac
