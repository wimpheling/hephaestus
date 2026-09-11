#!/usr/bin/env bash
# Build or verify one immutable, project-owned GCE runner image.
# The builder is deliberately SA-less. The startup payload is a local bundle
# of the reviewed bake recipe and its public scripts.
set -Eeuo pipefail
umask 077

readonly PROJECT_ID='hephaestus-508000'
readonly REGION='europe-west1'
readonly MACHINE_TYPE='n2-standard-8'
readonly DISK_SIZE='150GB'
readonly IMAGE_PREFIX='hephaestus-runner'
readonly STATE_FILE_DEFAULT="${RUNNER_TEMP:-/tmp}/gcp-runner-image-build-state"

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
state_file="${GCP_RUNNER_IMAGE_STATE:-$STATE_FILE_DEFAULT}"
repo_sha=''
recipe_sha=''
verifier_sha=''
startup_sha=''
bake_sha=''
manifest_generator_sha=''
lock_sha=''
zone="${GCP_ZONE:-europe-west1-d}"
fingerprint=''
image_name=''
builder_name=''
disk_name=''
startup_bundle=''
recovery_status=''
gcloud_json_output=''
gcloud_json_stderr=''
serial_log=''
serial_tail_log=''

die() { printf 'gcp-runner-image-build: %s\n' "$*" >&2; exit 1; }

run_json_gcloud() {
  local stderr_file rc
  stderr_file="$(mktemp "${TMPDIR:-/tmp}/gcp-runner-image-gcloud-stderr.XXXXXX")"
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
  local command
  for command in gcloud python3 timeout date; do
    command -v "$command" >/dev/null 2>&1 || die "$command is unavailable"
  done
}

validate_zone() { [[ "$zone" == "$REGION"-* ]] || die "zone must be in $REGION: $zone"; }

load_contract() {
  repo_sha="${GITHUB_SHA:-}"
  [[ "$repo_sha" =~ ^[0-9a-f]{40}$ ]] || die 'GITHUB_SHA must be the exact workflow commit SHA'
  source "$script_dir/gcp-runner-image-provision.sh"
  local lock_path="$script_dir/../e2e/playwright/package-lock.json"
  [[ -f "$lock_path" && ! -L "$lock_path" ]] || die "browser lockfile is unavailable: $lock_path"
  lock_sha="$(sha256sum "$lock_path" | awk '{print $1}')"
  recipe_sha="$(sha256sum "$script_dir/gcp-runner-image-provision.sh" | awk '{print $1}')"
  verifier_sha="$(sha256sum "$script_dir/gcp-runner-image-verify.py" | awk '{print $1}')"
  startup_sha="$(sha256sum "$script_dir/gcp-kvm-startup.sh" | awk '{print $1}')"
  bake_sha="$(sha256sum "$script_dir/gcp-runner-image-bake.sh" | awk '{print $1}')"
  manifest_generator_sha="$(sha256sum "$script_dir/gcp-runner-image-manifest.py" | awk '{print $1}')"
  local run_id="${GITHUB_RUN_ID:-manual}" attempt="${GITHUB_RUN_ATTEMPT:-1}"
  [[ "$run_id" =~ ^[a-z0-9-]+$ && "$attempt" =~ ^[a-z0-9-]+$ ]] || die 'workflow run identifiers are invalid'
  builder_name="${IMAGE_PREFIX}-builder-${run_id}-${attempt}-${repo_sha:0:12}"
  disk_name="${IMAGE_PREFIX}-disk-${run_id}-${attempt}-${repo_sha:0:12}"
  [[ "$builder_name" =~ ^[a-z][a-z0-9-]{0,62}$ ]] || die 'derived builder name is invalid'
  [[ "$disk_name" =~ ^[a-z][a-z0-9-]{0,62}$ ]] || die 'derived disk name is invalid'
}
not_found() {
  local name="$1" output="$2"
  grep -Eqi "${name}[^[:alnum:]_].*was not found|resource[[:space:]]+['\"]?[^'\"]*${name}[^'\"]*['\"]?[[:space:]]+was not found" <<<"$output"
}

