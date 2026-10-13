#!/usr/bin/env bash
# Protocol11 ordinary-user proof; separate from privileged named-volume controls.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly repo_root
readonly guest_target=x86_64-unknown-linux-musl
readonly image=docker.io/library/ubuntu@sha256:52df9b1ee71626e0088f7d400d5c6b5f7bb916f8f0c82b474289a4ece6cf3faf
: "${HEPHAESTUS_LIBKRUN_CGROUP_PARENT:?set a writable delegated cgroup parent}"
for command in podman tar cargo mkfs.ext4 debugfs sha256sum timeout install; do
    command -v "${command}" >/dev/null
done
test -r /dev/kvm && test -w /dev/kvm
podman image exists "${image}"

# Use the existing static integration probe, never a new image package or a
# privileged builder path. Cached roots and historical native logs are untouched.
cargo build --manifest-path "${repo_root}/Cargo.toml" --release -p vm-libkrun \
    --features integration-guest --target "${guest_target}" \
    --bin heph-init --bin heph-integration-check
cargo test --manifest-path "${repo_root}/Cargo.toml" -p vm-libkrun \
    --all-features --test libkrun_integration --no-run

fixture_root="$(mktemp -d /tmp/heph-guest-init.XXXXXX)"
container="heph-guest-init-proof-$$"
cgroup_root="${HEPHAESTUS_LIBKRUN_CGROUP_PARENT}/heph-guest-init-proof-$$"
cleanup() {
    podman rm "${container}" >/dev/null 2>&1 || true
    if [[ -d "${cgroup_root}" ]] && ! rmdir "${cgroup_root}" 2>/dev/null; then
        printf 'Retaining unresolved native guest fixture: %s\n' "${fixture_root}" >&2
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
install -D -m 0755 "${target_root}/${guest_target}/release/heph-integration-check" \
    "${fixture_root}/rootfs/usr/libexec/hephaestus/integration-check"
mkdir "${cgroup_root}"
printf '+cpu +io +memory +pids\n' >"${cgroup_root}/cgroup.subtree_control"
printf 'GUEST_INITIALIZATION_FIXTURE=1 protocol=11 fresh_init=1 fresh_root=1 pinned_image=%s init_sha256=%s probe_sha256=%s\n' \
    "${image}" "$(sha256sum "${fixture_root}/rootfs/usr/libexec/hephaestus/heph-init" | cut -d ' ' -f 1)" \
    "$(sha256sum "${fixture_root}/rootfs/usr/libexec/hephaestus/integration-check" | cut -d ' ' -f 1)"
HEPHAESTUS_LIBKRUN_INITIALIZATION_ROOTFS="${fixture_root}/rootfs" \
HEPHAESTUS_LIBKRUN_INITIALIZATION_CGROUP_ROOT="${cgroup_root}" \
timeout --signal=TERM --kill-after=15s 240s \
cargo test --manifest-path "${repo_root}/Cargo.toml" -p vm-libkrun \
    --all-features --test libkrun_integration -- \
    --exact guest_initialization::ordinary_user_typed_builtin_state_and_named_none_survive_restart \
    --ignored --nocapture
