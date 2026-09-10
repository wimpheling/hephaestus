#!/usr/bin/env bash
# Root startup script for one disposable nested-KVM integration-smoke VM.
# The VM has no service account or scopes.  It fetches only this public repo at
# the exact github-sha metadata value, and reports its result on serial output.
set -Eeuo pipefail
umask 077

readonly metadata_root='http://metadata.google.internal/computeMetadata/v1'
readonly repository_url='https://github.com/wimpheling/hephaestus.git'
readonly forge_uid=10001
readonly forge_gid=10001
readonly rust_version='1.88.0'
readonly libkrun_tag='v1.19.0'
readonly libkrunfw_tag='v5.5.0'
readonly work_root='/srv/hephaestus'
readonly checkout_root="${work_root}/checkout"
readonly source_root="${work_root}/src"
readonly temporary_root="${work_root}/tmp"
readonly evidence_root="${work_root}/evidence"
readonly guest_image='docker.io/library/ubuntu@sha256:52df9b1ee71626e0088f7d400d5c6b5f7bb916f8f0c82b474289a4ece6cf3faf'
readonly log_file='/var/log/hephaestus/gcp-kvm-startup.log'

phase='initializing'
revision='unknown'
trial_deadline=0

die() { printf 'gcp-kvm-startup: %s\n' "$*" >&2; return 1; }

install -d -m 0700 /var/log/hephaestus
install -m 0600 /dev/null "$log_file"
[[ -w /dev/ttyS0 ]] || die 'GCE serial console /dev/ttyS0 is unavailable'
exec > >(tee -a "$log_file" /dev/ttyS0) 2>&1

finish() {
  local status=$?
  trap - EXIT
  if ((status == 0)); then
    printf 'HEPHAESTUS_GCP_KVM_SMOKE: PASS\n'
  else
    printf 'HEPHAESTUS_GCP_KVM_SMOKE: FAIL phase=%s exit=%s revision=%s\n' \
      "$phase" "$status" "$revision"
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
  curl --fail --silent --show-error -H 'Metadata-Flavor: Google' \
    "${metadata_root}/instance/attributes/$1"
}

require_command() { command -v "$1" >/dev/null 2>&1 || die "missing command: $1"; }

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
  local remaining=$((trial_deadline - SECONDS))
  ((remaining > 0)) || die 'internal 40-minute deadline elapsed'
  printf '%s\n' "$remaining"
}

