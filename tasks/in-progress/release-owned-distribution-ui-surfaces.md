# Release-owned distribution UI surfaces

Owner: Astra orchestration / Luna bounded subtasks

## Current status

Persistent-service work is a completed prerequisite on the same feature branch
and PR 51 at source checkpoint `8c1fb51`; repository-wide quality v4 passed.
This UI task is now active. The first slices cover primitive release-UI
declaration model validation and source-level declaration parsing. Capture,
persistence, exact-ID resolution, authorized inspection, serving, browser
navigation, managed UI services, authority handoff, UI-kit publication, and
acceptance evidence remain unchecked until their bounded designs and
implementations are reviewed.

The source cross-manifest validator is now implemented and focused-validated.
The persistence schema and receive-side Git capture/build-link wiring are
implemented with focused PostgreSQL evidence below. Manual-build integration
now passes its basic application-role matrix; receive/manual ordering and
historical compatibility still need focused regression evidence. Publication,
serving, browser integration, and aggregate acceptance remain incomplete.
Earlier helper-only checkpoints below describe their state at the time;
the receive and manual integration checkpoints supersede pending-wiring notes.

## Current CI context (2026-09-20)

CI for manual-build checkpoint `1f3f76a` passed in run `35486471113`:
Rust/authorization, browser golden path, and Cooking applications all succeeded.
Size-limit checkpoint `ed96408` is still running in `35486716206`.
The preceding kit checkpoint `6c6a9c6` failed one Rust recovery-test teardown
while browser and Cooking passed. An idle `gateway-recovery-control` PostgreSQL
backend remained beyond the unchanged ten-second deadline. The exact owner is
unproven; a local Caddy future is dropped with its parent, and SQLx connection
return after cancellation is under investigation. No deadline or assertion has
been weakened. Bounded evidence is in
`/home/a/heph-kit-ci-teardown-failure-20260920.log` and
`/home/a/heph-kit-ci-teardown-sqlx-analysis-20260920.log`.

CI at `e052a2c` (`35482989628`) exposed an application bootstrap version
mismatch: the database reached migration 82 while `EXPECTED_DATABASE_MIGRATION`
still required 81. Both Rust golden bootstrap and the browser golden path
failed on this mismatch; Cooking applications passed. The application gate
now requires 82, and retention's minimum schema requirement remains 81.
The targeted production bearer-push bootstrap and real application retention
tests each passed against disposable PostgreSQL/NATS in
`/home/a/heph-schema82-bootstrap-retention-20260920.log`, including
`REAL_APP_SERVICE_LOG_MAINTENANCE=1 max_migration=82`. Scoped app formatting,
strict Clippy, and rustdoc passed in
`/home/a/heph-schema82-app-fmt-20260920-v2.log`,
`/home/a/heph-schema82-app-clippy-20260920-v3.log`, and
`/home/a/heph-schema82-app-doc-20260920.log`. This corrects the startup schema
gate; it is not a new complete UI or full-quality result.

Subsequent CI passed at `d0def95`, `7ccb01d`, and `44fe0c3`; the latest
successful run is `35481880564`. This does not establish the cause of the
earlier intermittent teardown failure. The separate Cooking E2E workflow for
`44fe0c3` remained pending when checked. Final integrated UI acceptance and
the branch-wide quality gate remain outstanding.

The initial UI-branch CI run for `e573563` (`35480143840`) failed in the Rust
test job at `daemon_loop_restores_active_service_without_manual_start`, during
teardown at `gateway_recovery_tests.rs:3891` with an idle backend reported
after pool shutdown. Browser and Cooking jobs passed. The preserved log is
`/home/a/heph-ui-initial-ci-20260920.log`; investigation is pending and no
functional root cause is claimed. Service-completion CI runs
`35479824078` (`8c1fb51`) and `35479909080` (`7494f9a`) passed, and local
quality v4 remains valid historical evidence. The current branch final gate
and UI capture/publication work remain pending; source-parser validation has
focused evidence below, with no capture or publication result claimed here.

