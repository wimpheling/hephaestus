#!/usr/bin/env bash
# Verify the vendored Hugo input used by the offline project image build.
set -Eeuo pipefail

[[ "$#" -le 1 ]] || { printf 'usage: %s [output-binary]\n' "$0" >&2; exit 2; }

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source_archive="${script_dir}/vendor/hugo-0.165.0-source.tar.gz"
module_archive="${script_dir}/vendor/hugo-0.165.0-go-vendor.tar.gz"
go_archive="${script_dir}/vendor/go1.27.1.linux-amd64.tar.gz"
dependency_patch="${script_dir}/vendor/hugo-0.165.0-dependencies.patch"
expected_binary="${script_dir}/vendor/hugo-0.165.0-rebuilt"
builder="${script_dir}/vendor/build-hugo.sh"
temporary="$(mktemp -d "${TMPDIR:-/tmp}/cooking-hugo.XXXXXX")"
trap 'rm -rf -- "${temporary}"' EXIT

[[ -x "${builder}" ]] || { printf 'Hugo builder is not executable: %s\n' "$builder" >&2; exit 1; }
for input in "${source_archive}" "${module_archive}" "${go_archive}" "${dependency_patch}"; do
    [[ -f "${input}" ]] || {
        printf 'missing pinned Hugo build input: %s\n' "${input}" >&2
        exit 1
    }
done
[[ -x "${expected_binary}" ]] || {
    printf 'missing verified Hugo executable: %s\n' "${expected_binary}" >&2
    exit 1
}

build() {
    local output="$1"
    HUGO_SOURCE_ARCHIVE="${source_archive}" \
    HUGO_MODULE_ARCHIVE="${module_archive}" \
    HUGO_GO_ARCHIVE="${go_archive}" \
    HUGO_DEPENDENCY_PATCH="${dependency_patch}" \
    HUGO_BUILD_ROOT="${temporary}/source-${output}" \
    HUGO_GO_ROOT="${temporary}/go-${output}" \
    HUGO_OUTPUT="${temporary}/${output}" \
        "${builder}"
}

build hugo-one
build hugo-two
cmp -- "${temporary}/hugo-one" "${temporary}/hugo-two"
cmp -- "${temporary}/hugo-one" "${expected_binary}"
"${temporary}/hugo-one" version | grep -F 'hugo v0.165.0' >/dev/null
printf 'verified reproducible Hugo v0.165.0 (%s)\n' \
    "$(sha256sum -- "${temporary}/hugo-one" | awk '{print $1}')"
if [[ "$#" -eq 1 ]]; then
    install -m 0555 -- "${temporary}/hugo-one" "$1"
fi
