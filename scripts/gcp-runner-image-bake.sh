#!/usr/bin/env bash
# Provision a disposable builder for a versioned runner image. This script
# performs no GCE calls; the publisher creates an image from the stopped disk.
set -Eeuo pipefail
umask 077

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=gcp-runner-image-provision.sh
source "$script_dir/gcp-runner-image-provision.sh"
# readonly shell variables are intentionally exported only to the manifest
# generator; the child provisioning commands receive explicit arguments.
export HEPH_IMAGE_RUST_VERSION HEPH_IMAGE_LIBKRUN_TAG HEPH_IMAGE_LIBKRUN_REVISION
export HEPH_IMAGE_LIBKRUNFW_TAG HEPH_IMAGE_PASST_REVISION HEPH_IMAGE_NODE_VERSION
export HEPH_IMAGE_NODE_SHA256 HEPH_IMAGE_PLAYWRIGHT_VERSION HEPH_IMAGE_ORAS_VERSION
export HEPH_IMAGE_ORAS_SHA256
repo_sha="${HEPH_GCP_IMAGE_BAKE_REPO_SHA:-}"
[[ "$repo_sha" =~ ^[0-9a-f]{40}$ ]] || {
  printf '%s\n' 'HEPH_GCP_RUNNER_IMAGE bake requires HEPH_GCP_IMAGE_BAKE_REPO_SHA' >&2
  exit 2
}
[[ "$(id -u)" == 0 ]] || { printf '%s\n' 'image bake must run as root' >&2; exit 2; }

install -d -m 0755 /usr/local/libexec/hephaestus /usr/share/hephaestus /etc/hephaestus
install -m 0755 "$script_dir/gcp-runner-image-provision.sh" /usr/local/libexec/hephaestus/gcp-runner-image-provision.sh
install -m 0755 "$script_dir/gcp-runner-image-verify.py" /usr/local/libexec/hephaestus/gcp-runner-image-verify.py
install -m 0755 "$script_dir/gcp-runner-image-manifest.py" /usr/local/libexec/hephaestus/gcp-runner-image-manifest.py
install -m 0755 "$script_dir/gcp-kvm-startup.sh" /usr/local/libexec/hephaestus/gcp-kvm-startup.sh
install -m 0755 "$script_dir/gcp-runner-image-bake.sh" /usr/local/libexec/hephaestus/gcp-runner-image-bake.sh
install -m 0644 /dev/null /etc/hephaestus/runner-image-required

# Reuse the exact package/source build phases in the GCE startup script. Bake
# mode exits after libkrun and before checkout or test state is created.
export HEPH_GCP_IMAGE_BAKE=1
export HEPH_GCP_IMAGE_BAKE_HELPER=/usr/local/libexec/hephaestus/gcp-runner-image-provision.sh
export HEPH_GCP_IMAGE_BAKE_REPO_SHA="$repo_sha"
export HEPH_GCP_LOCAL_PASST_PREFLIGHT="$script_dir/gcp-passt-preflight.sh"
bash "$script_dir/gcp-kvm-startup.sh"

work_root=/srv/hephaestus
checkout="$work_root/image-browser-checkout"
browser_root="$work_root/playwright-browsers"
node_version="$HEPH_IMAGE_NODE_VERSION"
node_sha256="$HEPH_IMAGE_NODE_SHA256"
oras_version=1.3.3
oras_sha256=9ce999f8d2de03fc03968b29d743077a58783e545e5eaa53917ca177352d0e59
install -d -m 0700 -o forge -g forge "$work_root" "$browser_root"
runuser -u forge -- env HOME=/home/forge GIT_TERMINAL_PROMPT=0 \
  git clone --filter=blob:none --no-checkout https://github.com/wimpheling/hephaestus.git "$checkout"
runuser -u forge -- git -C "$checkout" fetch --depth 1 origin "$repo_sha"
runuser -u forge -- git -C "$checkout" checkout --detach "$repo_sha"
[[ "$(runuser -u forge -- git -C "$checkout" rev-parse HEAD)" == "$repo_sha" ]] || exit 1

archive="$work_root/node-${node_version}-linux-x64.tar.xz"
stage="$work_root/node-stage"
curl --fail --location --silent --show-error --retry 3 \
  --output "$archive" "https://nodejs.org/dist/${node_version}/node-${node_version}-linux-x64.tar.xz"
[[ "$(sha256sum "$archive" | awk '{print $1}')" == "$node_sha256" ]] || {
  printf '%s\n' 'Node checksum mismatch' >&2
  exit 1
}
install -d -m 0700 "$stage"
tar -xJf "$archive" -C "$stage" --strip-components=1
install -d -m 0755 /opt/hephaestus
cp -a --no-preserve=ownership "$stage" "/opt/hephaestus/node-${node_version}"
ln -sfn "/opt/hephaestus/node-${node_version}/bin/node" /usr/local/bin/node
ln -sfn "/opt/hephaestus/node-${node_version}/bin/npm" /usr/local/bin/npm
ln -sfn "/opt/hephaestus/node-${node_version}/bin/npx" /usr/local/bin/npx
chown -R forge:forge "$checkout" "$browser_root"
runuser -u forge -- env HOME=/home/forge XDG_RUNTIME_DIR=/run/user/10001 \
  PATH=/opt/hephaestus/node-${node_version}/bin:/usr/local/bin:/usr/bin:/bin \
  PLAYWRIGHT_BROWSERS_PATH="$browser_root" bash -Eeuo pipefail -c \
  'cd "$1/e2e/playwright" && npm ci && npx playwright install chromium' -- "$checkout"
