# Platform OCI image release operation

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

After Buildah produces the platform image manifest, the operation creates a canonical
OCI index with exactly that one explicit `linux/amd64` descriptor and a local
`heph-sha256-…` digest tag. This is the layout contract used by the controlled
publisher; it intentionally does not claim an arm64 variant.

The Dockerfiles deliberately remain the reviewed, pinned build definitions.
Their documented download steps run during the trusted platform build. The
resulting layout is then scanned without network access; untrusted repository
builds are a different path and never receive this release operation's
credentials.

## Local containerized build

For the supported local Hephaestus workflow, do not install Buildah, Skopeo,
Syft, Trivy, ORAS, or jq on the host. `cargo dev platform-images build` builds
the pinned `platform/build-tools/Dockerfile` with rootless Podman, then runs
the reviewed release script inside that container. It creates private release
layouts under `.local/hephaestus/platform-images/releases/<revision>` and
persists only its Buildah storage and scanner cache in dedicated Podman named
volumes. This is an explicit, heavy operation; it never runs during startup,
migrations, or ordinary tests.

The host prerequisites for this path are readable/writable `/dev/kvm` and
rootless Podman. The outer Podman invocation remains rootless and
non-privileged. `/dev/fuse` is passed only to the tool container for its
rootless Buildah storage, while `BUILDAH_ISOLATION=chroot` avoids a nested OCI
runtime mount. No Podman socket, registry credential, or signing-key mount is
provided.

```sh
cargo dev doctor
cargo dev platform-images build \
  --source https://forge.example/hephaestus \
  --revision 0123456789abcdef0123456789abcdef01234567 \
  --created 2026-08-05T12:34:56Z
cargo dev platform-images status
```

The revision and timestamp are immutable release inputs. A pre-existing
release directory is rejected rather than overwritten. This operation builds
and records private OCI layouts and evidence only. To install one reviewed
release, start the local stack (for PostgreSQL), then explicitly publish it:

```sh
cargo dev platform-images publish \
  --revision 0123456789abcdef0123456789abcdef01234567
cargo dev platform-images status
```

`publish` starts local Zot, uses the pinned tool image to publish and read back
the four immutable layouts, approves them, and applies the OCI image catalog.
It writes the review, catalog, and catalog-application receipts beneath
`.local/hephaestus/platform-images/installations/<revision>/`. It never runs
automatically. `cargo dev platform-images clean --revision <revision>` removes
only those private local receipts; it deliberately does not delete approved
Zot content or catalog records. The four standard artifacts are ordinary OCI
images: an execution contract may select any approved image for either a build
or a guest execution. Image selection does not grant network access, secrets,
mounts, or resources; those remain execution-contract policy.

## Standalone operator tool configuration

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

## Operator smoke release

This is a manual operator smoke, not a commit-time test. It proves the four
reviewed builder definitions can be built, scanned, published, verified,
approved, and pulled through a real forge registry. Run it after changes to a
platform Dockerfile, pinned input/toolchain, registry publication code, or the
release tooling—not for ordinary application commits.

Before starting, use a fresh private output root and a real, current Trivy
database. The build itself runs offline scanning; refreshing the database is a
separate, observable preparation step. Use the exact reviewed tool paths and
versions from the configuration section, then run the build with a precise
source revision and timestamp. Do not substitute test fixtures for Buildah,
Syft, Trivy, the registry, or the signing key.

After the build, run the publication command against either a disposable
forge-shaped registry or the explicitly intended forge authority. Confirm all
four entries in `review.json` are approved internal digest references with SBOM,
provenance, and scan referrers. Review the generated catalog before applying
it through the operator catalog command; then pull every catalog digest with a
real OCI client and verify the declared OS/toolchain versions. Retain the
release input, review artifact, and catalog as smoke evidence.

For a disposable end-to-end operator smoke, use
`scripts/smoke-platform-builder-release.sh`. It starts ephemeral PostgreSQL and
authenticated Zot, migrates the schema, runs the trusted publisher against the
already-built private layouts, provisions the generated catalog, and pulls all
four approved digest references. It does not rebuild layouts and is never run
by CI. By default it reads `/tmp/hephaestus-platform-smoke/layouts`; override
that location with `HEPHAESTUS_PLATFORM_SMOKE_LAYOUT_ROOT`. The harness retains
its review and catalog artifacts in fresh private `/tmp/hephaestus-platform-smoke-*.json`
files after a successful run and prints their paths.

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
`platform/builders/<key>` repository and `pull,push` actions, publishes the
image and OCI evidence layouts through controlled Skopeo, reads Zot back with
the bounded direct bearer client, verifies evidence, and commits approval. The
token is never saved as a release input or written to the review/catalog output.

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
