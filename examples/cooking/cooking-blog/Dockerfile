# Hugo is rebuilt from the official v0.165.0 source tag with a pinned Go
# toolchain and vendored dependency graph, then packaged as a verified tool.
# Source archive SHA-256: e9c1e7d8e6e09356cc56317fd01b7493d712692390b89b3d33810cfe1305650e
# Go 1.27.1 archive SHA-256: 63d339f0da5ab53635a56f2490a7984dfe12dfcff22ad749f63edaf590168445
# Dependency vendor SHA-256: 9abcbb5581e55c4d4333228a9ad15a4c9323cf82177259aacfc9403b88e0edbc
# Dependency patch SHA-256: 07e8fe1c0e2c0a133ce0df89f7a45b91960002546f0845b52b699e972757fe1d
# Rebuilt executable SHA-256: 9eff60e74ba1387eddbfceb9e48028743d046f06f08eb837eea1daeec88c014c
FROM heph-base AS runtime
COPY vendor/hugo-0.165.0-rebuilt /usr/local/bin/hugo
COPY vendor/hugo-0.165.0-LICENSE /usr/local/share/licenses/hugo/LICENSE
RUN printf '%s\n' '9eff60e74ba1387eddbfceb9e48028743d046f06f08eb837eea1daeec88c014c  /usr/local/bin/hugo' | sha256sum --check - && chmod 0555 /usr/local/bin/hugo && test -d /opt/python/lib/python3.13/test && rm -rf -- /opt/python/lib/python3.13/test