The recovery teardown diagnostic instrumentation labels fixture pools and
captures bounded pool/backend metadata while preserving the 10-second deadline
and zero-session assertion; it records no raw queries or credentials. The exact
restore scenario passed 20/20 times against fresh PostgreSQL with migration 81
in `/home/a/heph-gateway-recovery-diagnostic-20260920.log`; compilation and
strict app Clippy passed in the corresponding `compile-v2` and `clippy-v2`
logs. The CI cause remains unproven because it was not reproduced, so this is
not a fixed-result claim. The earlier pending 17-test recovery-group
verification subsequently passed in v2: all 17 tests passed against
PostgreSQL migration 81 in 79.87 seconds; evidence is in
`/home/a/heph-gateway-recovery-diagnostic-group-v2-20260920.log` and
`/home/a/heph-gateway-recovery-diagnostic-group-postgres-v2.log`, with strict
app Clippy and formatting also passing. The v1 group failure came from a
diagnostic-only application-name rename changing an existing worker assertion;
the existing gateway-recovery-test name was restored, while new labels remain
limited to control/admin diagnostics. Cleanup completed without owned
containers. The 20-pass and 17-pass results are verification evidence, not a
claim that the original CI failure is fixed.

## Primitive declaration checkpoint (2026-09-20)

The first bounded slice adds the release-domain UI primitives in `ui.rs`:
`UiKey`, `UiRoutePath`, and `UiLabel` use checked Serde representations and
private fields. It also defines the scope, presentation, icon, and MIME enums,
plus a version-1 declaration helper. Label validation rejects control
characters, bidi controls, U+2028, and U+2029 before trimming spaces; the
validated boundaries are 64 bytes for keys, 256 bytes for route paths, and 80
Unicode characters for labels.

The release-domain test suite passed 14 tests, with formatting, strict Clippy,
and rustdoc passing in the v2 logs:
`/home/a/heph-release-domain-ui-fmt-v2-20260920.log`,
`/home/a/heph-release-domain-ui-test-v2-20260920.log`,
`/home/a/heph-release-domain-ui-clippy-v2-20260920.log`, and
`/home/a/heph-release-domain-ui-doc-v2-20260920.log`. Aggregate declaration
publication, serving, browser, authority, UI-kit, and end-to-end acceptance
checklist items remain unchecked.

## Source parser validation checkpoint (2026-09-20)

The `agent-config` UI module and types now validate the version-1 optional
manifest, including the 256 KiB manifest, 16 UI, 16 API, and 256 static-file
limits. The returned normalized configuration equals the hash serialization;
over-limit inputs fail fast with redacted diagnostics. Validation covers scoped
route collisions, HTML entrypoint and static-asset selection, and typed
managed-gateway/API route references.

The tests passed 22 unit tests, one existing integration test, and 10 new UI
tests. Workspace formatting, strict `agent-config` Clippy, and rustdoc passed
in `/home/a/heph-agent-config-ui-parser-fmt-v6-20260920.log`,
`/home/a/heph-agent-config-ui-parser-test-v6-20260920.log`,
`/home/a/heph-agent-config-ui-parser-clippy-v6-20260920.log`, and
`/home/a/heph-agent-config-ui-parser-doc-v6-20260920.log`. Focused
architecture validation also passed with `cargo +1.88.0 dev check architecture`
(61 enabled rules, 2 migration-gated, all semantic dry-runs clean); evidence is
`/home/a/heph-agent-config-ui-parser-architecture-v2-20260920.log`. This is
source parser validation only; capture, persistence, exact artifact/agent ID
resolution, authorized inspection, serving, and the aggregate UI checklist
remain unchecked.

## Source cross-manifest validation checkpoint (2026-09-20)

The source validator now resolves the exact gateway name and requires
authenticated exposure for managed UI content and declared UI APIs. It checks
the HTTP method and segment-wise route-prefix coverage, permits narrower UI
routes, and requires managed UI services to use `http.service.v1` with `GET`.
Missing or malformed gateway manifests, route broadening, public exposure, and
stateless managed references fail closed. Static UIs without APIs do not
require an otherwise unused gateway manifest, and persisted UI configuration
is revalidated before cross-manifest checks.

