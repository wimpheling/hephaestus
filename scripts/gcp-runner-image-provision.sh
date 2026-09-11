#!/usr/bin/env bash
# Shared public pins and image verification contract. The cold-host startup
# remains able to run without this file; baked images install it in
# /usr/local/libexec/hephaestus.
set -Eeuo pipefail

readonly HEPH_IMAGE_RUST_VERSION='1.88.0'
readonly HEPH_IMAGE_LIBKRUN_TAG='v1.19.0'
readonly HEPH_IMAGE_LIBKRUN_REVISION='9932c4b59d8f891e60c6aba20d22ebb99ceaa8e2'
readonly HEPH_IMAGE_LIBKRUNFW_TAG='v5.5.0'
readonly HEPH_IMAGE_PASST_REVISION='386b5f5472b89769c025f5d5056348532a823b93'
readonly HEPH_IMAGE_NODE_VERSION='v24.16.0'
readonly HEPH_IMAGE_NODE_SHA256='d804845d34eddc21dc1092b519d643ef40b1f58ec5dec5c22b1f4bd8fabde6c9'
readonly HEPH_IMAGE_PLAYWRIGHT_VERSION='1.62.0'
readonly HEPH_IMAGE_ORAS_VERSION='1.3.3'
readonly HEPH_IMAGE_ORAS_SHA256='9ce999f8d2de03fc03968b29d743077a58783e545e5eaa53917ca177352d0e59'
readonly HEPH_IMAGE_MANIFEST='/usr/share/hephaestus/runner-image-manifest.json'
readonly HEPH_IMAGE_VERIFIER='/usr/local/libexec/hephaestus/gcp-runner-image-verify.py'

runner_image_verify() {
  [[ -f "$HEPH_IMAGE_MANIFEST" && ! -L "$HEPH_IMAGE_MANIFEST" ]] || return 1
  [[ -f "$HEPH_IMAGE_VERIFIER" && ! -L "$HEPH_IMAGE_VERIFIER" ]] || return 1
  python3 "$HEPH_IMAGE_VERIFIER" "$HEPH_IMAGE_MANIFEST" \
    --rust-version "$HEPH_IMAGE_RUST_VERSION" \
    --libkrun-tag "$HEPH_IMAGE_LIBKRUN_TAG" \
    --libkrun-revision "$HEPH_IMAGE_LIBKRUN_REVISION" \
    --libkrunfw-tag "$HEPH_IMAGE_LIBKRUNFW_TAG" \
    --passt-revision "$HEPH_IMAGE_PASST_REVISION" \
    --node-version "$HEPH_IMAGE_NODE_VERSION" \
    --playwright-version "$HEPH_IMAGE_PLAYWRIGHT_VERSION" \
    --oras-version "$HEPH_IMAGE_ORAS_VERSION" \
    --oras-sha256 "$HEPH_IMAGE_ORAS_SHA256"
}
