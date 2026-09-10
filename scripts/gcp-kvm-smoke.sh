#!/usr/bin/env bash
# Read the regional quota or create one disposable nested-KVM smoke VM.
# This script never attaches a service account and never accepts credentials.
set -Eeuo pipefail

readonly PROJECT_ID="hephaestus-508000"
readonly REGION="europe-west1"
readonly STARTUP_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/gcp-kvm-startup.sh"
readonly PASST_PREFLIGHT_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/gcp-passt-preflight.sh"
readonly MACHINE_TYPE="n2-standard-8"
readonly DISK_SIZE="150GB"
readonly CACHE_BUCKET="hephaestus-508000-cooking-cache"
readonly CACHE_OBJECT="cooking/heph-gcp-cooking-cache.tar.zst"
smoke_created=false
smoke_name=""
smoke_zone=""
smoke_log="${GCP_SMOKE_LOG:-}"

die() { printf 'gcp-kvm-smoke: %s\n' "$*" >&2; exit 1; }

require_commands() {
  command -v gcloud >/dev/null 2>&1 || die 'gcloud is unavailable'
  command -v python3 >/dev/null 2>&1 || die 'python3 is unavailable'
}

quota_preflight() {
  local quota_json
  quota_json="$(gcloud compute regions describe "$REGION" --project="$PROJECT_ID" --format=json)" ||
    die "cannot read regional quota for $REGION"
  python3 -c 'import json,sys
quotas={q.get("metric"):q for q in json.load(sys.stdin).get("quotas",[])}
required={"N2_CPUS":8,"INSTANCES":1}
for metric,need in required.items():
    q=quotas.get(metric)
    if not q: raise SystemExit(f"missing quota metric {metric}")
    available=float(q.get("limit",0))-float(q.get("usage",0))
    print("{}: usage={} limit={} available={:g}".format(metric, q.get("usage",0), q.get("limit",0), available))
    if available < need: raise SystemExit(f"{metric} available quota {available:g} is below required {need}")' <<<"$quota_json" ||
    die 'regional quota is insufficient for the disposable smoke VM'
  printf 'Quota preflight passed for %s in %s; capacity is not guaranteed.\n' "$MACHINE_TYPE" "$REGION"
}

cache_preflight() {
  local object_uri="gs://${CACHE_BUCKET}/${CACHE_OBJECT}"
  local describe_output describe_status list_output list_status
  if describe_output="$(gcloud storage objects describe "$object_uri" \
      --project="$PROJECT_ID" --billing-project="$PROJECT_ID" 2>&1)"; then
    printf 'Private Cooking cache object is present: %s\n' "$object_uri"
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
    smoke|gcp-cooking) ;;
    *) die "unsupported test mode: $mode" ;;
  esac
  [[ -f "$STARTUP_SCRIPT" && ! -L "$STARTUP_SCRIPT" ]] || die "startup script is unavailable or symlinked: $STARTUP_SCRIPT"
  [[ -f "$PASST_PREFLIGHT_SCRIPT" && ! -L "$PASST_PREFLIGHT_SCRIPT" ]] || die "passthrough preflight script is unavailable or symlinked: $PASST_PREFLIGHT_SCRIPT"
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
  if [[ "$mode" == gcp-cooking ]]; then
    identity_args=(
      --service-account="hephaestus-cooking-runtime@${PROJECT_ID}.iam.gserviceaccount.com"
      --scopes=storage-ro
    )
  fi
  gcloud compute instances create "$smoke_name" \
    --project="$PROJECT_ID" --zone="$smoke_zone" --machine-type="$MACHINE_TYPE" \
    --network-interface=network=default,network-tier=PREMIUM \
    --image-family=ubuntu-2404-lts-amd64 --image-project=ubuntu-os-cloud \
    --boot-disk-size="$DISK_SIZE" --boot-disk-type=pd-balanced \
    --boot-disk-auto-delete --enable-nested-virtualization \
    --max-run-duration=45m --instance-termination-action=DELETE \
    --maintenance-policy=TERMINATE "${identity_args[@]}" \
    --labels="purpose=hephaestus-kvm-smoke,run_id=${GITHUB_RUN_ID:-manual},run_attempt=${GITHUB_RUN_ATTEMPT:-1},sha=$GITHUB_SHA" \
    --metadata="test-mode=${mode},github-sha=$GITHUB_SHA" \
    --metadata-from-file="startup-script=$STARTUP_SCRIPT,passt-preflight-script=$PASST_PREFLIGHT_SCRIPT" || {
      gcloud compute instances describe "$smoke_name" --project="$PROJECT_ID" --zone="$smoke_zone" >/dev/null 2>&1 && smoke_created=true
      die 'instance creation failed'
    }
  smoke_created=true
  local deadline=$((SECONDS + 2400)) serial='' last_serial=''
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
  while (( SECONDS < deadline )); do
    if serial="$(gcloud compute instances get-serial-port-output "$smoke_name" --project="$PROJECT_ID" --zone="$smoke_zone" --port=1 2>&1)"; then
      serial="${serial//$'\r'/}"
      last_serial="$serial"
      record_serial "$serial"
    else
      record_serial "serial retrieval error (bounded retry): $serial"
      [[ -z "$last_serial" ]] || record_serial "$last_serial"
      sleep 10
      continue
    fi
    if marker_matches "$pass_marker" "$serial"; then
      printf 'GCE %s passed for %s (%s); cleanup is automatic.\n' "$mode" "$smoke_name" "$GITHUB_SHA"
      return 0
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
  printf '%s\n' "$last_serial" | tail -80 >&2
  die 'timed out waiting for the startup smoke marker'
}

require_commands
case "${1:-preflight}" in
  preflight) quota_preflight ;;
  cache-preflight) quota_preflight; cache_preflight ;;
  smoke) quota_preflight; smoke smoke ;;
  gcp-cooking) quota_preflight; cache_preflight; smoke gcp-cooking ;;
  cleanup) cleanup_vm ;;
  *) die 'usage: scripts/gcp-kvm-smoke.sh [preflight|cache-preflight|smoke|gcp-cooking|cleanup]' ;;
esac
