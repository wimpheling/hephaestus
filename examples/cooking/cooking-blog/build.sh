#!/bin/sh
set -eu

# Hugo reads the sealed source; every generated file belongs to writable
# output or disposable guest scratch. The project image supplies pinned Hugo.
scratch="$(mktemp -d /tmp/cooking-site-build.XXXXXX)"
trap 'rm -rf -- "$scratch"' EXIT
# libkrun clears OCI ENV metadata before the build command starts. Preserve
# the pinned base image's Python toolchain contract for the verifier's shebang.
export PATH=/opt/python/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
/usr/local/bin/hugo \
    --source /workspace/source \
    --destination /workspace/output/public \
    --cacheDir "$scratch/cache" \
    --noBuildLock \
    --ignoreCache
install -D -m 0755 check_site.py /workspace/output/bin/check-site
/workspace/output/bin/check-site
