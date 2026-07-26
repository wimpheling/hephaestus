#!/usr/bin/env bash
#
# Build a pinned Fedora guest fixture and run the real, non-root libkrun smoke
# test. All generated files and cgroups are removed on exit.

set -Eeuo pipefail

readonly DEFAULT_FEDORA_IMAGE="registry.fedoraproject.org/fedora-minimal@sha256:8f42d200f04990b41081322d1c260ddf23b124b3b92538665ef4cc3064537249"
readonly GUEST_TARGET="x86_64-unknown-linux-musl"
readonly REQUIRED_CONTROLLERS=(cpu io memory pids)

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly script_dir
repo_root="$(cd -- "${script_dir}/.." && pwd -P)"
readonly repo_root
fedora_image="${HEPHAESTUS_LIBKRUN_FEDORA_IMAGE:-${DEFAULT_FEDORA_IMAGE}}"
readonly fedora_image

fixture_root=""
container_name=""
cgroup_root=""

die() {
    printf 'libkrun integration: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command is missing: $1"
}

contains_word() {
    local words="$1"
    local expected="$2"
    [[ " ${words} " == *" ${expected} "* ]]
}

network_snapshot() {
    {
        find /sys/class/net -mindepth 1 -maxdepth 1 -printf '%f\n' | sort
        cat /proc/net/route
        cat /proc/net/ipv6_route
    } | sha256sum | awk '{ print $1 }'
}

usable_cgroup_parent() {
    local candidate="$1"
    local available
    local enabled

    [[ -d "${candidate}" ]] || return 1
    [[ -w "${candidate}" && -w "${candidate}/cgroup.subtree_control" ]] || return 1
    available="$(<"${candidate}/cgroup.controllers")"
    enabled="$(<"${candidate}/cgroup.subtree_control")"
    for controller in "${REQUIRED_CONTROLLERS[@]}"; do
        contains_word "${available}" "${controller}" || return 1
        contains_word "${enabled}" "${controller}" || return 1
    done
}