owned_labels() {
  local purpose="$1"
  if [[ "$purpose" == hephaestus-runner-image ]]; then
    printf 'purpose=%s,run_id=%s,run_attempt=%s,repository_sha=%s,fingerprint=%s\n' \
      "$purpose" "${GITHUB_RUN_ID:-manual}" "${GITHUB_RUN_ATTEMPT:-1}" "$repo_sha" "${fingerprint:0:32}"
  else
    printf 'purpose=%s,run_id=%s,run_attempt=%s,repository_sha=%s\n' \
      "$purpose" "${GITHUB_RUN_ID:-manual}" "${GITHUB_RUN_ATTEMPT:-1}" "$repo_sha"
  fi
}

verify_labels() {
  local data="$1" purpose="$2" resource="$3"
  python3 - "$data" "$purpose" "${GITHUB_RUN_ID:-manual}" "${GITHUB_RUN_ATTEMPT:-1}" "$repo_sha" "$resource" <<'PY'
import json
import sys
value = json.loads(sys.argv[1])
labels = value.get("labels", {})
expected = {"purpose": sys.argv[2], "run_id": sys.argv[3],
            "run_attempt": sys.argv[4], "repository_sha": sys.argv[5]}
if any(labels.get(key) != item for key, item in expected.items()):
    raise SystemExit(f"{sys.argv[6]} ownership labels do not match")
PY
}

verify_image_labels() {
  local data="$1" resource="$2"
  python3 - "$data" "$fingerprint" "$recipe_sha" "$verifier_sha" "$startup_sha" "$repo_sha" "$resource" <<'PY'
import json
import re
import sys
value = json.loads(sys.argv[1])
labels = value.get("labels", {})
description = value.get("description", "")
expected = {"purpose": "hephaestus-runner-image", "repository_sha": sys.argv[6],
            "fingerprint": sys.argv[2][:32]}
if any(labels.get(key) != item for key, item in expected.items()):
        raise SystemExit(f"{sys.argv[7]} immutable image labels do not match")
for key, expected_value in (("manifest_sha256", sys.argv[2]), ("recipe_sha256", sys.argv[3]),
                            ("verifier_sha256", sys.argv[4]), ("startup_sha256", sys.argv[5])):
    if not re.search(rf"(?:^|[ ,]){key}={re.escape(expected_value)}(?:$|[ ,])", description):
        raise SystemExit(f"{sys.argv[7]} immutable image description anchor does not match: {key}")
PY
}
derive_image_name() {
  [[ "$fingerprint" =~ ^[0-9a-f]{64}$ ]] || die 'runner image fingerprint is invalid'
  image_name="${IMAGE_PREFIX}-${fingerprint:0:32}"
  [[ "$image_name" =~ ^[a-z][a-z0-9-]{0,62}$ ]] || die 'derived image name is invalid'
}

describe_image() {
  local output
  if run_json_gcloud compute images describe "$image_name" --project="$PROJECT_ID" --format=json; then
    output="$gcloud_json_output"
    report_gcloud_json_stderr
    verify_image_labels "$output" "$image_name" || return 1
    grep -Eq '"status"[[:space:]]*:[[:space:]]*"READY"' <<<"$output" || die "existing image is not READY: $image_name"
    printf 'Existing immutable runner image is READY: %s\n' "$image_name"
    return 0
  fi
  if not_found "$image_name" "$gcloud_json_stderr"; then return 1; fi
  printf '%s\n' "$gcloud_json_stderr" >&2
  die "cannot establish image state: $image_name"
}

