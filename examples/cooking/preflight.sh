#!/usr/bin/env bash
# Validate the host and immutable guest image before compiling or booting.
set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly script_dir
repo_root="$(cd -- "${script_dir}/../.." && pwd -P)"
readonly repo_root
readonly required_controllers=(cpu io memory pids)
readonly image="${HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE:?set HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE to a digest-pinned Python image}"
readonly rust_builder_image="${HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE:?set HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE to a digest-pinned Rust builder image}"
readonly caddy_image="${HEPHAESTUS_CADDY_TEST_IMAGE:-docker.io/library/caddy@sha256:d8c17a862962def15cde69863a3a463f25a2664942eafd7bdbf050e9c3116b83}"
readonly postgres_image="${HEPHAESTUS_POSTGRES_TEST_IMAGE:-docker.io/library/postgres@sha256:af194ccf3e2d7fe367012c7b88ce8b816c5c889b18a5b316799a1f0d7eac746a}"
readonly nats_image="${HEPHAESTUS_NATS_TEST_IMAGE:-docker.io/library/nats@sha256:e4bf19f15fd3218814a4e3c9e0064e1334bd8aa20d5984b9f1a0afd084f8cc00}"

die() {
    printf 'cooking preflight: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command is unavailable: $1"
}

require_digest_reference() {
    local name="$1"
    local reference="$2"
    [[ "${reference}" =~ ^[a-zA-Z0-9._:/-]+@sha256:[0-9a-f]{64}$ ]] ||
        die "${name} must be an immutable sha256 reference: ${reference}"
}

workflow_value() {
    local key="$1"
    awk -F= -v expected="${key}" '$1 == expected { print substr($0, index($0, "=") + 1); found=1 } END { if (!found) exit 1 }' \
        "${repository_image_workflow_file}"
}

contains_word() {
    local words="$1"
    local expected="$2"
    [[ " ${words} " == *" ${expected} "* ]]
}

usable_cgroup_parent() {
    local candidate="$1"
    local available enabled controller
    [[ -d "${candidate}" && -w "${candidate}" && -w "${candidate}/cgroup.subtree_control" ]] ||
        return 1
    available="$(<"${candidate}/cgroup.controllers")"
    enabled="$(<"${candidate}/cgroup.subtree_control")"
    for controller in "${required_controllers[@]}"; do
        contains_word "${available}" "${controller}" || return 1
        contains_word "${enabled}" "${controller}" || return 1
    done
}

discover_cgroup_parent() {
    local user_group candidate current
    if [[ -n "${HEPHAESTUS_LIBKRUN_CGROUP_PARENT:-}" ]]; then
        candidate="${HEPHAESTUS_LIBKRUN_CGROUP_PARENT}"
        usable_cgroup_parent "${candidate}" && { printf '%s\n' "${candidate}"; return; }
        die "configured cgroup parent lacks writable cpu/io/memory/pids delegation: ${candidate}"
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
    die 'no writable cgroup parent delegates cpu, io, memory, and pids'
}

[[ "$(id -u)" -ne 0 ]] || die 'the real guest test must run as a non-root account'
[[ "$(uname -m)" == x86_64 ]] || die 'the cooking guest fixture currently supports x86_64 only'
for command in awk cargo curl debugfs dirname grep head id ldconfig musl-gcc podman python3 rustup sed unshare timeout; do
    require_command "${command}"
done
[[ -r /dev/kvm && -w /dev/kvm ]] || die '/dev/kvm is not readable and writable'
[[ -x /usr/bin/passt ]] || die '/usr/bin/passt is unavailable at /usr/bin/passt'
loader_cache="$(ldconfig -p)"
grep -q 'libkrun\.so\.1' <<<"${loader_cache}" || die 'libkrun.so.1 is unavailable to the loader'
grep -q 'libkrunfw\.so\.5' <<<"${loader_cache}" || die 'libkrunfw.so.5 is unavailable to the loader'
if [[ "$(id -u)" -ne 10001 || "$(id -g)" -ne 10001 ]]; then
    unshare --map-user 10001 --map-group 10001 true ||
        die 'cannot map the test process to guest UID/GID 10001'
