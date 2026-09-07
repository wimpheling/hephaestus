#!/usr/bin/env bash
# Build one reviewed cooking guest builder and emit its OCI manifest digest.
#
# The resulting archive is evidence and a transport artifact.  A digest from a
# local Podman tag is not a registry reference; publish the archive through the
# reviewed platform-image operation before passing its read-back registry
# reference to run.sh.
set -Eeuo pipefail
umask 077

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd -- "${script_dir}/../.." && pwd -P)"

die() {
    printf 'cooking image provision: %s\n' "$*" >&2
    exit 1
}

usage() {
    local status="${1:-64}"
    cat >&2 <<'EOF'
usage: provision-builder-image.sh --builder python-ubuntu|rust-ubuntu \
    --source URI --revision SHA --created RFC3339UTC \
    [--tag LOCAL_IMAGE_TAG] [--archive OCI_ARCHIVE]
EOF
    exit "${status}"
}

builder=''
source=''
revision=''
created=''
tag=''
archive=''
while [[ $# -gt 0 ]]; do
    case "$1" in
        -h|--help) usage 0 ;;
        --builder) builder=${2:-}; shift 2 ;;
        --source) source=${2:-}; shift 2 ;;
        --revision) revision=${2:-}; shift 2 ;;
        --created) created=${2:-}; shift 2 ;;
        --tag) tag=${2:-}; shift 2 ;;
        --archive) archive=${2:-}; shift 2 ;;
        *) usage ;;
    esac
done

[[ "$builder" == python-ubuntu || "$builder" == rust-ubuntu ]] ||
    die '--builder must be python-ubuntu or rust-ubuntu'
[[ "$source" =~ ^[a-zA-Z][a-zA-Z0-9+.-]*://[^[:space:]]+$ ]] ||
    die '--source must be an absolute URI without whitespace'
[[ "$revision" =~ ^[0-9a-f]{40}([0-9a-f]{24})?$ ]] ||
    die '--revision must be an exact lowercase 40- or 64-character commit SHA'
[[ "$created" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]] ||
    die '--created must be an exact RFC3339 UTC timestamp'

command -v podman >/dev/null 2>&1 || die 'required command is unavailable: podman'
command -v skopeo >/dev/null 2>&1 || die 'required command is unavailable: skopeo'

if [[ -z "$tag" ]]; then
    tag="localhost/hephaestus/${builder}:cooking-${revision:0:12}"
fi
[[ "$tag" != *@* ]] || die '--tag must be a local tag, not a digest reference'
if [[ -z "$archive" ]]; then
    archive="$repo_root/.local/cooking-images/${builder}-${revision:0:12}.oci"
fi
[[ "$archive" = /* ]] || die '--archive must be an absolute path'
[[ ! -e "$archive" ]] || die "refusing to overwrite existing archive: $archive"

dockerfile="$repo_root/platform/builders/$builder/Dockerfile"
[[ -f "$dockerfile" && ! -L "$dockerfile" ]] || die "reviewed Dockerfile is missing: $dockerfile"
dockerfile_relative="platform/builders/$builder/Dockerfile"
git -C "$repo_root" cat-file -e "$revision^{commit}" 2>/dev/null ||
    die "--revision is not a commit in this checkout: $revision"
git -C "$repo_root" diff --quiet "$revision" -- "$dockerfile_relative" ||
    die "reviewed Dockerfile differs from --revision; use the commit matching this checkout"
if podman image exists "$tag"; then
    die "refusing to overwrite existing local image tag: $tag"
fi
mkdir -p -- "$(dirname -- "$archive")"

podman build --pull=missing --format oci \
    --file "$dockerfile" \
    --build-arg "SOURCE=$source" \
    --build-arg "REVISION=$revision" \
    --build-arg "CREATED=$created" \
    --tag "$tag" \
    "$repo_root/platform/builders/$builder"
podman push --format oci "$tag" "oci-archive:$archive:$builder"

digest="$(skopeo inspect --format '{{.Digest}}' "oci-archive:$archive")"
[[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] ||
    die "OCI archive did not produce an immutable sha256 manifest digest: $digest"

# Loading the archive records both the local tag and its repository digests.
# A bare `podman build` tag has no RepoDigest and therefore cannot be passed to
# the digest-only cooking preflight.
podman load --input "$archive" >/dev/null
local_reference="localhost/${builder}@${digest}"
podman image exists "$local_reference" ||
    die "Podman did not retain a digest reference for the loaded archive: $local_reference"

printf 'Cooking builder provisioned\n'
printf '  builder: %s\n' "$builder"
printf '  local tag: %s\n' "$tag"
printf '  OCI archive: %s\n' "$archive"
printf '  manifest digest: %s\n' "$digest"
printf '  local digest reference: %s\n' "$local_reference"
printf 'For CI or another host, publish this archive through the reviewed platform-image operation and use its read-back registry reference.\n'