find_existing_image() {
  local output selected
  run_json_gcloud compute images list --project="$PROJECT_ID" \
    --filter="labels.purpose=hephaestus-runner-image AND labels.repository_sha=$repo_sha" \
    --format=json || { printf '%s\n' "$gcloud_json_stderr" >&2; die 'cannot list existing runner images'; }
  output="$gcloud_json_output"
  report_gcloud_json_stderr
  selected="$(RUNNER_IMAGE_RECIPE_SHA="$recipe_sha" RUNNER_IMAGE_VERIFIER_SHA="$verifier_sha" \
    RUNNER_IMAGE_STARTUP_SHA="$startup_sha" python3 - "$output" <<'PY'
import json
import os
import sys
items = json.loads(sys.argv[1])
expected = {
    "recipe_sha256": os.environ["RUNNER_IMAGE_RECIPE_SHA"],
    "verifier_sha256": os.environ["RUNNER_IMAGE_VERIFIER_SHA"],
    "startup_sha256": os.environ["RUNNER_IMAGE_STARTUP_SHA"],
}
matches = []
for item in items:
    labels = item.get("labels", {})
    description = item.get("description", "")
    anchors = {key: (__import__("re").search(rf"(?:^|[ ,]){key}=([0-9a-f]{{64}})(?:$|[ ,])", description) or [None, ""])[1] for key in expected}
    fingerprint_match = __import__("re").search(r"(?:^|[ ,])manifest_sha256=([0-9a-f]{64})(?:$|[ ,])", description)
    fingerprint = labels.get("fingerprint", "")
    if item.get("status") == "READY" and all(anchors[k] == v for k, v in expected.items()) and len(fingerprint) == 32 and all(c in "0123456789abcdef" for c in fingerprint) and fingerprint_match:
        matches.append((item.get("name", ""), fingerprint_match.group(1)))
if len(matches) > 1:
    raise SystemExit("multiple matching immutable runner images exist")
if matches:
    print(*matches[0])
PY
  )" || die 'existing runner image metadata is ambiguous'
  if [[ -n "$selected" ]]; then
    read -r image_name fingerprint <<<"$selected"
    derive_image_name
    describe_image || return 1
    return 0
  fi
  return 1
}
describe_builder_absent() {
  local output
  if run_json_gcloud compute instances describe "$builder_name" --project="$PROJECT_ID" --zone="$zone" --format=json; then
    output="$gcloud_json_output"
    report_gcloud_json_stderr
    printf '%s\n' "$output" >&2
    return 1
  fi
  not_found "$builder_name" "$gcloud_json_stderr" || { printf '%s\n' "$gcloud_json_stderr" >&2; return 1; }
}

create_owned_disk() {
  gcloud compute disks create "$disk_name" --project="$PROJECT_ID" --zone="$zone" \
    --size="$DISK_SIZE" --type=pd-balanced --image-family=ubuntu-2404-lts-amd64 \
    --image-project=ubuntu-os-cloud --labels="$(owned_labels hephaestus-runner-image-builder)"
}

delete_owned_vm() {
  local output
  if run_json_gcloud compute instances describe "$builder_name" --project="$PROJECT_ID" --zone="$zone" --format=json; then
    output="$gcloud_json_output"
    report_gcloud_json_stderr
    verify_labels "$output" hephaestus-runner-image-builder "$builder_name" || return 1
    gcloud compute instances delete "$builder_name" --project="$PROJECT_ID" --zone="$zone" --quiet >/dev/null
    describe_builder_absent || return 1
  elif ! not_found "$builder_name" "$gcloud_json_stderr"; then
    printf '%s\n' "$gcloud_json_stderr" >&2
    return 1
  fi
}

describe_disk_absent() {
  local output
  if run_json_gcloud compute disks describe "$disk_name" --project="$PROJECT_ID" --zone="$zone" --format=json; then
    output="$gcloud_json_output"
    report_gcloud_json_stderr
    printf '%s\n' "$output" >&2
    return 1
  fi
  not_found "$disk_name" "$gcloud_json_stderr" || { printf '%s\n' "$gcloud_json_stderr" >&2; return 1; }
}

delete_owned_disk() {
  local output
  if run_json_gcloud compute disks describe "$disk_name" --project="$PROJECT_ID" --zone="$zone" --format=json; then
    output="$gcloud_json_output"
    report_gcloud_json_stderr
    verify_labels "$output" hephaestus-runner-image-builder "$disk_name" || return 1
    gcloud compute disks delete "$disk_name" --project="$PROJECT_ID" --zone="$zone" --quiet >/dev/null
    describe_disk_absent || return 1
  elif ! not_found "$disk_name" "$gcloud_json_stderr"; then
    printf '%s\n' "$gcloud_json_stderr" >&2
    return 1
  fi
}

