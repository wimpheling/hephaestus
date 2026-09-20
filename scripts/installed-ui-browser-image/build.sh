#!/usr/bin/env bash
set -Eeuo pipefail

readonly image_tag='localhost/hephestus-playwright:1.62.0-certutil'
readonly base_image='mcr.microsoft.com/playwright@sha256:02bbb2155cd7109e3e9c741941097ed1608cf8b6fa44ee2595896da2bdc1f471'
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"

podman pull --platform linux/amd64 "${base_image}"
podman build --pull=never --format oci \
    --file "${script_dir}/Dockerfile" \
    --tag "${image_tag}" \
    "${script_dir}"

podman run --rm --entrypoint sh "${image_tag}" -c '
    set -eu
    node --version
    npx --version
    command -v certutil
    browser="$(find /ms-playwright -type f \( -name chrome -o -name chrome-headless-shell \) -perm -0100 -print -quit)"
    test -n "$browser"
    "$browser" --version
    certutil -H >/dev/null
'

printf 'HEPHAESTUS_PLAYWRIGHT_IMAGE=%s\n' "${image_tag}"
