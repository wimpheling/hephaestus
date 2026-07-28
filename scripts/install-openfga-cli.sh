#!/usr/bin/env bash
set -euo pipefail

version=0.7.19
repository_root=$(git rev-parse --show-toplevel)
install_root="$repository_root/.tools/openfga-cli/$version"
binary="$install_root/fga"

if [[ -x "$binary" ]]; then
    printf '%s\n' "$binary"
    exit 0
fi

case "$(uname -m)" in
    x86_64)
        architecture=amd64
        archive_sha256=21da629e0f9d29e97d60a11c860e763915c57c354beda25b6e350168c86f67be
        ;;
    aarch64|arm64)
        architecture=arm64
        archive_sha256=32196f0f45c046057caab854778c84f05cdef87bfa8c3df1cadee56e31fed85c
        ;;
    *)
        printf 'unsupported OpenFGA CLI architecture\n' >&2
        exit 1
        ;;
esac

download_root=$(mktemp -d)
trap 'rm -rf "$download_root"' EXIT
archive_name="fga_${version}_linux_${architecture}.tar.gz"
archive="$download_root/$archive_name"
curl --fail --location --silent --show-error \
    --output "$archive" \
    "https://github.com/openfga/cli/releases/download/v${version}/${archive_name}"
actual_sha256=$(sha256sum "$archive" | cut -d' ' -f1)
if [[ "$actual_sha256" != "$archive_sha256" ]]; then
    printf 'OpenFGA CLI archive checksum mismatch\n' >&2
    exit 1
fi
mkdir -p "$install_root"
tar -xzf "$archive" -C "$install_root" fga LICENSE
chmod 0755 "$binary"
"$binary" version >&2
printf '%s\n' "$binary"