delete_owned_image() {
  local output
  if run_json_gcloud compute images describe "$image_name" --project="$PROJECT_ID" --format=json; then
    output="$gcloud_json_output"
    report_gcloud_json_stderr
    verify_labels "$output" hephaestus-runner-image "$image_name" || return 1
    gcloud compute images delete "$image_name" --project="$PROJECT_ID" --quiet >/dev/null
    describe_image_absent || return 1
  elif ! not_found "$image_name" "$gcloud_json_stderr"; then
    printf '%s\n' "$gcloud_json_stderr" >&2
    return 1
  fi
}

describe_image_absent() {
  local output
  [[ -n "$image_name" ]] || return 0
  if run_json_gcloud compute images describe "$image_name" --project="$PROJECT_ID" --format=json; then
    output="$gcloud_json_output"
    report_gcloud_json_stderr
    printf '%s\n' "$output" >&2
    return 1
  fi
  not_found "$image_name" "$gcloud_json_stderr" || { printf '%s\n' "$gcloud_json_stderr" >&2; return 1; }
}

cleanup() {
  local status=0
  [[ -n "$builder_name" ]] || return 0
  delete_owned_vm || status=1
  delete_owned_disk || status=1
  if [[ -n "$image_name" && "$recovery_status" != ready ]]; then
    delete_owned_image || status=1
  fi
  return "$status"
}

cleanup_on_exit() {
  local original_status="$?" cleanup_status=0
  trap - EXIT
  if ((original_status != 0)); then
    report_serial_diagnostics
  fi
  cleanup || cleanup_status=$?
  if ((original_status == 0 && cleanup_status != 0)); then
    report_serial_diagnostics
  fi
  if ((original_status != 0)); then
    exit "$original_status"
  fi
  exit "$cleanup_status"
}

on_signal() {
  local signal="$1" cleanup_status=0
  trap - EXIT INT TERM
  report_serial_diagnostics
  cleanup || cleanup_status=$?
  ((cleanup_status == 0)) || exit "$cleanup_status"
  exit "$((128 + signal))"
}

report_serial_diagnostics() {
  if [[ -f "$serial_log" && ! -L "$serial_log" ]]; then
    printf 'Bounded runner serial phase diagnostics: %s\n' "$serial_log" >&2
    tail -80 "$serial_log" >&2 || true
  else
    printf 'Bounded runner serial phase diagnostics unavailable\n' >&2
  fi
  if [[ -f "$serial_tail_log" && ! -L "$serial_tail_log" ]]; then
    printf 'Bounded runner serial error tail: %s\n' "$serial_tail_log" >&2
    tail -80 "$serial_tail_log" >&2 || true
  else
    printf 'Bounded runner serial error tail unavailable\n' >&2
  fi
}

prepare_diagnostics_file() {
  local path="$1" parent
  parent="$(dirname -- "$path")"
  if [[ -e "$parent" || -L "$parent" ]]; then
    [[ -d "$parent" && ! -L "$parent" ]] || die "serial diagnostics parent is not a directory: $parent"
  else
    install -d -m 0700 -- "$parent"
  fi
  [[ ! -L "$path" && ( ! -e "$path" || -f "$path" ) ]] ||
    die "serial diagnostics path is not a regular file: $path"
  : >"$path"
  chmod 0600 "$path"
}

prepare_serial_logs() {
  prepare_diagnostics_file "$serial_log"
  prepare_diagnostics_file "$serial_tail_log"
}

record_serial() {
  local value="$1" summary
  if ! summary="$(python3 -c '
import pathlib
import re
import sys
import importlib.util

phase_path = pathlib.Path(sys.argv[1])
tail_path = pathlib.Path(sys.argv[2])
evidence_path = pathlib.Path(sys.argv[3])
text = sys.stdin.read()
credential = re.compile(r"(?i)(?:authorization\s*:\s*bearer|bearer\s+[A-Za-z0-9._-]{16,}|(?:\x22|\x27)?[A-Za-z0-9_-]*(?:password|passwd|secret|token|credential|api[_ -]?key|private[_ -]?key)[A-Za-z0-9_-]*(?:\x22|\x27)?\s*[:=]|BEGIN [A-Z ]*PRIVATE KEY)")
generic_credential = re.compile(
    r"(?i)(?:-----BEGIN [A-Z ]*PRIVATE KEY-----|"
    r"fixture[-_ ](?:secret|credential|token|password)(?:[-_ ][A-Za-z0-9]+)*)"
)

spec = importlib.util.spec_from_file_location("serial_evidence", evidence_path)
if spec is None or spec.loader is None:
    raise SystemExit("serial evidence scanner is unavailable")
evidence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evidence)

