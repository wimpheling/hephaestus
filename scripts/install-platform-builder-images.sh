#!/usr/bin/env bash
# Explicitly publish a reviewed platform-builder release and apply its catalog.
# This script runs only inside the pinned platform-tools container.
set -Eeuo pipefail

die() { printf '%s\n' "$*" >&2; exit 65; }
[[ $# == 1 && $1 =~ ^[0-9a-f]{40}([0-9a-f]{24})?$ ]] || die 'usage: install-platform-builder-images.sh REVISION'
readonly revision=$1
readonly release_root="${HEPHAESTUS_PLATFORM_RELEASE_ROOT:-}"
readonly install_root="/install/${revision}"
[[ "$release_root" = /* && "${release_root##*/}" == "$revision" && -d $release_root && -f $release_root/.platform-builder-release.json ]] || die 'reviewed release is missing'
[[ ! -e $install_root ]] || die 'installation revision already exists; refuse to overwrite immutable state'
[[ -r /secrets/registry-token-signing-key.pem ]] || die 'registry signing key is unavailable'
[[ -x /tools/hephaestus-registry-release && -x /tools/hephaestus-operator ]] || die 'required reviewed host binaries are unavailable'

umask 077
mkdir -p "$install_root" /state/credentials /state/tools
printf '%s\n' '#!/bin/sh' 'if [ "$1" = --version ]; then exec /usr/local/bin/oras version; fi' \
    'exec /usr/local/bin/oras "$@"' > /state/tools/oras
chmod 0500 /state/tools/oras

export HEPHAESTUS_FORGE_REGISTRY_AUTHORITY HEPHAESTUS_REGISTRY_SERVICE
export HEPHAESTUS_REGISTRY_PRIVATE_ORIGIN HEPHAESTUS_DATABASE_URL
export HEPHAESTUS_REGISTRY_TOKEN_PRIVATE_KEY=/secrets/registry-token-signing-key.pem
export HEPHAESTUS_REGISTRY_TOKEN_ISSUER HEPHAESTUS_REGISTRY_TOKEN_KEY_ID HEPHAESTUS_REGISTRY_TOKEN_LIFETIME_SECONDS
export HEPHAESTUS_PLATFORM_CREDENTIAL_ROOT=/state/credentials
export HEPHAESTUS_SKOPEO=/usr/bin/skopeo HEPHAESTUS_SKOPEO_VERSION='skopeo version 1.22.2'
export HEPHAESTUS_ORAS=/state/tools/oras HEPHAESTUS_ORAS_VERSION='Version:        1.3.3'
export HEPHAESTUS_JQ=/usr/bin/jq HEPHAESTUS_JQ_VERSION='jq-1.8.1'
export HEPHAESTUS_REGISTRY_RELEASE=/tools/hephaestus-registry-release
export HEPHAESTUS_REGISTRY_RELEASE_VERSION
export HEPHAESTUS_REGISTRY_LAYOUT_ROOT="$release_root"
export HEPHAESTUS_REGISTRY_CREDENTIAL_ROOT=/state/credentials

/workspace/scripts/publish-platform-builders.sh \
    --input-root "$release_root" \
    --review-output "$install_root/review.json" \
    --catalog-output "$install_root/catalog.json"
/tools/hephaestus-operator provision-builder-catalog "$install_root/catalog.json" >"$install_root/catalog-apply.json"
printf '%s\n' "published and cataloged four approved platform builders for $revision"