discover_cgroup_parent() {
    local user_group
    local candidate
    local current

    if [[ -n "${HEPHAESTUS_LIBKRUN_CGROUP_PARENT:-}" ]]; then
        candidate="${HEPHAESTUS_LIBKRUN_CGROUP_PARENT}"
        usable_cgroup_parent "${candidate}" ||
            die "configured cgroup parent is not writable with cpu/io/memory/pids delegated: ${candidate}"
        printf '%s\n' "${candidate}"
        return
    fi

    if command -v systemctl >/dev/null 2>&1; then
        user_group="$(systemctl --user show -p ControlGroup --value 2>/dev/null || true)"
        candidate="/sys/fs/cgroup${user_group}"
        if [[ -n "${user_group}" ]] && usable_cgroup_parent "${candidate}"; then
            printf '%s\n' "${candidate}"
            return
        fi
    fi

    current="$(awk -F: '$1 == "0" { print $3 }' /proc/self/cgroup)"
    candidate="/sys/fs/cgroup${current}"
    while [[ "${candidate}" == /sys/fs/cgroup/* ]]; do
        if usable_cgroup_parent "${candidate}"; then
            printf '%s\n' "${candidate}"
            return
        fi
        candidate="$(dirname -- "${candidate}")"
    done
    die "no writable delegated cgroup parent found; set HEPHAESTUS_LIBKRUN_CGROUP_PARENT"
}

cleanup_cgroup() {
    [[ -n "${cgroup_root}" && -d "${cgroup_root}" ]] || return
    if [[ -w "${cgroup_root}/cgroup.kill" ]]; then
        printf '1\n' >"${cgroup_root}/cgroup.kill" 2>/dev/null || true
    fi
    for _attempt in {1..100}; do
        if ! grep -q '^populated 1$' "${cgroup_root}/cgroup.events" 2>/dev/null; then
            break
        fi
        sleep 0.01
    done
    find "${cgroup_root}" -mindepth 1 -depth -type d -exec rmdir -- {} + 2>/dev/null || true
    rmdir -- "${cgroup_root}" 2>/dev/null || true
}

cleanup() {
    local status=$?
    trap - EXIT INT TERM
    set +e
    cleanup_cgroup
    if [[ -n "${container_name}" ]]; then
        podman rm --force "${container_name}" >/dev/null 2>&1
    fi
    if [[ -n "${fixture_root}" && -f "${fixture_root}/.hephaestus-integration-fixture" ]]; then
        chmod -R u+w "${fixture_root}"
        rm -rf -- "${fixture_root}"
    fi
    exit "${status}"
}
trap cleanup EXIT INT TERM

[[ "$(id -u)" -ne 0 ]] || die "the integration test must run as a non-root service account"
[[ "$(uname -m)" == "x86_64" ]] ||
    die "the pinned fixture currently supports x86_64 only"

for command in awk cargo cat find grep head id install ldconfig mkfs.ext4 mktemp musl-gcc podman rustup sha256sum sort tar truncate; do
    require_command "${command}"
done
[[ -r /dev/kvm && -w /dev/kvm ]] || die "/dev/kvm is not readable and writable"
[[ -x /usr/bin/passt ]] || die "/usr/bin/passt is unavailable"
loader_cache="$(ldconfig -p)"
grep -q 'libkrun\.so\.1' <<<"${loader_cache}" ||
    die "libkrun.so.1 is unavailable to the loader"
grep -q 'libkrunfw\.so\.5' <<<"${loader_cache}" ||
    die "libkrunfw.so.5 is unavailable to the loader"

cgroup_parent="$(discover_cgroup_parent)"
readonly cgroup_parent

rustup target add "${GUEST_TARGET}"
cargo build \
    --manifest-path "${repo_root}/Cargo.toml" \
    --release \
    --package vm-libkrun \
    --bin heph-init \
    --bin heph-integration-check \
    --features integration-guest \
    --target "${GUEST_TARGET}"

fixture_root="$(mktemp -d "${TMPDIR:-/var/tmp}/hephaestus-libkrun.XXXXXX")"
touch "${fixture_root}/.hephaestus-integration-fixture"
chmod 0700 "${fixture_root}"
mkdir -p \
    "${fixture_root}/rootfs" \
    "${fixture_root}/runtime" \
    "${fixture_root}/disks" \
    "${fixture_root}/mounts/repository" \
    "${fixture_root}/mounts/workspace"
chmod 0700 "${fixture_root}/runtime"

container_name="hephaestus-libkrun-fixture-$$"
podman pull "${fedora_image}"
podman create --name "${container_name}" "${fedora_image}" /bin/true >/dev/null
podman export "${container_name}" | tar -C "${fixture_root}/rootfs" -xf -
podman rm "${container_name}" >/dev/null
container_name=""

install -D -m 0755 \
    "${repo_root}/target/${GUEST_TARGET}/release/heph-init" \
    "${fixture_root}/rootfs/usr/libexec/hephaestus/heph-init"
install -D -m 0755 \
    "${repo_root}/target/${GUEST_TARGET}/release/heph-integration-check" \
    "${fixture_root}/rootfs/usr/libexec/hephaestus/integration-check"
printf 'repository\n' >"${fixture_root}/mounts/repository/integration-marker"
truncate -s 128M "${fixture_root}/disks/sqlite.raw"
mkfs.ext4 -q -F "${fixture_root}/disks/sqlite.raw"

cgroup_root="${cgroup_parent}/hephaestus-integration-$$"
mkdir "${cgroup_root}"
printf '+cpu +io +memory +pids\n' >"${cgroup_root}/cgroup.subtree_control"

network_before="$(network_snapshot)"
readonly network_before
printf 'Host kernel: %s\n' "$(uname -r)"
printf 'passt: %s\n' "$(/usr/bin/passt --version | head -n 1)"
if command -v rpm >/dev/null 2>&1; then
    rpm -q libkrun libkrunfw
fi
printf 'Running libkrun smoke test with pinned image %s\n' "${fedora_image}"
HEPHAESTUS_LIBKRUN_INTEGRATION=1 \
HEPHAESTUS_LIBKRUN_RUNTIME_ROOT="${fixture_root}/runtime" \
HEPHAESTUS_LIBKRUN_IMAGE_ROOT="${fixture_root}" \
HEPHAESTUS_LIBKRUN_ROOTFS="${fixture_root}/rootfs" \
HEPHAESTUS_LIBKRUN_DISK_ROOT="${fixture_root}/disks" \
HEPHAESTUS_LIBKRUN_SQLITE_DISK="${fixture_root}/disks/sqlite.raw" \
HEPHAESTUS_LIBKRUN_MOUNT_ROOT="${fixture_root}/mounts" \
HEPHAESTUS_LIBKRUN_REPOSITORY="${fixture_root}/mounts/repository" \
HEPHAESTUS_LIBKRUN_WORKSPACE="${fixture_root}/mounts/workspace" \
HEPHAESTUS_LIBKRUN_CGROUP_ROOT="${cgroup_root}" \
cargo test \
    --manifest-path "${repo_root}/Cargo.toml" \
    --package vm-libkrun \
    --test libkrun_integration \
    -- --nocapture

if find "${fixture_root}/runtime" -mindepth 1 -print -quit | grep -q .; then
    die "runtime files leaked after the integration test"
fi
if find "${cgroup_root}" -mindepth 1 -maxdepth 1 -type d -print -quit | grep -q .; then
    die "per-VM cgroups leaked after the integration test"
fi
grep -q '^populated 0$' "${cgroup_root}/cgroup.events" ||
    die "the integration cgroup remains populated"
[[ "$(network_snapshot)" == "${network_before}" ]] ||
    die "host network interfaces or routes changed during the integration test"

printf 'libkrun integration smoke test passed; runtime and cgroup cleanup verified\n'