phases = {
    "initializing", "metadata", "diagnostic-bootstrap", "diagnostic-synthetic",
    "host-packages", "accounts", "passt-compat", "cgroup-podman", "passt-apparmor",
    "passt-preflight", "rust-toolchain", "libkrunfw", "libkrun", "image-bake-ready",
    "checkout", "gcp-cooking", "real-libkrun-smoke",
}
prefix = re.compile(r"^\[[^]]+\] google_metadata_script_runner\[\d+\]: ")
safe = []
safe_tail = []
normalized = []
terminal = None
for raw in text.splitlines():
    line = prefix.sub("", raw.strip())
    if "startup-script:" in line:
        line = line.split("startup-script:", 1)[1].lstrip()
    normalized.append(line)
    phase = re.fullmatch(r"HEPH_GCP_KVM_STARTUP event=phase-start phase=([a-z0-9-]+) revision=([0-9a-f]{40})", line)
    if phase and phase.group(1) in phases:
        safe.append(f"HEPH_GCP_IMAGE_BUILD phase={phase.group(1)} revision={phase.group(2)}")
        continue
    image_phase = re.fullmatch(r"HEPH_GCP_RUNNER_IMAGE phase=([a-z0-9-]+) status=(prebuilt|deferred-to-boot)", line)
    if image_phase and image_phase.group(1) in phases:
        safe.append(f"HEPH_GCP_IMAGE_BUILD phase={image_phase.group(1)} status={image_phase.group(2)}")
        continue
    ready = re.fullmatch(r"HEPH_GCP_RUNNER_IMAGE: READY fingerprint=([0-9a-f]{64})", line)
    if ready:
        terminal = f"ready fingerprint={ready.group(1)}"
        safe.append(f"HEPH_GCP_IMAGE_BUILD terminal=ready fingerprint={ready.group(1)}")
        continue
    failed = re.fullmatch(r"HEPH_GCP_RUNNER_IMAGE: FAIL exit=([0-9]+)", line)
    if failed:
        terminal = f"fail exit={failed.group(1)}"
        safe.append(f"HEPH_GCP_IMAGE_BUILD terminal=fail exit={failed.group(1)}")
        continue
    baked = re.fullmatch(r"HEPH_GCP_RUNNER_IMAGE bake=pass", line)
    if baked:
        safe.append("HEPH_GCP_IMAGE_BUILD bake=pass")

normalized_text = "\n".join(normalized)
normalized_bytes = normalized_text.encode("utf-8", errors="surrogateescape")
rejected = bool(
    any(pattern in normalized_bytes for pattern in evidence.STREAM_PATTERNS)
    or credential.search(normalized_text)
    or generic_credential.search(normalized_text)
)
if not rejected:
    safe_tail = normalized

old = phase_path.read_text(encoding="utf-8").splitlines() if phase_path.exists() else []
existing = set(old)
combined = []
for line in old + safe:
    if line not in combined:
        combined.append(line)
phase_path.write_text("\n".join(combined[-80:]) + ("\n" if combined else ""), encoding="utf-8")
old_tail = tail_path.read_text(encoding="utf-8").splitlines() if tail_path.exists() else []
tail = [] if rejected else (old_tail + safe_tail)[-80:]
tail_text = "\n".join(tail) + ("\n" if tail else "")
tail_bytes = tail_text.encode("utf-8")
if len(tail_bytes) > 64 * 1024:
    tail_text = tail_bytes[-(64 * 1024):].decode("utf-8", errors="ignore")
tail_path.write_text(tail_text, encoding="utf-8")
if rejected:
    print("serial diagnostics rejected by credential scanner", file=sys.stderr)
    # A hit invalidates the complete raw tail for this poll. Typed markers
    # above remain safe because they are parsed into fixed schemas.
    with phase_path.open("a", encoding="utf-8") as handle:
        handle.write("HEPH_GCP_IMAGE_BUILD serial_diagnostics=rejected reason=credential-scan\n")
for line in safe:
    if line not in existing:
        print(line)
        existing.add(line)
