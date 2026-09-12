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

preflight_marker() {
    local stage="$1" reason="$2" test_name="${3:-}"
    case "$stage" in
        host-identity|host-device|loader-checks|uid-map|cgroup-delegation|podman-rootless|immutable-references|workflow-inputs|oci-layout|python-image|rust-image|python-image-probe|rust-image-probe|preflight-command-checks) ;;
        *) return 1 ;;
    esac
    case "$reason" in
        unsupported-user|unsupported-architecture|kvm-unavailable|passt-unavailable|libkrun-unavailable|libkrunfw-unavailable|uid-map-unavailable|delegation-unavailable|rootful-podman|invalid-image-reference|workflow-file-unavailable|workflow-key-unavailable|layout-unavailable|manifest-unavailable|image-unavailable|image-pull-failed|image-probe-failed|missing-command) ;;
        *) return 1 ;;
    esac
    case "$test_name" in
        ''|awk|cargo|curl|debugfs|dirname|grep|head|id|ldconfig|musl-gcc|podman|python3|rustup|sed|unshare|timeout|python-image|rust-image|caddy-image|postgres-image|nats-image|builder-image|verifier-image|builder-layout|verifier-layout|base-layout-manifest) ;;
        *) return 1 ;;
    esac
    {
        printf 'HEPH_GCP_COOKING event=dependency-check operation=cooking-workload phase=cooking stage=%s reason_class=%s status=failed exit_code=1' \
            "$stage" "$reason"
        [[ -n "$test_name" ]] && printf ' test=%s' "$test_name"
        printf '\n'
    } >&2
}

die() {
    printf 'cooking preflight: %s\n' "$*" >&2
    exit 1
}

preflight_fail() {
    local stage="$1" reason="$2" message="$3" test_name="${4:-}"
    preflight_marker "$stage" "$reason" "$test_name" || true
    die "$message"
}

require_command() {
    local command_name="$1"
    command -v "$command_name" >/dev/null 2>&1 ||
        preflight_fail preflight-command-checks missing-command "required command is unavailable: ${command_name}" "$command_name"
}

require_digest_reference() {
    local name="$1" reference="$2" marker_name="$3"
    [[ "${reference}" =~ ^[a-zA-Z0-9._:/-]+@sha256:[0-9a-f]{64}$ ]] ||
        preflight_fail immutable-references invalid-image-reference "${name} must be an immutable sha256 reference: ${reference}" "$marker_name"
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
        preflight_fail cgroup-delegation delegation-unavailable "configured cgroup parent lacks writable cpu/io/memory/pids delegation: ${candidate}"
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
    preflight_fail cgroup-delegation delegation-unavailable 'no writable cgroup parent delegates cpu, io, memory, and pids'
}

[[ "$(id -u)" -ne 0 ]] || preflight_fail host-identity unsupported-user 'the real guest test must run as a non-root account'
[[ "$(uname -m)" == x86_64 ]] || preflight_fail host-identity unsupported-architecture 'the cooking guest fixture currently supports x86_64 only'
for command in awk cargo curl debugfs dirname grep head id ldconfig musl-gcc podman python3 rustup sed unshare timeout; do
    require_command "${command}"
done
[[ -r /dev/kvm && -w /dev/kvm ]] || preflight_fail host-device kvm-unavailable '/dev/kvm is not readable and writable'
[[ -x /usr/bin/passt ]] || preflight_fail host-device passt-unavailable '/usr/bin/passt is unavailable at /usr/bin/passt'
loader_cache="$(ldconfig -p)" || preflight_fail loader-checks libkrun-unavailable 'ldconfig failed while checking libkrun'
grep -q 'libkrun\.so\.1' <<<"${loader_cache}" || preflight_fail loader-checks libkrun-unavailable 'libkrun.so.1 is unavailable to the loader'
grep -q 'libkrunfw\.so\.5' <<<"${loader_cache}" || preflight_fail loader-checks libkrunfw-unavailable 'libkrunfw.so.5 is unavailable to the loader'
if [[ "$(id -u)" -ne 10001 || "$(id -g)" -ne 10001 ]]; then
    unshare --map-user 10001 --map-group 10001 true ||
        preflight_fail uid-map uid-map-unavailable 'cannot map the test process to guest UID/GID 10001'
fi

