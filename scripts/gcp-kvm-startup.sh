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
readonly passt_revision='386b5f5472b89769c025f5d5056348532a823b93'
readonly passt_source_url='https://passt.top/passt'
readonly work_root='/srv/hephaestus'
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
readonly log_file='/var/log/hephaestus/gcp-kvm-startup.log'

phase='initializing'
revision='unknown'
trial_deadline=0
test_mode='smoke'

die() { printf 'gcp-kvm-startup: %s\n' "$*" >&2; return 1; }

install -d -m 0700 /var/log/hephaestus
install -m 0600 /dev/null "$log_file"
[[ -w /dev/ttyS0 ]] || die 'GCE serial console /dev/ttyS0 is unavailable'
exec > >(tee -a "$log_file" /dev/ttyS0) 2>&1

finish() {
  local status=$?
  trap - EXIT
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

phase_start metadata
require_command curl
revision="${HEPHAESTUS_GCP_REVISION:-$(metadata_value github-sha)}"
[[ "$revision" =~ ^[0-9a-f]{40}$ ]] || die 'github-sha must be an exact lowercase 40-character commit SHA'
test_mode="$(metadata_value test-mode)"
case "$test_mode" in
  smoke|gcp-cooking) ;;
  *) die 'test-mode must be smoke or gcp-cooking' ;;
esac
trial_deadline=$((SECONDS + 2400))
marker "ready mode=$test_mode"

phase_start host-packages
run_with_deadline apt-get update -qq
run_with_deadline env DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
  apparmor bc bison build-essential ca-certificates clang cpio dwarves e2fsprogs flex \
  fuse-overlayfs git libcap-ng-dev libclang-dev libelf-dev libfdt-dev libglib2.0-dev \
  libncurses-dev libpixman-1-dev libseccomp-dev libslirp-dev libssl-dev \
  libzstd-dev llvm-dev lld make musl-tools openssl patch patchelf perl podman \
  python3 python3-pyelftools rsync rustup slirp4netns passt tar uidmap xz-utils
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
  "$smoke_temporary_root" /run/user/10001 /home/forge/.cargo /home/forge/.rustup
phase_pass

phase_start passt-compat
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
if grep -Eq '^[[:space:]]*#include( if exists)?[[:space:]]+<local/usr\.bin\.passt>[[:space:]]*$' \
    "$passt_profile_path"; then
  apparmor_parser -r "$passt_profile_path"
else
  awk '
    { lines[NR] = $0 }
    /^[[:space:]]*}[[:space:]]*$/ { closing = NR }
    END {
      if (!closing) exit 1
      for (line = 1; line <= NR; line++) {
        if (line == closing) print "#include <local/usr.bin.passt>"
        print lines[line]
      }
    }
  ' "$passt_profile_path" >"$passt_profile_overlay" ||
    die 'could not construct the passt AppArmor overlay'
  apparmor_parser -r -I /etc/apparmor.d "$passt_profile_overlay"
fi
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
run_with_deadline "${forge_env[@]}" rustup toolchain install "$rust_version" --profile minimal --no-self-update
run_with_deadline "${forge_env[@]}" rustup default "$rust_version"
run_with_deadline "${forge_env[@]}" rustup target add x86_64-unknown-linux-musl
phase_pass

phase_start libkrunfw
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
phase_pass

phase_start libkrun
run_with_deadline "${forge_env[@]}" git clone --depth 1 --branch "$libkrun_tag" \
  https://github.com/libkrun/libkrun.git "$source_root/libkrun"
libkrun_revision="$("${forge_env[@]}" git -C "$source_root/libkrun" rev-parse HEAD)"
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
phase_pass

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
    "$checkout_root/scripts/gcp-cooking-run.sh"
  phase_pass
else

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
  '
phase_pass
fi
if [[ "$test_mode" == gcp-cooking ]]; then
  printf 'HEPH_GCP_COOKING_EVIDENCE revision=%s diagnostics=%s\n' \
    "$revision" "${evidence_root}/cooking"
else
  printf 'HEPH_GCP_KVM_EVIDENCE revision=%s diagnostics=%s libkrun=%s libkrunfw=%s\n' \
    "$revision" "$smoke_log_dir" "$libkrun_revision" "$libkrunfw_revision"
fi
