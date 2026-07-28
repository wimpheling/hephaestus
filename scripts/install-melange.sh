#!/usr/bin/env bash
set -euo pipefail

version=0.8.5
repository_root=$(git rev-parse --show-toplevel)
install_root="$repository_root/.tools/melange/$version"
binary="$install_root/melange"

if [[ -x "$binary" ]] && [[ "$("$binary" version)" == melange\ "$version"* ]]; then
    printf '%s\n' "$binary"
    exit 0
fi

machine=$(uname -m)
case "$machine" in
    x86_64)
        architecture=amd64
        archive_sha256=6c4569544777bb5414532af8298098517a74e0afe39cef4a46f60c1cf8c9b051
        ;;
    aarch64|arm64)
        architecture=arm64
        archive_sha256=06b29b758eab9c7406635e57defb7a37924788295f6aa470ca1e77ac14355a7d
        ;;
    *)
        printf 'unsupported Mélange installation architecture: %s\n' "$machine" >&2
        exit 1
        ;;
esac

download_root=$(mktemp -d)
trap 'rm -rf "$download_root"' EXIT
archive_name="melange_${version}_linux_${architecture}.tar.gz"
archive="$download_root/$archive_name"
curl --fail --location --silent --show-error \
    --output "$archive" \
    "https://github.com/pthm/melange/releases/download/v${version}/${archive_name}"
actual_sha256=$(sha256sum "$archive" | cut -d' ' -f1)
if [[ "$actual_sha256" != "$archive_sha256" ]]; then
    printf 'Mélange archive checksum mismatch: expected %s, got %s\n' \
        "$archive_sha256" "$actual_sha256" >&2
    exit 1
fi

mkdir -p "$install_root"
tar -xzf "$archive" -C "$install_root" melange LICENSE THIRD_PARTY_NOTICES
chmod 0755 "$binary"
"$binary" version >&2
printf '%s\n' "$binary"
