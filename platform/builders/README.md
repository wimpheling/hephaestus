# Platform builder release operation

The four reviewed Dockerfiles in this directory are released by two
administrator-operated scripts and the trusted `hephaestus-registry-release`
command. They are not a GitHub Actions or GHCR workflow, and tags are never an
execution input.

`build-platform-builder-layouts.sh` creates private local OCI layouts for the
currently supported `x86_64` / `linux/amd64` platform. It runs rootless Buildah
with `--pull-never`: the digest-pinned Ubuntu base must already be available in
the trusted local image store through the separately reviewed platform-import
operation. This prevents the release command from silently treating arbitrary
upstream registry availability as its supply chain.

After Buildah produces the platform manifest, the operation creates a canonical
OCI index with exactly that one explicit `linux/amd64` descriptor and a local
`heph-sha256-…` digest tag. This is the layout contract used by the controlled
publisher; it intentionally does not claim an arm64 variant.

The Dockerfiles deliberately remain the reviewed, pinned build definitions.
Their documented download steps run during the trusted platform build. The
resulting layout is then scanned without network access; untrusted repository
builds are a different path and never receive this release operation's
credentials.

## Required tool configuration

Every executable is an administrator-owned absolute, non-symlink path. Each
version variable is an exact configured substring which the corresponding
`--version` output must contain. This makes tool upgrades an explicit reviewed
release-input change.

```sh
export HEPHAESTUS_BUILDAH=/opt/heph-tools/buildah
export HEPHAESTUS_BUILDAH_VERSION='buildah version 1.x.y'
export HEPHAESTUS_SKOPEO=/opt/heph-tools/skopeo
export HEPHAESTUS_SKOPEO_VERSION='skopeo version 1.x.y'
export HEPHAESTUS_SYFT=/opt/heph-tools/syft
export HEPHAESTUS_SYFT_VERSION='syft 1.x.y'
export HEPHAESTUS_TRIVY=/opt/heph-tools/trivy
export HEPHAESTUS_TRIVY_VERSION='Version: 0.x.y'
export HEPHAESTUS_ORAS=/opt/heph-tools/oras
export HEPHAESTUS_ORAS_VERSION='Version: 1.x.y'
export HEPHAESTUS_JQ=/opt/heph-tools/jq
export HEPHAESTUS_JQ_VERSION='jq-1.x'
export HEPHAESTUS_REGISTRY_RELEASE=/opt/heph-tools/hephaestus-registry-release
export HEPHAESTUS_REGISTRY_RELEASE_VERSION='hephaestus-registry-release 0.x.y'
```

The build command needs Buildah, Skopeo, Syft, Trivy, and jq. Trivy is invoked
with `--offline-scan --skip-db-update --skip-java-db-update`; its approved local
vulnerability database must therefore be present before a release starts. The
complete vulnerability result is retained and published as scan evidence. A
second deterministic policy report fails the build for fixable `HIGH` or
`CRITICAL` findings. Unfixed findings remain visible in the complete report but
do not make an image permanently unreleasable when no patched package exists.

For an optional operator approval, provide a private directory containing one
`<builder-key>.json` artifact per builder, together with a pinned Cosign and a
public verification key. The approval is verified before it is copied into the
private release output and attached as a referrer.

```sh
export HEPHAESTUS_PLATFORM_APPROVAL_DIRECTORY=/srv/hephaestus/approvals/release-2026-08-04
export HEPHAESTUS_PLATFORM_APPROVAL_PUBLIC_KEY=/etc/hephaestus/platform-approval.pub
export HEPHAESTUS_COSIGN=/opt/heph-tools/cosign
export HEPHAESTUS_COSIGN_VERSION='GitVersion: v2.x.y'
```

All private roots, database credentials, and signing-key files must have no
group or other permissions.

## Build, publish, review

The release inputs are exact: a source URI, a lowercase 40- or 64-character
commit SHA, and a UTC `YYYY-MM-DDTHH:MM:SSZ` timestamp. The build output must
be a fresh dedicated private directory; neither script overwrites one.

```sh
scripts/build-platform-builder-layouts.sh \
  --output-root /srv/hephaestus/platform-release/2026-08-04 \
  --source https://forge.example/hephaestus \
  --revision 0123456789abcdef0123456789abcdef01234567 \
  --created 2026-08-04T12:34:56Z
```

Publication goes through `hephaestus-registry-release`, not through
pre-issued token files. The command creates or resumes the durable publication
intent, issues an internal short-lived RS256 token for exactly one
`platform/builders/<key>` repository and `pull,push` actions, publishes through
the controlled Skopeo/ORAS adapter, reads Zot back, verifies evidence, and
commits approval. The token is written only to a private temporary OCI-client
credential file by that trusted command; it must never be passed as an argument,
saved as a release input, or written to the review/catalog output.

The calling account is therefore a privileged forge release operator. It needs
the database connection and registry signing-key access used by the command;
repository Dockerfiles, Buildah, and browser code do not receive either.

```sh
export HEPHAESTUS_FORGE_REGISTRY_AUTHORITY=registry.forge.example
export HEPHAESTUS_REGISTRY_SERVICE=registry.forge.example
export HEPHAESTUS_DATABASE_URL='postgres://…'
export HEPHAESTUS_REGISTRY_TOKEN_PRIVATE_KEY=/run/hephaestus/registry-signing-key.pem
export HEPHAESTUS_REGISTRY_TOKEN_ISSUER=https://forge.example/v1/registry/token
export HEPHAESTUS_REGISTRY_TOKEN_KEY_ID=registry-2026-08
export HEPHAESTUS_REGISTRY_TOKEN_LIFETIME_SECONDS=300
export HEPHAESTUS_PLATFORM_CREDENTIAL_ROOT=/run/hephaestus/platform-builder-credentials

scripts/publish-platform-builders.sh \
  --input-root /srv/hephaestus/platform-release/2026-08-04 \
  --review-output /srv/hephaestus/platform-release/2026-08-04/review.json \
  --catalog-output /srv/hephaestus/platform-release/2026-08-04/platform-builder-catalog.json
```

The wrapper accepts only `HEPHAESTUS_FORGE_REGISTRY_AUTHORITY` and the exact
`platform/builders/<key>` destinations. It supplies the private layout and
credential roots to `hephaestus-registry-release`, then accepts only its
approved, read-back digest and referrer identities. Required evidence is SPDX
SBOM, in-toto provenance, and vulnerability scan. A Cosign-verified approval
artifact is optional: when one is supplied during the build it is published as
the `signature` referrer; when omitted, the review and generated catalog record
`null` for the signature and approval still proceeds under the current
`without_signature` policy. An operator must review the resulting artifact
before applying the catalog.

`HEPHAESTUS_REGISTRY_SERVICE` must exactly equal the authority.
`HEPHAESTUS_REGISTRY_LAYOUT_ROOT` is set by the wrapper to `--input-root`; do
not set it to a broader directory. The release command additionally receives
the pinned `HEPHAESTUS_SKOPEO` and `HEPHAESTUS_ORAS` paths from the tool setup
above. `HEPHAESTUS_PLATFORM_CREDENTIAL_ROOT` is only the trusted command's
temporary OCI-client credential root; it contains no pre-issued JWT files.

`--dry-run` validates the configured contract and performs no build, token, or
registry operation. `--self-test` exercises the scripts' pure input validators:

```sh
scripts/build-platform-builder-layouts.sh --self-test
scripts/publish-platform-builders.sh --self-test
```