cgroup_parent="$(discover_cgroup_parent)"
podman_rootless="$(podman info --format '{{.Host.Security.Rootless}}' 2>/dev/null || true)"
[[ "${podman_rootless}" == true ]] || preflight_fail podman-rootless rootful-podman 'Podman must run rootless for the cooking fixture'
require_digest_reference 'Python guest image' "${image}" python-image
require_digest_reference 'Rust builder image' "${rust_builder_image}" rust-image
require_digest_reference 'Caddy image' "${caddy_image}" caddy-image
require_digest_reference 'PostgreSQL image' "${postgres_image}" postgres-image
require_digest_reference 'NATS image' "${nats_image}" nats-image

repository_image_workflow_file="${HEPHAESTUS_LOCAL_ROOT:-${repo_root}/.local/hephaestus}/repository-images/workflow.env"
[[ -f "${repository_image_workflow_file}" ]] ||
    preflight_fail workflow-inputs workflow-file-unavailable "repository OCI workflow inputs are unavailable: ${repository_image_workflow_file}"
builder_vm_image="$(workflow_value builder_vm_image)" || preflight_fail workflow-inputs workflow-key-unavailable 'OCI workflow builder image is unavailable'
verifier_vm_image="$(workflow_value verifier_vm_image)" || preflight_fail workflow-inputs workflow-key-unavailable 'OCI workflow verifier image is unavailable'
builder_layout="$(workflow_value builder_layout)" || preflight_fail workflow-inputs workflow-key-unavailable 'OCI workflow builder layout is unavailable'
verifier_layout="$(workflow_value verifier_layout)" || preflight_fail workflow-inputs workflow-key-unavailable 'OCI workflow verifier layout is unavailable'
base_layout_manifest="$(workflow_value base_layout_manifest)" || preflight_fail workflow-inputs workflow-key-unavailable 'OCI workflow base manifest is unavailable'
require_digest_reference 'OCI builder image' "${builder_vm_image}" builder-image
require_digest_reference 'OCI verifier image' "${verifier_vm_image}" verifier-image
[[ -f "${builder_layout}/index.json" && -f "${builder_layout}/oci-layout" ]] ||
    preflight_fail oci-layout layout-unavailable 'OCI builder layout is unavailable' builder-layout
[[ -f "${verifier_layout}/index.json" && -f "${verifier_layout}/oci-layout" ]] ||
    preflight_fail oci-layout layout-unavailable 'OCI verifier layout is unavailable' verifier-layout
[[ -f "${base_layout_manifest}" ]] ||
    preflight_fail oci-layout manifest-unavailable 'OCI base layout manifest is unavailable' base-layout-manifest

if podman image exists "${image}"; then
    :
elif [[ "${image}" == localhost/* ]]; then
    preflight_fail python-image image-unavailable "local Python guest image is unavailable; provision and publish it before running: ${image}"
else
    podman pull "${image}" >/dev/null || preflight_fail python-image image-pull-failed "cannot pull Python guest image: ${image}"
fi
if podman image exists "${rust_builder_image}"; then
    :
elif [[ "${rust_builder_image}" == localhost/* ]]; then
    preflight_fail rust-image image-unavailable "local Rust builder image is unavailable; provision and publish it before running: ${rust_builder_image}"
else
    podman pull "${rust_builder_image}" >/dev/null || preflight_fail rust-image image-pull-failed "cannot pull Rust builder image: ${rust_builder_image}"
fi
podman run --rm --network none "${image}" /bin/sh -ec \
    'test -x /usr/local/bin/python3; python3 -c "import sqlite3, socket, tomllib; assert hasattr(socket, \"AF_VSOCK\")"' ||
    preflight_fail python-image-probe image-probe-failed "Python guest image lacks /usr/local/bin/python3 with SQLite, tomllib, and AF_VSOCK: ${image}" python-image
podman run --rm --network none "${rust_builder_image}" /bin/sh -ec \
    'test -x /usr/local/bin/rustc; test -x /usr/local/bin/cargo' ||
    preflight_fail rust-image-probe image-probe-failed "Rust builder image lacks /usr/local/bin/rustc and cargo: ${rust_builder_image}" rust-image

printf 'Cooking preflight passed\n'
printf '  host: %s (%s) uid=%s:%s\n' "$(uname -sr)" "$(uname -m)" "$(id -u)" "$(id -g)"
printf '  libkrun: libkrun.so.1 + libkrunfw.so.5, passt=%s\n' "$(/usr/bin/passt --version | head -n 1)"
printf '  cgroup parent: %s\n' "${cgroup_parent}"
printf '  images: python=%s caddy=%s postgres=%s nats=%s\n' \
    "${image}" "${caddy_image}" "${postgres_image}" "${nats_image}"
printf '  rust-builder: %s\n' "${rust_builder_image}"
printf '  OCI workflow: %s\n' "${repository_image_workflow_file}"
