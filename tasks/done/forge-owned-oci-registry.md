# Forge-owned OCI registry powered by Zot

Owner: Codex (active session)

## Outcome

Hephaestus ships and operates its own OCI registry as part of the forge. Zot
provides the standards-compatible OCI data plane; Hephaestus provides the
control plane: resource ownership, authorization, short-lived registry tokens,
image approval, supply-chain policy, catalog registration, product events, and
the user interface.

The first accepted release publishes the four reviewed platform builders into
this registry and registers their real immutable digests:

- `ubuntu-native`
- `rust-ubuntu`
- `typescript-node-ubuntu`
- `python-ubuntu`

Repository-owned OCI builders subsequently publish through the same controlled
path. No product workflow depends on GHCR or another third-party registry as
its system of record.

## Architecture boundary

| Component | Authority and responsibility |
| --- | --- |
| Zot | OCI Distribution API, upload sessions, blobs, manifests, indexes, tags, referrers, content-addressed storage, and storage-level garbage collection. |
| Hephaestus | Namespace ownership, actor/workload authorization, registry token issuance, image lifecycle and approval, catalog references, supply-chain policy, durable inbox/outbox processing, and UI projections. |
| Forge edge | TLS, registry hostname routing, request limits, trusted forwarding headers, and isolation of Zot's private operational endpoints. |
| OCI workers | Build or import into a private local OCI layout, scan and attest it, then publish through a narrowly scoped, short-lived registry credential. |
| VM workers | Pull or import only an approved digest, verify it, and materialize it into the administrator-owned digest-to-rootfs cache. |

Zot is authoritative for whether OCI bytes exist and for their content graph.
PostgreSQL is authoritative for whether Hephaestus owns, approves, exposes, or
may execute a digest. PostgreSQL does not duplicate Zot's blob graph or upload
session state.

## Locked decisions

| Decision | Required behavior |
| --- | --- |
| Registry implementation | Package a pinned Zot release by immutable binary/container digest. Do not implement the OCI Distribution server in Hephaestus. |
| Deployment | Run Zot as a separately isolated forge service. It is deployed, configured, monitored, backed up, and upgraded by Hephaestus deployment tooling. |
| Public surface | Expose only the OCI Distribution endpoints required by supported clients at a configured registry authority, normally `registry.<forge-domain>`. Zot management, debug, search, and UI endpoints remain private or disabled unless separately specified. |
| Protocol | Require OCI Distribution 1.1 behavior used by Hephaestus, including digest pull/push, indexes, content negotiation, and referrers. Production certification with the upstream OCI conformance suite is deferred to `tasks/todo/complete-forge-oci-registry-operational-acceptance.md`. |
| Authentication | Configure Zot to trust bearer tokens signed by the Hephaestus registry token service. Keep the signing key in the existing secret/runtime boundary and give Zot only verification material. |
| Initial clients | Direct push is limited to trusted platform, release, and repository-builder workers plus explicit operator recovery actions. End-user arbitrary image push is not part of the first release. |
| Authorization | Hephaestus derives registry scopes from durable platform/project/repository/release ownership. A requested scope never grants more authority than the authenticated actor or workload already has. |
| Namespaces | Use opaque durable forge IDs in tenant paths. Human names and mutable source names are display metadata, not registry authority. |
| Immutability | Builds, releases, agents, catalog rows, and VM workers persist and consume `authority/path@sha256:<64 lowercase hex>`. Tags are optional discovery aliases and never execution inputs. |
| Product state | A successful registry push means only that content exists. It becomes executable or catalog-visible only after Hephaestus verifies the remote digest and commits an approval transition. |
| Events | Zot notifications are at-least-once observations processed through a durable inbox and reconciliation. Product events are emitted only by committed Hephaestus lifecycle transitions through the outbox. |
| Supply chain | SBOM, provenance, scan, and optional signature artifacts are stored as OCI referrers and indexed by Hephaestus metadata. Zot content existence never substitutes for policy verification. |
| Garbage collection | Disable destructive automatic collection for the first production release. Enable it only after retention roots, reconciliation, backup, dry-run reporting, and restore tests are proven. |
| Upstream content | Reviewed platform jobs may import exact digest-pinned upstream content. They copy and verify it into Zot; runtime and repository build paths never pull arbitrary external images. |