The focused suite passed 22 unit tests, one existing integration test, 10 UI
manifest tests, and 6 cross-manifest tests. Formatting, strict Clippy,
rustdoc, and focused architecture validation passed. Evidence is recorded in
`/home/a/heph-agent-config-ui-cross-test-v4-20260920.log` and
`/home/a/heph-agent-config-ui-cross-{fmt,clippy,doc,architecture}-v2-20260920.log`.
This checkpoint covers source validation only; capture, persistence, exact
artifact/agent ID resolution, authorized inspection, publication, serving,
browser behavior, and aggregate acceptance remain unchecked.

## Reviewed declaration architecture (planned, not implemented)

### Git inspection helper checkpoint (2026-09-20)

The bounded `forge-postgres` Git inspection helper is implemented and currently
registered under `cfg(test)` pending receive integration. It checks entry mode
and object headers before loading blobs, enforces the 256 KiB UI and 1 MiB
required-gateway caps, and preserves observed OIDs, sizes, and source hashes.
Missing UI preserves legacy behavior; invalid source entries and declarations
produce bounded redacted diagnostics. Corrupt regular objects remain fatal.
Valid gateway snapshots use canonical typed configurations with the existing
canonical TOML hash. Invalid snapshots retain observations without authoritative
normalized configurations.

Nine focused tests passed in
`/home/a/heph-forge-ui-manifest-tests-20260920-v4.log`. Strict scoped Clippy,
rustdoc, formatting, and architecture passed in the corresponding
`clippy-20260920-v5`, `doc-20260920`, `fmt-20260920-v4`, and
`architecture-20260920` logs. Production receive wiring and persistence adapter
integration remain pending; this checkpoint does not yet capture pushed UI
manifests.

### Capture persistence helper checkpoint (2026-09-20)

The PostgreSQL capture helper is implemented and registered under `cfg(test)`
pending receive integration. It inserts immutable source evidence in a caller
transaction, reuses identical repository/commit evidence, preserves the first
receive ID, and rejects conflicting evidence without an overwrite. It uses
read/insert privileges only and checked size/hash conversions. Build-link
creation remains a separate integration step.

The real PostgreSQL store test passed in
`/home/a/heph-ui-manifest-store-realpg-v5-20260920.log`; the checked-size unit
test also passed in `/home/a/heph-ui-manifest-store-unit-20260920.log`.
Formatting, strict scoped Clippy, rustdoc, and architecture passed in
`/home/a/heph-ui-manifest-store-{fmt,clippy,doc,architecture}-20260920.log`.
The v3 runner had a readiness race; v4 attempted to delete immutable fixture
rows during cleanup. V5 delegates disposal to container teardown and passes.

### Receive integration checkpoint (2026-09-20)

Production receive processing now inspects and persists present UI manifests
from the exact commit before an absent agent manifest can skip processing.
Invalid UI preserves the accepted ref and existing agent/run behavior while
preventing build creation. Valid UI uses the shared derived build identity and
links the returned build ID to the exact source revision in the same
transaction. Missing UI retains the legacy identity. The receive replay branch
now precedes Git storage validation and reuses durable results without Git.

The final receive suite passed seven tests, including valid UI linking,
invalid UI with and without an agent declaration, replay without Git, and one
capture shared by two ref-specific build requests. Evidence is in
`/home/a/heph-ui-manifest-receive-forgepg-v2-20260920.log`. The earlier complete
forge-postgres suite passed 20 tests across unit, integration, smart HTTP, and
schema checks. Strict Clippy, formatting, docs, and architecture passed in
`/home/a/heph-ui-manifest-receive-clippy-v3-20260920.log`,
`/home/a/heph-ui-manifest-receive-fmt-v2-20260920.log`, and the corresponding
`doc-20260920` and `architecture-20260920` logs.

Manual requests do not yet consume UI captures. Their next integration will
serialize with receive capture using a repository `FOR NO KEY UPDATE` lock
before inspection/lookup. This lock mode is compatible with foreign-key key
share locks and avoids concurrent receive lock-upgrade deadlocks. The ordering
must be proved with real concurrent PostgreSQL tests; no concurrency result is
claimed at this checkpoint.

### Exact-ID resolver checkpoint (2026-09-20)

The public pure resolvers in `agent-config::ui::{static_resolution,
gateway_resolution}` now bind validated declarations to caller-supplied
immutable IDs. Static resolution requires unique candidate paths/IDs, ordinary
file artifacts, and exact MIME equality. Gateway resolution requires unique
agent keys/IDs and exact gateway-agent key lookup after the existing
authenticated-exposure, service, method, and route validation. One release
agent may serve multiple declared gateways. Errors contain indexes rather
than source paths, MIME values, or agent names.

