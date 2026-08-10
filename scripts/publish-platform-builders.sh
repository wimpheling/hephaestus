#!/usr/bin/env bash
# Publish reviewed platform OCI layouts to the forge-owned Zot registry.
set -euo pipefail

readonly builders=(ubuntu-native rust-ubuntu typescript-node-ubuntu python-ubuntu)
readonly script_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
readonly catalog_generator="$script_root/scripts/write-platform-builder-catalog.sh"

die() { printf '%s\n' "$*" >&2; exit 65; }
usage() {
    printf '%s\n' \
        'usage: publish-platform-builders.sh --input-root DIRECTORY --review-output FILE --catalog-output FILE [--dry-run]' \
        '       publish-platform-builders.sh --self-test' >&2
    exit 64
}
is_digest() { [[ "$1" =~ ^sha256:[0-9a-f]{64}$ ]]; }
is_authority() { [[ "$1" =~ ^[a-z0-9]([a-z0-9.-]*[a-z0-9])?(:[1-9][0-9]{0,4})?$ ]]; }
is_absolute_uri() { [[ "$1" =~ ^[a-zA-Z][a-zA-Z0-9+.-]*://[^[:space:]\"\\]+$ ]]; }
is_revision() { [[ "$1" =~ ^[0-9a-f]{40}([0-9a-f]{24})?$ ]]; }
is_created() { [[ "$1" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]]; }
canonical_existing_directory() { [[ "$1" = /* && -d "$1" && ! -L "$1" ]] && realpath -e -- "$1"; }
canonical_new_file_parent() { [[ "$1" = /* && ! -L "$1" ]] && mkdir -p -- "$(dirname -- "$1")" && realpath -e -- "$(dirname -- "$1")"; }
require_private_directory() { local mode; mode=$(stat --format '%a' -- "$1") && (( (8#$mode & 0077) == 0 )); }
require_private_regular_file() { local mode; [[ "$1" = /* && -f "$1" && ! -L "$1" ]] || return 1; mode=$(stat --format '%a' -- "$1") && (( (8#$mode & 0077) == 0 )); }
require_tool() {
    local variable=$1 version_variable=$2 binary expected actual
    binary=${!variable:-}; expected=${!version_variable:-}
    [[ -n "$binary" && -n "$expected" ]] || die "configure $variable and $version_variable"
    [[ "$binary" = /* && -x "$binary" && ! -L "$binary" ]] || die "$variable must name an absolute non-symlink executable"
    binary=$(realpath -e -- "$binary") || die "$variable cannot be resolved"
    actual=$("$binary" --version 2>&1) || die "$variable cannot report its version"
    [[ "$actual" == *"$expected"* ]] || die "$variable version does not contain configured $version_variable"
    printf '%s' "$binary"
}
run_self_test() {
    is_authority registry.forge.example || exit 1
    ! is_authority https://registry.forge.example || exit 1
    is_digest "sha256:$(printf 'a%.0s' {1..64})" || exit 1
    ! is_digest 'sha256:ABC' || exit 1
    is_absolute_uri 'https://forge.example/releases' || exit 1
    is_revision '0123456789abcdef0123456789abcdef01234567' || exit 1
    is_created '2026-08-04T12:34:56Z' || exit 1
    printf '%s\n' 'publish-platform-builders self-test passed'
}

input_root=''; review_output=''; catalog_output=''; dry_run=false; self_test=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --input-root) input_root=${2:-}; shift 2 ;;
        --review-output) review_output=${2:-}; shift 2 ;;
        --catalog-output) catalog_output=${2:-}; shift 2 ;;
        --dry-run) dry_run=true; shift ;;
        --self-test) self_test=true; shift ;;
        *) usage ;;
    esac
done
if "$self_test"; then
    [[ -z "$input_root$review_output$catalog_output" && "$dry_run" == false ]] || usage
    run_self_test; exit 0
fi
[[ -n "$input_root" && -n "$review_output" && -n "$catalog_output" ]] || usage

authority=${HEPHAESTUS_FORGE_REGISTRY_AUTHORITY:-}
is_authority "$authority" || die 'HEPHAESTUS_FORGE_REGISTRY_AUTHORITY must be the exact lowercase forge registry authority, without scheme or path'
[[ ${HEPHAESTUS_REGISTRY_SERVICE:-} == "$authority" ]] || die 'HEPHAESTUS_REGISTRY_SERVICE must exactly match HEPHAESTUS_FORGE_REGISTRY_AUTHORITY'
[[ -n ${HEPHAESTUS_DATABASE_URL:-} ]] || die 'HEPHAESTUS_DATABASE_URL is required by the trusted release command'
private_key=${HEPHAESTUS_REGISTRY_TOKEN_PRIVATE_KEY:-}
require_private_regular_file "$private_key" || die 'HEPHAESTUS_REGISTRY_TOKEN_PRIVATE_KEY must be an absolute private regular file'
[[ -n ${HEPHAESTUS_REGISTRY_TOKEN_ISSUER:-} ]] || die 'HEPHAESTUS_REGISTRY_TOKEN_ISSUER is required by the trusted release command'
[[ -n ${HEPHAESTUS_REGISTRY_TOKEN_KEY_ID:-} ]] || die 'HEPHAESTUS_REGISTRY_TOKEN_KEY_ID is required by the trusted release command'
[[ ${HEPHAESTUS_REGISTRY_TOKEN_LIFETIME_SECONDS:-300} =~ ^[1-9][0-9]*$ ]] || die 'HEPHAESTUS_REGISTRY_TOKEN_LIFETIME_SECONDS must be a positive number of seconds'
skopeo=$(require_tool HEPHAESTUS_SKOPEO HEPHAESTUS_SKOPEO_VERSION)
oras=$(require_tool HEPHAESTUS_ORAS HEPHAESTUS_ORAS_VERSION)
jq_binary=$(require_tool HEPHAESTUS_JQ HEPHAESTUS_JQ_VERSION)
registry_release=$(require_tool HEPHAESTUS_REGISTRY_RELEASE HEPHAESTUS_REGISTRY_RELEASE_VERSION)
root=$(canonical_existing_directory "$input_root") || die '--input-root must be an absolute existing directory'
require_private_directory "$root" || die '--input-root must be private'
[[ -f "$root/.platform-builder-release.json" && ! -L "$root/.platform-builder-release.json" ]] || die 'input root is not a platform-builder release output'
credential_root=$(canonical_existing_directory "${HEPHAESTUS_PLATFORM_CREDENTIAL_ROOT:-}") || die 'HEPHAESTUS_PLATFORM_CREDENTIAL_ROOT must be an absolute existing directory'
require_private_directory "$credential_root" || die 'HEPHAESTUS_PLATFORM_CREDENTIAL_ROOT must be private'
[[ -x "$catalog_generator" ]] || die 'catalog generator is not executable'

for output in "$review_output" "$catalog_output"; do
    parent=$(canonical_new_file_parent "$output") || die 'output paths must be absolute non-symlink paths'
    require_private_directory "$parent" || die 'review and catalog output parents must be private'
    [[ ! -e "$output" ]] || die "refusing to overwrite existing output $output"
done

if "$dry_run"; then
    printf '%s\n' 'dry-run validated the forge-only publication contract; no token, registry, or catalog operation was used'
    exit 0
fi

umask 077
declare -A remote_digests=() sbom_refs=() provenance_refs=() scan_refs=() signature_refs=()
source=''; revision=''; created=''

for key in "${builders[@]}"; do
    metadata="$root/$key/release-input.json"
    [[ -f "$metadata" && ! -L "$metadata" && -d "$root/$key/image" ]] || die "missing release input for $key"
    digest=$("$jq_binary" -er '.manifest_digest' "$metadata") || die "invalid release input for $key"
    layout=$("$jq_binary" -er '.layout' "$metadata") || die "invalid layout declaration for $key"
    layout_tag=$("$jq_binary" -er '.layout_tag' "$metadata") || die "invalid immutable local reference tag for $key"
    [[ "$layout" == "$root/$key/image" && -f "$layout/index.json" && -f "$layout/oci-layout" ]] || die "unsafe or incomplete OCI layout for $key"
    is_digest "$digest" || die "invalid local manifest digest for $key"
    [[ "$layout_tag" == "heph-${digest//:/-}" ]] || die "local reference tag is not the required immutable digest tag for $key"
    [[ $("$jq_binary" -er '.builder' "$metadata") == "$key" ]] || die "builder identity mismatch for $key"
    [[ $("$jq_binary" -er '.architecture' "$metadata") == x86_64 ]] || die "$key claims an unsupported architecture"
    current_source=$("$jq_binary" -er '.source' "$metadata"); current_revision=$("$jq_binary" -er '.revision' "$metadata"); current_created=$("$jq_binary" -er '.created' "$metadata")
    [[ -z "$source" || "$source" == "$current_source" ]] || die 'release inputs disagree on source'
    [[ -z "$revision" || "$revision" == "$current_revision" ]] || die 'release inputs disagree on revision'
    [[ -z "$created" || "$created" == "$current_created" ]] || die 'release inputs disagree on created timestamp'
    source=$current_source; revision=$current_revision; created=$current_created
    is_absolute_uri "$source" || die "invalid source in release input for $key"
    is_revision "$revision" || die "invalid revision in release input for $key"
    is_created "$created" || die "invalid created timestamp in release input for $key"
    declare -a release_arguments=(publish-platform-builder --key "$key" --layout "$layout")
    for kind in sbom provenance scan approval; do
        # An approval artifact is optional under the current policy. `jq -e`
        # returns non-zero for its intentionally empty path, so do not let
        # `set -e` turn that absence into a silent release failure.
        path=$("$jq_binary" -er --arg kind "$kind" '.evidence[$kind].path // empty' "$metadata" || true)
        if [[ -z "$path" ]]; then
            [[ "$kind" == approval ]] || die "missing required $kind evidence for $key"
            continue
        fi
        [[ "$path" != /* && "$path" != *'..'* && -f "$root/$key/$path" && ! -L "$root/$key/$path" ]] || die "unsafe evidence path for $key/$kind"
        expected_file_digest=$("$jq_binary" -er --arg kind "$kind" '.evidence[$kind].digest' "$metadata")
        actual_file_digest=$(sha256sum -- "$root/$key/$path" | awk '{print "sha256:" $1}')
        [[ "$actual_file_digest" == "$expected_file_digest" ]] || die "local $kind evidence digest mismatch for $key"
        type=$("$jq_binary" -er --arg kind "$kind" '.evidence[$kind].artifact_type' "$metadata")
        case "$kind" in
            sbom) [[ "$type" == application/spdx+json ]] ;;
            provenance) [[ "$type" == application/vnd.in-toto+json ]] ;;
            scan) [[ "$type" == application/vnd.hephaestus.vulnerability-scan.v1+json ]] ;;
            approval) [[ "$type" == application/vnd.dev.cosign.simplesigning.v1+json ]] ;;
        esac || die "unexpected artifact type for $key/$kind"
        argument=$kind
        [[ "$kind" == approval ]] && argument=signature
        release_arguments+=("--$argument" "$root/$key/$path")
    done
    publication=$(HEPHAESTUS_REGISTRY_LAYOUT_ROOT="$root" \
        HEPHAESTUS_REGISTRY_CREDENTIAL_ROOT="$credential_root" \
        "$registry_release" "${release_arguments[@]}") || die "controlled registry publication failed for $key"
    [[ $("$jq_binary" -er '.state' <<<"$publication") == approved ]] || die "publication was not approved for $key"
    remote=$("$jq_binary" -er '.manifest_digest' <<<"$publication")
    [[ "$remote" == "$digest" ]] || die "approved Zot digest differs for $key"
    [[ $("$jq_binary" -er '.reference' <<<"$publication") == "$authority/platform/builders/$key@$digest" ]] || die "approved Zot namespace differs for $key"
    remote_digests[$key]=$remote
    for kind in sbom provenance scan signature; do
        referrer=$("$jq_binary" -er --arg kind "$kind" '.evidence[] | select(.kind == $kind) | .reference' <<<"$publication") || true
        [[ -z "$referrer" || "$referrer" =~ @sha256:[0-9a-f]{64}$ ]] || die "invalid approved $kind referrer for $key"
        case "$kind" in
            sbom) sbom_refs[$key]=$referrer ;;
            provenance) provenance_refs[$key]=$referrer ;;
            scan) scan_refs[$key]=$referrer ;;
            signature) signature_refs[$key]=$referrer ;;
        esac
    done
done

for key in "${builders[@]}"; do
    [[ -n ${sbom_refs[$key]:-} && -n ${provenance_refs[$key]:-} && -n ${scan_refs[$key]:-} ]] || die "required evidence was not read back for $key"
done

HEPHAESTUS_PLATFORM_PROVENANCE_SOURCE=$source \
HEPHAESTUS_UBUNTU_SIGNATURE_REFERENCE=${signature_refs[ubuntu-native]:-} \
HEPHAESTUS_UBUNTU_SBOM_REFERENCE=${sbom_refs[ubuntu-native]} \
HEPHAESTUS_RUST_SIGNATURE_REFERENCE=${signature_refs[rust-ubuntu]:-} \
HEPHAESTUS_RUST_SBOM_REFERENCE=${sbom_refs[rust-ubuntu]} \
HEPHAESTUS_TYPESCRIPT_SIGNATURE_REFERENCE=${signature_refs[typescript-node-ubuntu]:-} \
HEPHAESTUS_TYPESCRIPT_SBOM_REFERENCE=${sbom_refs[typescript-node-ubuntu]} \
HEPHAESTUS_PYTHON_SIGNATURE_REFERENCE=${signature_refs[python-ubuntu]:-} \
HEPHAESTUS_PYTHON_SBOM_REFERENCE=${sbom_refs[python-ubuntu]} \
"$catalog_generator" --output "$catalog_output" --registry "$authority/platform/builders" \
    --ubuntu-digest "${remote_digests[ubuntu-native]}" --rust-digest "${remote_digests[rust-ubuntu]}" \
    --typescript-digest "${remote_digests[typescript-node-ubuntu]}" --python-digest "${remote_digests[python-ubuntu]}"

"$jq_binary" -n -S \
    --slurpfile release_input "$root/.platform-builder-release.json" \
    --arg source "$source" --arg revision "$revision" --arg created "$created" --arg authority "$authority" \
    --arg ubuntu "${remote_digests[ubuntu-native]}" --arg ubuntu_sbom "${sbom_refs[ubuntu-native]}" --arg ubuntu_provenance "${provenance_refs[ubuntu-native]}" --arg ubuntu_scan "${scan_refs[ubuntu-native]}" --arg ubuntu_signature "${signature_refs[ubuntu-native]:-}" \
    --arg rust "${remote_digests[rust-ubuntu]}" --arg rust_sbom "${sbom_refs[rust-ubuntu]}" --arg rust_provenance "${provenance_refs[rust-ubuntu]}" --arg rust_scan "${scan_refs[rust-ubuntu]}" --arg rust_signature "${signature_refs[rust-ubuntu]:-}" \
    --arg typescript "${remote_digests[typescript-node-ubuntu]}" --arg typescript_sbom "${sbom_refs[typescript-node-ubuntu]}" --arg typescript_provenance "${provenance_refs[typescript-node-ubuntu]}" --arg typescript_scan "${scan_refs[typescript-node-ubuntu]}" --arg typescript_signature "${signature_refs[typescript-node-ubuntu]:-}" \
    --arg python "${remote_digests[python-ubuntu]}" --arg python_sbom "${sbom_refs[python-ubuntu]}" --arg python_provenance "${provenance_refs[python-ubuntu]}" --arg python_scan "${scan_refs[python-ubuntu]}" --arg python_signature "${signature_refs[python-ubuntu]:-}" \
    'def optional: if length > 0 then . else null end; {schema_version:1,kind:"hephaestus.platform-builder.release-review.v1",source:$source,revision:$revision,created:$created,authority:$authority,architecture:"x86_64",toolchain:$release_input[0].toolchain,policy_result:"evidence_attached_and_read_back",builders:[{key:"ubuntu-native",reference:($authority+"/platform/builders/ubuntu-native@"+$ubuntu),evidence:{sbom:$ubuntu_sbom,provenance:$ubuntu_provenance,scan:$ubuntu_scan,signature_or_approval:($ubuntu_signature | optional)}},{key:"rust-ubuntu",reference:($authority+"/platform/builders/rust-ubuntu@"+$rust),evidence:{sbom:$rust_sbom,provenance:$rust_provenance,scan:$rust_scan,signature_or_approval:($rust_signature | optional)}},{key:"typescript-node-ubuntu",reference:($authority+"/platform/builders/typescript-node-ubuntu@"+$typescript),evidence:{sbom:$typescript_sbom,provenance:$typescript_provenance,scan:$typescript_scan,signature_or_approval:($typescript_signature | optional)}},{key:"python-ubuntu",reference:($authority+"/platform/builders/python-ubuntu@"+$python),evidence:{sbom:$python_sbom,provenance:$python_provenance,scan:$python_scan,signature_or_approval:($python_signature | optional)}}]}' \
    >"$review_output"
printf '%s\n' "published four platform builders to $authority; review $review_output before applying $catalog_output"