if terminal is not None:
    print(f"__CONTROL__ terminal={terminal}")
' "$serial_log" "$serial_tail_log" "$script_dir/check-browser-evidence.py" <<<"$value")"; then
    return 1
  fi
  printf '%s\n' "$summary"
}

handle_serial_summary() {
  local summary="$1" ready_fingerprint public_summary
  public_summary="$(sed '/^__CONTROL__/d' <<<"$summary")"
  [[ -z "$public_summary" ]] || printf '%s\n' "$public_summary"
  if ready_fingerprint="$(sed -n 's/^__CONTROL__ terminal=ready fingerprint=\([0-9a-f]\{64\}\)$/\1/p' <<<"$summary")"; then
    if [[ -n "$ready_fingerprint" ]]; then
      fingerprint="$ready_fingerprint"
      derive_image_name
      printf 'Runner image provisioning reported READY for fingerprint %s\n' "$fingerprint"
      return 0
    fi
  fi
  if grep -Eq '^__CONTROL__ terminal=fail exit=[0-9]+$' <<<"$summary"; then
    return 1
  fi
  return 2
}

create_startup_bundle() {
  startup_bundle="$(mktemp "${TMPDIR:-/tmp}/gcp-runner-image-startup.XXXXXX.sh")"
  chmod 0700 "$startup_bundle"
  python3 - "$startup_bundle" "$script_dir" "$GITHUB_SHA" <<'PY'
import base64
import io
import pathlib
import sys
import tarfile

out, source, repo_sha = sys.argv[1:]
source = pathlib.Path(source)
names = ["gcp-runner-image-bake.sh", "gcp-runner-image-provision.sh",
         "gcp-runner-image-verify.py", "gcp-runner-image-manifest.py",
         "gcp-kvm-startup.sh", "gcp-passt-preflight.sh"]
buffer = io.BytesIO()
with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
    for name in names:
        path = source / name
        if not path.is_file() or path.is_symlink():
            raise SystemExit(f"startup bundle source is unavailable: {path}")
        archive.add(path, arcname=name, recursive=False)
payload = base64.b64encode(buffer.getvalue()).decode("ascii")
path = pathlib.Path(out)
with path.open("w", encoding="ascii") as handle:
    handle.write("#!/usr/bin/env bash\nset -Eeuo pipefail\n")
    handle.write('bundle_dir="$(mktemp -d /run/hephaestus-image-bake.XXXXXX)"\n')
    handle.write('finish() {\n')
    handle.write('  status=$?\n')
    handle.write('  trap - EXIT\n')
    handle.write('  rm -rf -- "$bundle_dir" || true\n')
    handle.write('  if ((status != 0)); then printf "HEPH_GCP_RUNNER_IMAGE: FAIL exit=%s\\n" "$status"; fi\n')
    handle.write('  exit "$status"\n')
    handle.write('}\n')
    handle.write('trap finish EXIT\n')
    handle.write("base64 -d >\"$bundle_dir/payload.tgz\" <<'HEPH_GCP_RUNNER_IMAGE_BUNDLE'\n")
    for start in range(0, len(payload), 120):
        handle.write(payload[start:start + 120] + "\n")
    handle.write("HEPH_GCP_RUNNER_IMAGE_BUNDLE\n")
    handle.write("tar -xzf \"$bundle_dir/payload.tgz\" -C \"$bundle_dir\"\n")
    handle.write(f"export HEPH_GCP_IMAGE_BAKE=1 HEPH_GCP_IMAGE_BAKE_REPO_SHA={repo_sha!r}\n")
    handle.write('export HEPH_GCP_IMAGE_BAKE_HELPER="$bundle_dir/gcp-runner-image-provision.sh"\n')
    handle.write('export HEPH_GCP_LOCAL_PASST_PREFLIGHT="$bundle_dir/gcp-passt-preflight.sh"\n')
    handle.write('bash "$bundle_dir/gcp-runner-image-bake.sh"\n')
PY
}