The full agent-config suite passed 55 tests (38 unit, one Cooking manifest,
six cross-manifest, ten UI parser) in
`/home/a/agent-config-ui-resolvers-test-20260920-v4.log`. Strict Clippy,
formatting, rustdoc with warnings denied, and architecture passed in
`/home/a/agent-config-ui-resolvers-clippy-20260920.log`,
`/home/a/agent-config-ui-resolvers-fmt-doc-20260920.log`, and
`/home/a/agent-config-ui-resolvers-architecture-20260920.log`.
These are resolution helpers; release publication does not invoke them yet.

### Publication contract (planned)

The reviewed v1 model optionally reads a repository sibling `heph.ui.toml`,
captured from the exact Git commit during receive/build-request creation
alongside the referenced `heph.gateways.toml`. An immutable build-request
snapshot is consumed atomically by `CompleteBuild` so UI child records freeze
with resolved artifact and agent IDs. Existing gateway declarations are read
from the same commit at install; UI publication additionally validates and
persists them at publication time.

Static directory outputs flatten to files. A static declaration therefore
maps each explicit route to an artifact path and MIME type, then resolves exact
artifact IDs at publication. Managed UI declarations reference the exact
same-release gateway name, route, and entrypoint. API access is declared by
explicit gateway, method, and route bindings.

Static resolution will require an ordinary `File` artifact at the exact path
with MIME equal to the declaration. Executables, manifests, and build logs are
not UI assets. Build output MIME defaults to `application/octet-stream`, so
static UI outputs must declare an allowed MIME explicitly. Duplicate candidate
paths or IDs fail resolution. Publication currently exports one agent; managed
gateway references must match that agent's exact key and supplied immutable ID.
Unknown agent keys fail publication rather than generating an agent ID or
introducing a separate multi-agent feature. These resolution rules remain
unimplemented until the focused resolver and publication checks pass.

The v1 limits are `no_store` caching, kit version 1, at most 16 UIs, 256
static files, 16 APIs, and a 256 KiB manifest. Route-prefix collisions are
forbidden within a scope, and UI keys are globally unique. Source-level parser
validation is implemented and tested as recorded above; capture, persistence,
publication, serving, browser, authority, and aggregate acceptance remain
unchecked.

Receive semantics remain planned: an absent `heph.ui` file preserves legacy
behavior. A present but invalid UI declaration accepts the Git push/ref update
while persisting redacted structured UI diagnostics and creating no
UI-bearing build request. A valid declaration is captured from the exact
commit and linked atomically to the build request. Database or Git-object
failures are fatal. Replay of an existing receive reuses durable results
without reinspection, and immutable-capture conflicts fail closed without
overwriting the capture.

The planned design stores a UI-specific source-manifest revision so an invalid
manifest can retain diagnostics without a build request. The gateway manifest
is captured only when the UI references a managed gateway or APIs; a static UI
without APIs does not require it. Existing `heph.images` behavior remains
outside this UI scope. Implementation remains pending.

### Capture schema checkpoint (2026-09-20)

The capture design uses two UI-specific append-only tables: one
`source_manifest_revisions` row for each present `heph.ui.toml`, and one
build-request link row. A missing `heph.ui.toml` creates no row and preserves
legacy behavior. A present manifest with a missing or invalid referenced
gateway creates an invalid manifest row with diagnostics but no UI-bearing
build request. Gateway evidence remains inline optional snapshot fields on the
UI revision; no generic manifest table is introduced.

Each revision binds exactly to `repository_id`, `source_commit`, and the Git
entry metadata. Invalid rows may retain the observed size even when it exceeds
256 KiB; symlink, tree, or gitlink entries may have no size or SHA-256. No
source bytes are stored. Valid rows require a regular blob, actual size at
most 256 KiB, source SHA-256, normalized UI configuration, and its hash.
Referenced gateway/API configurations additionally require a normalized
gateway snapshot and hash. Diagnostics are bounded to 64 entries and 32 KiB;
normalized UI and gateway JSON are bounded to 1 MiB and 2 MiB respectively.