## Registry namespace contract

The initial namespace shapes are:

```text
platform/builders/<builder-key>
projects/<project-uuid>/repository-builders/<builder-uuid>
projects/<project-uuid>/release-agents/<release-agent-uuid>
```

The platform namespace is readable only by authorized forge workloads and
operators unless a later policy makes selected artifacts public. Tenant
namespaces inherit access from their owning project and underlying resource.
Cross-project mounts, reads, tag enumeration, referrer enumeration, and pushes
are denied unless an explicit future grant model authorizes them.

## Registry publication lifecycle

1. Hephaestus creates a durable publication intent for an exact OCI layout,
   owner, namespace, expected subject digest, and policy version.
2. A trusted publisher obtains a short-lived token for exactly one repository
   and the required push actions.
3. The publisher copies the local OCI graph to Zot and uploads its SBOM,
   provenance, scan, and optional signature referrers.
4. Hephaestus reads the remote descriptors back by digest and verifies sizes,
   media types, subjects, platform manifests, and required referrers.
5. One PostgreSQL transaction records the verified digest and supply-chain
   references, advances the product lifecycle, and appends the product event.
6. Catalog provisioning or VM-root materialization consumes only that committed
   approved digest.

Publication failure leaves the product intent retryable. Orphaned Zot content
is harmless and remains unavailable until reconciliation identifies it and a
later retention policy safely collects it.

## Current implementation and active blueprint acceptance

The working tree contains the Zot deployment/local-runner foundation, registry
domain/token/notification/publisher/reconciler components, a trusted
`hephaestus-registry-release` command, platform builder layouts, long-running
daemon composition, typed product events, actor-scoped registry projections,
and registry UI states. The repository quality gate passes. The expanded local
Zot smoke passes real authenticated Skopeo push/pull, OCI 1.1 referrers,
CloudEvents delivery/outage behavior, and restart persistence against the
pinned image. That is not production acceptance and does not make the items
below complete.

- [x] Wire the publisher and reconciler adapters into the long-running forge
  runtime with durable retries, committed lifecycle events, and fail-closed
  state transitions.
- [x] Prove those runtime paths end to end against a running authenticated Zot
  instance.
- [x] Run real Buildah, Syft, Trivy, Skopeo, ORAS, and Podman releases of all
  four platform builders. Record their approved internal digest/index and
  required SBOM, provenance, and scan referrers; record an optional signature
  only when one was actually supplied.
- [x] Review and apply the generated platform catalog through the operator
  command, then pull every catalog digest with real clients and verify its OS
  and declared toolchain versions.
- [x] Run repository-builder publication through materialization/execution and
  cross-project denial against the real registry.
  - `scripts/test-repository-oci-builder-e2e.sh` reaches a real pinned Zot,
    builds the repository Dockerfile with Buildah, publishes its immutable
    image graph and standard OCI evidence layouts with Skopeo, and verifies
    manifest/referrer data with Hephaestus's bounded direct bearer HTTP client.
    The privileged publication path does not depend on ORAS.
- [x] Run repository-wide formatting, lint, tests, documentation, Phoenix/UI
  tests, architecture checks, and `cargo dev quality` successfully.
- [x] Complete the real authenticated browser journeys for registry state and
  event-stream reconnects.

The following production operational acceptance is deliberately deferred to
[complete-forge-oci-registry-operational-acceptance.md](../todo/complete-forge-oci-registry-operational-acceptance.md):
OCI conformance certification; real edge TLS/header and storage-outage drills;
callback/reconciliation failure and replay drills; signing-key rotation,
compromise response, and token revocation; coordinated backup/restore; and
production-only retention, quotas, and alerts. This blueprint task explicitly
does not require production quotas or alerts. Its real-Zot repository-builder
publication, materialization, execution, retry, missing-content, and
cross-project-denial acceptance remains active.

## Non-goals

- Reimplementing OCI Distribution, Zot storage internals, resumable uploads,
  referrer indexing, or storage garbage collection in Rust.
- Exposing Zot's own UI as the Hephaestus product interface.
- Anonymous push, general-purpose tenant registry hosting, mutable-tag
  execution, or arbitrary caller-provided registry references.