wait_serial_ready() {
  local deadline="$1" serial='' summary='' summary_status
  while (( $(date +%s) < deadline )); do
    if serial="$(gcloud compute instances get-serial-port-output "$builder_name" --project="$PROJECT_ID" --zone="$zone" --port=1 2>&1)"; then
      if ! summary="$(record_serial "$serial")"; then return 1; fi
      if handle_serial_summary "$summary"; then return 0; else summary_status=$?; fi
      ((summary_status == 1)) && return 1
    else
      if ! summary="$(record_serial "$serial")"; then return 1; fi
      if handle_serial_summary "$summary"; then return 0; else summary_status=$?; fi
      ((summary_status == 1)) && return 1
    fi
    sleep 10
  done
  return 1
}

write_ready_state() {
  write_state ready
}

write_state() {
  local status="$1"
  recovery_status="$status"
  install -d -m 0700 "$(dirname -- "$state_file")"
  {
    printf 'schema=1\nstatus=%s\nproject=%s\nzone=%s\nrun_id=%s\nrun_attempt=%s\nrepository_sha=%s\n' \
      "$status" "$PROJECT_ID" "$zone" "${GITHUB_RUN_ID:-manual}" \
      "${GITHUB_RUN_ATTEMPT:-1}" "$repo_sha"
    printf 'builder=%s\ndisk=%s\nimage=%s\nfingerprint=%s\n' \
      "$builder_name" "$disk_name" "$image_name" "$fingerprint"
  } >"$state_file"
  chmod 0600 "$state_file"
}

load_recovery_state() {
  [[ -f "$state_file" && ! -L "$state_file" ]] || return 0
  local status saved_project saved_zone saved_run_id saved_attempt saved_repo
  local saved_builder saved_disk saved_image saved_fingerprint
  awk -F= 'BEGIN { split("schema status project zone run_id run_attempt repository_sha builder disk image fingerprint", keys, /[[:space:]]+/); for (i in keys) allowed[keys[i]]=1 }
    index($0, "=") == 0 || !allowed[$1] { exit 1 }' "$state_file" || die 'runner image state contains an unknown or malformed field'
  state_field() {
    local matches count
    matches="$(awk -F= -v key="$1" '$1 == key { print substr($0, index($0, "=") + 1) }' "$state_file")" || return 1
    count="$(awk -F= -v key="$1" '$1 == key { count += 1 } END { print count + 0 }' "$state_file")"
    [[ "$count" == 1 ]] || return 1
    printf '%s\n' "$matches"
  }
  status="$(state_field status)" || die 'runner image state is missing status'
  [[ "$(state_field schema)" == 1 ]] || die 'runner image state schema is unsupported'
  saved_project="$(state_field project)" || die 'runner image state is missing project'
  saved_zone="$(state_field zone)" || die 'runner image state is missing zone'
  saved_run_id="$(state_field run_id)" || die 'runner image state is missing run_id'
  saved_attempt="$(state_field run_attempt)" || die 'runner image state is missing run_attempt'
  saved_repo="$(state_field repository_sha)" || die 'runner image state is missing repository_sha'
  saved_builder="$(state_field builder)" || die 'runner image state is missing builder'
  saved_disk="$(state_field disk)" || die 'runner image state is missing disk'
  saved_image="$(state_field image)" || die 'runner image state is missing image'
  saved_fingerprint="$(state_field fingerprint)" || die 'runner image state is missing fingerprint'
  [[ "$status" == building || "$status" == ready ]] || die 'runner image state status is invalid'
  [[ "$saved_project" == "$PROJECT_ID" && "$saved_zone" == "$zone" ]] || die 'runner image state ownership scope mismatch'
  [[ "$saved_run_id" == "${GITHUB_RUN_ID:-manual}" && "$saved_attempt" == "${GITHUB_RUN_ATTEMPT:-1}" ]] ||
    die 'runner image state workflow ownership mismatch'
  [[ "$saved_repo" == "$repo_sha" ]] || die 'runner image state repository ownership mismatch'
  [[ "$saved_builder" =~ ^[a-z][a-z0-9-]{0,62}$ && "$saved_disk" =~ ^[a-z][a-z0-9-]{0,62}$ ]] ||
    die 'runner image state resource name is invalid'
  [[ -z "$saved_image" || "$saved_image" =~ ^[a-z][a-z0-9-]{0,62}$ ]] || die 'runner image state image name is invalid'
  [[ -z "$saved_fingerprint" || "$saved_fingerprint" =~ ^[0-9a-f]{64}$ ]] || die 'runner image state fingerprint is invalid'
  builder_name="$saved_builder"
  disk_name="$saved_disk"
  image_name="$saved_image"
  fingerprint="$saved_fingerprint"
  recovery_status="$status"
}