env PATH=/opt/hephaestus/node-${node_version}/bin:/usr/local/bin:/usr/bin:/bin \
  bash -Eeuo pipefail -c 'cd "$1/e2e/playwright" && npx playwright install-deps chromium' -- "$checkout"
chown -R forge:forge "$browser_root"
browser_executable="$(find "$browser_root" -type f \( -name chrome-headless-shell -o -name chrome \) -perm -0100 -print -quit 2>/dev/null)"
[[ -n "$browser_executable" ]] || { printf '%s\n' 'Chromium executable was not installed' >&2; exit 1; }
browser_version="$($browser_executable --version 2>/dev/null || true)"
[[ -n "$browser_version" ]] || { printf '%s\n' 'Chromium executable cannot report its version' >&2; exit 1; }

oras_archive="$work_root/oras-${oras_version}.tar.gz"
oras_stage="$work_root/oras-stage"
curl --fail --location --silent --show-error --retry 3 \
  --output "$oras_archive" \
  "https://github.com/oras-project/oras/releases/download/v${oras_version}/oras_${oras_version}_linux_amd64.tar.gz"
[[ "$(sha256sum "$oras_archive" | awk '{print $1}')" == "$oras_sha256" ]] || {
  printf '%s\n' 'ORAS checksum mismatch' >&2
  exit 1
}
install -d -m 0700 "$oras_stage"
tar -xzf "$oras_archive" -C "$oras_stage" --no-same-owner --no-same-permissions oras
[[ -x "$oras_stage/oras" ]] || { printf '%s\n' 'ORAS archive is missing its executable' >&2; exit 1; }
install -m 0555 "$oras_stage/oras" /usr/local/bin/oras

lock_sha="$(sha256sum "$checkout/e2e/playwright/package-lock.json" | awk '{print $1}')"
recipe_sha="$(sha256sum /usr/local/libexec/hephaestus/gcp-runner-image-provision.sh | awk '{print $1}')"
startup_sha="$(sha256sum "$script_dir/gcp-kvm-startup.sh" | awk '{print $1}')"
bake_sha="$(sha256sum "$script_dir/gcp-runner-image-bake.sh" | awk '{print $1}')"
verifier_sha="$(sha256sum /usr/local/libexec/hephaestus/gcp-runner-image-verify.py | awk '{print $1}')"
manifest_generator_sha="$(sha256sum /usr/local/libexec/hephaestus/gcp-runner-image-manifest.py | awk '{print $1}')"
python3 "$script_dir/gcp-runner-image-manifest.py" \
  --output /usr/share/hephaestus/runner-image-manifest.json \
  --repository-sha "$repo_sha" --browser-lock-sha256 "$lock_sha" \
  --browser-version "$browser_version" \
  --recipe-sha256 "$recipe_sha" --startup-sha256 "$startup_sha" \
  --bake-sha256 "$bake_sha" --verifier-sha256 "$verifier_sha" \
  --manifest-generator-sha256 "$manifest_generator_sha"
python3 /usr/local/libexec/hephaestus/gcp-runner-image-verify.py /usr/share/hephaestus/runner-image-manifest.json \
  --rust-version "$HEPH_IMAGE_RUST_VERSION" --libkrun-tag "$HEPH_IMAGE_LIBKRUN_TAG" \
  --libkrun-revision "$HEPH_IMAGE_LIBKRUN_REVISION" --libkrunfw-tag "$HEPH_IMAGE_LIBKRUNFW_TAG" \
  --passt-revision "$HEPH_IMAGE_PASST_REVISION" --node-version "$HEPH_IMAGE_NODE_VERSION" \
  --playwright-version "$HEPH_IMAGE_PLAYWRIGHT_VERSION" --oras-version "$HEPH_IMAGE_ORAS_VERSION" \
  --oras-sha256 "$HEPH_IMAGE_ORAS_SHA256"
fingerprint="$(python3 -c 'import json; print(json.load(open("/usr/share/hephaestus/runner-image-manifest.json"))["manifest_sha256"])')"

# Keep installed host tools and browser binaries; remove checkout, sources,
# downloads, transient build state, package indexes, and user cache.
rm -rf -- "$checkout" "$stage" "$archive" "$oras_stage" "$oras_archive" /srv/hephaestus/src /srv/hephaestus/tmp
rm -rf -- /var/lib/apt/lists/* /root/.cache /home/forge/.cache/npm \
  /home/forge/.npm /home/forge/.cargo/registry /home/forge/.cargo/git
rm -rf -- /var/log/hephaestus/* /tmp/hephaestus-libkrun/*
find /home/forge -maxdepth 2 -type f -name '.bash_history' -delete
printf '%s\n' 'HEPH_GCP_RUNNER_IMAGE bake=pass'
printf 'HEPH_GCP_RUNNER_IMAGE: READY fingerprint=%s\n' "$fingerprint"
