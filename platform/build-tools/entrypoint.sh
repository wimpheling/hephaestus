#!/bin/sh
set -eu

mkdir -p "$HOME" "$XDG_CACHE_HOME" "$TRIVY_CACHE_DIR"
exec "$@"