The build link uses exact repository/commit composite foreign keys and a
constant-valid-status foreign key, preventing invalid or cross-repository
links. Same-commit capture is immutable and conflicting replay evidence fails
closed. Both tables use forced repository/build RLS, insert/read permissions
following the existing patterns, and append-only immutability triggers; the
trusted worker remains the only elevated writer. Existing receive transaction
and outbox mechanisms can carry future capture effects without a new event
framework.

Migration `0082_ui_source_manifest_revisions.sql` implements this boundary.
The source table is named `ui_source_manifest_revisions`; its build links are
in `build_request_ui_source_manifests`. All present entries require a Git OID;
valid referenced gateway sources are capped at 1 MiB. Multiple matching build
requests may share a snapshot. The real PostgreSQL schema test proves shape
checks, oversized invalid evidence, invalid-status linkage denial, exact
repository/commit binding, shared snapshots, immutability, and app-role
isolation, with SQLSTATE assertions for rejected operations.

The corrected schema suite passed against fresh PostgreSQL in
`/home/a/heph-ui-source-manifest-schema-realpg-final-20260920.log`.
Formatting passed; strict scoped Clippy and rustdoc passed in the corresponding
`clippy-final` and `doc-final` logs. Architecture subsequently passed with
61 enabled rules and two migration-gated rules in
`/home/a/heph-agent-config-gateway-canonical-architecture-20260920.log`,
after the Git helper fixture correction. Receive capture and adapter wiring
remain unimplemented.

### Build identity compatibility decision (historical design checkpoint)

A captured valid UI snapshot must participate in build identity. UI-bearing
builds will use a domain-separated SHA-256 over the existing build-definition
hash, normalized UI hash, and an explicit presence marker plus normalized
gateway hash when required. Hash components are fixed-width bytes; the domain
is `hephaestus.build-definition-with-ui.v1`. Without captured UI, the existing
build-definition hash remains byte-for-byte unchanged. Both receive-created
and manually requested builds must apply the same rule.

This prevents a newly captured UI snapshot from being attached retroactively
to a historical build that used the legacy identity. Multiple build requests
may share one immutable source snapshot, but each link must match the exact
repository and commit of its build request. Invalid captured UI must prevent
build creation through both paths. Historical commits without capture remain
legacy until inspected through the receive path; no historical build is
silently upgraded to carry UI. The manual API will accept either the trusted,
recomputed base hash or the recomputed UI-aware hash and store the UI-aware
hash when a valid snapshot exists. This preserves existing request callers
and clients retrying with the hash exposed by build inspection, without a
protobuf change. Both database paths remain to be implemented and verified.

The shared `agent-config::build_identity` helper is implemented. Its fixed
vectors verify the existing `BuildConfig` serialization and domain-separated
encoding, including optional gateway presence. The full agent-config suite
passed 25 unit tests, one Cooking manifest test, six gateway cross-validation
tests, and ten UI manifest tests in
`/home/a/heph-agent-config-full-tests-20260920.log`. Workspace formatting,
strict scoped Clippy, and rustdoc passed in
`/home/a/heph-agent-config-build-identity-{fmt,clippy,doc}-20260920.log`.
No database caller uses the new helper yet.

`canonical_repository_gateways` now exposes the existing gateway normalization
without changing the parser's returned source ordering. Its normalized hash
continues to use canonical TOML, whereas UI normalization uses JSON. The
existing gateway ordering test now verifies equal canonical configurations and
their TOML hash. The full agent-config suite, strict Clippy, rustdoc, formatting,
and architecture passed; evidence is in
`/home/a/heph-agent-config-gateway-canonical-{tests,clippy,doc}-v2-20260920.log`
and the architecture log above.

## Outcome

Let a published release provide a user interface that Hephaestus can mount as
a project or repository tab, or register as an explicitly installed global
interface such as the distribution's Assistant entry point. The release UI may
be either a static artifact served by Hephaestus or an application served from a
Hephaestus-managed VM. It reaches only explicitly declared gateway APIs and
inherits the platform's authorization, lifecycle, revocation, and audit
boundaries.

The first usable result may be visually minimal: a tab that hosts a same-site
iframe or opens a full-page release UI. It must nevertheless be a real,
securely bounded distribution surface, not an arbitrary external URL or an
escape hatch for custom HTML in core pages.