fi

cgroup_parent="$(discover_cgroup_parent)"
podman_rootless="$(podman info --format '{{.Host.Security.Rootless}}' 2>/dev/null || true)"
[[ "${podman_rootless}" == true ]] || die 'Podman must run rootless for the cooking fixture'
require_digest_reference 'Python guest image' "${image}"
require_digest_reference 'Rust builder image' "${rust_builder_image}"
require_digest_reference 'Caddy image' "${caddy_image}"
require_digest_reference 'PostgreSQL image' "${postgres_image}"
require_digest_reference 'NATS image' "${nats_image}"

repository_image_workflow_file="${HEPHAESTUS_LOCAL_ROOT:-${repo_root}/.local/hephaestus}/repository-images/workflow.env"
[[ -f "${repository_image_workflow_file}" ]] ||
    die "repository OCI workflow inputs are unavailable: ${repository_image_workflow_file}"
builder_vm_image="$(workflow_value builder_vm_image)"
verifier_vm_image="$(workflow_value verifier_vm_image)"
builder_layout="$(workflow_value builder_layout)"
verifier_layout="$(workflow_value verifier_layout)"
base_layout_manifest="$(workflow_value base_layout_manifest)"
require_digest_reference 'OCI builder image' "${builder_vm_image}"
require_digest_reference 'OCI verifier image' "${verifier_vm_image}"
[[ -f "${builder_layout}/index.json" && -f "${builder_layout}/oci-layout" ]] ||
    die "OCI builder layout is unavailable: ${builder_layout}"
[[ -f "${verifier_layout}/index.json" && -f "${verifier_layout}/oci-layout" ]] ||
    die "OCI verifier layout is unavailable: ${verifier_layout}"
[[ -f "${base_layout_manifest}" ]] ||
    die "OCI base layout manifest is unavailable: ${base_layout_manifest}"

if podman image exists "${image}"; then
    :
elif [[ "${image}" == localhost/* ]]; then
    die "local Python guest image is unavailable; provision and publish it before running: ${image}"
else
    podman pull "${image}" >/dev/null || die "cannot pull Python guest image: ${image}"
fi
if podman image exists "${rust_builder_image}"; then
    :
elif [[ "${rust_builder_image}" == localhost/* ]]; then
    die "local Rust builder image is unavailable; provision and publish it before running: ${rust_builder_image}"
else
    podman pull "${rust_builder_image}" >/dev/null || die "cannot pull Rust builder image: ${rust_builder_image}"
fi
podman run --rm --network none "${image}" /bin/sh -ec \
    'test -x /usr/local/bin/python3; python3 -c "import sqlite3, socket, tomllib; assert hasattr(socket, \"AF_VSOCK\")"' ||
    die "Python guest image lacks /usr/local/bin/python3 with SQLite, tomllib, and AF_VSOCK: ${image}"
podman run --rm --network none "${rust_builder_image}" /bin/sh -ec \
    'test -x /usr/local/bin/rustc; test -x /usr/local/bin/cargo' ||
    die "Rust builder image lacks /usr/local/bin/rustc and cargo: ${rust_builder_image}"

printf 'Cooking preflight passed\n'
printf '  host: %s (%s) uid=%s:%s\n' "$(uname -sr)" "$(uname -m)" "$(id -u)" "$(id -g)"
printf '  libkrun: libkrun.so.1 + libkrunfw.so.5, passt=%s\n' "$(/usr/bin/passt --version | head -n 1)"
printf '  cgroup parent: %s\n' "${cgroup_parent}"
printf '  images: python=%s caddy=%s postgres=%s nats=%s\n' \
    "${image}" "${caddy_image}" "${postgres_image}" "${nats_image}"
printf '  rust-builder: %s\n' "${rust_builder_image}"
printf '  OCI workflow: %s\n' "${repository_image_workflow_file}"
