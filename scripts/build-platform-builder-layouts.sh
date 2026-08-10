#!/usr/bin/env bash
# Build the reviewed platform builders into administrator-owned OCI layouts.
#
# This is intentionally a release operation, not a general Dockerfile runner:
# it accepts no caller-selected build context, image name, architecture, or
# network policy.  Publishing is a separate trusted operation.
set -euo pipefail

readonly builders=(ubuntu-native rust-ubuntu typescript-node-ubuntu python-ubuntu)
readonly script_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
readonly source_root="$script_root/platform/builders"

die() { printf '%s\n' "$*" >&2; exit 65; }

usage() {
    printf '%s\n' \
        'usage: build-platform-builder-layouts.sh --output-root DIRECTORY --source URI --revision SHA256 --created RFC3339UTC [--dry-run]' \
        '       build-platform-builder-layouts.sh --self-test' >&2
    exit 64
}

is_absolute_uri() {
    [[ "$1" =~ ^[a-zA-Z][a-zA-Z0-9+.-]*://[^[:space:]\"\\]+$ ]]
}

is_revision() {
    [[ "$1" =~ ^[0-9a-f]{40}([0-9a-f]{24})?$ ]]
}

is_created() {
    [[ "$1" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]]
}

is_digest() {
    [[ "$1" =~ ^sha256:[0-9a-f]{64}$ ]]
}

canonical_existing_directory() {
    local path=$1
    [[ "$path" = /* && -d "$path" && ! -L "$path" ]] || return 1
    realpath -e -- "$path"
}

canonical_new_directory() {
    local path=$1
    [[ "$path" = /* && ! -L "$path" ]] || return 1
    mkdir -p -- "$path"
    canonical_existing_directory "$path"
}

require_private_directory() {
    local path=$1 mode
    mode=$(stat --format '%a' -- "$path") || return 1
    (( (8#$mode & 0077) == 0 ))
}

require_tool() {
    local variable=$1 version_variable=$2 binary expected actual
    binary=${!variable:-}
    expected=${!version_variable:-}
    [[ -n "$binary" && -n "$expected" ]] || die "configure $variable and $version_variable"
    [[ "$binary" = /* && ! -L "$binary" && -x "$binary" ]] || die "$variable must name an absolute non-symlink executable"
    binary=$(realpath -e -- "$binary") || die "$variable cannot be resolved"
    actual=$("$binary" --version 2>&1) || die "$variable cannot report its version"
    [[ "$actual" == *"$expected"* ]] || die "$variable version does not contain configured $version_variable"
    printf '%s' "$binary"
}

write_provenance() {
    local output=$1 key=$2 digest=$3 source=$4 revision=$5 created=$6 jq_binary=$7
    "$jq_binary" -n -S \
        --arg source "$source" \
        --arg revision "$revision" \
        --arg created "$created" \
        --arg builder "$key" \
        --arg digest "$digest" \
        '{"_type":"https://in-toto.io/Statement/v1", "subject":[{"name":$builder,"digest":{"sha256":($digest | ltrimstr("sha256:"))}}], "predicateType":"https://slsa.dev/provenance/v1", "predicate":{"buildDefinition":{"buildType":"https://hephaestus.dev/platform-builder/v1", "externalParameters":{"source":$source,"revision":$revision,"created":$created,"architecture":"amd64"}}, "runDetails":{"builder":{"id":"https://hephaestus.dev/platform-builder-release"}}}}' \
        >"$output"
}

sha256_file() {
    sha256sum -- "$1" | awk '{print "sha256:" $1}'
}

normalize_single_platform_layout() {
    local source_layout=$1 output_layout=$2 jq_binary=$3
    local inner_digest inner_size inner_type inner_hex inner_blob wrapper wrapper_digest wrapper_hex tag
    [[ -f "$source_layout/index.json" && -f "$source_layout/oci-layout" ]] || return 1
    inner_digest=$("$jq_binary" -er '
        if (.schemaVersion == 2 and (.manifests | type) == "array" and (.manifests | length) == 1)
        then .manifests[0].digest else error("expected one Buildah manifest") end
    ' "$source_layout/index.json") || return 1
    inner_size=$("$jq_binary" -er '.manifests[0].size' "$source_layout/index.json") || return 1
    inner_type=$("$jq_binary" -er '.manifests[0].mediaType' "$source_layout/index.json") || return 1
    is_digest "$inner_digest" || return 1
    [[ "$inner_type" == application/vnd.oci.image.manifest.v1+json && "$inner_size" =~ ^[1-9][0-9]*$ ]] || return 1
    inner_hex=${inner_digest#sha256:}
    inner_blob="$source_layout/blobs/sha256/$inner_hex"
    [[ -f "$inner_blob" && ! -L "$inner_blob" && $(stat --format '%s' -- "$inner_blob") == "$inner_size" ]] || return 1
    [[ $(sha256_file "$inner_blob") == "$inner_digest" ]] || return 1

    mkdir -p -- "$output_layout"
    cp --archive -- "$source_layout/." "$output_layout/"
    wrapper="$output_layout/.manifest-index.json"
    "$jq_binary" -n -S --arg digest "$inner_digest" --arg type "$inner_type" --argjson size "$inner_size" \
        '{schemaVersion:2,mediaType:"application/vnd.oci.image.index.v1+json",manifests:[{mediaType:$type,digest:$digest,size:$size,platform:{os:"linux",architecture:"amd64"}}]}' \
        >"$wrapper"
    wrapper_digest=$(sha256_file "$wrapper")
    wrapper_hex=${wrapper_digest#sha256:}
    tag="heph-${wrapper_digest//:/-}"
    cp -- "$wrapper" "$output_layout/blobs/sha256/$wrapper_hex"
    "$jq_binary" -n -S --arg digest "$wrapper_digest" --argjson size "$(stat --format '%s' -- "$wrapper")" --arg tag "$tag" \
        '{schemaVersion:2,manifests:[{mediaType:"application/vnd.oci.image.index.v1+json",digest:$digest,size:$size,annotations:{"org.opencontainers.image.ref.name":$tag}}]}' \
        >"$output_layout/index.tmp"
    mv -- "$output_layout/index.tmp" "$output_layout/index.json"
    rm -- "$wrapper"
    printf '%s\t%s\n' "$wrapper_digest" "$tag"
}

run_self_test() {
    is_absolute_uri 'https://forge.example/releases' || exit 1
    ! is_absolute_uri 'forge.example/releases' || exit 1
    is_revision '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' || exit 1
    ! is_revision 'not-a-revision' || exit 1
    is_created '2026-08-04T12:34:56Z' || exit 1
    ! is_created '2026-08-04T12:34:56+00:00' || exit 1
    is_digest "sha256:$(printf 'a%.0s' {1..64})" || exit 1
    ! is_digest 'sha512:deadbeef' || exit 1
    printf '%s\n' 'build-platform-builder-layouts self-test passed'
}

output_root=''
source=''
revision=''
created=''
dry_run=false
self_test=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --output-root) output_root=${2:-}; shift 2 ;;
        --source) source=${2:-}; shift 2 ;;
        --revision) revision=${2:-}; shift 2 ;;
        --created) created=${2:-}; shift 2 ;;
        --dry-run) dry_run=true; shift ;;
        --self-test) self_test=true; shift ;;
        *) usage ;;
    esac
done

if "$self_test"; then
    [[ -z "$output_root$source$revision$created" && "$dry_run" == false ]] || usage
    run_self_test
    exit 0
fi

[[ -n "$output_root" && -n "$source" && -n "$revision" && -n "$created" ]] || usage
is_absolute_uri "$source" || die '--source must be an absolute URI without whitespace'
is_revision "$revision" || die '--revision must be an exact lowercase 40- or 64-character commit SHA'
is_created "$created" || die '--created must be an exact RFC3339 UTC timestamp (YYYY-MM-DDTHH:MM:SSZ)'
[[ $(uname -m) == x86_64 ]] || die 'platform builders currently support only an x86_64 build host; arm64 is not produced or claimed'

buildah=$(require_tool HEPHAESTUS_BUILDAH HEPHAESTUS_BUILDAH_VERSION)
skopeo=$(require_tool HEPHAESTUS_SKOPEO HEPHAESTUS_SKOPEO_VERSION)
syft=$(require_tool HEPHAESTUS_SYFT HEPHAESTUS_SYFT_VERSION)
trivy=$(require_tool HEPHAESTUS_TRIVY HEPHAESTUS_TRIVY_VERSION)
jq_binary=$(require_tool HEPHAESTUS_JQ HEPHAESTUS_JQ_VERSION)

umask 077
if "$dry_run"; then
    [[ "$output_root" = /* && ! -L "$output_root" ]] || die '--output-root must be an absolute non-symlink directory path'
    parent=$(canonical_existing_directory "$(dirname -- "$output_root")") || die '--output-root parent must already exist'
    require_private_directory "$parent" || die '--output-root parent must not be accessible to group or other users'
    [[ ! -e "$output_root" ]] || die '--dry-run requires a fresh output root, as does a real release'
else
    root=$(canonical_new_directory "$output_root") || die '--output-root must be an absolute directory path'
    require_private_directory "$root" || die '--output-root must not be accessible to group or other users'
    [[ "$root" != / && "$root" != "$script_root" ]] || die '--output-root must be a dedicated private directory'
    [[ -z $(find "$root" -mindepth 1 -maxdepth 1 -print -quit) ]] || die '--output-root must be a fresh empty directory'
fi

rootless=$(
    "$buildah" info --format '{{.host.rootless}}' 2>/dev/null || true
)
[[ "$rootless" == true ]] || die 'configured Buildah must run rootless'

if "$dry_run"; then
    printf '%s\n' 'dry-run validated the release contract; no layout, image, evidence, or network operation was created'
    exit 0
fi

declare -a local_images=()
cleanup() {
    local image
    for image in "${local_images[@]:-}"; do
        "$buildah" rmi -- "$image" >/dev/null 2>&1 || true
    done
}
trap cleanup EXIT

for key in "${builders[@]}"; do
    context="$source_root/$key"
    [[ -d "$context" && -f "$context/Dockerfile" && ! -L "$context" ]] || die "missing reviewed Dockerfile for $key"
    destination="$root/$key"
    [[ ! -e "$destination" ]] || die "refusing to overwrite existing output $destination"
    mkdir -p -- "$destination/evidence"
    image="heph-platform-${key}-${revision:0:12}"
    local_images+=("$image")

    "$buildah" bud \
        --arch amd64 \
        --format oci \
        --pull-never \
        --layers=false \
        --build-arg "SOURCE=$source" \
        --build-arg "REVISION=$revision" \
        --build-arg "CREATED=$created" \
        --tag "$image" \
        "$context"
    source_layout="$destination/source-layout"
    "$buildah" push --format oci "$image" "oci:$source_layout:release"
    normalized=$(normalize_single_platform_layout "$source_layout" "$destination/image" "$jq_binary") || die "could not create a canonical OCI index for $key"
    digest=${normalized%%$'\t'*}
    layout_tag=${normalized#*$'\t'}
    is_digest "$digest" || die "could not obtain a canonical OCI index digest for $key"
    [[ "$layout_tag" == "heph-${digest//:/-}" ]] || die "could not create the immutable local reference tag for $key"

    read_back_digest=$("$skopeo" inspect --format '{{.Digest}}' "oci:$destination/image:$layout_tag")
    [[ "$read_back_digest" == "$digest" ]] || die "could not read back canonical OCI index digest for $key"
    architecture=$("$skopeo" inspect --format '{{.Os}}/{{.Architecture}}' "oci:$destination/image:$layout_tag")
    [[ "$architecture" == linux/amd64 ]] || die "$key did not produce linux/amd64"

    "$syft" "oci-dir:$destination/image" --output "spdx-json=$destination/evidence/sbom.spdx.json"
    # Preserve the complete vulnerability observation as release evidence, then
    # evaluate the release policy separately. Unfixed findings stay visible in
    # the evidence but cannot be remediated by rebuilding this image; the gate
    # rejects every fixable high or critical finding.
    "$trivy" image --input "$destination/image" --offline-scan --skip-db-update --skip-java-db-update \
        --scanners vuln --exit-code 0 \
        --format json --output "$destination/evidence/vulnerability-scan.json"
    "$trivy" image --input "$destination/image" --offline-scan --skip-db-update --skip-java-db-update \
        --scanners vuln --ignore-unfixed --exit-code 1 --severity HIGH,CRITICAL \
        --format json --output "$destination/evidence/vulnerability-policy.json"
    write_provenance "$destination/evidence/provenance.intoto.json" "$key" "$digest" "$source" "$revision" "$created" "$jq_binary"

    approval=''
    if [[ -n ${HEPHAESTUS_PLATFORM_APPROVAL_DIRECTORY:-} ]]; then
        approval_root=$(canonical_existing_directory "$HEPHAESTUS_PLATFORM_APPROVAL_DIRECTORY") || die 'HEPHAESTUS_PLATFORM_APPROVAL_DIRECTORY must be an absolute existing directory'
        require_private_directory "$approval_root" || die 'HEPHAESTUS_PLATFORM_APPROVAL_DIRECTORY must be private'
        approval_input="$approval_root/$key.json"
        [[ -f "$approval_input" && ! -L "$approval_input" ]] || die "missing approval input $approval_input"
        cosign=$(require_tool HEPHAESTUS_COSIGN HEPHAESTUS_COSIGN_VERSION)
        approval_key=${HEPHAESTUS_PLATFORM_APPROVAL_PUBLIC_KEY:-}
        [[ "$approval_key" = /* && -f "$approval_key" && ! -L "$approval_key" ]] || die 'HEPHAESTUS_PLATFORM_APPROVAL_PUBLIC_KEY must be an absolute regular file'
        "$cosign" verify-blob --key "$approval_key" "$approval_input" >/dev/null
        cp --no-preserve=mode "$approval_input" "$destination/evidence/approval.json"
        approval=approval.json
    fi

    "$jq_binary" -n -S \
        --arg builder "$key" --arg source "$source" --arg revision "$revision" --arg created "$created" \
        --arg digest "$digest" --arg architecture x86_64 --arg layout "$destination/image" --arg layout_tag "$layout_tag" \
        --arg sbom "$(sha256_file "$destination/evidence/sbom.spdx.json")" \
        --arg provenance "$(sha256_file "$destination/evidence/provenance.intoto.json")" \
        --arg scan "$(sha256_file "$destination/evidence/vulnerability-scan.json")" \
        --arg scan_policy "$(sha256_file "$destination/evidence/vulnerability-policy.json")" \
        --arg approval "$approval" \
        '{schema_version:1,builder:$builder,source:$source,revision:$revision,created:$created,manifest_digest:$digest,architecture:$architecture,layout:$layout,layout_tag:$layout_tag,evidence:{sbom:{path:"evidence/sbom.spdx.json",digest:$sbom,artifact_type:"application/spdx+json"},provenance:{path:"evidence/provenance.intoto.json",digest:$provenance,artifact_type:"application/vnd.in-toto+json"},scan:{path:"evidence/vulnerability-scan.json",digest:$scan,artifact_type:"application/vnd.hephaestus.vulnerability-scan.v1+json",policy:{name:"no_fixable_high_or_critical",result:"passed",path:"evidence/vulnerability-policy.json",digest:$scan_policy}}} + (if $approval == "" then {} else {approval:{path:("evidence/" + $approval),digest:null,artifact_type:"application/vnd.dev.cosign.simplesigning.v1+json"}} end)}' \
        >"$destination/release-input.json"
    if [[ -n "$approval" ]]; then
        approval_digest=$(sha256_file "$destination/evidence/approval.json")
        "$jq_binary" --arg digest "$approval_digest" '.evidence.approval.digest = $digest' "$destination/release-input.json" >"$destination/release-input.tmp"
        mv -- "$destination/release-input.tmp" "$destination/release-input.json"
    fi
done

"$jq_binary" -n -S \
    --arg source "$source" --arg revision "$revision" --arg created "$created" \
    --arg buildah "$($buildah --version 2>&1 | head -n 1)" \
    --arg skopeo "$($skopeo --version 2>&1 | head -n 1)" \
    --arg syft "$($syft --version 2>&1 | head -n 1)" \
    --arg trivy "$($trivy --version 2>&1 | head -n 1)" \
    '{schema_version:1,kind:"hephaestus.platform-builder.release-input.v1",source:$source,revision:$revision,created:$created,architecture:"x86_64",toolchain:{buildah:$buildah,skopeo:$skopeo,syft:$syft,trivy:$trivy},builders:["ubuntu-native","rust-ubuntu","typescript-node-ubuntu","python-ubuntu"]}' \
    >"$root/.platform-builder-release.json"
printf '%s\n' "built private OCI layouts under $root; pass this directory to publish-platform-builders.sh"
