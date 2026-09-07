#!/usr/bin/env bash
# Build the Hugo project image binary from pinned upstream source and toolchain.
# The caller supplies only repository-vendored inputs; module resolution stays
# offline so the output is reproducible from these bytes.
set -Eeuo pipefail

source_archive="${HUGO_SOURCE_ARCHIVE:-/tmp/hugo-0.165.0-source.tar.gz}"
module_archive="${HUGO_MODULE_ARCHIVE:-/tmp/hugo-0.165.0-go-vendor.tar.gz}"
go_archive="${HUGO_GO_ARCHIVE:-/tmp/go1.27.1.linux-amd64.tar.gz}"
dependency_patch="${HUGO_DEPENDENCY_PATCH:-/tmp/hugo-0.165.0-dependencies.patch}"
source_root="${HUGO_BUILD_ROOT:-/tmp/hugo-source}"
go_root="${HUGO_GO_ROOT:-/opt/go}"
output="${HUGO_OUTPUT:-/out/hugo}"

verify_checksum() {
    local expected="$1"
    local path="$2"
    local actual
    actual="$(sha256sum -- "$path" | awk '{print $1}')"
    [[ "$actual" == "$expected" ]] || {
        printf 'checksum mismatch for %s: expected %s, got %s\n' "$path" "$expected" "$actual" >&2
        exit 1
    }
}

verify_checksum e9c1e7d8e6e09356cc56317fd01b7493d712692390b89b3d33810cfe1305650e "$source_archive"
verify_checksum 9abcbb5581e55c4d4333228a9ad15a4c9323cf82177259aacfc9403b88e0edbc "$module_archive"
verify_checksum 63d339f0da5ab53635a56f2490a7984dfe12dfcff22ad749f63edaf590168445 "$go_archive"
verify_checksum 07e8fe1c0e2c0a133ce0df89f7a45b91960002546f0845b52b699e972757fe1d "$dependency_patch"

# Refuse reused roots: verification must start clean, and caller-supplied paths
# must never authorize recursively deleting an existing directory.
mkdir -- "$source_root" "$go_root"
mkdir -p -- "$(dirname -- "$output")"
tar --extract --gzip --file "$source_archive" --directory "$source_root" --strip-components=1
tar --extract --gzip --file "$module_archive" --directory "$source_root"
tar --extract --gzip --file "$go_archive" --directory "$go_root" --strip-components=1
git -C "$source_root" apply --check "$dependency_patch"
git -C "$source_root" apply "$dependency_patch"

cd "$source_root"
GOTOOLCHAIN=local CGO_ENABLED=0 GOPROXY=off "$go_root/bin/go" build \
    -mod=vendor -trimpath -buildvcs=false -ldflags='-s -w' -o "$output" ./
chmod 0555 "$output"