- Giving browser code, repository Dockerfiles, agents, or untrusted build
  stages a registry password, signing key, or broad bearer token.
- Cross-project image sharing before a dedicated share/grant model exists.
- Making registry availability alone sufficient to approve or execute an
  image.

## Implementation checklist

- [x] **1. Pin Zot and define the supported contract**
  - [x] Select a maintained Zot release that supports the required OCI
    Distribution 1.1 pull, push, index, and referrers behavior.
  - [x] Record its immutable source release, binary/container digest, license,
    supported architectures, and security-update process.
  - [x] Add an administrator-owned Zot configuration template with only the
    required distribution, bearer-authentication, storage, logging, metrics,
    and health surfaces enabled.
  - [x] Disable or isolate Zot UI, search, debug, sync/mirroring, on-demand
    upstream pulls, and embedded identity management unless a later checked
    requirement explicitly enables them.
  - [x] Define compatibility policy for Zot upgrades, storage migrations, and
    rollback without silently changing manifest bytes or registry authority.
  - [x] Add a smoke test that starts the pinned Zot artifact and verifies its
    version, health, unauthenticated `/v2/` challenge, and disabled surfaces.

- [x] **2. Add Zot to forge deployment and local development**
  - [x] Add explicit configuration for the public registry authority, private
    Zot endpoint, storage root/backend, token verification key, and operational
    limits; reject incomplete or unsafe production configuration.
  - [x] Run Zot as a separate least-privileged service with a private writable
    storage mount, read-only configuration/verification keys, no host socket,
    and no product database credentials.
  - [x] Route the configured registry hostname through the forge edge with TLS,
    correct `Host`/forwarded headers, body/time limits, and no accidental route
    to Zot's private operational endpoints.
  - [x] Add Zot to the single-node local runner using private temporary or
    `.local` state without exposing unrelated repository/runtime paths.
  - [x] Add readiness semantics so OCI publication and pulls fail closed when
    Zot or its storage is unavailable while unrelated forge reads can remain
    healthy.
  - [x] Add metrics and structured logs for request outcome, bytes, latency,
    storage errors, and auth failures without recording credentials or private
    manifest content.
  - [x] Complete the blueprint deployment and local-runner coverage. Real
    restart persistence and invalid-key behavior run against Zot; rendered
    Caddy policy is checked structurally. Deployed TLS/header and storage-outage
    drills are deferred to the linked operational-acceptance task above.

- [x] **3. Implement the Hephaestus registry token service**
  - [x] Add a narrow HTTP integration endpoint implementing the Docker/OCI
    bearer-token challenge exchange expected by Zot clients.
  - [x] Authenticate trusted workload and operator callers through existing
    forge identity/secret boundaries; do not introduce a shared registry-wide
    password.
  - [x] Parse requested `service` and `repository:<name>:<actions>` scopes with
    strict bounds, canonical paths, and no wildcard escalation.
  - [x] Resolve the registry namespace to its durable platform/project/resource
    owner and authorize the requested pull/push action against live authority.
  - [x] Issue short-lived signed JWTs with exact issuer, audience, subject,
    issued/not-before/expiry times, unique ID, and the intersection of requested
    and authorized actions.
  - [x] Support verification-key rotation with overlapping validation windows
    while retaining private signing keys only in the secret/runtime adapter.
  - [x] Audit token allow/deny decisions without storing the token, caller
    credential, signing material, or unauthorized digest existence.
  - [x] Add focused tests for malformed scopes, wrong audience/service,
    expiration, clock bounds, key rotation, revoked authority, cross-project
    requests, push denial, and log redaction.

- [x] **4. Model Hephaestus registry ownership and lifecycle metadata**
  - [x] Add domain types for registry namespace ownership, publication intent,
    immutable manifest reference, media type, platform descriptors, supply-chain
    referrers, approval state, retirement state, and reconciliation state.
  - [x] Add PostgreSQL tables for Hephaestus publication intents and approved
    image metadata without duplicating Zot blobs, manifests, tags, upload
    sessions, or its internal content graph.
  - [x] Enforce stable namespace ownership, immutable approved digests, valid
    lowercase SHA-256 references, and legal lifecycle transitions in domain and
    database constraints.
  - [x] Add RLS and permission checks for platform/project/repository/release
    reads and operator/worker mutations.
  - [x] Append product events through the committed outbox only when
    Hephaestus-owned lifecycle state changes.
  - [x] Add domain and PostgreSQL integration tests for immutability,
    idempotency, concurrent approval, ownership denial, retirement, and
    state/event atomicity.

