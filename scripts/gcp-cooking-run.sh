#!/usr/bin/env bash
# Run the complete Cooking acceptance path from the immutable private cache.
# This helper is invoked as root by the disposable GCE startup flow. It uses
# the runtime service account only to fetch the cache, then runs all Cooking
# work as forge (UID/GID 10001) in a delegated transient systemd unit.
set -Eeuo pipefail
umask 077
# This process remains root while it stages inputs and collects evidence.  Keep
# its command lookup independent of the forge-writable cargo bin; workload
# units receive workload_path explicitly below.
readonly trusted_path='/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin'
PATH="$trusted_path"
export PATH

readonly metadata_root='http://metadata.google.internal/computeMetadata/v1'
readonly gcs_bucket='hephaestus-508000-cooking-cache'
readonly gcs_object='cooking/heph-gcp-cooking-cache.tar.zst'
readonly cache_sha256='0ed20efcc1aa019b79405d1eed626b13d4702019e9ceeba2bdde54e45ae29296'
readonly forge_uid=10001
readonly forge_gid=10001
readonly work_root='/srv/hephaestus'
readonly checkout_root="${work_root}/checkout"
readonly cache_root="${work_root}/cooking-cache"
readonly evidence_root="${work_root}/evidence/cooking"
readonly browser_root="${work_root}/playwright-browsers"
readonly node_version='v24.16.0'
readonly node_sha256='d804845d34eddc21dc1092b519d643ef40b1f58ec5dec5c22b1f4bd8fabde6c9'
readonly oras_version='1.3.3'
readonly oras_sha256='9ce999f8d2de03fc03968b29d743077a58783e545e5eaa53917ca177352d0e59'
readonly metadata_guard_table='hephaestus_gcp_metadata_guard'
readonly metadata_ip='169.254.169.254'
readonly metadata_ipv6='fd20:ce::254'
readonly log_file='/var/log/hephaestus/gcp-cooking-run.log'
readonly gate_results_path='/var/log/hephaestus/cooking-gate-results.json'
readonly gate_results_helper="${HEPH_GCP_COOKING_GATE_RESULTS_HELPER:-${checkout_root}/scripts/cooking-gate-results.py}"
readonly diagnostics_scanner_script="${HEPH_GCP_DIAGNOSTICS_SCANNER_SCRIPT:-${checkout_root}/scripts/check-browser-evidence.py}"
readonly browser_summary_script="${HEPH_GCP_BROWSER_SUMMARY_SCRIPT:-${checkout_root}/scripts/project-playwright-browser-summary.py}"
readonly workload_cleanup_reserve_seconds=120
readonly pr_state_root="${work_root}/pr-state"
readonly pr_home="${pr_state_root}/home"
readonly pr_npm_cache="${pr_state_root}/npm-cache"
readonly pr_runtime="${pr_state_root}/runtime"

phase='initializing'
stage_root=''
archive_path=''
node_bin=''
node_path=''
workload_path=''
deadline_epoch="${HEPH_GCP_COOKING_DEADLINE_EPOCH:-}"
token_json=''
token_header=''
runner_image_verified="${HEPH_GCP_RUNNER_IMAGE_VERIFIED:-false}"
runner_image_browser_lock_sha="${HEPH_GCP_RUNNER_IMAGE_BROWSER_LOCK_SHA256:-}"
runner_image_browser_version="${HEPH_GCP_RUNNER_IMAGE_BROWSER_VERSION:-}"
runner_image_node_version="${HEPH_GCP_RUNNER_IMAGE_NODE_VERSION:-}"
workload_trust="${HEPH_GCP_WORKLOAD_TRUST:-trusted}"
pr_sandbox_args=()
workload_home='/home/forge'
workload_cargo_home='/home/forge/.cargo'
workload_rustup_home='/home/forge/.rustup'
workload_npm_cache=''
workflow_image_sandbox_args=()
gate_results_initialized=false
gate_results_write_failed=false
runtime_phase_timing_script="${HEPH_GCP_PHASE_TIMING_SCRIPT:-}"
runtime_phase_timing_path="${HEPH_GCP_SUPERVISOR_PHASE_TIMING_PATH:-}"
runtime_phase_timing_open=''
runtime_phase_timing_cache_state=''
runtime_phase_timing_cache_sha256=''
runtime_phase_timing_cache_generation=''
runtime_phase_timing_cache_bytes=''
runtime_phase_timing_image_fingerprint="${HEPH_GCP_PHASE_TIMING_IMAGE_FINGERPRINT:-}"
declare -A runtime_phase_timing_occurrence=()

fail() { printf 'gcp-cooking-run: %s\n' "$*" >&2; return 1; }

runtime_phase_timing_outcome() {
    case "$1" in
        124) printf '%s\n' timed-out ;;
        130|143) printf '%s\n' cancelled ;;
        *) printf '%s\n' failed ;;
    esac
}

validate_sha256() {
    local name="$1" value="$2"
    [[ "$value" =~ ^[0-9a-f]{64}$ ]] ||
        fail "$name must be exactly 64 lowercase hexadecimal characters"
}

configure_pr_sandbox() {
    [[ "$workload_trust" == untrusted-pr ]] || return 0
    # Keep all PR-owned setup and the final workload in the same systemd
    # boundary.  These are deliberately array elements: each systemd
    # property must remain one complete argv value, including its path list.
    install -d -m 0700 -o forge -g forge \
        "$pr_state_root" "$pr_home" "$pr_npm_cache" "$pr_runtime"
    workload_home="$pr_home"
    # The baked image's Rust toolchain and cargo executable live under forge's
    # home.  Expose only those reviewed paths; cargo may update its local
    # package cache during an offline build, while rustup is read-only.
    workload_cargo_home='/home/forge/.cargo'
    workload_rustup_home='/home/forge/.rustup'
    workload_npm_cache="$pr_npm_cache"
    pr_sandbox_args=(
        '--property=NoNewPrivileges=yes'
        '--property=ProtectProc=invisible'
        '--property=ProcSubset=pid'
        '--property=ProtectSystem=strict'
        '--property=ProtectHome=tmpfs'
        '--property=PrivateTmp=yes'
        "--property=BindReadOnlyPaths=$cache_root"
        "--property=BindReadOnlyPaths=$browser_root"
        '--property=BindReadOnlyPaths=/home/forge/.rustup'
        '--property=BindPaths=/home/forge/.cargo'
        "--property=BindPaths=$pr_state_root"
        "--property=BindPaths=$pr_runtime:/run/user/10001"
        "--property=ReadWritePaths=$checkout_root $evidence_root $pr_state_root"
        '--property=InaccessiblePaths=/var/log/hephaestus /run/hephaestus /root'
    )
    # Trusted image import must use the same private Podman runtime namespace
    # that the later PR workload receives. The import still runs before any
    # PR-controlled setup starts.
    workflow_image_sandbox_args=(
        "--property=BindPaths=$pr_runtime:/run/user/10001"
    )
}

[[ "$(id -u)" -eq 0 ]] || fail 'this helper must be invoked as root'
[[ "$runner_image_verified" == true || "$runner_image_verified" == false ]] ||
    fail 'HEPH_GCP_RUNNER_IMAGE_VERIFIED must be true or false'
case "$workload_trust" in
    trusted|untrusted-pr) ;;
    *) fail 'HEPH_GCP_WORKLOAD_TRUST is invalid' ;;
esac
[[ "$(id -u forge 2>/dev/null || true)" == "${forge_uid}" ]] ||
    fail 'the common startup must create forge with UID 10001'
[[ -d "${checkout_root}" && ! -L "${checkout_root}" ]] ||
    fail "the common startup checkout is unavailable: ${checkout_root}"
[[ -x "${checkout_root}/examples/cooking/run.sh" ]] ||
    fail 'the checked-out Cooking entrypoint is unavailable'