## Locked decisions

| Area | Decision |
| --- | --- |
| Declaration compatibility | The first declaration schema is version 1; unsupported versions are rejected. |
| UI ownership | A release owns its routes, markup, client behavior, and domain semantics. Hephaestus owns publication, serving, authorization, lifecycle, gateway mediation, and the bounded host shell. |
| Artifact kinds | A release may declare a static UI artifact or a UI service executed in a Hephaestus-managed VM. Both are immutable release inputs and are served only after the ordinary release/runtime checks succeed. |
| Origin and routing | Deployment routes are platform-owned under the exact installation, release, and project binding. Hephaestus serves a release UI through a gateway-owned same-site route. Direct arbitrary third-party iframe URLs are unsupported. |
| Declaration paths | UI declaration paths are relative path components; they are not arbitrary URLs. |
| Browser origin | UI JavaScript never runs on the Phoenix host origin. A dedicated origin is required per immutable installed UI binding, including the exact release/UI/generation/project context; unrelated release UIs do not share one origin. This prevents full-page/opener same-origin access across bindings and from a revoked revision to a new revision. Origin routing, CSP, and runtime behavior remain unimplemented. |
| Artifact references | Source references resolve to immutable artifact or agent IDs before publication; authorized inspection omits opaque storage keys. |
| Managed UI gateway authority | All managed UI content and declared UI API gateway references require `heph_authenticated` exposure to prevent public bypass. Source cross-manifest validation resolves the exact gateway name, requires the declared prefix to cover the selected UI route segment-wise, requires the exact HTTP method, and requires managed UI to use `http.service.v1` with `GET`. Narrower UI routes are allowed; route broadening, public exposure, and stateless managed references are denied. This is source validation only; runtime authorization is not implemented here. |
| Embedding | A bounded tab or global-interface declaration can select a scope, label, icon, route, and initial presentation (`iframe` or full page). The host rejects undeclared routes and does not let arbitrary project content alter core navigation. |
| API access | A UI reaches APIs only through explicitly declared gateway routes/capabilities. It receives no ambient database access, platform bearer token, raw secret, or authority greater than its release declaration. |
| Authentication | The host establishes the human browser session. The UI does not receive reusable human credentials; any browser-to-service identity handoff is audience-bound, short-lived, revocable, and scoped to the exact release UI route/API. |
| Isolation | Iframes use a restrictive sandbox and CSP. Static UI has no server-side execution. VM UI service authority is an exact runtime/gateway capability, not host or project authority. |
| Revocation | Removing access, disabling a release/UI tab, changing a route binding, ending a runtime, or revoking the browser session stops future serving/API use and gives a deterministic embedded failure state. |
| UI kit | Hephaestus publishes a versioned, documented UI-kit package derived from its design-system tokens/components. Projects may reuse it, but it does not expose Phoenix internals or bypass the bounded distribution API. |

## Dependencies

### Browser-origin deployment findings (2026-09-20)

Current local Phoenix uses loopback port 4000; production takes `PHX_HOST`.
Its session cookie has no Domain attribute. Current Caddy reconciliation
replaces a single `hephaestus.gateway` subroute with path-based routes and
forwards to one trusted dispatcher authority. It has no release-UI host route
or UI namespace, and the repository supplies no production wildcard DNS or
certificate configuration.

UI deployment will require an explicitly configured platform-owned hostname
namespace, with each immutable installed binding receiving its own hostname.
DNS and TLS must cover those hostnames; production configuration remains an
operator responsibility, while integration tests must use explicit local
resolution and a trusted test CA. Host routing and binding lookup must be
reconciled through the existing Caddy owner so gateway and UI updates cannot
overwrite each other. Unknown UI hosts must fail closed. The configured service
dispatcher authority remains distinct from browser UI origin identity.

This is a deployment constraint and implementation direction, not evidence of
working UI routing or browser authentication. Audience-bound handoff, exact
host-to-binding resolution, cookie/CSP policy, revocation, and real browser/TLS
acceptance tests remain outstanding.

- Gateway routing and invocation authority from MVP-03.
- Runtime authority and release-artifact materialization from MVP-01 through
  MVP-04.
