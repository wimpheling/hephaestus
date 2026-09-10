#!/usr/bin/env bash
# Run the complete Cooking acceptance path from the immutable private cache.
# This helper is invoked as root by the disposable GCE startup flow. It uses
# the runtime service account only to fetch the cache, then runs all Cooking
# work as forge (UID/GID 10001) in a delegated transient systemd unit.
set -Eeuo pipefail
umask 077

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

phase='initializing'
stage_root=''
archive_path=''
node_bin=''
node_path=''
deadline_epoch="${HEPH_GCP_COOKING_DEADLINE_EPOCH:-}"
token_json=''
token_header=''

fail() { printf 'gcp-cooking-run: %s\n' "$*" >&2; return 1; }

validate_sha256() {
    local name="$1" value="$2"
    [[ "$value" =~ ^[0-9a-f]{64}$ ]] ||
        fail "$name must be exactly 64 lowercase hexadecimal characters"
}

[[ "$(id -u)" -eq 0 ]] || fail 'this helper must be invoked as root'
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

phase_start() {
    phase="$1"
    printf 'HEPH_GCP_COOKING event=phase-start phase=%s\n' "$phase"
}
phase_pass() { printf 'HEPH_GCP_COOKING event=phase-pass phase=%s\n' "$phase"; }
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

phase_start host-tools
validate_sha256 cache_sha256 "$cache_sha256"
for command in awk bash curl date find git grep install ldconfig podman python3 readlink sha256sum systemd-run tar timeout; do
    require_command "$command"
done
# Keep this list aligned with the actual full Cooking scripts: repository image
# import/build proof, the browser harness, and the compressed private cache.
missing_packages=()
for pair in 'skopeo:skopeo' 'nft:nftables' 'zstd:zstd'; do
    command_name="${pair%%:*}"
    package_name="${pair#*:}"
    command -v "$command_name" >/dev/null 2>&1 || missing_packages+=("$package_name")
done
if ((${#missing_packages[@]})); then
    run_with_deadline apt-get update -qq
    run_with_deadline env DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends "${missing_packages[@]}"
fi
for command in nft podman python3 skopeo systemd-run tar timeout zstd; do
    require_command "$command"
done
if ! command -v oras >/dev/null 2>&1; then
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
if [[ "$node_major" =~ ^[0-9]+$ && "$node_major" -ge 20 ]] &&
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
PATH="/home/forge/.cargo/bin:${node_path}:${PATH}"
export PATH
node_major="$($node_bin --version | sed -E 's/^v([0-9]+).*/\1/')"
[[ "$node_major" -ge 20 ]] || fail 'Node 20 or newer is required by the Playwright lockfile'
runuser -u forge -- env PATH="$PATH" HOME=/home/forge "$node_bin" --version
runuser -u forge -- env PATH="$PATH" HOME=/home/forge npm --version
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
run_with_deadline runuser -u forge -- env HOME=/home/forge XDG_RUNTIME_DIR=/run/user/10001 PATH="$PATH" \
    bash -Eeuo pipefail -c '
        cache="$1"; py="$2"; rust="$3"; shift 3
        import_one() {
            local source="$1" destination="$2"
            podman image exists "$destination" ||
                # containers-storage has no multi-image group destination; copy
                # the accepted manifest and preserve its digest exactly.
                skopeo copy --preserve-digests "$source" "containers-storage:$destination" >/dev/null
            [[ "$(skopeo inspect --format "{{.Digest}}" "containers-storage:$destination")" == "${destination##*@}" ]]
        }
        import_one "oci:$cache/layouts/python-ubuntu/image" "$py"
        import_one "oci-archive:$cache/guest-images/rust-ubuntu-profile.oci" "$rust"
        import_one "oci:$cache/layouts/oci-builder-ubuntu/image" "$(awk -F= '\''$1=="builder_vm_image" {print $2}'\'' "$cache/repository-images/workflow.env")"
        import_one "oci:$cache/layouts/oci-verifier-ubuntu/image" "$(awk -F= '\''$1=="verifier_vm_image" {print $2}'\'' "$cache/repository-images/workflow.env")"
    ' -- "$cache_root" "$runtime_python_ref" "$runtime_rust_ref"
phase_pass

phase_start browser-host
# Match the reviewed CI host setup: install browser OS dependencies as root,
# then install the browser itself into a forge-owned shared cache.
run_with_deadline runuser -u forge -- env HOME=/home/forge XDG_RUNTIME_DIR=/run/user/10001 \
    PATH="$PATH" PLAYWRIGHT_BROWSERS_PATH="$browser_root" \
    bash -Eeuo pipefail -c 'cd "$1/e2e/playwright" && npm ci' -- "$checkout_root"
run_with_deadline env PATH="$PATH" bash -Eeuo pipefail -c \
    'cd "$1/e2e/playwright" && npx playwright install-deps chromium' -- "$checkout_root"
chown -R forge:forge "$browser_root"
run_with_deadline runuser -u forge -- env HOME=/home/forge XDG_RUNTIME_DIR=/run/user/10001 \
    PATH="$PATH" PLAYWRIGHT_BROWSERS_PATH="$browser_root" \
    bash -Eeuo pipefail -c 'cd "$1/e2e/playwright" && npx playwright install chromium' -- "$checkout_root"
phase_pass

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

phase_start cooking
cooking_remaining="$(remaining_seconds)"
if ((cooking_remaining > 1500)); then
    cooking_timeout=1500
else
    cooking_timeout="$cooking_remaining"
fi
cooking_unit="heph-gcp-cooking-${HEPH_GCP_RUN_ID:-manual}"
install -d -m 0700 -o forge -g forge "$evidence_root"
set +e
run_with_deadline systemd-run --unit="$cooking_unit" --service-type=oneshot --wait --pipe --collect \
    --expand-environment=no --property=Delegate=yes --property=RuntimeMaxSec="${cooking_remaining}s" \
    --property=TimeoutStopSec=30s --property=TasksMax=infinity \
    --property=LimitNOFILE=65536 --uid="$forge_uid" --gid="$forge_gid" \
    --working-directory="$checkout_root" --setenv=HOME=/home/forge \
    --setenv=XDG_RUNTIME_DIR=/run/user/10001 --setenv=RUSTUP_HOME=/home/forge/.rustup \
    --setenv=CARGO_HOME=/home/forge/.cargo --setenv=TMPDIR=/tmp/hephaestus-libkrun \
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
    --setenv=PATH="$PATH" \
    /bin/bash -Eeuo pipefail -c '
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
status=$?
set -e
if ((status != 0)); then
    printf 'Cooking systemd unit failed with status=%s unit=%s\n' "$status" "$cooking_unit" >&2
    systemctl status "$cooking_unit" --no-pager 2>&1 | tail -80 || true
fi
phase_start evidence
set +e
run_with_deadline python3 -B "$checkout_root/scripts/check-browser-evidence.py" "$evidence_root"
scan_status=$?
set -e
if ((status != 0)); then
    # Keep the acceptance-suite result authoritative when both it and the
    # retained-evidence scanner fail.
    exit "$status"
fi
((scan_status == 0)) || exit "$scan_status"
phase_pass