run_with_deadline() {
  local remaining
  remaining="$(remaining_seconds)"
  timeout --kill-after=30s "${remaining}s" "$@"
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

phase_start metadata
require_command curl
revision="${HEPHAESTUS_GCP_REVISION:-$(metadata_value github-sha)}"
[[ "$revision" =~ ^[0-9a-f]{40}$ ]] || die 'github-sha must be an exact lowercase 40-character commit SHA'
trial_deadline=$((SECONDS + 2400))
marker ready

phase_start host-packages
run_with_deadline apt-get update -qq
run_with_deadline env DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
  bc bison build-essential ca-certificates clang cpio dwarves e2fsprogs flex \
  fuse-overlayfs git libcap-ng-dev libelf-dev libfdt-dev libglib2.0-dev \
  libncurses-dev libpixman-1-dev libseccomp-dev libslirp-dev libssl-dev \
  libzstd-dev lld make musl-tools openssl patch patchelf perl podman \
  python3 python3-pyelftools rsync rustup slirp4netns passt tar uidmap xz-utils
phase_pass

phase_start accounts
[[ "$(uname -m)" == x86_64 ]] || die 'host must be x86_64'
[[ -r /dev/kvm && -w /dev/kvm ]] || die '/dev/kvm is not readable and writable'
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
kvm_group="$(stat --format='%G' /dev/kvm)"
[[ -n "$kvm_group" && "$kvm_group" != UNKNOWN ]] || die 'KVM device has no usable group'
getent group "$kvm_group" >/dev/null || die "KVM group unavailable: $kvm_group"
usermod --append --groups "$kvm_group" forge
for file in /etc/subuid /etc/subgid; do
  [[ -e "$file" ]] || install -m 0644 /dev/null "$file"
done
ensure_subordinate_range
install -d -m 0700 -o forge -g forge "$work_root" "$temporary_root" "$evidence_root" \
  /run/user/10001 /home/forge/.cargo /home/forge/.rustup
phase_pass

phase_start cgroup-podman
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
    mapped_uid="$(unshare --user --map-user 10001 --map-group 10001 id -u)"
    [[ "$mapped_uid" == 10001 ]]
  '
phase_pass

phase_start rust-toolchain
run_with_deadline "${forge_env[@]}" rustup toolchain install "$rust_version" --profile minimal --no-self-update
run_with_deadline "${forge_env[@]}" rustup default "$rust_version"
run_with_deadline "${forge_env[@]}" rustup target add x86_64-unknown-linux-musl
phase_pass

phase_start libkrunfw
install -d -m 0700 -o forge -g forge "$source_root"
run_with_deadline "${forge_env[@]}" git clone --depth 1 --branch "$libkrunfw_tag" \
  https://github.com/libkrun/libkrunfw.git "$source_root/libkrunfw"
libkrunfw_revision="$("${forge_env[@]}" git -C "$source_root/libkrunfw" rev-parse HEAD)"
run_with_deadline "${forge_env[@]}" make -C "$source_root/libkrunfw" -j8
run_with_deadline make -C "$source_root/libkrunfw" PREFIX=/usr/local install
printf '/usr/local/lib64\n' >/etc/ld.so.conf.d/hephaestus-libkrun.conf
run_with_deadline ldconfig
libkrunfw_so="$(find /usr/local/lib64 -maxdepth 1 -type f -name 'libkrunfw.so.5*' -print -quit)"
[[ -n "$libkrunfw_so" ]] || die 'libkrunfw install artifact is missing'
readelf -d "$libkrunfw_so" | grep -q 'SONAME.*libkrunfw\.so\.5' || die 'libkrunfw SONAME is incompatible'
phase_pass

phase_start libkrun
run_with_deadline "${forge_env[@]}" git clone --depth 1 --branch "$libkrun_tag" \
  https://github.com/libkrun/libkrun.git "$source_root/libkrun"
libkrun_revision="$("${forge_env[@]}" git -C "$source_root/libkrun" rev-parse HEAD)"
run_with_deadline "${forge_env[@]}" make -C "$source_root/libkrun" BLK=1 NET=1 -j8
run_with_deadline make -C "$source_root/libkrun" BLK=1 NET=1 PREFIX=/usr/local install
run_with_deadline ldconfig
libkrun_so="$(find /usr/local/lib64 -maxdepth 1 -type f -name 'libkrun.so.1*' -print -quit)"
[[ -n "$libkrun_so" ]] || die 'libkrun install artifact is missing'
readelf -d "$libkrun_so" | grep -q 'SONAME.*libkrun\.so\.1' || die 'libkrun SONAME is incompatible'
ldconfig -p | grep -q 'libkrun\.so\.1' || die 'libkrun.so.1 missing from loader cache'
ldconfig -p | grep -q 'libkrunfw\.so\.5' || die 'libkrunfw.so.5 missing from loader cache'
printf 'HEPH_GCP_KVM_LIBS libkrun_tag=%s commit=%s libkrunfw_tag=%s commit=%s features=blk,net\n' \
  "$libkrun_tag" "$libkrun_revision" "$libkrunfw_tag" "$libkrunfw_revision"
phase_pass

phase_start checkout
run_with_deadline "${forge_env[@]}" git clone --filter=blob:none --no-checkout "$repository_url" "$checkout_root"
run_with_deadline "${forge_env[@]}" git -C "$checkout_root" fetch --depth 1 origin "$revision"
run_with_deadline "${forge_env[@]}" git -C "$checkout_root" checkout --detach "$revision"
[[ "$("${forge_env[@]}" git -C "$checkout_root" rev-parse HEAD)" == "$revision" ]] || die 'checkout SHA mismatch'
phase_pass

phase_start real-libkrun-smoke
smoke_unit="heph-gcp-kvm-smoke-${GITHUB_RUN_ID:-manual}"
smoke_log_dir="${evidence_root}/integration"
install -d -m 0700 -o forge -g forge "$smoke_log_dir"
run_with_deadline systemd-run --unit="$smoke_unit" --service-type=oneshot --wait --pipe --collect \
  --expand-environment=no \
  --property=Delegate=yes --property=RuntimeMaxSec="$(remaining_seconds)s" \
  --property=TimeoutStopSec=30s --property=TasksMax=infinity --property=LimitNOFILE=65536 \
  --property=CPUAccounting=yes --property=MemoryAccounting=yes --property=TasksAccounting=yes \
  --property=IOAccounting=yes --uid="$forge_uid" --gid="$forge_gid" \
  --working-directory="$checkout_root" --setenv=HOME=/home/forge \
  --setenv=XDG_RUNTIME_DIR=/run/user/10001 --setenv=RUSTUP_HOME=/home/forge/.rustup \
  --setenv=CARGO_HOME=/home/forge/.cargo \
  --setenv=PATH=/home/forge/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  --setenv=HEPH_GCP_SMOKE_SCRIPT="$checkout_root/scripts/run-libkrun-integration.sh" \
  --setenv=HEPH_GCP_SMOKE_IMAGE="$guest_image" \
  --setenv=HEPH_GCP_SMOKE_TMP="$temporary_root" \
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
  '
phase_pass
printf 'HEPH_GCP_KVM_EVIDENCE revision=%s diagnostics=%s libkrun=%s libkrunfw=%s\n' \
  "$revision" "$smoke_log_dir" "$libkrun_revision" "$libkrunfw_revision"