- The existing Phoenix design system remains the host-shell authority; this
  task does not loosen its raw-markup or arbitrary-class restrictions.

## Non-goals

- Arbitrary external iframe URLs, unrestricted custom HTML in Hephaestus
  pages, or loading project JavaScript into the core Phoenix origin.
- A generic chat/session protocol, generic workflow/forms engine, WebSocket
  passthrough, generic file upload, or provider-specific UI semantics.
- Giving release UIs host shell access, database access, raw secret values,
  long-lived user tokens, or cross-project/release authority.
- Replacing the existing gateway, browser authorization, or design system.

## Implementation checklist

### Static asset size checkpoint (2026-09-20)

Static resolution now consumes the importer's immutable byte length and rejects
referenced files above 16 MiB or total unique referenced artifacts above 64 MiB
per release. Shared IDs count once across declarations; unrelated build outputs
do not consume the UI budget. Exact path, file-kind, and MIME checks precede
size accounting. The existing declaration bounds already limit references to
4,096 files. Static HTTP streaming and concurrency limits remain unimplemented.

All 41 agent-config unit tests and 17 integration tests passed, including ten
focused resolver tests. Exact size boundaries, one-byte overflow, repeated
references, and unrelated oversized outputs are covered. Scoped strict Clippy,
rustdoc, formatting, and architecture also passed.

### Manual-build integration checkpoint (2026-09-20)

Manual requests now read the exact immutable UI capture, reject invalid captures,
accept either the recomputed base or derived hash for valid UI, and persist the
derived hash in the build and its outbox event. Missing captures preserve the
legacy hash. Both manual and receive paths acquire the same repository
`FOR NO KEY UPDATE` lock, compatible with receive foreign-key key-share locks.

Real application-role testing exposed two existing permissions issues. Migration
83 grants catalog SELECT to the application under the existing forced-RLS public
catalog policy, and application startup expects schema 83. Manual insertion now
preselects under the repository lock, inserts without `ON CONFLICT`/`RETURNING`,
then reads after the build authorization tuple exists. This preserves RLS.

The real PostgreSQL basic matrix passed (1/1), covering base/derived deduplication,
exact capture linking, invalid rejection without build or outbox insertion,
legacy absence, and wrong-hash rejection. Production bootstrap passed (1/1,
35 filtered). Concurrent receive capture passed separately (1/1). Scoped
control-plane/forge/app Clippy, rustdoc, formatting, and architecture checks
passed; logs use `/home/a/heph-manual-ui-build-` with `realpg-20260920.log`
and `{control-clippy,forge-clippy,app-clippy,doc,fmt,architecture}-v2-20260920.log`.
The receive writer's application-role reusable-build path, manual/receive
ordering, and historical no-retrofit regression remain to be verified.

### UI-kit package checkpoint (2026-09-20)

`web/assets/release_ui_kit` now provides the standalone CSS package
`@hephaestus/release-ui-kit` version 1.0.0. It builds from the existing semantic
token authority and a small prefixed component layer, with deterministic
versioned output and SHA-256 manifest. Generated files remain ignored;
`prepack` builds them and a nonmutating check detects stale or missing output.
No registry publication has been performed.

Node tests, stale-output rejection, the check command, and offline package
file-list validation passed. Browser smoke used the actual generated CSS and
document `data-theme` attributes, with no token overrides. It verified light
and dark styles, overflow/insets, keyboard focus, and asset loading. Primary
button text contrast measured 5.44/6.63 in light default/hover and 7.79/5.45
in dark default/hover. The final CSS hash is
`79a23f981523db70c39e07be76830be59f419d2b7d3336485fe56b8a09af5c1d`.
The smoke script is `/home/a/heph-ui-kit-browser-smoke-20260920.mjs`; reviewed
screenshots are `/home/a/heph-ui-kit-smoke-light-20260920.png` and
`/home/a/heph-ui-kit-smoke-dark-20260920.png`. These checks do not replace
the reference-release browser flow or a complete accessibility audit.

`cargo dev check ui` and `cargo dev quality` now run the kit's `npm test`
before the existing Phoenix UI checks. The existing Node 24 browser CI job
runs the same command without an additional dependency installation. Direct
kit tests, workflow YAML parsing, formatting, strict hephaestus-dev Clippy,
rustdoc, and architecture checks passed. Rust evidence is recorded in
`/home/a/heph-kit-gate-{clippy,doc}-20260920.log` and
`/home/a/heph-architecture-final-20260920.log`.

