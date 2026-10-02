#!/usr/bin/env bash
# Actual owned-provider proof; all image/runtime/cgroup roots are task disposable.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly repo_root
readonly guest_target=x86_64-unknown-linux-musl
readonly image=docker.io/library/ubuntu@sha256:52df9b1ee71626e0088f7d400d5c6b5f7bb916f8f0c82b474289a4ece6cf3faf
: "${HEPHAESTUS_LIBKRUN_CGROUP_PARENT:?set a writable delegated cgroup parent}"
for command in podman tar cargo sha256sum; do command -v "${command}" >/dev/null; done
test -r /dev/kvm && test -w /dev/kvm
podman image exists "${image}"
cargo build --manifest-path "${repo_root}/Cargo.toml" --release -p vm-libkrun \
    --target "${guest_target}" --bin heph-init
fixture_root="$(mktemp -d /tmp/heph-owned-vm.XXXXXX)"
container="heph-owned-vm-proof-$$"
cgroup_root="${HEPHAESTUS_LIBKRUN_CGROUP_PARENT}/heph-owned-vm-proof-$$"
cleanup() {
    podman rm "${container}" >/dev/null 2>&1 || true
    if [[ -d "${cgroup_root}" ]] && ! rmdir "${cgroup_root}" 2>/dev/null; then
        printf 'Retaining unresolved native fixture: %s\n' "${fixture_root}" >&2
        return
    fi
    rm -rf -- "${fixture_root}"
}
trap cleanup EXIT
mkdir "${fixture_root}/rootfs"
podman create --pull never --name "${container}" "${image}" /bin/true >/dev/null
podman export "${container}" | tar -C "${fixture_root}/rootfs" -xf -
podman rm "${container}" >/dev/null
target_root="${CARGO_TARGET_DIR:-${repo_root}/target}"
install -D -m 0755 "${target_root}/${guest_target}/release/heph-init" \
    "${fixture_root}/rootfs/usr/libexec/hephaestus/heph-init"
mkdir "${cgroup_root}"
printf '+cpu +io +memory +pids\n' >"${cgroup_root}/cgroup.subtree_control"
printf 'OWNED_VM_FIXTURE=1 protocol=10 fresh_init=1 fresh_root=1 pinned_image=%s init_sha256=%s\n' \
    "${image}" "$(sha256sum "${fixture_root}/rootfs/usr/libexec/hephaestus/heph-init" | cut -d ' ' -f 1)"
HEPHAESTUS_LIBKRUN_OWNED_ROOTFS="${fixture_root}/rootfs" \
HEPHAESTUS_LIBKRUN_OWNED_CGROUP_ROOT="${cgroup_root}" \
cargo test --manifest-path "${repo_root}/Cargo.toml" -p vm-libkrun --all-features \
    --test owned_vm -- --exact real_owned_vm_lifetime_scoped_cleanup_and_supervisor_restart \
    --ignored --nocapture