build_image() {
  local trial_start deadline vm_status='' image_output
  if [[ "$recovery_status" == building ]]; then
    cleanup || die 'could not clean up the previous incomplete runner image build'
    rm -f -- "$state_file"
    image_name=''
    fingerprint=''
    recovery_status=''
  fi
  if find_existing_image; then write_ready_state; return 0; fi
  create_startup_bundle
  trial_start="$(date +%s)"
  deadline=$((trial_start + 2400))
  write_state building
  printf 'Building immutable runner image from repository %s; final name follows manifest fingerprint\n' "$repo_sha"
  create_owned_disk
  gcloud compute instances create "$builder_name" \
    --project="$PROJECT_ID" --zone="$zone" --machine-type="$MACHINE_TYPE" \
    --network-interface=network=default,network-tier=PREMIUM \
    --disk="name=$disk_name,boot=yes,auto-delete=no" \
    --max-run-duration=45m --instance-termination-action=DELETE \
    --maintenance-policy=TERMINATE --no-service-account --no-scopes \
    --labels="$(owned_labels hephaestus-runner-image-builder)" \
    --metadata="runner-image-repository-sha=$repo_sha" \
    --metadata-from-file="startup-script=$startup_bundle"
  wait_serial_ready "$deadline" || die 'runner image provisioning did not report READY'
  derive_image_name
  gcloud compute instances stop "$builder_name" --project="$PROJECT_ID" --zone="$zone" --quiet >/dev/null
  while (( $(date +%s) < deadline )); do
    vm_status="$(gcloud compute instances describe "$builder_name" --project="$PROJECT_ID" --zone="$zone" --format='get(status)' 2>/dev/null || true)"
    [[ "$vm_status" == TERMINATED ]] && break
    sleep 5
  done
  [[ "$vm_status" == TERMINATED ]] || die 'builder VM did not reach TERMINATED before deadline'
  delete_owned_vm || die 'builder VM deletion failed'
  write_state building
  gcloud compute images create "$image_name" --project="$PROJECT_ID" --source-disk="$disk_name" \
    --source-disk-zone="$zone" --labels="$(owned_labels hephaestus-runner-image)" \
    --description="hephaestus-runner manifest_sha256=$fingerprint recipe_sha256=$recipe_sha verifier_sha256=$verifier_sha startup_sha256=$startup_sha"
  run_json_gcloud compute images describe "$image_name" --project="$PROJECT_ID" --format=json || {
    printf '%s\n' "$gcloud_json_stderr" >&2; die 'created image cannot be described'; }
  image_output="$gcloud_json_output"
  report_gcloud_json_stderr
  verify_image_labels "$image_output" "$image_name"
  grep -Eq '"status"[[:space:]]*:[[:space:]]*"READY"' <<<"$image_output" || die 'created image is not READY'
  delete_owned_disk || die 'source disk deletion failed after image creation'
  write_ready_state
  printf 'Immutable runner image READY: %s\n' "$image_name"
}

require_commands
validate_zone
load_contract
serial_log="${GCP_RUNNER_IMAGE_SERIAL_LOG:-${RUNNER_TEMP:-/tmp}/gcp-runner-image-serial-${GITHUB_RUN_ID:-manual}-${GITHUB_RUN_ATTEMPT:-1}.log}"
serial_tail_log="${GCP_RUNNER_IMAGE_SERIAL_TAIL_LOG:-${serial_log}.tail}"
if [[ "${1:-build}" == build ]]; then
  prepare_serial_logs
fi
load_recovery_state
trap cleanup_on_exit EXIT
trap 'on_signal 2' INT
trap 'on_signal 15' TERM
case "${1:-build}" in
  build) build_image ;;
  cleanup)
    cleanup_status=0
    cleanup || cleanup_status=$?
    report_serial_diagnostics
    exit "$cleanup_status"
    ;;
  *) die 'usage: scripts/gcp-runner-image-build.sh [build|cleanup]' ;;
esac