- [ ] **1. Specify the distribution UI declaration and publication model**
  - [ ] Define a versioned release declaration for UI artifact kind, immutable
    artifact reference, scope (`project`, `repository`, or explicitly installed
    `global`), bounded route base, tab label/icon, presentation mode, declared
    gateway APIs, and compatibility behavior.
  - [ ] Validate names, paths, route ownership, icon allowlist, MIME types,
    artifact size, entrypoint, cache policy, and duplicate/conflicting tabs.
  - [ ] Persist an immutable published UI declaration tied to the release and
    expose only redacted, authorized inspection projections.

- [ ] **2. Serve static artifacts and managed UI services**
  - [ ] Materialize and integrity-check static UI artifacts from the published
    release; serve only safe declared files with deterministic MIME types,
    cache headers, range/size limits, path traversal rejection, and no
    directory listing.
  - [ ] Define the managed-VM UI service lifecycle: launch, readiness,
    health, restart/recovery, cleanup, resource limits, and failure state.
  - [ ] Route both artifact kinds through the gateway-owned same-site base;
    prevent a service from claiming another release/project route.

- [ ] **3. Add bounded host navigation and embedding**
  - [ ] Add a release/UI-tab projection to the appropriate project/repository
    page, and add an explicitly authorized global-interface projection for
    releases installed as global entries; render declared tabs through the
    existing design system.
  - [ ] Implement exact `iframe` and full-page presentations with a stable
    loading, unavailable, access-revoked, and terminated state.
  - [ ] Apply a restrictive iframe sandbox, referrer policy, CSP `frame-src`
    and `frame-ancestors` policy, and an allowlist of same-site release routes.
  - [ ] Prevent embedded content from navigating the parent, redefining core
    tabs, rendering into the host DOM, or using undeclared origins.

- [ ] **4. Bind browser and gateway authority**
  - [ ] Define the scoped browser-to-release-UI/API handoff: audience,
    expiration, replay resistance, route/release/project binding, and safe
    revocation behavior.
  - [ ] Enforce the declared API bindings at the gateway and reauthorize every
    UI route/API request against the current human/project/release state.
  - [ ] Audit allowed and denied UI serving, embeds, handoffs, and API calls
    without recording user payloads, credentials, or secret values.

- [ ] **5. Publish the reusable UI kit**
  - [x] Choose and document a versioned package format that projects the
    design-system tokens and supported components without coupling projects to
    Phoenix internals.
  - [ ] Provide a small static and managed-service reference UI that uses the
    package, including day/night theme behavior and accessible tab content.
  - [ ] Document compatibility, upgrades, local development, release build,
    serving, and the boundary between reusable presentation and host authority.

- [ ] **6. Prove isolation, recovery, and user flow**
  - [ ] Add unit/property tests for declaration/path validation, MIME/path
    handling, artifact integrity, tab collision, handoff binding, and audit
    redaction.
  - [ ] Add real gateway/VM/static-artifact tests for authorized serving,
    service restart, failed readiness, cleanup, route conflict, and release
    removal.
  - [ ] Add browser tests for tab navigation, iframe/full-page rendering,
    dark/light theme, error/revocation state, and blocked parent-navigation or
    undeclared-origin attempts.
  - [ ] Add negative tests for cross-project/release access, stale/replayed
    handoff, undeclared APIs, arbitrary URLs, traversal, unsafe MIME handling,
    CSP/sandbox escape attempts, and raw-secret/token disclosure.

- [ ] **7. Verify and document**
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run `cargo dev quality` and `git diff --check`.

## Completion evidence

Record the declaration schema/version; static and managed-service release
fixtures; project/repository and global-interface registration evidence; artifact
integrity and route-binding evidence; browser screenshots or tests for both
presentation modes and themes; allowed/denied gateway and browser authority
fixtures; sandbox/CSP checks; revocation/restart/cleanup evidence; UI-kit
package/version and reference project; and all verification results.

Only after this task is complete may a distribution journey such as MVP-06 use
a release-owned chat tab.
