#!/usr/bin/env bash
# Read the regional quota or create one disposable nested-KVM smoke VM.
# This script never attaches a service account and never accepts credentials.
set -Eeuo pipefail

readonly PROJECT_ID="hephaestus-508000"
readonly REGION="europe-west1"
readonly STARTUP_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/gcp-kvm-startup.sh"
readonly PASST_PREFLIGHT_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/gcp-passt-preflight.sh"
readonly DIAGNOSTICS_COLLECTOR_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/collect-cooking-diagnostics.py"
readonly DIAGNOSTICS_SCANNER_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/check-browser-evidence.py"
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

die() { printf 'gcp-kvm-smoke: %s\n' "$*" >&2; exit 1; }

require_commands() {
  command -v gcloud >/dev/null 2>&1 || die 'gcloud is unavailable'
  command -v python3 >/dev/null 2>&1 || die 'python3 is unavailable'
  command -v timeout >/dev/null 2>&1 || die 'timeout is unavailable'
}

quota_preflight() {
  local mode="${1:-full}" quota_json
  quota_json="$(gcloud compute regions describe "$REGION" --project="$PROJECT_ID" --format=json)" ||
    die "cannot read regional quota for $REGION"
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
  if describe_output="$(gcloud storage objects describe "$object_uri" \
      --project="$PROJECT_ID" --billing-project="$PROJECT_ID" \
      --raw --format='json(name,size,md5Hash,generation)' 2>&1)"; then
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
    printf '%s\n' "$metadata_output"
    return 0
  else
    describe_status=$?
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

diagnostics_object_for_run() {
  [[ "${GITHUB_RUN_ID:-}" =~ ^[0-9]+$ ]] || die 'GITHUB_RUN_ID is required for private diagnostics'
  [[ "${GITHUB_RUN_ATTEMPT:-}" =~ ^[0-9]+$ ]] || die 'GITHUB_RUN_ATTEMPT is required for private diagnostics'
  [[ "${GITHUB_SHA:-}" =~ ^[0-9a-f]{40}$ ]] || die 'GITHUB_SHA is required for private diagnostics'
  printf '%s/%s/%s/%s.tar.gz\n' "$DIAGNOSTICS_OBJECT_PREFIX" "$GITHUB_RUN_ID" \
    "$GITHUB_RUN_ATTEMPT" "$GITHUB_SHA"
}

verify_disposable_vm_absent() {
  local zone="${GCP_ZONE:-europe-west1-b}" name="heph-kvm-smoke-${GITHUB_RUN_ID:-manual}-${GITHUB_RUN_ATTEMPT:-1}" describe_output
  [[ "$zone" == "$REGION"-* ]] || { printf 'cleanup zone is outside %s: %s\n' "$REGION" "$zone" >&2; return 1; }
  if describe_output="$(gcloud compute instances describe "$name" --project="$PROJECT_ID" --zone="$zone" 2>&1)"; then
    printf 'cleanup verification found disposable VM still present: %s\n' "$name" >&2
    return 1
  fi
  if grep -Eqi "instances/${name}[^[:alnum:]_].*was not found" <<<"$describe_output"; then
    printf 'cleanup verified VM absent: %s (%s)\n' "$name" "$zone"
    return 0
  fi
  printf 'cleanup verification was inconclusive for %s:\n%s\n' "$name" "$describe_output" >&2
  return 1
}

_download_diagnostics() {
  local object="$(diagnostics_object_for_run)" destination="${GCP_DIAGNOSTICS_ARCHIVE:-${RUNNER_TEMP:-/tmp}/gcp-diagnostics.tar.gz}"
  local status_path="${GCP_DIAGNOSTICS_STATUS:-${RUNNER_TEMP:-/tmp}/gcp-diagnostics-status.json}"
  local extract_root="${destination}.extract" output digest archive_bytes download_timeout
  download_timeout="${GCP_DIAGNOSTICS_DOWNLOAD_TIMEOUT_SECONDS:-60}"
  [[ "$download_timeout" =~ ^[1-9][0-9]*$ ]] || { diagnostics_download_error='download-timeout-invalid'; return 1; }
  if ! verify_disposable_vm_absent; then
    diagnostics_download_error='cleanup-unverified'
    return 1
  fi
  mkdir -p -- "$(dirname -- "$destination")" "$(dirname -- "$status_path")"
  rm -rf -- "$extract_root"
  mkdir -m 700 -- "$extract_root"
  printf 'HEPH_GCP_DIAGNOSTICS event=download status=start object=gs://%s/%s\n' "$DIAGNOSTICS_BUCKET" "$object"
  if ! output="$(timeout --kill-after=5s "${download_timeout}s" gcloud storage cp "gs://${DIAGNOSTICS_BUCKET}/${object}" "$destination" \
      --project="$PROJECT_ID" --billing-project="$PROJECT_ID" --quiet 2>&1)"; then
    diagnostics_download_error='provider-download-failed'
    printf '{"schema":1,"object":"gs://%s/%s","download":"failed","error":"provider download failed"}\n' \
      "$DIAGNOSTICS_BUCKET" "$object" >"$status_path"
    printf '%s\n' "$output" >&2
    return 1
  fi
  [[ -f "$destination" && ! -L "$destination" ]] || {
    diagnostics_download_error='archive-missing'
    printf '{"schema":1,"object":"gs://%s/%s","download":"failed","error":"archive missing"}\n' \
      "$DIAGNOSTICS_BUCKET" "$object" >"$status_path"
    return 1
  }
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
    return 1
  fi
  if ! tar -xzf "$destination" -C "$extract_root" --no-same-owner --no-same-permissions; then
    diagnostics_download_error='archive-extraction-failed'
    return 1
  fi
  if ! python3 - "$extract_root/cooking-diagnostics" <<'PY'
import hashlib
import json
import pathlib
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
    return 1
  fi
  if [[ ! -f "$DIAGNOSTICS_SCANNER_SCRIPT" ]]; then
    diagnostics_download_error='scanner-unavailable'
    return 1
  fi
  if ! python3 -B "$DIAGNOSTICS_SCANNER_SCRIPT" "$extract_root/cooking-diagnostics" >/dev/null; then
    diagnostics_download_error='credential-scan-failed'
    return 1
  fi
  printf '{"schema":1,"object":"gs://%s/%s","cleanup":"verified-absent","upload":"verified-by-download","download":"passed","scan":"passed","archiveBytes":%s,"archiveSha256":"%s","manifest":"%s"}\n' \
    "$DIAGNOSTICS_BUCKET" "$object" "$archive_bytes" "$digest" "$extract_root/cooking-diagnostics/manifest.json" \
    >"$status_path"
  printf 'HEPH_GCP_DIAGNOSTICS event=download status=pass bytes=%s sha256=%s manifest=%s\n' \
    "$archive_bytes" "$digest" "$extract_root/cooking-diagnostics/manifest.json"
  printf 'Authenticated download: gcloud storage cp gs://%s/%s ./gcp-diagnostics.tar.gz\n' "$DIAGNOSTICS_BUCKET" "$object"
}

