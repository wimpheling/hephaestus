#!/usr/bin/env bash
# Native named-volume proof. All guest roots and backing data are disposable.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly repo_root
readonly guest_target=x86_64-unknown-linux-musl
readonly image=docker.io/library/ubuntu@sha256:52df9b1ee71626e0088f7d400d5c6b5f7bb916f8f0c82b474289a4ece6cf3faf
: "${HEPHAESTUS_LIBKRUN_CGROUP_PARENT:?set a writable delegated cgroup parent}"

for command in podman tar cargo mkfs.ext4 debugfs sha256sum; do
    command -v "${command}" >/dev/null
done
test -r /dev/kvm && test -w /dev/kvm
podman image exists "${image}"

# Build both the initializer and native probe from this checkout; cached guest
# roots are never reused or modified. The worker is rebuilt by cargo test.
cargo build --manifest-path "${repo_root}/Cargo.toml" --release \
    -p vm-libkrun --features integration-guest --target "${guest_target}" \
    --bin heph-init --bin heph-integration-check

fixture_root="$(mktemp -d /tmp/heph-named.XXXXXX)"
container="heph-named-proof-$$"
cgroup_root="${HEPHAESTUS_LIBKRUN_CGROUP_PARENT}/heph-named-proof-$$"
cleanup() {
    podman rm "${container}" >/dev/null 2>&1 || true
    # Remove only the task-owned, empty cgroup root. A leaked VM remains visible.
    if [[ -d "${cgroup_root}" ]] && ! rmdir "${cgroup_root}" 2>/dev/null; then
        printf 'Retaining native fixture with unresolved VM evidence: %s\n' "${fixture_root}" >&2
        return
    fi
    rm -rf -- "${fixture_root}"
}
trap cleanup EXIT
mkdir -p "${fixture_root}/rootfs"
podman create --pull never --name "${container}" "${image}" /bin/true >/dev/null
podman export "${container}" | tar -C "${fixture_root}/rootfs" -xf -
podman rm "${container}" >/dev/null
target_root="${CARGO_TARGET_DIR:-${repo_root}/target}"
install -D -m 0755 "${target_root}/${guest_target}/release/heph-init" \
    "${fixture_root}/rootfs/usr/libexec/hephaestus/heph-init"
# The exact trusted builder path grants root only inside this private fixture.
# The probe needs root to distinguish block-layer RO from ordinary permissions.
install -D -m 0755 "${target_root}/${guest_target}/release/heph-integration-check" \
    "${fixture_root}/rootfs/usr/libexec/hephaestus/oci-build"
mkdir "${cgroup_root}"
printf '+cpu +io +memory +pids\n' >"${cgroup_root}/cgroup.subtree_control"
printf 'NAMED_VOLUME_FIXTURE=1 protocol=10 fresh_init=1 fresh_root=1 pinned_image=%s init_sha256=%s probe_sha256=%s\n' \
    "${image}" "$(sha256sum "${fixture_root}/rootfs/usr/libexec/hephaestus/heph-init" | cut -d ' ' -f 1)" \
    "$(sha256sum "${fixture_root}/rootfs/usr/libexec/hephaestus/oci-build" | cut -d ' ' -f 1)"
HEPHAESTUS_LIBKRUN_NAMED_INTEGRATION=1 \
HEPHAESTUS_LIBKRUN_NAMED_ROOTFS="${fixture_root}/rootfs" \
HEPHAESTUS_LIBKRUN_NAMED_CGROUP_ROOT="${cgroup_root}" \
cargo test --manifest-path "${repo_root}/Cargo.toml" -p vm-libkrun \
    --all-features --test libkrun_integration -- \
    --exact named_volumes::kernel_enforces_named_read_only_and_writable_volume_contracts \
    --nocapture