install -d -m 0700 /var/log/hephaestus
install -m 0600 /dev/null "$log_file"
[[ -w /dev/ttyS0 ]] || fail 'GCE serial console /dev/ttyS0 is unavailable'
exec > >(tee -a "$log_file" /dev/ttyS0) 2>&1

finish() {
    local status=$?
    trap - EXIT
    if [[ -n "$runtime_phase_timing_open" ]]; then
        runtime_phase_timing_end "$(runtime_phase_timing_outcome "$status")" || true
    fi
    if [[ "$gate_results_initialized" == true ]]; then
        local gate_finalize_status=0
        set +e
        # Startup owns the supervisor's observed process exit.  This runtime
        # helper records only its aggregate result; startup may add its status
        # in one follow-up finalize while preserving this result.
        python3 -B "$gate_results_helper" --path "$gate_results_path" finalize \
            --overall-exit-code "$status"
        gate_finalize_status=$?
        set -e
        if ((gate_finalize_status != 0)); then
            gate_results_write_failed=true
            printf 'HEPH_GCP_COOKING event=gate-results status=failed reason=finalize-failed exit_code=%s\n' \
                "$gate_finalize_status" >&2
            if ((status == 0)); then
                status=$gate_finalize_status
            fi
        fi
        if [[ "$gate_results_write_failed" == true && "$status" == 0 ]]; then
            status=2
        fi
    fi
    # Keep the metadata guard installed after this helper exits. The provider
    # deletes this disposable VM; retaining the rule also covers stragglers
    # while systemd finishes stopping a timed-out Cooking unit.
    if [[ -n "${stage_root}" && -d "${stage_root}" ]]; then
        rm -rf -- "${stage_root}"
    fi
    if [[ -n "${token_json}" ]]; then
        rm -f -- "${token_json}"
    fi
    if [[ -n "${token_header}" ]]; then
        rm -f -- "${token_header}"
    fi
    if ((status == 0)); then
        printf 'HEPHAESTUS_GCP_COOKING: PASS phase=%s\n' "$phase"
    else
        printf 'HEPHAESTUS_GCP_COOKING: FAIL phase=%s exit=%s\n' "$phase" "$status"
    fi
    exit "$status"
}
trap finish EXIT

gate_update() {
    local update_status=0
    set +e
    python3 -B "$gate_results_helper" --path "$gate_results_path" "$@"
    update_status=$?
    set -e
    if ((update_status != 0)); then
        gate_results_write_failed=true
        printf 'HEPH_GCP_COOKING event=gate-results status=failed reason=update-failed exit_code=%s\n' \
            "$update_status" >&2
    fi
    return "$update_status"
}

runtime_phase_name() {
    case "$1" in
        host-tools) printf '%s\n' startup-host-packages ;;
        node|browser-host) printf '%s\n' browser-setup ;;
        cache-download) printf '%s\n' cache-download ;;
        cache-extract) printf '%s\n' cache-extract ;;
        workflow-images) printf '%s\n' workflow-images ;;
        metadata-guard) printf '%s\n' metadata-guard ;;
        cooking) printf '%s\n' cooking-supervisor ;;
        evidence) printf '%s\n' evidence-scan ;;
        *) return 1 ;;
    esac
}

runtime_phase_timing_start() {
    local name="$1" canonical occurrence source_sha="${gate_results_revision:-}"
    [[ -n "$runtime_phase_timing_script" && -n "$runtime_phase_timing_path" ]] || return 0
    [[ "$source_sha" =~ ^[0-9a-f]{40}$ ]] || return 0
    canonical="$(runtime_phase_name "$name")" || return 0
    occurrence=$(( ${runtime_phase_timing_occurrence[$canonical]:-0} + 1 ))
    runtime_phase_timing_occurrence[$canonical]="$occurrence"
    runtime_phase_timing_cache_state=''
    runtime_phase_timing_cache_sha256=''
    runtime_phase_timing_cache_generation=''
    runtime_phase_timing_cache_bytes=''
    local cache_args=()
    if [[ "$canonical" == cache-download ]]; then
        runtime_phase_timing_cache_state=miss
        runtime_phase_timing_cache_sha256="$cache_sha256"
        runtime_phase_timing_cache_generation="${HEPH_GCP_CACHE_GENERATION:-}"
        [[ "$runtime_phase_timing_cache_generation" =~ ^[1-9][0-9]*$ ]] ||
            fail 'cache generation provenance is missing or invalid'
        cache_args=(--cache-state miss --cache-sha256 "$cache_sha256" \
            --cache-generation "$runtime_phase_timing_cache_generation")
    fi
    python3 -B "$runtime_phase_timing_script" start \
        --path "$runtime_phase_timing_path" --phase "$canonical" --trust supervisor \
    --clock-domain guest-runtime --run-id "${HEPH_GCP_RUN_ID:-manual}" \
    --attempt "${GITHUB_RUN_ATTEMPT:-1}" --occurrence "$occurrence" \
        --source-sha "$source_sha" --image-fingerprint "$runtime_phase_timing_image_fingerprint" "${cache_args[@]}"
    runtime_phase_timing_open="$canonical:$occurrence"
}

runtime_phase_timing_end() {
    local outcome="$1" name occurrence cache_args=()
    [[ -n "$runtime_phase_timing_open" ]] || return 0
    name="${runtime_phase_timing_open%:*}"
    occurrence="${runtime_phase_timing_open##*:}"
    if [[ "$runtime_phase_timing_cache_state" == miss ]]; then
        cache_args=(--cache-state miss --cache-sha256 "$runtime_phase_timing_cache_sha256" \
            --cache-generation "$runtime_phase_timing_cache_generation")
        [[ -n "$runtime_phase_timing_cache_bytes" ]] &&
            cache_args+=(--bytes "$runtime_phase_timing_cache_bytes")
    fi
    python3 -B "$runtime_phase_timing_script" end \
        --path "$runtime_phase_timing_path" --phase "$name" --trust supervisor \
    --clock-domain guest-runtime --run-id "${HEPH_GCP_RUN_ID:-manual}" \
    --attempt "${GITHUB_RUN_ATTEMPT:-1}" --occurrence "$occurrence" \
        --source-sha "${gate_results_revision:-}" --outcome "$outcome" \
        --image-fingerprint "$runtime_phase_timing_image_fingerprint" "${cache_args[@]}"
    runtime_phase_timing_open=''
}

phase_start() {
    phase="$1"
    printf 'HEPH_GCP_COOKING event=phase-start phase=%s\n' "$phase"
    runtime_phase_timing_start "$phase"
}
phase_pass() {
    printf 'HEPH_GCP_COOKING event=phase-pass phase=%s\n' "$phase"
    runtime_phase_timing_end passed
}
require_command() { command -v "$1" >/dev/null 2>&1 || fail "missing command: $1"; }

if [[ -z "$deadline_epoch" ]]; then
    deadline_epoch=$(( $(date +%s) + 2400 ))
fi
[[ "$deadline_epoch" =~ ^[0-9]+$ ]] || fail 'HEPH_GCP_COOKING_DEADLINE_EPOCH must be an epoch integer'
remaining_seconds() {
    local remaining=$((deadline_epoch - $(date +%s)))
    ((remaining > 0)) || fail 'the common startup deadline has elapsed'
    printf '%s\n' "$remaining"
}
run_with_deadline() {
    local remaining
    remaining="$(remaining_seconds)"
    timeout --kill-after=30s "${remaining}s" "$@"
}
workload_budget_seconds() {
    local remaining="$1"
    ((remaining > workload_cleanup_reserve_seconds)) || return 1
    printf '%s\n' "$((remaining - workload_cleanup_reserve_seconds))"
}