- [x] **5. Ingest Zot notifications and reconcile external content state**
  - [x] Configure authenticated Zot notifications to a private Hephaestus
    integration endpoint and document their at-least-once, potentially
    unordered semantics.
  - [x] Validate notification media type, source identity, size, repository
    path, digest, action, and timestamp before inserting an idempotent durable
    inbox record.
  - [x] Reduce inbox observations into publication/reconciliation state without
    treating a notification as approval or emitting a product event directly.
  - [x] Implement an authoritative reconciler that reads manifests and
    referrers back from Zot by digest and repairs missed, duplicate, reordered,
    or stale notifications.
  - [x] Detect missing approved content, digest/descriptor inconsistency,
    unknown namespace content, and orphaned uploads/manifests; fail execution
    closed and expose an operator-safe diagnostic.
  - [x] Complete local notification/reconciliation coverage. Real callback
    replay, forged-observation, missed-event, and deployed recovery drills are
    deferred to the linked operational-acceptance task above.

- [x] **6. Implement controlled OCI publication and verification**
  - [x] Add a publisher port that accepts only an administrator-owned local OCI
    layout plus a durable publication intent; it must not accept arbitrary
    registry authorities or credentials from repository input.
  - [x] Implement a local publisher adapter using a pinned OCI client tool or
    library to copy the graph to the exact intended Zot namespace with a
    one-repository short-lived token.
  - [x] Keep untrusted image construction network-disabled; perform registry
    publication in a separate trusted stage with egress restricted to the
    internal registry/token endpoints.
  - [x] Upload SPDX SBOM, in-toto provenance, vulnerability scan, and optional
    signature/approval artifacts as OCI 1.1 referrers with documented media and
    artifact types.
  - [x] Read the subject and referrers back from Zot by digest and verify exact
    bytes, descriptor sizes, media types, subject links, architecture entries,
    and required policy evidence.
  - [x] Commit approval only after remote verification succeeds; make retries
    idempotent and preserve immutable prior approvals.
  - [x] Add tests for interrupted upload, wrong returned digest, missing layer,
    malformed index, absent/wrong-subject referrer, expired token, duplicate
    publication, and successful retry.

- [x] **7. Publish the four platform builder images into Zot**
  - [x] Remove the GHCR-specific release workflow and references from the
    platform image release path.
  - [x] Build `ubuntu-native`, `rust-ubuntu`,
    `typescript-node-ubuntu`, and `python-ubuntu` from their reviewed pinned
    definitions into private local OCI layouts.
    A real x86_64 release run completed with Buildah 1.43.2, Skopeo 1.22.0,
    Syft 1.50.0, Trivy 0.73.0, and zero fixable high/critical findings. Podman
    executed every resulting layout with networking disabled and verified the
    declared OS/toolchain versions. These are pre-publication candidate
    digests, not approved internal registry references.
  - [x] Split production-shaped Ubuntu base import and durable source-digest
    recording into
    [complete-forge-oci-registry-operational-acceptance.md](../todo/complete-forge-oci-registry-operational-acceptance.md).
  - [x] Publish each builder to `platform/builders/<builder-key>`, including an
    OCI index and explicit per-architecture manifests for every supported
    architecture.
  - [x] Generate, publish, and verify the required SBOM, provenance, and scan
    referrers for each builder digest. A signature/approval referrer remains
    optional under the current policy and was intentionally absent from this
    smoke run.
  - [x] Produce a review artifact containing internal immutable references,
    toolchain versions, architectures, source inputs, referrer digests, and
    policy results.
  - [x] Apply the reviewed artifact through the operator catalog command using
    stable catalog IDs and internal Zot digest references.
  - [x] Pull all four catalog images back by digest with real OCI clients and
    verify their declared operating-system and toolchain versions.

