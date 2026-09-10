#!/usr/bin/env bash
# Read the regional quota or create one disposable nested-KVM smoke VM.
# This script never attaches a service account and never accepts credentials.
set -Eeuo pipefail

readonly PROJECT_ID="hephaestus-508000"
readonly REGION="europe-west1"
readonly STARTUP_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/gcp-kvm-startup.sh"
readonly MACHINE_TYPE="n2-standard-8"
readonly DISK_SIZE="150GB"
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
    gcloud compute instances delete "$name" --project="$PROJECT_ID" --zone="$zone" --quiet || { printf 'gcp-kvm-smoke: failed to delete owned VM: %s\n' "$name" >&2; return 1; }
    for _attempt in {1..30}; do
      local gone
      if gone="$(gcloud compute instances describe "$name" --project="$PROJECT_ID" --zone="$zone" 2>&1)"; then
        :
      elif grep -Eqi "instances/${name}[^[:alnum:]_].*was not found" <<<"$gone"; then
        printf 'Confirmed disposable VM absent: %s\n' "$name"
        return 0
      else
        printf '%s\n' "$gone" >&2
        return 1
      fi
      sleep 2
    done
    printf 'gcp-kvm-smoke: VM still exists after delete request: %s\n' "$name" >&2
    return 1
  fi
  if grep -Eqi "instances/${name}[^[:alnum:]_].*was not found" <<<"$data"; then
    printf 'Disposable VM already absent: %s\n' "$name"
    return 0
  fi
  printf '%s\n' "$data" >&2
  printf 'gcp-kvm-smoke: cannot inspect disposable VM for cleanup: %s\n' "$name" >&2
  return 1
}

smoke() {
  [[ -f "$STARTUP_SCRIPT" && ! -L "$STARTUP_SCRIPT" ]] || die "startup script is unavailable or symlinked: $STARTUP_SCRIPT"
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
  gcloud compute instances create "$smoke_name" \
    --project="$PROJECT_ID" --zone="$smoke_zone" --machine-type="$MACHINE_TYPE" \
    --network-interface=network=default,network-tier=PREMIUM \
    --image-family=ubuntu-2404-lts-amd64 --image-project=ubuntu-os-cloud \
    --boot-disk-size="$DISK_SIZE" --boot-disk-type=pd-balanced \
    --boot-disk-auto-delete --enable-nested-virtualization \
    --max-run-duration=45m --instance-termination-action=DELETE \
    --maintenance-policy=TERMINATE --no-service-account --no-scopes \
    --labels="purpose=hephaestus-kvm-smoke,run_id=${GITHUB_RUN_ID:-manual},run_attempt=${GITHUB_RUN_ATTEMPT:-1},sha=$GITHUB_SHA" \
    --metadata="github-sha=$GITHUB_SHA" --metadata-from-file="startup-script=$STARTUP_SCRIPT" || {
      gcloud compute instances describe "$smoke_name" --project="$PROJECT_ID" --zone="$smoke_zone" >/dev/null 2>&1 && smoke_created=true
      die 'instance creation failed'
    }
  smoke_created=true
  local deadline=$((SECONDS + 2400)) serial='' last_serial=''
  while (( SECONDS < deadline )); do
    if serial="$(gcloud compute instances get-serial-port-output "$smoke_name" --project="$PROJECT_ID" --zone="$smoke_zone" --port=1 2>&1)"; then
      last_serial="$serial"
      record_serial "$serial"
    else
      record_serial "serial retrieval error (bounded retry): $serial"
      [[ -z "$last_serial" ]] || record_serial "$last_serial"
      sleep 10
      continue
    fi
    if grep -q 'HEPHAESTUS_GCP_KVM_SMOKE: PASS' <<<"$serial"; then
      printf 'KVM smoke passed for %s (%s); cleanup is automatic.\n' "$smoke_name" "$GITHUB_SHA"
      return 0
    fi
    if grep -q 'HEPHAESTUS_GCP_KVM_SMOKE: FAIL' <<<"$serial"; then
      printf '%s\n' "$serial" | tail -80 >&2
      die 'startup smoke reported failure'
    fi
    sleep 10
  done
  printf '%s\n' "$last_serial" | tail -80 >&2
  die 'timed out waiting for the startup smoke marker'
}

require_commands
case "${1:-preflight}" in
  preflight) quota_preflight ;;
  smoke) quota_preflight; smoke ;;
  cleanup) cleanup_vm ;;
  *) die 'usage: scripts/gcp-kvm-smoke.sh [preflight|smoke]' ;;
esac