# Initialize the sidecar before the workload starts.  The checkout is already
# the exact immutable revision selected by startup, so it is the authoritative
# revision anchor for this helper.
install -d -m 0700 /var/log/hephaestus
# Startup checks the checkout out as forge, while this root-owned supervisor
# runs the Cooking helper.  Trust only this exact immutable checkout path for
# the revision lookup; a global safe.directory entry would be too broad.
gate_results_revision="$(git -c "safe.directory=$checkout_root" -C "$checkout_root" rev-parse HEAD)"
gate_results_script_file="${BASH_SOURCE[0]}"
gate_results_script_sha256="$(sha256sum "$gate_results_script_file" | awk '{print $1}')"
if ! python3 -B "$gate_results_helper" --path "$gate_results_path" \
    --script-file "$gate_results_script_file" init \
    --revision "$gate_results_revision" --script-sha256 "$gate_results_script_sha256" \
    --test-mode gcp-cooking; then
    fail 'cooking gate sidecar initialization failed'
fi
gate_results_initialized=true

phase_start host-tools
validate_sha256 cache_sha256 "$cache_sha256"
for command in awk bash curl date find git grep install ldconfig podman python3 readlink sha256sum systemd-run tar timeout; do
    require_command "$command"
done
# Keep this list aligned with the actual full Cooking scripts: repository image
# import/build proof, the browser harness, and the compressed private cache.
missing_packages=()
if [[ "$runner_image_verified" == false ]]; then
    for pair in 'skopeo:skopeo' 'nft:nftables' 'zstd:zstd'; do
        command_name="${pair%%:*}"
        package_name="${pair#*:}"
        command -v "$command_name" >/dev/null 2>&1 || missing_packages+=("$package_name")
    done
    if ((${#missing_packages[@]})); then
        run_with_deadline apt-get update -qq
        run_with_deadline env DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends "${missing_packages[@]}"
    fi
fi
for command in nft podman python3 skopeo systemd-run tar timeout zstd; do
    require_command "$command"
done
if [[ "$runner_image_verified" == true ]]; then
    require_command oras
elif ! command -v oras >/dev/null 2>&1; then
    oras_stage="$(mktemp -d "${work_root}/oras.XXXXXX")"
    oras_archive="${oras_stage}/oras.tar.gz"
    run_with_deadline curl --fail --location --silent --show-error --retry 3 \
        --output "$oras_archive" \
        "https://github.com/oras-project/oras/releases/download/v${oras_version}/oras_${oras_version}_linux_amd64.tar.gz"
    [[ "$(sha256sum "$oras_archive" | awk '{print $1}')" == "$oras_sha256" ]] ||
        fail 'ORAS release checksum mismatch'
    tar -xzf "$oras_archive" -C "$oras_stage" --no-same-owner --no-same-permissions oras
    [[ -x "$oras_stage/oras" ]] || fail 'ORAS release is missing its executable'
    install -m 0555 "$oras_stage/oras" /usr/local/bin/oras
    rm -rf -- "$oras_stage"
fi
require_command oras
oras_version_output="$(oras version)"
oras_actual_version="$(awk '$1 == "Version:" { print $2; exit }' <<<"$oras_version_output")"
[[ "$oras_actual_version" == "$oras_version" ]] ||
    fail 'installed ORAS version does not match the reviewed pin'
phase_pass

phase_start node
# GitHub's reviewed browser job uses setup-node 24. A fresh Ubuntu image has
# no such toolchain, so install the exact official x64 tarball only when the
# existing node is absent or older than the Playwright package requires.
node_major=0
if command -v node >/dev/null 2>&1; then
    node_major="$(node --version | sed -E 's/^v([0-9]+).*/\1/' || true)"
fi
if [[ "$runner_image_verified" == true ]]; then
    [[ "$runner_image_node_version" == "$node_version" ]] ||
        fail 'verified runner image Node pin does not match the Cooking helper'
    [[ "$node_major" =~ ^[0-9]+$ && "$node_major" -ge 20 ]] ||
        fail 'verified runner image does not contain a usable Node runtime'
    [[ "$(node --version)" == "$runner_image_node_version" ]] ||
        fail 'verified runner image Node executable version does not match its manifest'
    node_bin="$(readlink -f "$(command -v node)")"
    node_path="$(dirname "$node_bin")"
elif [[ "$node_major" =~ ^[0-9]+$ && "$node_major" -ge 20 ]] &&
    command -v npm >/dev/null 2>&1 && command -v npx >/dev/null 2>&1; then
    node_bin="$(readlink -f "$(command -v node)")"
    node_path="$(dirname "$node_bin")"
else
    node_archive="${work_root}/node-${node_version}-linux-x64.tar.xz"
    node_stage="${work_root}/node-stage-${node_version}"
    node_archive_name="node-${node_version}-linux-x64.tar.xz"
    install -d -m 0700 "$node_stage"
    node_shasums="${node_stage}/SHASUMS256.txt"
    run_with_deadline curl --fail --location --silent --show-error --retry 3 \
        --output "$node_shasums" \
        "https://nodejs.org/dist/${node_version}/SHASUMS256.txt"
    official_node_sha256="$(awk -v name="$node_archive_name" '$2 == name { print $1; found=1 } END { if (!found) exit 1 }' "$node_shasums")" ||
        fail 'pinned Node archive is absent from the official checksum index'
    [[ "$official_node_sha256" == "$node_sha256" ]] ||
        fail 'pinned Node checksum disagrees with the official checksum index'
    if [[ ! -f "$node_archive" ]] || [[ "$(sha256sum "$node_archive" | awk '{print $1}')" != "$node_sha256" ]]; then
        run_with_deadline curl --fail --location --silent --show-error --retry 3 \
            --output "$node_archive" \
            "https://nodejs.org/dist/${node_version}/${node_archive_name}"
    fi
    [[ "$(sha256sum "$node_archive" | awk '{print $1}')" == "$node_sha256" ]] ||
        fail 'Node tarball checksum mismatch'
    rm -f -- "$node_shasums"
    tar -xJf "$node_archive" -C "$node_stage" --strip-components=1
    [[ -x "$node_stage/bin/node" && -x "$node_stage/bin/npm" && -x "$node_stage/bin/npx" ]] ||
        fail 'Node tarball is missing node/npm/npx'
    node_bin="$node_stage/bin/node"
    node_path="$node_stage/bin"
    chmod -R a+rX "$node_stage"
fi
workload_path="/home/forge/.cargo/bin:${node_path}:${trusted_path}"
node_major="$($node_bin --version | sed -E 's/^v([0-9]+).*/\1/')"
[[ "$node_major" -ge 20 ]] || fail 'Node 20 or newer is required by the Playwright lockfile'
runuser -u forge -- env PATH="$workload_path" HOME=/home/forge "$node_bin" --version
runuser -u forge -- env PATH="$workload_path" HOME=/home/forge npm --version
phase_pass

phase_start cache-download
install -d -m 0700 -o forge -g forge "$work_root" "$evidence_root" "$browser_root"
cache_parent="${work_root}/cooking-cache-staging"
install -d -m 0700 "$cache_parent"
stage_root="$(mktemp -d "${cache_parent}/bundle.XXXXXX")"
archive_path="${stage_root}/cache.tar.zst"
token_json="$(mktemp "${cache_parent}/token.XXXXXX.json")"
token_header="$(mktemp "${cache_parent}/header.XXXXXX")"
chmod 600 "$token_json" "$token_header"
curl --fail --silent --show-error -H 'Metadata-Flavor: Google' \
    "${metadata_root}/instance/service-accounts/default/token" >"$token_json"
python3 - "$token_json" "$token_header" <<'PY'
import json
import os
import pathlib
import sys
payload = json.loads(pathlib.Path(sys.argv[1]).read_text())
token = payload.get("access_token")
if not isinstance(token, str) or not token or any(c in token for c in "\r\n\x00\"\\"):
    raise SystemExit("metadata token response is invalid")
path = pathlib.Path(sys.argv[2])
path.write_text(f'header = "Authorization: Bearer {token}"\n')
os.chmod(path, 0o600)
PY
encoded_object="$(python3 -c 'import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1],safe=""))' "$gcs_object")"
run_with_deadline curl --fail --location --silent --show-error --retry 3 --retry-all-errors \
    --config "$token_header" \
    --output "$archive_path" \
    "https://storage.googleapis.com/download/storage/v1/b/${gcs_bucket}/o/${encoded_object}?alt=media"
rm -f -- "$token_json" "$token_header"
token_json=''
token_header=''
actual_cache_sha256="$(sha256sum "$archive_path" | awk '{print $1}')"
if [[ "$actual_cache_sha256" != "$cache_sha256" ]]; then
    printf 'gcp-cooking-run: private Cooking cache checksum mismatch expected=%s actual=%s\n' \
        "$cache_sha256" "$actual_cache_sha256" >&2
    fail 'private Cooking cache checksum mismatch'
fi
runtime_phase_timing_cache_bytes="$(stat -c '%s' -- "$archive_path")"
phase_pass

phase_start cache-extract
python3 - "$archive_path" <<'PY'
import pathlib, subprocess, sys, tarfile
archive = pathlib.Path(sys.argv[1])
process = subprocess.Popen(['zstd', '-dc', str(archive)], stdout=subprocess.PIPE)
try:
    with tarfile.open(fileobj=process.stdout, mode='r|') as outer:
        count = 0
        total = 0
        for member in outer:
            name = pathlib.PurePosixPath(member.name)
            if name.is_absolute() or '..' in name.parts or member.name.startswith('./../'):
                raise SystemExit(f'archive path escapes extraction root: {member.name}')
            if member.issym() or member.islnk() or not (member.isfile() or member.isdir()):
                raise SystemExit(f'archive contains unsupported member: {member.name}')
            count += 1
            total += member.size
            if count > 100000 or total > 8 * 1024 * 1024 * 1024:
                raise SystemExit('archive extraction budget exceeded')
finally:
    assert process.stdout is not None
    process.stdout.close()
    if process.wait() != 0:
        raise SystemExit('zstd failed while reading the cache archive')
PY
# Extract the already checksum-verified archive without trusting its ownership.
run_with_deadline bash -Eeuo pipefail -c \
    'zstd -dc "$1" | tar -xf - -C "$2" --no-same-owner --no-same-permissions --no-overwrite-dir' \
    -- "$archive_path" "$stage_root"
rm -f -- "$archive_path"
[[ -f "$stage_root/sha256sums" && -f "$stage_root/cache-manifest.json" ]] || fail 'cache manifest or checksum inventory is missing'
(cd "$stage_root" && sha256sum --strict --check sha256sums >/dev/null)
python3 - "$stage_root" <<'PY'
import hashlib, json, pathlib, re, sys
root=pathlib.Path(sys.argv[1])
m=json.loads((root/'cache-manifest.json').read_text())
required={'python-ubuntu','rust-ubuntu','typescript-node-ubuntu','ubuntu-native','oci-builder-ubuntu','oci-verifier-ubuntu'}
if m.get('platform_revision') != '581b939d5ad5e5a81e77ad01ad8931487a8d2bcf': raise SystemExit('unexpected platform revision')
if set(m.get('required_layouts',[])) != required: raise SystemExit('required layout set is incomplete')
refs=m.get('runtime_image_references',{})
for key in ('HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE','HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE'):
    if not re.fullmatch(r'[A-Za-z0-9._:/-]+@sha256:[0-9a-f]{64}', refs.get(key,'')): raise SystemExit(f'{key} is not immutable')
for key, layout_name in {'HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE': 'python-ubuntu'}.items():
    expected=refs[key].split('@',1)[1]
    index=json.loads((root/'layouts'/layout_name/'image'/'index.json').read_text())
    if expected not in {entry.get('digest') for entry in index.get('manifests', [])}:
        raise SystemExit(f'{key} does not match its cached OCI layout')
base=json.loads((root/'base-layouts.json').read_text())
if len(base) != 4: raise SystemExit('execution base layout manifest must contain four entries')
for ref, value in base.items():
    p=pathlib.PurePosixPath(value)
    if p.is_absolute() or '..' in p.parts or not (root/value/'index.json').is_file() or not (root/value/'oci-layout').is_file():
        raise SystemExit(f'invalid base layout path: {value}')
release_inputs=m.get('release_inputs',{})
workflow={line.split('=',1)[0]: line.split('=',1)[1] for line in
          (root/'workflow.env.template').read_text().splitlines() if '=' in line}
for name, workflow_key in (('oci-builder-ubuntu','builder_vm_image'), ('oci-verifier-ubuntu','verifier_vm_image')):
    rel=release_inputs.get(name,'')
    p=pathlib.PurePosixPath(rel)
    if p.is_absolute() or '..' in p.parts or not (root/rel).is_file():
        raise SystemExit(f'release input is unavailable: {name}')
    release=json.loads((root/rel).read_text())
    if release.get('revision') != m['platform_revision']:
        raise SystemExit(f'release input revision mismatch: {name}')
    workflow_ref=workflow.get(workflow_key,'')
    if release.get('manifest_digest') != workflow_ref.split('@',1)[-1]:
        raise SystemExit(f'release input digest mismatch: {name}')
guest=m.get('guest_images',{}).get('rust-ubuntu-profile',{})
archive=root/guest.get('archive','')
if not archive.is_file() or hashlib.sha256(archive.read_bytes()).hexdigest()!=guest.get('archive_sha256'):
    raise SystemExit('profile Rust archive inventory mismatch')
if guest.get('manifest_digest') != refs['HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE'].split('@',1)[1]:
    raise SystemExit('profile Rust archive manifest does not match runtime reference')
print('cache manifest, paths, inventory, and accepted image references: PASS')
PY
# The extracted bundle is private but must be readable by the forge process.
chown -R forge:forge "$stage_root"
find "$stage_root" -type d -exec chmod 700 {} +
find "$stage_root" -type f -exec chmod 600 {} +
if [[ -e "$cache_root" ]]; then
    [[ -d "$cache_root" && ! -L "$cache_root" ]] || fail 'existing Cooking cache path is unsafe'
    rm -rf -- "$cache_root"
fi
mv -- "$stage_root" "$cache_root"
stage_root=''
phase_pass

if [[ "$workload_trust" == untrusted-pr ]]; then
    # Configure the private HOME/runtime before trusted image import so the
    # rootless Podman store is visible to the later PR workload.
    configure_pr_sandbox
fi
phase_start workflow-images
# Convert the relocatable bundle workflow to the absolute paths expected by
# preflight.sh and the Rust provisioning helper, without changing references.
install -d -m 0700 -o forge -g forge "$cache_root/repository-images"
python3 - "$cache_root" <<'PY'
import json, pathlib, sys
root=pathlib.Path(sys.argv[1]); template=(root/'workflow.env.template').read_text().splitlines()
out=[]
for line in template:
    if line.startswith('builder_layout='): line=f'builder_layout={root}/layouts/oci-builder-ubuntu/image'
    elif line.startswith('verifier_layout='): line=f'verifier_layout={root}/layouts/oci-verifier-ubuntu/image'
    elif line.startswith('base_layout_manifest='): line=f'base_layout_manifest={root}/repository-images/base-layouts.json'
    out.append(line)
base=json.loads((root/'base-layouts.json').read_text())
(root/'repository-images/base-layouts.json').write_text(json.dumps({k:str(root/v) for k,v in base.items()},sort_keys=True,indent=2)+'\n')
path=root/'repository-images/workflow.env'; path.write_text('\n'.join(out)+'\n'); path.chmod(0o600)
path.with_name('base-layouts.json').chmod(0o600)
PY
chown forge:forge "$cache_root/repository-images/workflow.env" "$cache_root/repository-images/base-layouts.json"
bundle_manifest="$cache_root/cache-manifest.json"
python3 - "$bundle_manifest" "$cache_root/repository-images/workflow.env" <<'PY'
import json, pathlib, re, sys
m=json.loads(pathlib.Path(sys.argv[1]).read_text()); values={}
for line in pathlib.Path(sys.argv[2]).read_text().splitlines():
    if '=' in line: values[line.split('=',1)[0]]=line.split('=',1)[1]
for key in ('builder_vm_image','verifier_vm_image','builder_layout','verifier_layout','base_layout_manifest'):
    if not values.get(key): raise SystemExit(f'workflow missing {key}')
for key in ('builder_layout','verifier_layout','base_layout_manifest'):
    if not pathlib.Path(values[key]).is_absolute() or not pathlib.Path(values[key]).exists(): raise SystemExit(f'workflow path unavailable: {key}')
print('absolute workflow materialization: PASS')
PY
runtime_python_ref="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["runtime_image_references"]["HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE"])' "$bundle_manifest")"
runtime_rust_ref="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["runtime_image_references"]["HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE"])' "$bundle_manifest")"
run_with_deadline systemd-run --unit="heph-gcp-cooking-images-${HEPH_GCP_RUN_ID:-manual}" \
    --expand-environment=no --service-type=oneshot --wait --pipe --collect \
    --property=Delegate=yes --property=RuntimeMaxSec="$(remaining_seconds)s" \
    --property=TasksMax=infinity --property=LimitNOFILE=65536 \
    --property=CPUAccounting=yes --property=MemoryAccounting=yes \
    --property=TasksAccounting=yes --property=IOAccounting=yes \
    --uid="$forge_uid" --gid="$forge_gid" \
    --working-directory="$checkout_root" --setenv=HOME="$workload_home" \
    --setenv=XDG_DATA_HOME="$workload_home/.local/share" \
    --setenv=XDG_RUNTIME_DIR=/run/user/10001 --setenv=PATH="$workload_path" \
    "${workflow_image_sandbox_args[@]}" \
    /bin/bash -Eeuo pipefail -c '
        mkdir -p -m 700 /tmp/hephaestus-libkrun
        candidate="/sys/fs/cgroup$(awk -F: '\''$1 == "0" { print $3 }'\'' /proc/self/cgroup)"
        test -d "$candidate" -a -w "$candidate" -a -w "$candidate/cgroup.subtree_control"
        [[ "$(<"$candidate/cgroup.type")" == domain ]]
        manager="$candidate/heph-cooking-images-manager"
        mkdir "$manager"
        # Move the shell into a child before enabling domain controllers so the
        # delegated parent obeys the cgroup-v2 no-internal-process rule.
        printf "%s\n" "$BASHPID" >"$manager/cgroup.procs"
        available="$(<"$candidate/cgroup.controllers")"
        for controller in cpu io memory pids; do [[ " $available " == *" $controller "* ]]; done
        printf "+cpu +io +memory +pids\n" >"$candidate/cgroup.subtree_control"
        enabled="$(<"$candidate/cgroup.subtree_control")"
        for controller in cpu io memory pids; do [[ " $enabled " == *" $controller "* ]]; done
        printf "HEPH_GCP_COOKING event=workflow-images-delegation status=pass parent=%s\n" "$candidate"
        cache="$1"; py="$2"; rust="$3"
        import_one() {
            local label="$1" source="$2" destination="$3" status digest
            printf "HEPH_GCP_COOKING event=image-step phase=workflow-images command=podman-image-exists image=%s status=start\n" "$label"
            set +e
            podman image exists "$destination"
            status=$?
            set -e
            if ((status == 0)); then
                printf "HEPH_GCP_COOKING event=image-step phase=workflow-images command=podman-image-exists image=%s status=pass result=present\n" "$label"
            elif ((status == 1)); then
                printf "HEPH_GCP_COOKING event=image-step phase=workflow-images command=podman-image-exists image=%s status=pass result=absent\n" "$label"
                printf "HEPH_GCP_COOKING event=image-step phase=workflow-images command=skopeo-copy image=%s status=start\n" "$label"
                # containers-storage has no multi-image group destination; copy
                # the accepted manifest and preserve its digest exactly.
                set +e
                podman unshare skopeo copy --preserve-digests "$source" "containers-storage:$destination" >/dev/null
                status=$?
                set -e
                if ((status != 0)); then
                    printf "HEPH_GCP_COOKING event=image-step phase=workflow-images command=skopeo-copy image=%s status=fail exit=%s\n" "$label" "$status"
                    return "$status"
                fi
                printf "HEPH_GCP_COOKING event=image-step phase=workflow-images command=skopeo-copy image=%s status=pass\n" "$label"
            else
                printf "HEPH_GCP_COOKING event=image-step phase=workflow-images command=podman-image-exists image=%s status=fail exit=%s\n" "$label" "$status"
                return "$status"
            fi
            printf "HEPH_GCP_COOKING event=image-step phase=workflow-images command=skopeo-inspect image=%s status=start\n" "$label"
            set +e
            digest="$(podman unshare skopeo inspect --format "{{.Digest}}" "containers-storage:$destination")"
            status=$?
            set -e
            if ((status != 0)); then
                printf "HEPH_GCP_COOKING event=image-step phase=workflow-images command=skopeo-inspect image=%s status=fail exit=%s\n" "$label" "$status"
                return "$status"
            fi
            if [[ "$digest" != "${destination##*@}" ]]; then
                printf "HEPH_GCP_COOKING event=image-step phase=workflow-images command=skopeo-inspect image=%s status=fail reason=digest-mismatch\n" "$label"
                return 1
            fi
            printf "HEPH_GCP_COOKING event=image-step phase=workflow-images command=skopeo-inspect image=%s status=pass digest=%s\n" "$label" "$digest"
        }
        import_one python-ubuntu "oci:$cache/layouts/python-ubuntu/image" "$py"
        import_one rust-ubuntu "oci-archive:$cache/guest-images/rust-ubuntu-profile.oci" "$rust"
        import_one oci-builder-ubuntu "oci:$cache/layouts/oci-builder-ubuntu/image" "$(awk -F= '\''$1=="builder_vm_image" {print $2}'\'' "$cache/repository-images/workflow.env")"
        import_one oci-verifier-ubuntu "oci:$cache/layouts/oci-verifier-ubuntu/image" "$(awk -F= '\''$1=="verifier_vm_image" {print $2}'\'' "$cache/repository-images/workflow.env")"
    ' -- "$cache_root" "$runtime_python_ref" "$runtime_rust_ref"
phase_pass

# The cache is a trusted immutable input for PR runs.  Freeze it before any
# PR-owned setup starts; the derived workflow paths above are the last trusted
# writes.  Keep the existing manual path's filesystem behavior.
if [[ "$workload_trust" == untrusted-pr ]]; then
    find "$cache_root" -type d -exec chmod a-w {} +
    find "$cache_root" -type f -exec chmod a-w {} +
fi

if [[ "$workload_trust" == untrusted-pr ]]; then
    # npm lifecycle hooks and browser setup consume PR-controlled files.  The
    # forge metadata guard must therefore be active before this setup starts.
    phase_start metadata-guard
    require_command nft
    if nft list table inet "$metadata_guard_table" >/dev/null 2>&1; then
        fail "metadata guard table already exists: $metadata_guard_table"
    fi
    nft -f - <<EOF
 table inet $metadata_guard_table {
     chain output {
         type filter hook output priority -150; policy accept;
         meta skuid $forge_uid ip daddr $metadata_ip tcp dport 80 reject with tcp reset
         meta skuid $forge_uid ip daddr $metadata_ip tcp dport 443 reject with tcp reset
         meta skuid $forge_uid ip6 daddr $metadata_ipv6 tcp dport 80 reject with tcp reset
         meta skuid $forge_uid ip6 daddr $metadata_ipv6 tcp dport 443 reject with tcp reset
     }
 }
EOF
    nft list table inet "$metadata_guard_table" | grep -q "$metadata_ip" || fail 'IPv4 metadata guard rule was not installed'
    nft list table inet "$metadata_guard_table" | grep -q "$metadata_ipv6" || fail 'IPv6 metadata guard rule was not installed'
    if curl --noproxy '*' --connect-timeout 1 --max-time 2 -H 'Metadata-Flavor: Google' \
        --fail --silent "http://${metadata_ip}/computeMetadata/v1/instance/id" >/dev/null 2>&1; then
        :
    else
        fail 'root cannot reach the GCE metadata endpoint before guard installation'
    fi
    if runuser -u forge -- env HOME=/home/forge curl --noproxy '*' --connect-timeout 1 --max-time 2 \
        -H 'Metadata-Flavor: Google' --fail --silent "http://${metadata_ip}/computeMetadata/v1/instance/id" >/dev/null 2>&1; then
        fail 'forge can still reach the GCE metadata HTTP endpoint'
    fi
    if runuser -u forge -- env HOME=/home/forge curl --noproxy '*' --connect-timeout 1 --max-time 2 \
        -k --fail --silent "https://[${metadata_ipv6}]/computeMetadata/v1/instance/id" >/dev/null 2>&1; then
        fail 'forge can still reach the GCE metadata HTTPS endpoint'
    fi
    phase_pass
    # The stock-image dependency installer invokes the checked-out Playwright
    # package as root.  PR mode requires the reviewed runner image so that this
    # root-only setup is already complete and no PR-controlled code crosses the
    # trust boundary.
    [[ "$runner_image_verified" == true ]] ||
        fail 'PR workload requires a verified runner image with browser dependencies preinstalled'
fi

phase_start browser-host
# Match the reviewed CI host setup: install browser OS dependencies as root,
# then install the browser itself into a forge-owned shared cache.
playwright_npm_env=()
pr_playwright_npm_args=()
if [[ "$runner_image_verified" == true ]]; then
    # The verified image already contains the exact browser for this lock;
    # npm lifecycle hooks must not silently download another revision.
    playwright_npm_env+=(PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1)
    pr_playwright_npm_args+=(--setenv=PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1)
fi
if [[ "$workload_trust" == untrusted-pr ]]; then
    run_with_deadline systemd-run \
        --unit="heph-gcp-pr-browser-setup-${HEPH_GCP_RUN_ID:-manual}" \
        --service-type=oneshot --wait --pipe --collect --expand-environment=no \
        --property=KillMode=control-group \
        --uid="$forge_uid" --gid="$forge_gid" \
        --working-directory="$checkout_root" \
        --setenv=HOME="$workload_home" --setenv=XDG_RUNTIME_DIR=/run/user/10001 \
        --setenv=PATH="$workload_path" --setenv=PLAYWRIGHT_BROWSERS_PATH="$browser_root" \
        --setenv=npm_config_cache="$workload_npm_cache" \
        "${pr_sandbox_args[@]}" "${pr_playwright_npm_args[@]}" \
        /bin/bash -Eeuo pipefail -c 'cd "$1/e2e/playwright" && npm ci' -- "$checkout_root"
else
    run_with_deadline runuser -u forge -- env HOME=/home/forge XDG_RUNTIME_DIR=/run/user/10001 \
        PATH="$workload_path" PLAYWRIGHT_BROWSERS_PATH="$browser_root" \
        "${playwright_npm_env[@]}" \
        bash -Eeuo pipefail -c 'cd "$1/e2e/playwright" && npm ci' -- "$checkout_root"
fi
browser_lock_sha="$(sha256sum "$checkout_root/e2e/playwright/package-lock.json" | awk '{print $1}')"
if [[ "$runner_image_verified" == true ]]; then
    [[ "$runner_image_browser_lock_sha" =~ ^[0-9a-f]{64}$ ]] ||
        fail 'verified runner image browser lock anchor is missing or invalid'
    [[ -n "$runner_image_browser_version" ]] ||
        fail 'verified runner image browser version anchor is missing'
    [[ "$browser_lock_sha" == "$runner_image_browser_lock_sha" ]] ||
        fail 'checked-out browser lock does not match the baked browser assets'
    browser_executable="$(find "$browser_root" -type f \( -name chrome-headless-shell -o -name chrome \) -perm -0100 -print -quit 2>/dev/null)"
    [[ -n "$browser_executable" ]] || fail 'verified runner image Chromium executable is missing'
    browser_version="$($browser_executable --version 2>/dev/null || true)"
    [[ -n "$browser_version" && "$browser_version" == "$runner_image_browser_version" ]] ||
        fail 'verified runner image Chromium executable version does not match its manifest'
    printf 'HEPH_GCP_COOKING baked-browser status=pass lock_sha256=%s version=%s\n' \
        "$browser_lock_sha" "$browser_version"
else
    run_with_deadline env PATH="$workload_path" bash -Eeuo pipefail -c \
        'cd "$1/e2e/playwright" && npx playwright install-deps chromium' -- "$checkout_root"
    chown -R forge:forge "$browser_root"
    run_with_deadline runuser -u forge -- env HOME=/home/forge XDG_RUNTIME_DIR=/run/user/10001 \
        PATH="$workload_path" PLAYWRIGHT_BROWSERS_PATH="$browser_root" \
        bash -Eeuo pipefail -c 'cd "$1/e2e/playwright" && npx playwright install chromium' -- "$checkout_root"
fi
phase_pass

phase_start metadata-guard
require_command nft
if nft list table inet "$metadata_guard_table" >/dev/null 2>&1; then
    [[ "$workload_trust" == untrusted-pr ]] || fail "metadata guard table already exists: $metadata_guard_table"
else
nft -f - <<EOF
 table inet $metadata_guard_table {
     chain output {
         type filter hook output priority -150; policy accept;
         meta skuid $forge_uid ip daddr $metadata_ip tcp dport 80 reject with tcp reset
         meta skuid $forge_uid ip daddr $metadata_ip tcp dport 443 reject with tcp reset
         meta skuid $forge_uid ip6 daddr $metadata_ipv6 tcp dport 80 reject with tcp reset
         meta skuid $forge_uid ip6 daddr $metadata_ipv6 tcp dport 443 reject with tcp reset
     }
 }
EOF
fi
nft list table inet "$metadata_guard_table" | grep -q "$metadata_ip" || fail 'IPv4 metadata guard rule was not installed'
nft list table inet "$metadata_guard_table" | grep -q "$metadata_ipv6" || fail 'IPv6 metadata guard rule was not installed'
if curl --noproxy '*' --connect-timeout 1 --max-time 2 -H 'Metadata-Flavor: Google' \
    --fail --silent "http://${metadata_ip}/computeMetadata/v1/instance/id" >/dev/null 2>&1; then
    :
else
    fail 'root cannot reach the GCE metadata endpoint before guard installation'
fi
if runuser -u forge -- env HOME=/home/forge curl --noproxy '*' --connect-timeout 1 --max-time 2 \
    -H 'Metadata-Flavor: Google' --fail --silent "http://${metadata_ip}/computeMetadata/v1/instance/id" >/dev/null 2>&1; then
    fail 'forge can still reach the GCE metadata HTTP endpoint'
fi
if runuser -u forge -- env HOME=/home/forge curl --noproxy '*' --connect-timeout 1 --max-time 2 \
    -k --fail --silent "https://[${metadata_ipv6}]/computeMetadata/v1/instance/id" >/dev/null 2>&1; then
    fail 'forge can still reach the GCE metadata HTTPS endpoint'
fi
phase_pass

phase_start cooking
cooking_remaining="$(remaining_seconds)"
cooking_timeout=''
workload_started=false
if cooking_timeout="$(workload_budget_seconds "$cooking_remaining")"; then
    workload_started=true
else
    # Preserve time for the bounded stop/evidence path when startup hands us
    # too little of the common trial deadline to run Cooking safely.
    printf 'HEPH_GCP_COOKING event=workload-budget operation=cooking-workload phase=cooking status=failed exit_code=124 duration_ms=0 stage=deadline reason_class=insufficient-budget remaining_seconds=%s reserve_seconds=%s\n' \
        "$cooking_remaining" "$workload_cleanup_reserve_seconds"
    status=124
fi
cooking_unit="heph-gcp-cooking-${HEPH_GCP_RUN_ID:-manual}"
install -d -m 0700 -o forge -g forge "$evidence_root"
gate_update begin workload || true
run_cooking_workload() {
if [[ "$workload_started" != true ]]; then
    return 124
fi
local phase_timing_path="$evidence_root/phase-timing-workload.jsonl"
local workload_trust_value="${workload_trust:-trusted}"
local workload_home_value="${workload_home:-/home/forge}"
local workload_cargo_home_value="${workload_cargo_home:-/home/forge/.cargo}"
local workload_rustup_home_value="${workload_rustup_home:-/home/forge/.rustup}"
if [[ "$workload_trust_value" == untrusted-pr ]]; then
    # PR code is a forge workload.  Keep the KVM/passthrough devices and
    # network available, while hiding controller state, credentials, and the
    # metadata path.  The metadata nftables guard below also covers forge's
    # passt process and nested guest traffic.
    ((${#pr_sandbox_args[@]} > 0)) || {
        printf 'gcp-cooking-run: PR sandbox was not configured\n' >&2
        return 1
    }
    install -d -m 0700 -o forge -g forge "$(dirname -- "$phase_timing_path")"
fi
timeout --kill-after=30s "${cooking_remaining}s" systemd-run \
    --unit="$cooking_unit" --service-type=oneshot --wait --pipe --collect \
    --expand-environment=no --property=Delegate=yes --property=RuntimeMaxSec="${cooking_remaining}s" \
    --property=TimeoutStartSec="${cooking_remaining}s" --property=TimeoutStopSec=15s --property=TasksMax=infinity \
    "${pr_sandbox_args[@]}" \
    --property=LimitNOFILE=65536 --uid="$forge_uid" --gid="$forge_gid" \
    --working-directory="$checkout_root" --setenv=HOME="$workload_home_value" \
    --setenv=XDG_DATA_HOME="$workload_home_value/.local/share" \
    --setenv=XDG_RUNTIME_DIR=/run/user/10001 --setenv=RUSTUP_HOME="$workload_rustup_home_value" \
    --setenv=CARGO_HOME="$workload_cargo_home_value" --setenv=TMPDIR=/tmp/hephaestus-libkrun \
    --setenv=HEPHAESTUS_LIBKRUN_TMP_ROOT=/tmp/hephaestus-libkrun \
    --setenv=HEPHAESTUS_COOKING_SOURCE_ROOT="$checkout_root/examples/cooking" \
    --setenv=HEPHAESTUS_LOCAL_ROOT="$cache_root" \
    --setenv=HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE="$runtime_python_ref" \
    --setenv=HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE="$runtime_rust_ref" \
    --setenv=HEPHAESTUS_COOKING_TIMEOUT_SECONDS="$cooking_timeout" \
    --setenv=HEPHAESTUS_APP_COOKING_E2E=1 \
    --setenv=HEPHAESTUS_APP_COOKING_BUILD_PROOF=1 \
    --setenv=HEPHAESTUS_COOKING_UPDATE_E2E=1 \
    --setenv=HEPHAESTUS_COOKING_OCI_BASE_IMPORT_DIAGNOSTIC=0 \
    --setenv=HEPHAESTUS_APP_UPDATE_ADMISSION_E2E=0 \
    --setenv=HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E=0 \
    --setenv=HEPHAESTUS_COOKING_DIAGNOSTICS_DIR="$evidence_root" \
    --setenv=HEPHAESTUS_COOKING_BROWSER_E2E=1 \
    --setenv=PLAYWRIGHT_BROWSERS_PATH="$browser_root" \
    --setenv=HEPH_GCP_PHASE_TIMING_PATH="$phase_timing_path" \
    --setenv=HEPH_GCP_PHASE_TIMING_SOURCE_SHA="${gate_results_revision:-unknown}" \
    --setenv=HEPH_GCP_PHASE_TIMING_RUN_ID="${HEPH_GCP_RUN_ID:-manual}" \
    --setenv=HEPH_GCP_PHASE_TIMING_ATTEMPT="${GITHUB_RUN_ATTEMPT:-1}" \
    --setenv=HEPH_GCP_PHASE_TIMING_IMAGE_FINGERPRINT="${runtime_phase_timing_image_fingerprint:-}" \
    --setenv=PATH="$workload_path" \
    /bin/bash -Eeuo pipefail -c '
        mkdir -p -m 700 /tmp/hephaestus-libkrun
        candidate="/sys/fs/cgroup$(awk -F: '\''$1 == "0" { print $3 }'\'' /proc/self/cgroup)"
        test -d "$candidate" -a -w "$candidate" -a -w "$candidate/cgroup.subtree_control"
        [[ "$(<"$candidate/cgroup.type")" == domain ]]
        manager="$candidate/heph-cooking-manager"
        mkdir "$manager"
        printf "%s\n" "$BASHPID" >"$manager/cgroup.procs"
        available="$(<"$candidate/cgroup.controllers")"
        for controller in cpu io memory pids; do [[ " $available " == *" $controller "* ]]; done
        printf "+cpu +io +memory +pids\n" >"$candidate/cgroup.subtree_control"
        enabled="$(<"$candidate/cgroup.subtree_control")"
        for controller in cpu io memory pids; do [[ " $enabled " == *" $controller "* ]]; done
        exec "$1/examples/cooking/run.sh"
    ' -- "$checkout_root"
}
set +e
run_cooking_workload
status=$?
set -e
workload_result='passed'
((status == 0)) || workload_result='failed'
printf 'HEPH_GCP_COOKING event=workload-result operation=cooking-workload phase=cooking status=%s exit_code=%s\n' \
    "$workload_result" "$status"
workload_gate_state='failed'
workload_reason_class='workload-failed'
if ((status == 0)); then
    workload_gate_state='passed'
    workload_reason_class='none'
elif ((status == 124)); then
    workload_gate_state='timed-out'
    workload_reason_class='timeout'
fi
gate_update complete workload --state "$workload_gate_state" --exit-code "$status" \
    --reason-class "$workload_reason_class" || true
if ((status != 0)) && [[ "$workload_started" == true ]]; then
    # The outer deadline can kill systemd-run while the delegated oneshot is
    # still activating.  Stop that unit from this supervisor's cgroup before
    # collecting evidence; otherwise the workload can consume the collection
    # reserve and keep fixture-bearing processes alive.
    set +e
    run_with_deadline timeout --kill-after=2s 15s systemctl stop "$cooking_unit" --no-pager >/dev/null 2>&1
    stop_status=$?
    set -e
    active_state="$(timeout --kill-after=1s 5s systemctl show "$cooking_unit" --no-pager --property=ActiveState --value 2>/dev/null || true)"
    if [[ "$active_state" == active || "$active_state" == activating || "$active_state" == deactivating ]]; then
        printf 'Cooking systemd unit remained %s after stop (stop_exit=%s); issuing bounded kill\n' \
            "$active_state" "$stop_status" >&2
        set +e
        run_with_deadline timeout --kill-after=2s 15s systemctl kill "$cooking_unit" --kill-who=all --signal=KILL >/dev/null 2>&1
        run_with_deadline timeout --kill-after=2s 15s systemctl stop "$cooking_unit" --no-pager >/dev/null 2>&1
        set -e
        active_state="$(timeout --kill-after=1s 5s systemctl show "$cooking_unit" --no-pager --property=ActiveState --value 2>/dev/null || true)"
    fi
    if [[ "$active_state" == active || "$active_state" == activating || "$active_state" == deactivating ]]; then
        printf 'Cooking systemd unit did not stop before evidence collection: state=%s\n' "$active_state" >&2
    fi
    printf 'Cooking systemd unit failed with status=%s unit=%s\n' "$status" "$cooking_unit" >&2
    # `systemctl status` includes the process tree and command arguments.  The
    # cooking environment can contain fixture credentials, so retain only the
    # allowlisted unit state fields needed to classify the failure.
    timeout --kill-after=1s 5s systemctl show "$cooking_unit" --no-pager \
        --property=ActiveState,SubState,Result,ExecMainCode,ExecMainStatus,MainPID \
        2>&1 || true
fi
phase_start evidence
gate_update begin evidence-scan || true
# Keep the evidence-root spelling visible for older local contract checks; the
# scanner report is deliberately relocated immediately to the root-owned path
# copied by startup, so it cannot be confused with workload evidence.
scan_status_report="$evidence_root/evidence-scan-status.json"
scan_status_report='/var/log/hephaestus/evidence-scan-status.json'
rm -f -- "$scan_status_report"
set +e
  run_with_deadline python3 -B "${diagnostics_scanner_script:-${checkout_root:-}/scripts/check-browser-evidence.py}" \
    "$evidence_root" --status-output "$scan_status_report"
scan_status=$?
set -e
evidence_scan_result='passed'
((scan_status == 0)) || evidence_scan_result='failed'
scan_report='report_status=unavailable rule=status-report-unavailable file_class=metadata path_sha256=none checked_files=0 checked_bytes=0'
if [[ -s "$scan_status_report" ]]; then
if scan_report_fields="$(python3 -B - "$scan_status_report" <<'PY'
import json
import re
import sys
from pathlib import Path

value = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
rules = {
    "none", "browser-secret-org", "browser-secret-project", "golden-provider-sentinel",
    "cooking-inbound-sentinel", "cooking-model-sentinel", "cooking-relay-sentinel",
    "cooking-model-rotated-sentinel", "cooking-inbound-rotated-sentinel",
    "cooking-relay-rotated-sentinel", "archive-nesting-limit", "archive-size-limit",
    "archive-member-size-limit", "archive-invalid", "file-size-limit", "symlink",
    "evidence-root", "no-files", "read-error", "scan-error",
}
classes = {"none", "archive", "structured", "text", "binary", "directory", "filesystem", "content", "metadata", "unknown"}
if not isinstance(value, dict) or set(value) != {
    "schema", "status", "rule", "file_class", "path_sha256", "checked_files", "checked_bytes"
}:
    raise SystemExit("scanner status fields are invalid")
if value["schema"] != 1 or value["status"] not in {"passed", "failed"}:
    raise SystemExit("scanner status classification is invalid")
if value["rule"] not in rules or value["file_class"] not in classes:
    raise SystemExit("scanner status rule is invalid")
if (value["status"] == "passed") != (value["rule"] == "none"):
    raise SystemExit("scanner status result does not match its rule")
path_digest = value["path_sha256"]
if path_digest is not None and (not isinstance(path_digest, str) or re.fullmatch(r"[0-9a-f]{64}", path_digest) is None):
    raise SystemExit("scanner status path digest is invalid")
for field in ("checked_files", "checked_bytes"):
    if type(value[field]) is not int or value[field] < 0:
        raise SystemExit("scanner status count is invalid")
print("report_status=%s rule=%s file_class=%s path_sha256=%s checked_files=%s checked_bytes=%s" % (
    value["status"], value["rule"], value["file_class"], path_digest or "none",
    value["checked_files"], value["checked_bytes"],
))
PY
    )"; then
        scan_report="$scan_report_fields"
    else
        if ((scan_status == 0)); then
            scan_status=1
            evidence_scan_result='failed'
        fi
    fi
else
    if ((scan_status == 0)); then
        scan_status=1
        evidence_scan_result='failed'
    fi
fi
printf 'HEPH_GCP_COOKING event=evidence-scan operation=evidence-scan phase=evidence status=%s exit_code=%s %s\n' \
    "$evidence_scan_result" "$scan_status" "$scan_report"
scan_gate_state='failed'
scan_reason_class='evidence-scan-failed'
if ((scan_status == 0)); then
    scan_gate_state='passed'
    scan_reason_class='none'
elif ((scan_status == 124)); then
    scan_gate_state='timed-out'
    scan_reason_class='timeout'
elif [[ "$scan_report" == report_status=unavailable* ]]; then
    scan_reason_class='evidence-scan-report-invalid'
fi
gate_update complete evidence-scan --state "$scan_gate_state" --exit-code "$scan_status" \
    --reason-class "$scan_reason_class" || true
gate_update begin browser-validation || true
set +e
run_with_deadline python3 -B "${browser_summary_script:-${checkout_root:-}/scripts/project-playwright-browser-summary.py}" \
    "$evidence_root" "$evidence_root/browser-summary.json" --require-complete-journey
browser_summary_status=$?
set -e
browser_report_state='unknown'
if [[ -s "$evidence_root/browser-summary.json" ]]; then
    browser_report_state="$(python3 -B - "$evidence_root/browser-summary.json" <<'PY'
import json
import sys
try:
    value = json.load(open(sys.argv[1], encoding="utf-8"))
except (OSError, ValueError):
    print("unknown")
else:
    state = value.get("report_state") if isinstance(value, dict) else None
    print(state if isinstance(state, str) else "unknown")
PY
    )" || browser_report_state='unknown'
fi
case "$browser_report_state" in
    complete|missing|partial|malformed|truncated|report-error) ;;
    *) browser_report_state='unknown' ;;
esac
browser_validation_result='passed'
browser_validation_reason='complete'
if ((browser_summary_status != 0)); then
    browser_validation_result='failed'
    case "$browser_summary_status" in
        2) browser_validation_reason='invalid-report' ;;
        3) browser_validation_reason='incomplete-phases' ;;
        4) browser_validation_reason='browser-tests-not-passed' ;;
        124) browser_validation_reason='timeout' ;;
        *) browser_validation_reason='report-validation-failed' ;;
    esac
fi
printf 'HEPH_GCP_COOKING event=browser-report-validation operation=browser-report-validation phase=evidence status=%s report_state=%s reason=%s exit_code=%s\n' \
    "$browser_validation_result" "$browser_report_state" "$browser_validation_reason" "$browser_summary_status"
browser_gate_state='failed'
browser_reason_class='browser-validation-failed'
if ((browser_summary_status == 0)); then
    browser_gate_state='passed'
    browser_reason_class='none'
elif ((browser_summary_status == 124)); then
    browser_gate_state='timed-out'
    browser_reason_class='timeout'
elif ((browser_summary_status == 2)); then
    browser_reason_class='browser-report-invalid'
elif ((browser_summary_status == 4)); then
    browser_reason_class='browser-tests-not-passed'
fi
gate_update complete browser-validation --state "$browser_gate_state" --exit-code "$browser_summary_status" \
    --reason-class "$browser_reason_class" || true
if ((status != 0)); then
    # Keep the acceptance-suite result authoritative when both it and the
    # retained-evidence scanner fail.
    exit "$status"
fi
((scan_status == 0)) || exit "$scan_status"
((browser_summary_status == 0)) || exit "$browser_summary_status"
phase_pass