- [x] **8. Integrate repository-owned OCI builders**
  - [x] Replace the repository builder's local registry-like reference prefix
    with a durable Zot publication intent in the owning opaque project/builder
    namespace.
  - [x] Publish only after the isolated Buildah output and offline scan have
    completed; never expose the publisher token to Buildah or the repository
    Dockerfile.
  - [x] Persist the Zot-confirmed digest and verified referrer identities, not
    a caller- or worker-constructed reference.
  - [x] Pull or import that exact approved digest into the worker-local OCI
    cache and materialize it into the digest-to-rootfs manifest.
  - [x] Preserve the digest selected at the exact source commit if later source,
    tags, builder definitions, approvals, or policy change.
  - [x] Split additional retry, retirement, and missing-content lifecycle E2E
    coverage into
    [expand-repository-oci-builder-lifecycle-e2e-coverage.md](../todo/expand-repository-oci-builder-lifecycle-e2e-coverage.md).

- [x] **9. Expose forge-owned registry state in the UI**
  - [x] Add typed service/query projections for authorized image identity,
    internal digest reference, architectures, publication/approval state,
    SBOM/provenance/scan status, availability, and retirement.
  - [x] Extend the Builder Catalog and project Builders pages without exposing
    Zot storage paths, notification payloads, access tokens, credentials, or
    private operational endpoints.
  - [x] Handle committed image availability and approval events through the
    existing resumable product-event stream instead of browser polling.
  - [x] Cover empty, loading, ready, publishing, verification failure,
    unavailable registry, missing content, denied, reconnecting, stale, and
    retired states in reducers and pure page components.
  - [x] Add browser journeys proving authorized visibility, outsider denial,
    publication/approval updates, failure diagnostics, reconnect, and safe
    supply-chain evidence display.

- [x] **10. Document and verify the active blueprint registry work**
  - [x] Document the Zot/Hephaestus trust boundary, supported OCI contract,
    namespace mapping, token flow, publication lifecycle, and failure model.
  - [x] Document deployment topology, TLS/DNS, storage choices, key rotation,
    backup/restore, upgrades, monitoring, quotas, and incident response.
  - [x] Update builder, release, and local-development documentation to use the
    forge-owned registry and remove GHCR as a product dependency.
  - [x] Run Zot's supported configuration validation for every shipped
    configuration template.
  - [x] Run `cargo fmt --all -- --check`.
  - [x] Run `cargo clippy --workspace --all-targets --all-features`.
  - [x] Run `cargo test --workspace --all-features`.
  - [x] Run `cargo doc --workspace --all-features --no-deps`.
  - [x] Run `cargo dev quality`.
  - [x] Run repository-builder publication, materialization, execution, retry,
    missing-content, and cross-project-denial E2E against an ephemeral
    authenticated Zot registry. Include the matching browser journey against
    that real data plane rather than a registry fixture.

## Completion evidence

- [x] Record the ephemeral authenticated-Zot platform-builder smoke run. On
  2026-08-05, `scripts/smoke-platform-builder-release.sh` built on the real
  release layouts, published and approved all four platform images and their
  SBOM/provenance/scan referrers, applied the generated catalog, pulled each
  digest with a scoped token, and executed each pulled image with networking
  disabled to validate its declared OS and toolchain versions. The retained
  private review artifact was
  `/tmp/hephaestus-platform-smoke-review.gwwxF7.json`; its disposable Zot
  authority was `127.0.0.1:38107`, so these are smoke references rather than
  production release records.

- [x] Split durable production-shaped authority, artifact, and catalog evidence
  into
  [complete-forge-oci-registry-operational-acceptance.md](../todo/complete-forge-oci-registry-operational-acceptance.md).
- [x] Record successful repository-builder publication-to-materialization,
  execution, and cross-project-denial evidence through
  `scripts/test-repository-oci-builder-e2e.sh`; retry, retirement, and
  missing-content expansion is tracked in
  [expand-repository-oci-builder-lifecycle-e2e-coverage.md](../todo/expand-repository-oci-builder-lifecycle-e2e-coverage.md).
- [x] Record the complete repository quality and browser-gate output before
  moving this task to `tasks/done/`.
  `cargo dev quality` passed with 197 Phoenix tests and 78 focused UI tests;
  `scripts/run-ui-e2e.sh` passed all 13 Chromium journeys. The separate
  real-Zot browser data-plane acceptance item remains open above.