download_diagnostics() {
  local status_path="${GCP_DIAGNOSTICS_STATUS:-${RUNNER_TEMP:-/tmp}/gcp-diagnostics-status.json}"
  local status object='private-diagnostics'
  if [[ "${GITHUB_RUN_ID:-}" =~ ^[0-9]+$ && "${GITHUB_RUN_ATTEMPT:-}" =~ ^[0-9]+$ && "${GITHUB_SHA:-}" =~ ^[0-9a-f]{40}$ ]]; then
    object="gs://${DIAGNOSTICS_BUCKET}/${DIAGNOSTICS_OBJECT_PREFIX}/${GITHUB_RUN_ID}/${GITHUB_RUN_ATTEMPT}/${GITHUB_SHA}.tar.gz"
  fi
  mkdir -p -- "$(dirname -- "$status_path")"
  diagnostics_download_error='download-failed'
  set +e
  _download_diagnostics
  status=$?
  set -e
  if ((status != 0)); then
    printf '{"schema":1,"object":"%s","cleanup":"unverified","download":"failed","scan":"not-run","error":"%s"}\n' \
      "$object" "$diagnostics_download_error" >"$status_path"
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

cleanup_vm() {
  local zone="${GCP_ZONE:-europe-west1-b}"
  local name="heph-kvm-smoke-${GITHUB_RUN_ID:-manual}-${GITHUB_RUN_ATTEMPT:-1}"
  [[ "$zone" == "$REGION"-* ]] || die "zone must be in $REGION: $zone"
  local data
  if data="$(gcloud compute instances describe "$name" --project="$PROJECT_ID" --zone="$zone" --format=json 2>&1)"; then
    python3 -c 'import json,sys
d=json.load(sys.stdin); labels=d.get("labels",{})
expected={"purpose":"hephaestus-kvm-smoke","run_id":sys.argv[1],"run_attempt":sys.argv[2],"sha":sys.argv[3]}
if any(labels.get(k)!=v for k,v in expected.items()): raise SystemExit("ownership labels do not match this workflow run")' \
      "${GITHUB_RUN_ID:-manual}" "${GITHUB_RUN_ATTEMPT:-1}" "${GITHUB_SHA:-}" <<<"$data" || { printf 'gcp-kvm-smoke: refusing to delete an unowned VM: %s\n' "$name" >&2; return 1; }
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
  if grep -Eqi "instances/${name}[^[:alnum:]_].*was not found" <<<"$data"; then
    printf 'Disposable VM already absent (verified by describe): %s\n' "$name"
    return 0
  fi
  printf '%s\n' "$data" >&2
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
  if [[ "$mode" == gcp-cooking || "$mode" == diagnostic ]]; then
    [[ -f "$DIAGNOSTICS_COLLECTOR_SCRIPT" && ! -L "$DIAGNOSTICS_COLLECTOR_SCRIPT" ]] ||
      die "diagnostics collector is unavailable or symlinked: $DIAGNOSTICS_COLLECTOR_SCRIPT"
    [[ -f "$DIAGNOSTICS_SCANNER_SCRIPT" && ! -L "$DIAGNOSTICS_SCANNER_SCRIPT" ]] ||
      die "diagnostics scanner is unavailable or symlinked: $DIAGNOSTICS_SCANNER_SCRIPT"
  fi
  [[ "${GITHUB_SHA:-}" =~ ^[0-9a-f]{40}$ ]] || die 'GITHUB_SHA must be the exact 40-character workflow commit SHA'
  smoke_zone="${GCP_ZONE:-europe-west1-b}"
  smoke_name="heph-kvm-smoke-${GITHUB_RUN_ID:-manual}-${GITHUB_RUN_ATTEMPT:-1}"
  [[ "$smoke_zone" == "$REGION"-* ]] || die "zone must be in $REGION: $smoke_zone"
  smoke_created=false
  cleanup() {
    if [[ "$smoke_created" == true ]]; then
      cleanup_vm || printf 'warning: cleanup will be retried by the workflow cleanup step\n' >&2
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
  if inspect="$(gcloud compute instances describe "$smoke_name" --project="$PROJECT_ID" --zone="$smoke_zone" --format=json 2>&1)"; then
    die "refusing to reuse existing smoke VM: $smoke_name"
  elif ! grep -Eqi "instances/${smoke_name}[^[:alnum:]_].*was not found" <<<"$inspect"; then
    printf '%s\n' "$inspect" >&2
    die "cannot establish that smoke VM name is absent: $smoke_name"
  fi
  local identity_args=(--no-service-account --no-scopes)
  local machine_type="$MACHINE_TYPE" disk_size="$DISK_SIZE" nested_args=(--enable-nested-virtualization) \
    max_run_duration='45m' maintenance_args=(--maintenance-policy=TERMINATE)
  local diagnostics_metadata=( )
  if [[ "$mode" == gcp-cooking || "$mode" == diagnostic ]]; then
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
  fi
  # Capture the conservative VM-start anchor immediately before the create
  # request.  Startup derives both workload and collection deadlines from it.
  local trial_start_epoch="$(date +%s)"
  local metadata_values="test-mode=${mode},github-sha=$GITHUB_SHA,trial-start-epoch=${trial_start_epoch}"
  local metadata_value_item
  local metadata_file_values="startup-script=$STARTUP_SCRIPT,passt-preflight-script=$PASST_PREFLIGHT_SCRIPT"
  for metadata_value_item in "${diagnostics_metadata[@]}"; do
    metadata_values+=",${metadata_value_item}"
  done
  if [[ "$mode" == gcp-cooking || "$mode" == diagnostic ]]; then
    metadata_file_values+=",diagnostics-collector-script=$DIAGNOSTICS_COLLECTOR_SCRIPT,diagnostics-scanner-script=$DIAGNOSTICS_SCANNER_SCRIPT"
  fi
  gcloud compute instances create "$smoke_name" \
    --project="$PROJECT_ID" --zone="$smoke_zone" --machine-type="$machine_type" \
    --network-interface=network=default,network-tier=PREMIUM \
    --image-family=ubuntu-2404-lts-amd64 --image-project=ubuntu-os-cloud \
    --boot-disk-size="$disk_size" --boot-disk-type=pd-balanced \
    --boot-disk-auto-delete "${nested_args[@]}" \
    --max-run-duration="$max_run_duration" --instance-termination-action=DELETE \
    "${maintenance_args[@]}" "${identity_args[@]}" \
    --labels="purpose=hephaestus-kvm-smoke,run_id=${GITHUB_RUN_ID:-manual},run_attempt=${GITHUB_RUN_ATTEMPT:-1},sha=$GITHUB_SHA" \
    --metadata="$metadata_values" \
    --metadata-from-file="$metadata_file_values" || {
      gcloud compute instances describe "$smoke_name" --project="$PROJECT_ID" --zone="$smoke_zone" >/dev/null 2>&1 && smoke_created=true
      die 'instance creation failed'
    }
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
  if [[ "$mode" == gcp-cooking ]]; then
    pass_marker='HEPHAESTUS_GCP_COOKING: PASS'
    fail_marker='HEPHAESTUS_GCP_COOKING: FAIL .*'
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
      printf 'GCE %s passed for %s (%s); cleanup is automatic.\n' "$mode" "$smoke_name" "$GITHUB_SHA"
      return 0
    fi
    if [[ "$mode" == diagnostic ]] &&
        marker_matches 'HEPHAESTUS_GCP_DIAGNOSTIC: TEST-FAIL expected=true .*' "$serial" &&
        marker_matches 'HEPHAESTUS_GCP_DIAGNOSTIC: DIAGNOSTICS PASS .*' "$serial"; then
      printf 'GCE diagnostic produced the expected test failure and passed its evidence gate for %s.\n' "$smoke_name"
      return 0
    fi
    if [[ "$mode" == diagnostic ]] &&
        marker_matches 'HEPHAESTUS_GCP_DIAGNOSTIC: DIAGNOSTICS FAIL .*' "$serial"; then
      printf '%s\n' "$serial" | tail -80 >&2
      die 'diagnostic evidence pipeline failed'
    fi
    if marker_matches "$fail_marker" "$serial"; then
      printf '%s\n' "$serial" | tail -80 >&2
      die 'startup smoke reported failure'
    fi
    if [[ "$mode" == gcp-cooking ]] &&
        marker_matches 'HEPHAESTUS_GCP_KVM_SMOKE: FAIL .*' "$serial"; then
      printf '%s\n' "$serial" | tail -80 >&2
      die 'common startup reported failure before Cooking helper'
    fi
    sleep 10
  done
  if vm_absent_before_terminal; then
    die "disposable VM disappeared before a terminal marker: $smoke_name"
  fi
  printf '%s\n' "$last_serial" | tail -80 >&2
  die "timed out waiting for the startup ${mode} marker before its cleanup reserve"
}

require_commands
case "${1:-preflight}" in
  preflight) quota_preflight full ;;
  cache-preflight) quota_preflight full; cache_preflight ;;
  diagnostic) quota_preflight diagnostic; smoke diagnostic ;;
  smoke) quota_preflight full; smoke smoke ;;
  gcp-cooking) quota_preflight full; cache_preflight; smoke gcp-cooking ;;
  download-diagnostics) download_diagnostics ;;
  cleanup) cleanup_vm ;;
  *) die 'usage: scripts/gcp-kvm-smoke.sh [preflight|cache-preflight|diagnostic|smoke|gcp-cooking|download-diagnostics|cleanup]' ;;
esac
