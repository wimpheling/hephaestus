# Release-owned distribution UI surfaces

Owner: Astra orchestration / Luna bounded subtasks

## Current status

Work resumed on 2026-09-20 from the
[session handoff](release-ui-session-handoff-2026-09-20.md). Isolated-database
CI execution, organization-owned static installations, disable/remove lifecycle,
browser credential storage, handoff issuance/exchange, managed/API installation,
and application-role request authentication have passed focused verification.
Activation/rollback and explicit-organization navigation also pass focused
verification. The UI RPC transport checkpoint now passes its expanded real
matrix. Current work verifies gateway admission and prepares hosting and
Phoenix integration.
The handoff remains the record of the stopped session; new evidence is recorded
below as each resumed slice passes review and verification.

Persistent services are a completed prerequisite on the same branch and PR 51;
service checkpoint `8c1fb51` passed repository-wide quality. Its later SQLx
cancellation fix has focused runtime and CI evidence in the completed task.
The retained-cleanup CI follow-up now has a focused-validated fixture isolation
fix. CI passed at `9c0c0ac` (`35495099582`); final integrated quality remains
required. The original CI lock sequence was not captured directly.

Release UI declaration, immutable capture, receive/manual build identities,
exact artifact/agent resolution, transactional publication, and authorized
inspection through `GetRelease` are implemented and focused-validated. Checklist
section 1 is complete. The versioned CSS kit and its local/CI checks are also
implemented; reference-release integration remains open.

Verified static-byte loading and read-only release-page metadata display are
implemented. Durable browser sessions are integrated through RPC and Phoenix.
UI installation is the current slice; static/managed hosting, browser
navigation/authorization/isolation, real Caddy/VM/browser proofs, and the final
integrated quality gate remain incomplete. The user approved organization-owned
global installations and organization isolation. Historical checkpoints record the state
at their time; later integration checkpoints supersede earlier pending notes.

## Reference UI theme helper checkpoint (2026-09-20)

The optional versioned kit helper applies bounded light/dark hints, accepts theme
messages only from the configured exact parent origin and window, and announces
iframe readiness to that origin. Missing or invalid hints retain the existing
system-theme CSS fallback. It uses no credentials or storage. Both reference
releases declare and build the helper; the managed service serves its exact
JavaScript route with a closed MIME value.

Kit `npm test` (including four helper cases and the managed HTTP route regression)
and `npm run check` passed. Both fixture builds passed in disposable output
directories. An ephemeral loopback service returned status 200,
`text/javascript; charset=utf-8`, and the exact 2,106 canonical helper bytes,
then shut down cleanly. Evidence is in
`/home/a/heph-ui-kit-npm-{test,check}-20260920.log`,
`/home/a/heph-reference-ui-build-20260920.log`,
`/home/a/heph-reference-service-ui-build-20260920.log`, and
`/home/a/heph-reference-service-ui-route-20260920.log`.
Installed Caddy/VM/browser acceptance remains outstanding.

## Recovery teardown follow-up (2026-09-20)

The successful active-route read now awaits SQLx connection return. The isolated
recovery fixture repeatedly drains its known pools within the existing ten-second
deadline, retaining the actual session-free database assertion and diagnostics.
It does not terminate backends or force database deletion. A detached SQLx return
can race pool shutdown; the precise owner of the original CI backend remains
unestablished.

The failing test passed once and then five fresh repetitions; the full sequential
recovery group passed all 17 tests. The active-route cancellation/reuse regression
also passed with zero retained sessions. Strict scoped Clippy, documentation,
formatting, architecture, and diff checks passed. Evidence:
`/home/a/heph-ci-teardown-exact-v1-20260920.log`,
`/home/a/heph-ci-teardown-exact-repeat-v2-20260920.log`,
`/home/a/heph-ci-teardown-group-v6-20260920.log`, and
`/home/a/heph-gateway-active-routes-reuse-v1-20260920.log`.

## UI host/resource projection checkpoint (2026-09-20)

Migration 92 adds narrowly granted application-role functions for active
generation-host existence and authenticated resource projection. The host read
returns only the generation ID. The resource function invokes migration 90's
canonical verifier and selects the exact current declaration in one SQL
statement; it does not duplicate permission predicates. Both functions pin
their search path and revoke public execution. Application-role direct access
to the underlying private tables remains denied.

The application port accepts a checked raw HTTP path and method. It rejects
zero or ambiguous authorized matches, preserves ordinary managed/API paths,
and returns an explicit authenticated Redirect for static or managed document
base aliases. Static results carry immutable hash/size/MIME/cache metadata for
the later verified-byte reader; no artifact bytes or HTTP behavior are claimed
by this projection checkpoint.

Two real restricted-role tests pass in
`/home/a/heph-resource-real-v17-20260920.log`, covering both redirects, static
GET/HEAD, long managed paths, app table denial, gateway/account/lifecycle
revocation, and an exact two-candidate legacy ambiguity that fails closed.
Seven host/path units pass in
`/home/a/heph-resource-host-path-unit-v2-20260920.log`; the app-pool test at
migration 92 passes in `/home/a/heph-app-pool-floor92-20260920.log`. Scoped
Clippy, rustdoc, architecture, formatting, and diff checks pass. Disposable
containers were cleaned.

## Phoenix installed-navigation checkpoint (2026-09-20)

Organization workspace, project, and repository-files pages now render the
scoped installation projection through the existing design system. Safe
pagination state retains per-page cursors, stable display order, and fresh
generation precedence. Bounded refreshes prioritize an active entry's page;
replacement, removal, disablement, or revoked access closes the frame. Invalid
cursors clear retained pagination and active selection.

The hook supports iframe and full-page launches, canonical HTTPS generation
hosts and configured ports, current host theme plus exact-origin theme updates,
strict frame/source message checks, a loading timeout, persistent terminal
status outside the hidden frame, and explicit-close focus restoration. Handoff
URLs remain one-shot events rather than LiveView assigns. The shell supplies
the configured frame namespace and restrictive iframe sandbox.

Four named Node harness tests and 27 focused Phoenix helper/navigation/page/RPC
tests pass, along with formatting and asset generation, in
`/home/a/heph-ui-phoenix-navigation-20260920-final.log`. Direct Config.Reader
regressions subsequently verified ordinary no-port HTTPS origins, canonical
default/nondefault ports, and invalid origin rejection in
`/home/a/heph-ui-runtime-origin-regression-20260920.log`. These checks do not
prove installed rendering through the still-unwired UI listener; real browser
isolation, failure, and accessibility acceptance remain open.

## UI gateway admission checkpoint (2026-09-20)

The edge now has a distinct UI admission path sharing the established VM
execution and invocation accounting core. The public path remains public-only.
The UI envelope contains safe child/actor/tenant/installation/generation
identities and the exact request tuple, never a caller-selected gateway or
revision. The worker derives current bindings and calls the canonical
migration-90 verifier. Migration 91 grants that verifier's exact signature to
the worker without broadening application table access.

The recorder defaults to denial unless it implements UI admission. Its worker
implementation locks the route and service authority, rechecks the child after
all lock waits, and invokes the canonical verifier again in the final invocation
INSERT. That same statement checks the safe identities, route/method, service
readiness, fencing token, and fresh lease. Managed paths are mapped from the
descriptor base, and query strings remain opaque, including an empty `?`.
Browser credentials and forwarding headers are stripped before guest execution;
guest Set-Cookie rejects the response before success accounting.

The real restricted-role PostgreSQL matrix passes three tests in
`/home/a/heph-gateway-authority-final-v8-20260920.log`, including observed
service-lock waits followed by parent revocation or child expiry with no
invocation row. All 191 edge unit tests pass in
`/home/a/heph-gateway-edge-unit-final-v5-20260920.log`. Strict edge/adapter/test
Clippy, both crates' rustdoc, workspace formatting, and architecture pass in
the corresponding final gateway logs. Disposable runtime resources were
cleaned. Actual HTTP bridge/listener wiring and request audit remain pending.

CI at RPC checkpoint `cbbc026` failed an existing service-recovery teardown:
an idle control-pool connection remained after pool closure. The failed log is
`/tmp/heph-ci-cbbc026-failure.log`; investigation is in progress. This is not
counted as a green whole-repository gate.

## Bootstrap script checkpoint (2026-09-20)

The unregistered bootstrap handler now includes its production JavaScript from
a separate file. The actual script clears the bearer fragment before any
query/theme rejection, sends exactly the one-time fragment as the POST body,
and forwards only the canonical theme. It rejects unknown/duplicate query
parameters, foreign platform origins, and unsafe response paths, including
terminal dot segments before URL normalization. The final document receives
only appearance metadata and the exact configured platform origin.

Eleven named Node tests pass in
`/home/a/heph-ui-bootstrap-script-20260920.log`, using
`node --test --test-isolation=none --test-reporter=tap
crates/hephaestus-app/tests/ui_bootstrap_script.mjs`. File-wrapper-only test
output is not counted as named execution evidence. The Rust handler remains
unregistered and its HTTP tests and real browser flow are still pending.

## UI request audit design (2026-09-20)

Request-level UI audit will use a dedicated append-only stream, following the
existing capability-audit model. It will not advance installation owner event
revisions or trigger navigation refreshes for each asset request. These records
are audit persistence, not product events; any product events continue to use
committed outboxes.

The provider-neutral port will accept closed surface, decision, outcome, and
reason values with request correlation and optional safe actor, organization,
installation, generation, child-session, and gateway identities. Anonymous
denials remain anonymous; caller-supplied identities must not be represented as
verified authority. Parent session IDs, credentials, digests, headers, paths,
queries, and payloads are excluded structurally. Successful handoff writes and
their audit record share a transaction. Denied attempts need an independent
append so command rollback does not erase evidence. Existing gateway invocation
accounting remains authoritative for accepted execution and completion.

The worker-owned adapter, migration, denial/rollback behavior, redaction tests,
and HTTP/gateway integration remain unimplemented. A successful audit lookup
or logging statement alone is not acceptance evidence.

## UI RPC generation and unit checkpoint (2026-09-20)

The seven ReleaseService UI methods now have generated Rust/Elixir bindings and
application adapters for installation, activation, rollback, disable, removal,
navigation, and handoff issuance. Install passes the explicit organization into
the locked application command; lifecycle mutations require generation CAS.
Repository receipts use the repository aggregate and project primary scope.
Handoff issuance obtains the canonical parent from middleware, accepts exactly
32 sensitive request bytes, and rejects idempotency keys. Its receipt-policy
exception is limited to this ephemeral operation.

Generation succeeds. Fifteen descriptor-policy tests and one protocol interop
test pass in `/home/a/heph-backend-rpc-proto-tests-20260920-v2.log`; ten release
RPC tests pass in `/home/a/heph-backend-rpc-app-release-tests-20260920.log`.
Cursor tests include a binary HMAC containing the delimiter byte; decoding uses
a fixed-length signature boundary and constant-time verification. The app
compiled before the subsequent serving modules were introduced. Proto Clippy,
architecture, scoped formatting, and diff checks pass. Generated doctests were
ignored and are not counted as executed tests.

The first real production-router matrix also passes with disposable PostgreSQL
and NATS in `/home/a/heph-ui-installation-rpc-real-v14-20260920.log`: exact
install replay, listing, 32-byte handoff input and parent redaction, all five
mutations, stale CAS, wrong audience, expiry, and revocation. The runner requires
the real completion marker and selects the isolated test explicitly. UI browser
writes now use the existing worker-role pool rather than the general
control-plane pool. Disposable resources were cleaned after the run.

The expanded real matrix passes in
`/home/a/heph-ui-installation-rpc-real-v24-20260920.log` (one executed test):
organization/repository installs and receipts, explicit wrong-tenant rejection,
cross-actor/organization/target cursors, and tampered cursors supplement the
earlier cases. The runner inherits its caller's Cargo target directory and
cleans its disposable database. No live test handles remain.

App all-target Clippy passes in
`/home/a/heph-ui-rpc-clippy-app-final2-20260920.log`; gateway Clippy, architecture,
workspace formatting, generated-code validation, and app rustdoc pass in the
corresponding `/home/a/heph-ui-rpc-*-final*-20260920.log` files. Integrated
serving verification remains in progress. These results do not establish a
working installed browser flow. Main CI
at the preceding committed prerequisite
`ce8e12d` passes in run `35514511262`; its Cooking run is still pending.
The isolated UI RPC runner still needs quality/CI integration; it must not run
against the shared workspace-test database. Final quality must execute this
proof as well as the already isolated browser-session lifecycle proof.

## Phoenix UI client helper checkpoint (2026-09-20)

The Phoenix client now lists installed UIs within an explicit organization and
owner target, preserves scoped cursors, and projects generated oneof/enum values
into safe navigation metadata. Handoff creation generates 32 cryptographic
bytes, uses a fresh request ID with no idempotency key or retries, and exposes
the bearer only to a successful transient launch callback. Default responses
contain safe metadata only.

The URL helper constructs the exact generation HTTPS bootstrap URL and checks
namespace, UUID, route, theme, canonical port, and fragment constraints. Eleven
focused client/helper tests pass, including all owner projections and failure
without retry or callback. Scoped formatting passes. Checks ran through
`localhost/hephaestus-elixir-review:latest` with Podman because native Elixir/Mix
are absent. Navigation, embedding, and real browser verification remain open.

## Authenticated parent-session context checkpoint (2026-09-20)

Active mediator authentication retains the canonical internal browser-session
ID already returned by the session store. Signed-only routes leave it absent;
revocation still performs no active-session lookup. Future handoff handlers can
derive parent identity from this trusted context without accepting it from a
client or introducing another lookup.

All twelve focused RPC authentication tests pass in
`/home/a/heph-auth-metadata-20260920.log`, including exact active identity,
signed-only absence, existing expiry/revocation/audience/SID behavior, and
redaction. Formatting and diff checks pass; prior app-floor Clippy/docs cover
the same implementation. No UI RPC exposure is claimed by this context change.

## RPC installation tenant guard checkpoint (2026-09-20)

General install accepts an optional expected organization for the forthcoming
RPC boundary. It checks that organization immediately after locking the owner,
before replay or writes. Explicit tenant expectations use a distinct v2 input
digest; internal callers with no expectation retain the prior v1 digest and
static-wrapper replay compatibility.

Nine domain tests pass, preserving existing golden vectors and distinguishing
explicit organization values, in `/home/a/heph-tenantguard-domain-20260920.log`.
Three real PostgreSQL tests pass in `/home/a/heph-ui-tenantguard-20260920.log`,
covering dual-member wrong-tenant denial, a matching tenant, changed tenant with
the same caller key, static replay row/event counts, and the nine-case generic
binding/permission regression. Scoped formatting, Clippy/docs, architecture,
and diff checks pass in `/home/a/heph-tenantguard-*-20260920.log`. Disposable
resources were cleaned; an unrelated existing development server was preserved.
No RPC endpoint is claimed by this application-layer guard.

## Installed UI navigation checkpoint (2026-09-20)

The release application port and application-role PostgreSQL navigator project
safe current-generation metadata for one explicitly requested organization and
global/project/repository target. Pagination is bounded and cursors must belong
to that exact target and tenant. Removed entries are excluded; disabled entries
remain unavailable. Current source read controls metadata visibility, while
source/agent use contributes a launchability hint that never grants serving
authority. No runtime URL, credential, or gateway binding authority is returned.

The real application-role navigation matrix passes in
`/home/a/heph-ui-navigation-20260920.log`, including all owner scopes, dual-org
membership, paging/cursors, invalid sizes, removed exclusion, source-use denial,
source-read hiding while target access remains, and owner revocation with the
other organization still visible. Scoped Clippy/docs, formatting, architecture,
and diff whitespace checks pass in `/home/a/heph-ui-navigation-*-20260920.log`.
No disposable test resources remain. RPC exposure and Phoenix rendering still
require separate integration and browser checks.

## Activation and rollback checkpoint (2026-09-20)

Activation and rollback both append a new immutable generation with incremented
generation number and newly resolved current bindings. Neither repoints a
historical generation. Current owner authorization precedes exact receipt
replay; fresh commands enforce generation CAS, stable UI key/scope, same-tenant
source authority, and terminal removal. Generation/bindings, pointer/lifecycle,
command outcome, event, and outbox commit together.

The real PostgreSQL generation matrix passes project, repository, and global
owners, fresh IDs/history, repeated source with a new caller key, exact replay,
source-revocation replay, input/CAS conflicts, and invalid-binding row absence in
`/home/a/heph-ui-installation-generation-matrix-20260920.log`. Six affected
installation/lifecycle/concurrency regressions pass in
`/home/a/heph-ui-installation-regressions-20260920.log`, including the nine-case
binding/permission denial matrix. Release and app-floor Clippy/docs, formatting,
architecture, and diff whitespace checks pass in
`/home/a/heph-ui-installation-*-final-20260920.log`. The accompanying migration-90
change removes one trailing space only; its runtime behavior is unchanged.
No disposable databases or test processes remain. External RPC CAS enforcement
and generation-origin browser behavior still require their own integration.

## Application-role UI verifier checkpoint (2026-09-20)

Migration 90 adds a restricted SECURITY DEFINER child-session verifier; the
application role receives function execution without direct credential-table
access. It derives the actor from the canonical parent, ignores caller actor
GUCs, and checks child/parent time, revocation, account state, current enabled
generation, tenant, publication, permissions, route, and the complete current
managed/API binding set. Static zero-API requests need no gateway permission.
Static UIs with APIs also fail closed when those bindings lose authority.
The Rust adapter uses its application pool for verification and implements the
existing session-store port; worker issuance/exchange remain separate paths.

Four real PostgreSQL schema/issuance/exchange/authentication tests pass in
`/home/a/heph-ui-browser-schema-auth-20260920.log`. They cover static base/assets,
managed base/descendants, exact API methods/routes, paused gateways, revision
cutover, revoked publication, spoofed actor context, direct-table denial, parent
time/revocation, lifecycle/current-generation changes, and retained concurrency
proofs. A distinct source/target fixture proves target read remains allowed
while source read is revoked, plus release-agent use revocation; cross-project
and global positive cases pass. The source/target evidence marker explicitly
records these permission states.

Production application-pool bootstrap passes at migration 90 in
`/home/a/heph-app-pool-migration90-20260920.log`. Scoped release and app-floor
Clippy/docs, workspace formatting, and architecture pass in the
`heph-browser-*-auth-final*-20260920.log` files. Architecture reports 61 enabled
rules and two migration-gated rules. Disposable databases and processes were
cleaned. HTTP serving, cookies, RPC wiring, audit, and browser acceptance remain
open. General-installation commit `9c58d98` passes all CI jobs in run
`35512072235`, which predates this verifier checkpoint.

## General UI installation checkpoint (2026-09-20)

The general installer supports static and managed UIs with declared APIs. It
pins preexisting enabled gateways and their exact active release-agent revisions,
authenticated exposure, methods, and segment-covered published routes. Source
and target remain in one organization; cross-project/global reuse requires
current source-project read and release/agent use. Gateway read authority derives
from its project. Installation does not create or retarget gateways or services.
Owner authorization precedes durable exact replay; replay returns its receipt
without re-resolving mutable source or gateway state. The static zero-API entry
point remains a compatibility wrapper.

Thirteen installer/regression tests and four schema/RLS tests pass in
`/home/a/heph-general-install-final3-20260920.log` and
`/home/a/heph-general-install-schema-20260920.log`. Nine additional real
PostgreSQL denial cases pass in
`/home/a/heph-general-install-negatives-success-20260920.log`: wrong release,
wrong agent, public exposure, paused gateway, inactive revision, disabled route,
wrong method, revoked source-project read, and revoked release-agent use. Each
denial retains baseline installation/generation/binding/command/event/outbox
counts. The resolver now locks source parents before insertion, so the tenant
move test observes that earlier lock and rejects before insertion; the historical
post-insert rollback proof describes the older static implementation.
Scoped formatting, Clippy, and docs pass in the general-install final logs.
Activation/rollback, navigation, and browser verifier integration remain separate
uncommitted slices at this checkpoint.

Exchange commit `5796b46` passes all CI jobs in run `35510440397`; this result
predates general installation and does not replace final integrated quality.

## Browser handoff exchange checkpoint (2026-09-20)

The worker adapter atomically exchanges one generation-bound handoff for one
digest-only child session, then consumes the handoff in the same transaction.
It derives actor and parent from the locked handoff and shares current authority
checks with issuance. Expiry is checked after all mutable-state locks, including
gateway waits; child lifetime is twelve hours capped by the parent expiry.
Static UIs without bindings retain target-read and release-use requirements.
Managed/API binding checks use the source project and exact pinned gateway
revision without requiring a running service instance before admission.

The real PostgreSQL suite passes all three schema, issuance, and exchange tests
in `/home/a/heph-ui-browser-exchange-final4-20260920.log`. Two named worker
connections are observed waiting on the same handoff before release; exactly
one exchange succeeds. Tests check exact child digests, lifetime caps, replay,
wrong generation, and denial without child creation or handoff consumption.
Scoped formatting, Clippy, rustdoc, and architecture pass in
`/home/a/heph-ui-browser-exchange-release-*-20260920.log`. Disposable databases
were cleaned. Managed/API runtime fixtures, the application-role verifier,
transport, audit, and HTTP serving remain incomplete.

Issuance checkpoint `e9da357` also passes all CI jobs in run `35509613687`.
This CI result predates the exchange changes and is not the final quality gate.

## Browser handoff issuance checkpoint (2026-09-20)

The worker adapter issues digest-only, sixty-second handoffs for the exact
enabled installation generation and published route. It binds the authenticated
actor to the canonical parent session, derives the tenant from the owner, and
checks current owner/source authority. Mutable account, session, owner,
installation, and release state are locked before a fresh database-time check;
immutable publication rows require no added write grants.

The real PostgreSQL issuance matrix passes project, repository, and organization
owners, exact domain-separated digest bytes, request provenance, wrong actors,
revoked/suspended/expired/future parent state, wrong routes, stale/disabled/removed
installations, target/source permission revocation, and revoked releases. A named
connection and observed blocker prove expiry rejection after an account-lock
wait. Both schema and issuance tests pass without skips in
`/home/a/heph-ui-browser-schema-expanded-final-20260920.log`; the focused issuance
run is `/home/a/heph-ui-browser-expanded-final-20260920.log`.
Scoped formatting, Clippy, rustdoc, and architecture pass, with logs under
`/home/a/heph-ui-browser-release-*-final-20260920.log`. Disposable databases
were cleaned. Exchange, application-role authentication, managed/API runtime
fixtures, audit, transport, and serving remain unverified and incomplete.

## Resumed CI isolation checkpoint (2026-09-20)

CI and the opt-in local quality gate now use the same browser-session lifecycle
runner. It creates a fresh database on the supplied PostgreSQL server, requires
both service URLs, and requires an explicit successful lifecycle marker. It drops
the database on exit. Host `psql` is the CI path; local container administration
requires explicit `HEPHAESTUS_BROWSER_SESSION_POSTGRES_MODE=container` and a
matching `HEPHAESTUS_POSTGRES_CONTAINER`.

With `REAL_APP_BROWSER_SESSION_RPC=1`, `cargo dev quality` removes the flag from
its shared workspace test pass and runs the isolated proof afterward. This
supersedes the earlier instruction to run the opt-in test inside the shared
workspace database. The final quality run must enable this proof.

The current runner passed against fresh disposable PostgreSQL/NATS: one lifecycle
test, the `REAL_BROWSER_SESSION_LIFECYCLE=1` marker, and zero isolated databases
remaining after cleanup. Workspace formatting, architecture, scoped
release-domain/release-service/dev tests, strict release-domain/release-service/
release-postgres/dev Clippy, and docs passed with Rust 1.88.0. This is focused
local evidence; updated CI and final integrated quality remain pending.

## Feature-specific CI fixture checkpoint (2026-09-20)

CI run `35507675859` passes workspace tests, the isolated browser-session
lifecycle proof, Cooking applications, and Phoenix/browser checks. Its later
update-admission step exposed a durable outsider-session fixture declared even
when its `test-fixtures` consumers were excluded. The declaration now uses the
same feature condition; no authentication fallback or lint allowance was added.

The exact PostgreSQL/NATS update-admission script passes all branches, including
the no-feature guard, in
`/home/a/heph-update-admission-feature-fix-final-20260920.log`. Scoped app
formatting, Clippy, and docs pass in `/home/a/heph-golden-fixture-*-final-20260920.log`.
CI run `35509033607` passes all jobs at `7017ffb`, including the Rust and
authorization job, Cooking applications, and Phoenix/browser checks. This
confirms the feature-specific fixture fix. Later implementation checkpoints
still require their own CI result and the final integrated quality gate.

## Disable/remove lifecycle checkpoint (2026-09-20)

The release service now disables and terminally removes project, repository,
and organization UI installations. Both operations preserve the current immutable
generation, require current owner management before replay, enforce optional
internal generation CAS, and atomically retain the command outcome plus one owner
event/outbox pair. Source permission revocation does not prevent owner cleanup.
Exact replay adds no event; a fresh disable of an already disabled installation
records a new command/event. Removed identities remain terminal and release their
owner/key for a new installation identity.

The real PostgreSQL lifecycle matrix passes owner scopes, stale CAS, replay and
input conflicts, provenance, source revocation, terminal removal, key reuse, and
per-command event/outbox counts. A two-owner concurrent caller-key race proves one
committed command and one conflict after bounded ledger retry and reauthorization.
The owner loader derives project/repository organization through the project;
it does not decode their intentionally null direct organization column.

Evidence: `/home/a/heph-ui-lifecycle-matrix-exact-final-20260920.log` and
`/home/a/heph-ui-lifecycle-ledger-race-exact-final-20260920.log`. Existing static
project/global/replay/rollback regressions pass in matching
`heph-static-*-regression-final-20260920.log` files. Scoped strict Clippy/docs,
workspace formatting, and architecture pass in the lifecycle final gate logs.
Activation/rollback generation creation, managed/API binding commands, transport,
and serving remain incomplete.

## UI-origin serving integration decision (2026-09-20)

The existing Caddy namespace will route to one bounded loopback Rust UI-origin
handler. That handler will canonicalize the host, resolve the exact immutable
generation, reject unknown/revoked hosts before content, own the reserved
bootstrap exchange route and separate host-only child cookie, authorize each
request, and serve verified artifacts or proxy exact declared gateways. It will
reuse the existing gateway dispatcher rather than introduce another Caddy writer.

Phoenix remains the platform session owner and generates the one-time handoff
secret through an authenticated sensitive RPC. The handoff travels through a
fragment-to-same-origin-POST bootstrap, removed before release navigation. The
Rust UI handler generates the child secret; no raw parent SID reaches that origin
or a guest. Browser credentials are stripped before proxying and guest cookie
writes are rejected. This is the implementation direction, not serving evidence.

The deterministic generation host is `g-<32 lowercase UUID hex digits>` under
the configured UI namespace. Only that exact single label is accepted; DNS case
is normalized, trailing-dot/deep-prefix aliases are rejected, and ports must
match the configured public HTTPS port. No persisted hostname mapping is needed.
The reserved bootstrap endpoint is `/_heph/bootstrap` for GET and POST. Its
trusted page clears a base64url handoff fragment before a same-origin POST and
then replaces the location with the validated host-relative release route.
The distinct child cookie is `__Host-hephaestus_ui`, with Secure, HttpOnly,
Path=/, SameSite=Strict, and the fixed child expiry. Reserved `/_heph/` routes
must not collide with release declarations. These are implementation contracts,
not yet runtime or browser verification claims.

The UI namespace must be a strict subdomain of the configured platform host.
Generation hosts must fit the DNS length limit, and explicit ports use canonical
decimal spelling. SameSite cookies alone do not isolate sibling generation
origins: unsafe content/API requests require the exact generation Origin, and
foreign or null supplied origins are rejected. The platform controls response
CSP for static and managed content, including the exact platform
`frame-ancestors`, same-origin connections, and disabled form submission.

The HTTP handler supplies only a canonical raw path and method to the serving
port. The adapter requires exactly one authorized static, managed, or API
declaration; ambiguous legacy declarations fail closed. The narrow
security-definer resource function performs canonical child verification and
resource projection in the same SQL statement. Query strings remain opaque and never participate
in authority matching. Static reads verify the full artifact before conditional
or range responses and retain the published cache policy. These contracts still
require runtime and browser proof.

The gateway bridge converts managed HTTP paths to the edge's platform-relative
path contract by removing exactly one leading slash from both the safe
authority and request envelope. API paths remain absolute. Queries, including
an empty trailing `?`, remain opaque. The same verifier request ID reaches
gateway invocation accounting. The external wiring draft still needs these
conversions and the request-ID DTO field before integration.

Authenticated document-base requests redirect to the descriptor's canonical
`/{route_base}/{entrypoint}` for both static and managed content. This preserves
relative CSS/JS resolution in the reference releases without rewriting artifact
bytes. The redirect retains the opaque query, including theme metadata, and
uses the serving path's 514-byte bound rather than widening stored handoff
routes. API requests do not receive this redirect. The projection is verified;
the handler integration remains pending.

Bootstrap propagates only the validated initial theme to its POST. The final
document receives `heph_theme` and `heph_theme_origin`, with the latter derived
solely from the exact configured platform origin. Theme messages are
appearance-only and require the expected parent window, exact origin, and a
light/dark value. Content CSP permits published same-origin scripts, styles,
images, and fonts while denying workers, objects, and nested frames. The
shared public UI port setting is `HEPHAESTUS_UI_PORT`.

Before HTTP wiring, consolidate exact platform-origin validation and canonical
serialization across bootstrap and content configuration: lowercase DNS and
omit default `:443` so browser `MessageEvent.origin` comparisons agree. The
platform and UI HTTPS ports are independent; the content draft's asymmetric
port-equality check must be removed. Paths, userinfo, queries, fragments, and
noncanonical explicit ports remain invalid configuration.

Managed/API installation will resolve preexisting, currently active gateway
revisions and pin them in the generation. It will not silently create or retarget
agent instances or gateways. Same-organization cross-project reuse requires
explicit source gateway/project read access plus release/agent use permission;
organization management alone does not confer that access. The resolver must
preserve the declaration validator's supported HTTP contracts and segment-wise
route-prefix coverage, including declared narrower routes. Activation and
rollback create fresh generations; disable/removal require current target
management even when source access is revoked. Internal CAS remains optional;
external lifecycle actions will require the displayed generation expectation.

## Static installation race and rollback checkpoint (2026-09-20)

Two named worker connections now prove concurrent exact replay through the
production owner-row lock. Both return the same result and commit one installation,
generation, command, owner event, and outbox row. A separate real source-project
move blocks the installation at `COMMIT`, then causes deferred tenant validation
to reject it; installation, generation, command, event, and outbox counts are all
zero. No production failpoint or test-only failure trigger is used.

Both proofs pass with observed blocker PIDs and named sessions in
`/home/a/heph-static-install-replay-race-20260920.log` and
`/home/a/heph-static-install-rollback-race-20260920.log`. They ran with migration
89 applied; the locking and rollback behavior is enforced by migration 88 and
the installation adapter. Scoped strict Clippy/docs, formatting, and architecture
pass in the migration-89 final gate logs. These close the static-install replay
and natural post-insert rollback gaps; lifecycle/managed/API work remains open.

## UI handoff and child-session storage checkpoint (2026-09-20)

Migration 89 adds worker-owned handoff and child-session records with digest-only
bearers, canonical parent-session references, exact installation/generation and
organization binding, request provenance, and immutable metadata. Handoffs last
sixty seconds. Child expiry is bounded by twelve hours and the parent expiry.
One child and its one-time handoff consumption must commit together; initially
consumed handoffs, invalid issue/consume intervals, and child-only commits fail.
The application role has no direct access to these tables or digest enumeration.

The real PostgreSQL matrix checks the intended SQLSTATEs, binding/actor/tenant
and timing failures, one-time consumption, rollback row absence, and restricted
role denial. It passes in `/home/a/heph-ui-browser-schema-0089-final-20260920.log`.
The four installation schema tests, project/repository and global installation
matrices, and restricted app-pool bootstrap also pass at migration 89, in the
matching `heph-ui-schema88-on-89-final`, `heph-static-install-*-on-89-final`, and
`heph-ui-app-pool-89-final` logs. Formatting, scoped strict Clippy/docs, and
architecture pass in `/home/a/heph-migration89-*-final-20260920.log`.

This is storage enforcement only. Issuance/exchange adapters, the restricted
application verifier, RPC/Phoenix integration, HTTP serving, and per-request
browser authorization remain incomplete. Records retain canonical provenance;
no transient-record deletion policy is introduced by this migration.

## Global static installation checkpoint (2026-09-20)

The static zero-API installation command now supports organization-owned global
UIs. It locks the organization owner, requires current organization management
and source-release use permissions, enforces same-organization source ownership,
and records the installation/generation/command plus one organization event and
outbox entry atomically. Replay still requires current owner permission.
Project/repository authorization and event behavior remain intact.

The real PostgreSQL global matrix passes owner shape, same-organization source
reuse, exact replay, changed-input conflict, cross-organization rejection despite
dual ownership, the same UI key in two organizations, event/outbox counts, and
revoked-owner replay denial. Existing project/repository installation and all four
schema/barrier tests also pass. Logs: `/home/a/heph-global-static-install-20260920.log`,
`/home/a/heph-static-install-regression-20260920.log`, and
`/home/a/heph-schema88-regression-20260920.log`. Formatting, scoped strict
Clippy/docs, and architecture pass in the `heph-global-install-*` logs.

Concurrent replay and natural post-insert rollback probes remain pending.
Managed/API bindings, generation lifecycle commands, transport, navigation,
serving, and browser acceptance remain incomplete.

## Browser domain and port checkpoint (2026-09-20)

The release domain now contains distinct handoff/child identities, redacted
bearer/digest types, validated relative routes, sixty-second handoff lifetime,
and fixed twelve-hour child expiry capped by the parent. The release application
port binds an explicit actor to the canonical internal parent-session ID and
carries request provenance for issue/exchange. Safe result metadata contains no
bearer or digest. Static, managed, and exact-method API request kinds remain
separate; implementations must validate them against immutable declarations.

The port documents current account/parent, organization, generation, lifecycle,
owner/source/agent authorization, and atomic one-time exchange requirements.
It does not implement persistence, RPC, cookies, routing, or working browser
authorization. Those requirements remain unchecked.

Fresh focused tests pass: 25 release-domain tests, release-service compilation
and doctests, and 68 dev tests with one preexisting ignored test. Scoped strict
Clippy/docs, workspace formatting, and architecture pass. This resumed evidence
was captured in tool output; earlier handoff log paths are historical evidence,
not logs of these current checks.

## Organization-owner schema checkpoint (2026-09-20)

Migration 88 adds organization-owned global installation identities, owner-shape
constraints, scoped uniqueness, and authorized organization reads. It retains
`ui_installations_active_owner_key`. Deferred generation validation locks the
source release and the relevant repositories/projects, rereads parent pointers,
and rejects cross-organization bindings. Parent-move guards apply only to
retained installation/generation history. This does not claim deadlock freedom.

Four real PostgreSQL schema tests pass, including dual-organization membership,
explicit organization filtering, unauthorized reads, tenant rejection, the
source-project move barrier, and a direct source-release lock barrier using a
legal draft mutation. Immutable source/build provenance already prevents a
standalone release-repository reassignment. The existing static-installation
matrix and restricted app-pool bootstrap also pass at migration 88. Evidence:
`/home/a/heph-migration88-validation-retry-20260920.log`.

Scoped Rust tests, strict Clippy/docs, workspace formatting, architecture, and
diff checks pass. Global installation commands remain explicitly unsupported by
the current adapter until the next slice; this checkpoint adds storage and domain
support only. Final workspace quality remains pending.

## Approved organization ownership model (2026-09-20)

The organization is the tenant and permission boundary. A user account may belong
to several organizations, but membership in both does not authorize implicit
resource sharing between them. Project/repository scopes are subordinate to that
boundary. Global UI declarations mean organization-wide installations, managed
through organization permissions and shared through that organization's navigation.
Personal pins, ordering, and hidden entries are presentation preferences within
an organization; there is no separate user-owned resource universe in this work.

UI installation must verify that source release and target owner belong to the
same organization, in addition to the existing target-management and source-use
permissions. Cross-project reuse within that organization remains supported;
cross-organization imports/sharing require a separately designed explicit mechanism.
Serving, handoff, child sessions, cache identity, and guest/API admission must
preserve the same organization boundary. Navigation context alone is not authority.

Migration 87 and the initial domain types cover project/repository owners only.
A subsequent migration will add an explicit organization owner for global entries,
with strict owner-shape constraints, scoped uniqueness, and organization read
policies. Historical migrations will not be rewritten. These are approved
requirements; organization-wide installation and serving remain unimplemented.

## First static installation command checkpoint (2026-09-20)

The release service now installs published static UIs with zero declared APIs
into project or repository owners. It checks current owner permissions and
source-release use, requires source and target organizations to match, and locks
the owner before looking up prior commands. Installation, first generation,
immutable command result, and one existing owner-change event/outbox commit
together. Exact replay preserves IDs and original request provenance; changed
canonical input conflicts. Current owner permission is required even for replay.

The real PostgreSQL matrix explicitly verifies the worker role and passes project
and repository installation, same-organization cross-project reuse, rejection
across organizations despite actor permissions, replay/event counts, input
conflicts, owner revocation, and wrong-scope/managed/API-bearing UI rejection.
Log: `/home/a/heph-ui-installation-matrix-20260920-final.log`. Scoped strict
Clippy, docs, workspace formatting, and architecture pass in the matching
`heph-release-ui-installation-{clippy,fmt,docs,architecture}-20260920*` logs.

The ledger is immutable and read without a row lock; the worker correctly lacks
the update privilege required by its former `FOR SHARE` read. Owner locks remain.
Concurrent replay and post-insert rollback probes are still pending. Global
installation, activation/rollback/disable/removal commands, RPC exposure, and
hosting/navigation remain incomplete. Migration 88 will add the global owner and
database enforcement of the organization invariant, including parent-move guards.

CI run `35504017675` passes workspace tests after the event-watch fixture fix,
but fails the subsequent browser-session lifecycle step. Its cause is under
investigation; this checkpoint does not claim integrated CI success.

## UI browser-session implementation contract (2026-09-20)

One-time handoffs expire after 60 seconds. Child UI sessions have a fixed
12-hour maximum lifetime capped by their parent session's expiry, without sliding
renewal. Parent logout/revocation, current account state, current installation
generation/lifecycle, and current owner/source permissions are checked on every
UI request. A child can never extend its parent authority. These are implementation
decisions; handoff and child-session persistence/serving are not implemented yet.

Handoff and child secrets are distinct types and only their digests are stored.
Records bind the parent internal session ID, installation, immutable generation,
and organization. Validation derives ownership again and requires exact equality;
the selected navigation organization is not an authority input. Consumption changes
an unconsumed handoff exactly once in the same transaction that creates the child.
Multiple organization tabs remain independent; changing navigation does not
invalidate another organization's otherwise authorized session.

## Durable-session event-watch fixture checkpoint (2026-09-20)

CI exposed a standalone Connect event-watch test router that still omitted the
production authentication middleware and sent a SID-less assertion. The fixture
now seeds a durable session through a worker pool, verifies it through a separate
restricted application pool, and uses the production middleware and SID-bearing
JWT. Its existing event pool and resume/duplicate-wake/permission-revocation
assertions remain intact; no authentication fallback was restored.

Disposable PostgreSQL/NATS checks pass all ten event-durability tests and the
watch transport test without skips in `/home/a/heph-event-watch-20260920-v3.log`.
Final workspace formatting, app all-target/all-feature Clippy, architecture, and
diff checks pass in `/home/a/heph-event-watch-checks-20260920-v3.log`; app docs
pass in `/home/a/heph-event-watch-checks-20260920.log`. CI confirmation remains
pending; the earlier failed runs did not reach the browser-session lifecycle step.

## UI installation identity primitives checkpoint (2026-09-20)

The release domain now provides typed installation/generation IDs, structurally
distinct project/repository targets, lifecycle and operation values, and bounded
opaque caller keys. Actor/operation/caller identity determines the command key;
canonical input is hashed separately, including source release/UI and expected
generation where applicable. Initial install input excludes server-generated IDs.
Rollback has a distinct operation domain. These types permit same-state commands
on nonterminal installations; actual generation writes belong to the adapter.

All 21 release-domain tests pass, including stable hash vectors, actor/key/input
separation, project/repository UUID distinction, optional CAS distinction, and
terminal removal. Strict scoped Clippy, docs, workspace formatting, and architecture
pass. Logs: `/home/a/heph-ui-installation-release-test-v2-20260920.log`,
`/home/a/heph-ui-installation-release-clippy-v2-20260920.log`, and the matching
release-doc/workspace-fmt/architecture logs. No installation command is exposed yet.

## UI installation schema checkpoint (2026-09-20)

Migration 87 adds project/repository installation identities, immutable activation
generations, exact managed/API binding records, and immutable command outcomes.
Each committed installation retains a current generation through a deferred
composite foreign key. Owner/key uniqueness includes disabled entries and excludes
removed identities; removal is terminal. Identity/creation metadata cannot change.
Worker grants permit installation updates but only append to evidence tables;
the application role has authorized read access under forced RLS.

The real PostgreSQL matrix passes owner-key duplicates, reuse across repositories
and after removal, immutable metadata/grants, cross-installation pointers,
scope/key and genuine foreign-release binding failures, terminal removal, forced
RLS on all four tables, authorized history reads, and zero outsider visibility.
Log: `/home/a/heph-release-ui-installation-schema-20260920-v2.log`.
Production app-pool bootstrap passes at migration 87, as do scoped release
Clippy/docs, app all-feature compilation, workspace formatting, and architecture.
Related logs use `heph-app-pool-migration87`, `heph-ui-installation-app-check`,
`heph-release-ui-installation-schema-clippy`, and `heph-ui-installation-architecture`
under `/home/a`, dated 20260920.

This is storage enforcement only. Authorized installation commands, generation
lifecycle adapters, transactional owner events, navigation, and serving are not
implemented by this migration. Global ownership was resolved in the later approved
organization model above; its storage extension remains pending.

## Reference UI automatic-theme checkpoint (2026-09-20)

The kit now derives a dark system-preference fallback from the canonical dark
tokens. An explicit root `data-theme="light"` or `data-theme="dark"` takes
precedence. Both reference documents omit their former forced-light attribute.
Canonical generation, manifest hashes, vendored copies, and the static artifact
checker remain synchronized; the CSS digest is
`777b9bbaf8bd1fbf3a121b8f4252a1e3c389dd2d2078d98309bd2a625e36571f`.

Node build/test/check, package dry-run, and both fixture builds/checkers pass in
`/home/a/heph-ui-theme-canonical-20260920.log`. Local Firefox computed-style
evidence verifies automatic light/dark and the opposite explicit override for
both built reference documents, including root tokens and actual body colors:
`/home/a/heph-ui-theme-browser-20260920-script.log`. The reproducible probe is
`/home/a/heph-ui-theme-browser-smoke-20260920.sh`. This verifies system-theme
fallback, not host-selected theme propagation or installed Caddy/browser serving.

## Browser-session RPC and Phoenix checkpoint (2026-09-20)

The integration adds separate bootstrap-authorized session creation and
signed self-revocation RPCs. Ordinary mediator RPCs require the durable session
verifier; request conversion no longer falls back to signature-only identity.
Self-revocation deliberately accepts signed inactive-session claims so logout
can replay safely. Raw SIDs remain sensitive request material and are excluded
from mutation responses and general identity claims.

Generated consistency, 15 descriptor-policy tests, protocol interoperability,
62 app RPC tests, scoped app Clippy/docs, workspace formatting, and architecture
checks pass. Logs use `/home/a/heph-identity-session-rpc-*-20260920.log`.
Existing mediated integration fixtures now use distinct random, persisted SIDs;
their updated call graph passes app all-target/all-feature Clippy and test
compilation. VM-dependent golden acceptance was not rerun for this wiring change.

Phoenix creates a session after verified OIDC identity resolution, validates the
typed response and matching user, and stores its SID and Unix expiry in the
signed cookie. Legacy or expired identities require sign-in. Callback failure
clears a preexisting identity; logout requests self-revocation and clears the
local session even when the RPC returns unavailability. Channel-provider exits
now normalize to that transport error. Failed remote revocation is not evidence
that the durable session was revoked.

The full Phoenix gate passes 264 tests, formatting, and 19 architecture rules in
`/home/a/heph-phoenix-full-20260920-v3.log`. Tests cover typed generated response
projection, callback session writes/clearing, SID/expiry validation, and transport
failure handling. These are Plug/controller tests, not an HTTPS browser proof.

The production HTTP router test passes against disposable PostgreSQL/NATS:
creation, authorized ordinary RPC, self-revocation, denial with the same JWT
still cryptographically valid, exact receipt replay without extra events/outbox
rows, and an unaffected second user. It checks response expiry against storage
and actor/request provenance, and rejects malformed SID/bootstrap identity
mismatch. Final log: `/home/a/heph-browser-session-rpc-real-20260920.log`.
Run the `hephaestus-app` integration target `browser_session_lifecycle` with
all features and `REAL_APP_BROWSER_SESSION_RPC=1`, plus disposable
`HEPHAESTUS_POSTGRES_TEST_URL` and `HEPHAESTUS_NATS_TEST_URL`.
The generated client lives in the declared test composition boundary; no
architecture rule was weakened. Final architecture, strict Clippy, workspace
formatting, and app docs pass in the matching `architecture-v4`, `clippy-v5`,
`workspace-fmt-v2`, and `doc` logs. UI handoff/child sessions, browser isolation,
and final repository-wide quality remain outstanding.

CI now explicitly runs the opt-in browser-session lifecycle target after the
serial workspace tests, using that job's existing disposable PostgreSQL/NATS.
The ordering keeps its named-stream cleanup outside other workspace tests.
Workflow YAML parsing and diff checks pass; CI execution of this wiring remains
pending. Local `cargo dev quality` must receive the same service URLs and
`REAL_APP_BROWSER_SESSION_RPC=1` to include this proof in its workspace-test phase.

CI for the preceding managed reference fixture commit `9d506bc` passed in run
`35499684144`; that run predates these session integration changes.

## Managed reference fixture checkpoint (2026-09-20)

`examples/cooking/cooking-reference-service-ui` declares an authenticated
`http.service.v1` managed UI at `/reference` with `index.html` as its entrypoint.
The self-contained build materializes the Python executable, HTML, and canonical
kit CSS together. The service uses fixed local files, a single request worker,
bounded header checks/timeouts, health/readiness endpoints, and a stable startup
identity probe. Build/guest network and workspace/state mounts are disabled.
Kit drift checks now cover both reference fixtures.

The actual agent/gateway/UI parser test passes, as do scoped Clippy/docs,
workspace formatting, and Node kit checks. Local build/tampered-CSS rejection and
HTTP checks pass for the exact HTML/CSS entrypoint, readiness/health, stable
identity, missing-path rejection, and header non-reflection. The declared
python-ubuntu image was inspected to verify the executable's Python launcher.
Logs: `/home/a/heph-cooking-managed-ui-agent-config-test-20260920-v4.log`,
`/home/a/heph-cooking-managed-ui-build-http-20260920-v4.log`, and
`/home/a/heph-cooking-managed-ui-python-image-20260920.log`.
No VM publication, installed Caddy serving, browser authorization, automatic
theme behavior, or restart acceptance is claimed by this local fixture checkpoint.

## Browser-session self-revocation adapter checkpoint (2026-09-20)

The PostgreSQL adapter now implements the complete browser-session port.
Self-revocation locks the signed user, compares any immutable command replay,
then locks only that user's exact SID. A fresh database timestamp after locking
determines whether the session is active. The final ledger outcome, optional
logout revocation, and safe identity event commit together. Fresh commands for
absent, expired, future-issued, or already-revoked sessions are durable no-ops;
exact retries preserve their original result and request provenance. Existing
inactive users may revoke; missing users and changed-SID key reuse fail explicitly.

The real PostgreSQL matrix passes actual revoked timestamp/reason checks,
successful authentication before revocation and denial afterward, typed receipt
loading for active/inactive/no-op outcomes, original request preservation,
cross-user protection, and concurrent exact replay with one ledger/event/outbox.
Creation/replay, authentication, and migration-86 constraints also pass.
Workspace formatting, scoped Clippy/docs, and architecture pass. Logs:
`/home/a/heph-browser-session-revoke-20260920.log` and
`/home/a/heph-browser-session-revoke-{clippy,doc}-final-20260920.log`.
This is persistence behavior; RPC logout and Phoenix clearing are not wired yet.

## Production browser-cookie checkpoint (2026-09-20)

The endpoint now selects shared HTTP/LiveView session options at compile time.
Production uses `__Host-hephaestus_web_key`, `Secure`, `HttpOnly`, `Path=/`,
`SameSite=Lax`, and no Domain attribute. Development/test retain an HTTP-compatible
cookie. The session remains signed, not encrypted; no dual-read compatibility
path accepts the old production cookie name.

Six focused endpoint/controller tests pass, including real Set-Cookie headers,
endpoint option wiring, and legacy-cookie rejection with a positive control
that reads that same valid cookie through its old profile. Owned formatting and
a production compile pass; a production-mode assertion verifies the compiled
endpoint options. Logs: `/home/a/heph-browser-cookie-tests-20260920.log` and
`/home/a/heph-browser-cookie-prod-options-20260920.log`.
SID validation and login/logout wiring are separate pending work. These Plug
checks do not replace the required HTTPS browser sibling-origin isolation proof.

## Revocation command ledger checkpoint (2026-09-20)

Migration 86 adds immutable worker-owned revocation command records. Each
actor-bound command key binds one user and SID digest, including an absent-session
no-op. New keys may target the same SID; a composite foreign key binds any matched
session to that exact user and digest. Changed outcomes require a matched session.
Forced RLS and explicit grants deny application access and worker update/delete.
Restrictive foreign keys deliberately retain referenced audit parents rather than
cascading away command evidence.

Four real PostgreSQL matrices pass under migration 86: session schema, creation,
authentication, and the new ledger. The focused ledger test also passes separately,
including new-key SID reuse, absent-session rows, wrong owner/digest rejection,
and immutable access. Production application-role bootstrap passes at version 86.
Scoped Clippy/docs, app compilation, workspace formatting, and architecture pass.
Evidence: `/home/a/heph-human-session-real-schema86-v2-20260920.log`,
`/home/a/heph-human-session-revocation-schema-real-v2-20260920.log`, and
`/home/a/heph-human-session-app-pool86-20260920.log`.
The initial stale test binary lacked embedded migration 86; rebuilding resolved
that fixture failure. The revocation adapter and user-facing logout remain pending.

## Application-role session verification checkpoint (2026-09-20)

The PostgreSQL session adapter now has separate worker and application pools.
Active verification hashes the supplied SID and calls only the narrow database
verifier through the application role. Missing or inactive sessions produce an
authentication failure; database/invariant failures produce opaque unavailability.
There is no cache or worker-role fallback.

A real non-superuser, non-RLS-bypass application-role test passes valid creation
and verification, direct table denial, wrong SID/user rejection, suspension after
successful authentication, and a closed verifier pool. The creation/replay matrix
also passes. Scoped Clippy/docs, workspace formatting, and all 61 enabled
architecture checks pass. Evidence: `/home/a/heph-browser-session-auth-20260920.log`
and `/home/a/heph-browser-session-auth-checks-20260920.log`.
Mediator enforcement, revocation, RPC/Phoenix wiring, and UI authority are still
pending; this adapter checkpoint alone does not change login behavior.

## Static reference fixture checkpoint (2026-09-20)

`examples/cooking/cooking-reference-ui` declares a project-scoped static page
using the versioned CSS kit. Its build checks the vendored manifest and hashes;
the executable artifact checks the materialized HTML/CSS layout. Build and guest
networking are disabled. The kit checks compare both vendored generated files
with canonical output, so drift fails validation.

The Rust semantic fixture test derives artifact candidates from the actual agent
declaration and checked-in bytes, then resolves the UI routes. Its focused test,
Clippy, formatting, and documentation checks passed, as did kit npm tests and
drift checks. A local standalone build and tampered-CSS rejection were exercised.
Local Chromium light/dark screenshots were reviewed; this is local rendering
evidence, not an installed release or browser-authorization proof.

Cooking pipeline publication, managed-service reference content, installation,
and Caddy-hosted browser integration remain pending; section 5 stays incomplete.
Focused Rust logs use `/home/a/heph-agent-config-reference-fixture-*-20260920.log`;
local screenshots are `/home/a/heph-reference-ui-smoke-{light,dark}-20260920.png`.

## Browser and routing prerequisite decisions (2026-09-20)

UI origins require an opt-in static namespace guard in the existing Caddy
configuration owner, before generic platform/gateway routes. The guard forwards
all namespace descendants (including malformed prefixes) to a private loopback
UI upstream; that upstream must canonicalize Host and deny unknown generations.
No per-install Caddy catalog or second configuration writer is needed. Explicit
DNS/TLS provisioning and browser-authority proofs remain separate requirements.
The routing foundation is implemented and validated as described below.

Browser authority needs a durable human session bound to the verified OIDC
identity, with current active-user, expiry, and revocation checks at the mediator
boundary. Session creation will be a separate bootstrap-authorized operation;
legacy cookies without a session ID must require login. Storage/domain support,
creation/verification/revocation adapters, and production cookie isolation are
implemented and validated below. RPC/middleware/Phoenix integration is in progress;
UI handoff remains unimplemented.

Project tabs currently repeat across six page components; repository tabs use
`RepositoryRouteModel` and `RepositoryShell`. Installed entries will be projected
under current owner authorization and appended through shared tab construction,
with namespaced opaque identity so declarations cannot replace core routes.
Installation generations must change on fresh activation, including reactivation
of the same release; command replay alone reuses the prior result. Global entries
belong to an explicit organization under the approved model above.

Project/repository installation decisions for the next implementation slice:

- Stable installation identity records the navigation owner, UI key, and enabled,
  disabled, or removed state. Project-scoped entries have no target repository;
  repository-scoped entries name an exact repository in that project. The source
  repository belongs to the immutable release generation, not the project tab's
  navigation owner. Source-release use authority is separate from target ownership;
  permit cross-project/repository reuse only within the same organization and
  after the required source-use checks. Do not silently narrow a distribution
  surface to its source repository or implicitly share across organizations.
- Require current project management and release use authority, plus use authority
  for every bound release agent. Each fresh activation, reactivation, or rollback
  creates a new generation even for the same release; only exact command replay
  reuses a generation. Removed installations are terminal, but a new identity may
  reuse their UI key. Uniqueness excludes removed entries and follows the actual
  project/repository navigation owner.
- Each generation binds the exact published release/UI and every managed/API
  gateway revision, release agent, method/route, and authenticated exposure. Binding
  identity includes its kind so managed and API keys cannot collide. It carries
  no VM or service-instance identity. Gateway cutover makes a stale generation
  unavailable; it never silently retargets the UI or changes its desired state.
- The mutation, command replay record, generation, and existing owner-invalidation
  event must commit together. Use the owner's actual lifecycle state and correct
  related IDs in project/repository events; UI enabled/disabled state is not owner
  lifecycle state. Navigation consumers must refresh installed-entry projections.
- Shell routes use opaque platform-owned identities. Declarations cannot replace
  core routes. Global installations require an explicit organization owner;
  personal navigation preferences do not alter installation authority.

These are reviewed design directions, not implemented installation behavior.
Source-to-target adapter review: `ReleaseService::import_agent` permits a published
release agent in another consuming project after target management and source-agent
use checks; repository attachments follow that consuming project. In contrast,
`GatewayInstallApplication` and `require_repository_boundary`, together with
migration 35's gateway revision constraint, bind a gateway to the release's source
repository/project. Static UI installation can use the cross-owner model within
one organization under explicit release-use authorization. Managed/API bindings must respect the existing
gateway boundary; supporting a new consuming gateway in another owner requires
an explicit gateway import/reference capability, not silent retargeting. Requiring
release-level CanUse for UI installation is an explicit policy (needed for static
UIs), not a claim that existing agent imports already enforce that permission.
Removed installation identities remain terminal; a new installation may reuse the
key, while enabled/disabled installations may receive fresh activation generations.

The reviewed project/repository storage direction uses four tables: stable
installations, immutable generations, exact managed/API bindings, and immutable
command outcomes. Composite foreign keys bind generations to their installation
key/scope, bindings to the generation's release/UI, and gateway revisions to the
same gateway/release/agent/exposure. A deferred current-generation foreign key
permits allocating both IDs before insertion while retaining a non-null pointer
in every committed lifecycle state. Existing migrations remain unchanged.

Command identity derives from actor, operation, and the caller's bounded
idempotency key. Canonical input, including the expected-generation CAS, is
compared separately: changed input under one key conflicts, while a fresh key
can activate the same release again. The adapter will append one existing
project/repository owner event through the committed-outbox function, rather
than adding a second event trigger. These remain implementation directions;
no installation schema or adapter has been applied at this checkpoint.

Browser handoff implementation direction (still unimplemented):

- Give each immutable generation a distinct platform-owned hostname. A revoked
  or replaced generation's origin is never reassigned to a new generation.
- The trusted web mediator generates a random handoff value and submits it only
  as sensitive request material. Persist its digest bound to the authenticated
  parent session, exact generation, intended route, short expiry, and single use.
  Do not add a raw credential response merely to move a server-generated value
  back through RPC.
- A platform-owned bootstrap page on the UI origin consumes a fragment-carried
  handoff through a same-origin POST, removes the fragment, and then navigates to
  the declared entrypoint. No release script or external asset runs before that
  exchange. Reserve the bootstrap path namespace so release routes cannot shadow
  it. Unknown generation hosts fail closed before rendering the bootstrap page.
- Exchange the handoff for a separate host-only secure HttpOnly UI cookie. Store
  only its digest, with a parent-session reference and exact generation binding.
  The platform SID never enters release content or a guest request. Recheck parent
  session, current user/owner/release authority, generation state, and exact gateway
  revisions on subsequent UI/API access; cookie possession alone is insufficient.
- Strip browser credentials before guest forwarding and reject guest cookie
  writes. UI API calls require exact origin/method/route checks; SameSite alone
  is not a CSRF boundary between sibling origins. Browser TLS and adversarial
  replay/cookie/origin tests must prove this flow before the task is complete.

The initial fixtures forced light mode, and their manually selected dark checks
demonstrated kit styling only. The later automatic-theme checkpoint verifies
system-preference fallback. Host-selected theme propagation remains part of
browser integration acceptance.

The retained-cleanup fixture isolation follow-up is implemented below. Production
lease semantics and cleanup behavior remain unchanged.

## Durable human-session foundation checkpoint (2026-09-20)

Migration 85 adds durable session rows with separate internal identity and opaque
browser SID, storing only a domain-separated SID digest. Immutable creation
idempotency/request IDs and a length-delimited issuer/subject binding support
exact replay without storing the raw SID. Expiry is immutable (12-hour default,
24-hour maximum); revocation is one-way. Worker mutation grants are explicit,
application table access is denied, and forced RLS remains enabled.

The narrow application-role verifier requires an exact user/digest match, active
user, issued time reached, unexpired session, and no revocation. Qualified table
references and a hardened search path prevent temporary-table substitution.
Domain types redact SID and digest formatting and expose only explicit protocol
serialization. App schema expectation and the bootstrap gate advance to 85.

The real restricted-role schema matrix passes 1/1 with its execution marker in
`/home/a/heph-browser-session-schema-20260920-v5.log`. It covers role grants,
expiry/revocation/inactive users, wrong identities, immutable fields, duplicate
SID/idempotency, bounds, and a fake temporary row on the pinned verifier
connection. Domain tests pass 5/5 in
`/home/a/heph-human-session-final-tests-20260920.log`; the production application
pool bootstrap passes 1/1 at migration 85 in
`/home/a/heph-human-session-app-pool-20260920.log`. Scoped Clippy/docs, downstream
app all-feature compilation, formatting, and architecture pass. The final domain
review removed an unnecessary lint allowance by exporting both TTL constants.

This checkpoint does not create sessions during login or enforce them on RPCs.
Creation/replay, verification/revocation adapters, exact bootstrap/self-logout
RPC boundaries, Phoenix cookies/logout, and browser UI handoffs remain pending.
Creation will use a Phoenix-generated SID in a sensitive request, verified OIDC
issuer/subject (no actor selector), and existing actor-bound idempotency. Expired
or revoked creation replays must fail; logout must clear the cookie and revoke
only the signed mediator user's own SID, including an inactive-session no-op.

## Browser-session creation checkpoint (2026-09-20)

The identity application now defines narrow session commands/results and an
opaque-error persistence port. `PostgresBrowserSessionStore` implements creation
as an inherent method; the full port is not implemented until authentication and
self-revocation are added. Creation locks the exact verified OIDC mapping and
user, requires active status, derives the existing actor-bound idempotency ID,
and stores only SID/identity digests with a server-selected 12-hour lifetime.
The session and safe identity-profile invalidation commit in one transaction.

An exact active retry returns the original metadata and preserves the creation
request ID even if the transport request ID changes. Changed SID or identity,
SID reuse under another key, and expired/revoked replay are rejected. Conflict
lookup locks only the creation key; it does not inspect another user's session.
Events carry the current user state and safe IDs, without OIDC or SID material.

The real worker-role matrix passes 1/1 in
`/home/a/heph-browser-session-create-real-v4-20260920.log`, covering successful
creation, exact replay, conflicts, inactive/unmapped users, expiry/revocation,
concurrent identical calls, one committed outbox event, lifetime, and event
redaction. Scoped application/adapter Clippy and rustdoc, workspace formatting,
and architecture pass; final Clippy is
`/home/a/heph-browser-session-create-clippy-v4-20260920.log` and workspace format
is `/home/a/heph-browser-session-create-workspace-fmt-final-20260920.log`.
Authentication/revocation persistence and RPC/Phoenix integration remain open.

## Recovery fixture isolation checkpoint (2026-09-20)

The old expiry fixture held A's instance row until its lease expired. Production
renewal locks the gateway before the instance, so that barrier could also block
B's renewal and violate the fixture's healthy-B premise. This is a source-based
explanation; the original failing CI lock sequence was not captured or replayed.
The replacement pauses only A's renewal through the existing ownership observer,
before any database lock, and observes natural PostgreSQL lease expiry. It asserts
B remains ready, active, and unexpired. Exact expired-claim recovery retains the
real PostgreSQL adapter even when ordinary ownership is observed; the initial
observer experiment accidentally disabled that adapter and is superseded.

The test proves A's exact VM retries with fencing incremented once, C has no
instance before A is durably cleaned, C then becomes active/ready, and B reaches
physical and durable cleanup. No production lifecycle code changed. Five isolated
expiry runs pass in
`/home/a/heph-gateway-recovery-diagnostic-focused-v10-20260920.log`; all 17 recovery
tests pass against real PostgreSQL in
`/home/a/heph-gateway-recovery-diagnostic-group-v11-20260920.log`. After adding the
final B-cleanup assertion, both affected variants pass in the retained-v12 and
expired-v13 logs with the same prefix/date. Scoped app Clippy, formatting,
documentation, and architecture pass; post-assertion Clippy/format also pass in
`/home/a/heph-gateway-recovery-postassert-checks-20260920.log`.

## Caddy UI namespace foundation checkpoint (2026-09-20)

`LocalCaddyConfigurationTemplate::with_ui_namespace` validates the configured DNS
namespace and loopback upstream, requires a unique first `hephaestus.ui` slot,
and renders a terminal namespace proxy through the existing configuration owner.
Legacy templates remain supported. This is opt-in library support; application
configuration, the authoritative UI listener, and session/installation wiring are
still pending.

The real pinned Caddy smoke uses the Rust-rendered full configuration and `/load`.
An outside-host platform request succeeds; the same platform path on an unknown
UI host returns the UI upstream's distinct 404. Known hosts and malformed deep
namespace hosts remain on the UI path across gateway reconciliation, while the
updated public gateway works. Both real ingress tests pass in
`/home/a/heph-gateway-ui-caddy-smoke-final-20260920.log`; 182 edge unit tests pass
in `/home/a/heph-gateway-ui-edge-unit-final-20260920.log`. Strict scoped Clippy,
rustdoc with warnings denied, formatting, and all 61 enabled architecture rules
pass. The disposable Caddy container was removed. This is HTTP routing evidence,
not TLS, browser authentication, or authoritative generation resolution evidence.

## Public gateway exposure checkpoint (2026-09-20)

Reserved `heph_authenticated` revisions are excluded from public route lookup
and Caddy projection. The dispatcher rejects them before handler invocation,
and admission rechecks the immutable revision exposure under the authoritative
gateway lock, including when a caller supplies a forged public binding. Service
lifecycle behavior is unchanged; authenticated browser admission is still pending.

The strengthened real PostgreSQL test first admits a public control through a
persisted-session issuer, then proves the authenticated revision creates neither
an issuer call nor an invocation. The full service-acceptance target passes 9/9
without database skips in
`/home/a/heph-gateway-service-acceptance-full-realpg-v5-20260920.log`.
Edge unit tests (180), gateway adapter library tests (9), scoped Clippy,
documentation, formatting, and architecture checks pass. The opt-in Caddy test
was skipped because its admin endpoint was not configured; these are projection
and authority proofs, not a new real-Caddy proof. The pre-fix exposure gap was
identified in source, not reproduced in a pre-fix runtime test.

## Build-completion publication checkpoint (2026-09-20)

`CompleteBuild` loads the immutable capture linked to its locked build request,
verifies canonical UI/gateway hashes and the derived build identity, and resolves
references to the supplied exact artifact and exported-agent IDs. Resolution
precedes release writes. UI bindings are inserted after artifact/agent rows and
before build success and command-inbox completion, in the same transaction.
Legacy builds with no UI link retain their behavior, including stored agent
configurations that omit a build declaration. No Git checkout is needed.

The real worker-role integration case verifies the exact static artifact and
source-capture IDs and successful legacy no-UI completion. The final complete
release package run passed 9 library tests and 5 integration tests with real
PostgreSQL/NATS in `/home/a/heph-release-complete-build-ui-20260920.log`.
Earlier attempts in that log include a database-readiness failure and a unit
fixture that incorrectly depended on UI ordering; both are corrected in the
final passing run. Strict scoped Clippy, documentation, and architecture passed
in `/home/a/heph-release-complete-build-clippy-20260920-v3.log`,
`/home/a/heph-release-complete-build-doc-20260920-v2.log`, and
`/home/a/heph-release-complete-build-architecture-20260920.log`.
Managed/API persistence, transactional failure/replay cases, and authorized
inspection are the next bounded slices; this is not hosting or browser evidence.

## Managed/API publication and rollback checkpoint (2026-09-20)

The focused real PostgreSQL matrix now exercises managed-service and API exact
release-agent IDs, gateway routes/methods, and successful command replay without
duplicate publication rows. It rejects missing artifacts, wrong MIME, oversized
referenced files, unexported agents, mismatched UI/gateway/build hashes, and a
linked capture without a build declaration. Rejections leave no candidate
release/artifact/agent/UI/inbox rows and preserve the `importing` build state.

A valid candidate then deliberately collides with an existing artifact primary
key. The resulting `23505` occurs after the candidate release insert; the test
verifies rollback of the release/family and all candidate child/inbox rows while
preserving the original artifact and build state. No test-only production fault
seam was added. The role is explicitly `hephaestus_worker`, not superuser; its
existing trusted-worker `BYPASSRLS` attribute from migration 0004 is intentional.

Final focused run: 1 matrix test passed, 5 other integration tests filtered, in
`/home/a/heph-release-managed-matrix-collision-ready-20260920.log`.
The disposable database passed readiness and a query before test launch and was
cleaned up. The earlier connection-reset attempt failed before setup and is not
counted as acceptance evidence. Scoped Clippy, format, and docs passed in
`/home/a/heph-release-managed-matrix-collision-clippy-20260920.log`,
`/home/a/heph-release-managed-matrix-collision-fmt-20260920.log`, and
`/home/a/heph-release-managed-matrix-doc-final-20260920.log`.
This is publication evidence only; inspection and hosting remain separate work.

## Authorized inspection adapter checkpoint (2026-09-20)

`ReleaseApplication::get_release` now includes typed immutable UI descriptors,
static file/artifact IDs, managed-service/agent IDs, and declared API metadata.
Actor transactions retain explicit release read checks and forced table RLS.
Four bounded queries reject excessive counts instead of truncating results.
Assembly rejects missing or cross-content child bindings, invalid metadata,
non-HTML static entrypoints, and per-UI file/API overflow. The projection does
not expose storage locators, source configuration, credentials, or browser URLs.
Legacy releases return an empty descriptor list.

The focused PostgreSQL test uses a separate `hephaestus_app` pool and asserts
that it is neither superuser nor `BYPASSRLS`. It covers authorized static and
managed/API identities, outsider `NotFound` and direct RLS visibility, legacy
empty UI, malformed bindings, and 17-descriptor/257-file/17-API overflow cases.
It passed 1/1 in `/home/a/heph-release-ui-authorization-20260920-v3.log`.
The existing artifact authorization regression passed 1/1 separately in
`/home/a/heph-release-ui-artifact-authorization-20260920.log`.
Scoped all-target/all-feature Clippy, documentation, and formatting passed in
`/home/a/heph-control-plane-release-ui-clippy-20260920-v3.log`,
`/home/a/heph-control-plane-release-ui-doc-20260920.log`, and
`/home/a/heph-control-plane-release-ui-fmt-20260920-v4.log`.
The disposable database passed readiness before testing and was removed.
This checkpoint is the database adapter only: protobuf generation, RPC mapping,
and browser-facing presentation remain subsequent slices.

## Authorized inspection RPC checkpoint (2026-09-20)

The existing authenticated `GetRelease` response now includes additive field 22,
`ui_descriptors`, with typed scope/icon/presentation/cache enums, static versus
managed content, exact artifact/agent IDs, and explicit API metadata. Existing
fields and authorization remain unchanged. Gateway route fields are declaration
metadata, not authorized browser URLs. Rust and Elixir bindings are regenerated;
the transport converts the validated application DTO at the RPC boundary.

Generation, generated consistency, and Buf breaking checks passed. Descriptor
policy passed 15 tests, protocol interoperability passed 1, and focused mapper
coverage passed 2 tests for static/API and managed identities (including empty
UI-list mapping). Scoped app Clippy, formatting, rustdoc, and all 61 enabled
architecture rules passed. Evidence:
`/home/a/heph-release-ui-rpc-generation-20260920.log`,
`/home/a/heph-release-ui-generated-check-20260920.log`,
`/home/a/heph-release-ui-protobuf-breaking-20260920.log`,
`/home/a/heph-release-ui-rpc-descriptor-20260920.log`,
`/home/a/heph-release-ui-rpc-interop-20260920.log`,
`/home/a/heph-release-ui-rpc-mapper-test-v3-20260920.log`,
`/home/a/heph-release-ui-rpc-app-clippy-v3-20260920.log`,
`/home/a/heph-release-ui-rpc-app-doc-20260920.log`, and
`/home/a/heph-release-ui-rpc-architecture-20260920.log`.
This completes declaration/publication/authorized-inspection scope. Release-page
metadata display, UI installation, hosting, and browser authority are still
separate incomplete work.

## Verified static-byte storage prerequisite (2026-09-20)

`LocalArtifactStore::read_verified` accepts an opaque storage key, expected hash
and length, and a caller-supplied hard limit. It opens the object once with
portable `libc::O_NOFOLLOW | libc::O_NONBLOCK`, rejects nonregular/hard-linked
objects, bounds allocation and reading, and checks exact length and SHA-256
before returning owned bytes. Returned content cannot change if the stored file
is modified later. Fallible allocation returns a bounded error. Existing generic
download behavior is unchanged; this helper is not an HTTP serving endpoint.
The only dependency change is the existing workspace libc package edge.

All 9 artifact-store tests pass, including hash/length/limit mismatches,
symlink/hard-link rejection, a FIFO without a writer, and owned-buffer behavior:
`/home/a/heph-verified-artifact-read-tests-v3-20260920.log`.
Scoped Clippy, format, docs, and downstream control-plane compilation pass in
`/home/a/heph-verified-artifact-read-clippy-v3-20260920.log`,
`/home/a/heph-verified-artifact-read-fmt-v3-20260920.log`,
`/home/a/heph-verified-artifact-read-doc-v3-20260920.log`, and
`/home/a/heph-verified-artifact-read-control-plane-check-v3-20260920.log`.
The future asynchronous serving adapter must run this synchronous operation off
the executor, apply the UI limits, and enforce installation/browser authority.
No serving or browser acceptance is claimed by this storage checkpoint.

## Release-page metadata checkpoint (2026-09-20)

The release page now lists declared interfaces with label/key, friendly scope
and presentation labels, static versus managed content, and declared API count.
The state reducer preserves descriptors across loads and mutation refreshes;
legacy absent metadata becomes an empty list. The RPC projection explicitly
normalizes the new enum prefixes and expands the generated content oneof.
Rendering uses existing design-system components and adds no launch link,
authorized browser URL, or installation action.

The final focused Phoenix state/page/projection suite passed 19 tests in
`/home/a/heph-release-ui-page-tests-20260920-v2.log`; formatting passed in
`/home/a/heph-release-ui-page-format-20260920-v2.log`. UI architecture passed
all 14 enabled rules and its focused suite passed 100 tests in
`/home/a/heph-release-ui-architecture-20260920.log`.
This is release inspection only. Project/repository/global navigation entries,
embedding, loading/revocation states, and actual browser hosting remain open.

## Current CI context (2026-09-20)

CI also passed public-admission checkpoint `46f54e0` (`35493223081`) and Caddy
namespace checkpoint `937f9af` (`35494134171`).

CI passed the publication matrix `8ab3dca` (`35490762779`), inspection adapter
`83659e1` (`35490987211`), verified reads `94cdf99` (`35491439692`), and release
page `60992c5` (`35491550325`). These later passes did not change lifecycle code
and do not resolve the intermittent failure below. RPC checkpoint `542c2b5` (`35491311596`) passed browser
and Cooking but failed one service recovery test in the Rust job: 95 app tests
passed, and `daemon_loop_recovers_expired_owned_cleanup_before_admitting_next_revision`
observed healthy B destruction while A cleanup was held (`[A, B, A]`). The exact
cause is unproven. B was awaited active/ready before C became desired, and the
ordinary current-revision reconciliation branch already preserves B, so a
capacity-based retirement change is not justified without further evidence.
The later fixture-isolation checkpoint removes the identified lock coupling
and preserves exact expired-claim recovery, with focused/full recovery evidence.
Final CI and integrated quality are still required. Historical failure evidence:
`/home/a/heph-release-ui-rpc-ci-failure-20260920.log`.


CI passed at `f2dbfbf` and `b6b86e2` (runs `35488587270` and
`35488863317`). Schema checkpoint `02080ac` failed only workspace Clippy
on two documentation-markdown and three long-test-helper diagnostics in
`ui_schema_tests.rs`; tests/docs were skipped, while browser and Cooking passed.
The publication slice corrects these with documentation backticks and narrow,
explained fixture allowances. Failure evidence:
`/home/a/heph-schema84-ci-failure-20260920.log`.

Earlier CI passed through checkpoint `17d6dbc` in run `35487956392`:
Rust/authorization, browser golden path, and Cooking applications all succeeded.
Receive fix `52bbbf7`, historical/kit gate `d51449d`, size limits `ed96408`, and
manual-build integration `1f3f76a` also have passing CI runs.
The preceding kit checkpoint `6c6a9c6` failed one Rust recovery-test teardown
while browser and Cooking passed. An idle `gateway-recovery-control` PostgreSQL
backend remained beyond the unchanged ten-second deadline. A controlled
normal-versus-cancelled SQLx read differential subsequently reproduced a leaked
control backend only after cancellation. The active-route adapter now uses a
cancellation-only connection close guard. Its actual adapter regression, all
76 gateway package tests, and all 17 application recovery tests pass. The
historical CI PID's owner was not directly observed; no deadline or assertion
was weakened. Follow-up evidence is recorded in the completed service task.
Initial bounded evidence is in
`/home/a/heph-kit-ci-teardown-failure-20260920.log` and
`/home/a/heph-kit-ci-teardown-sqlx-analysis-20260920.log`.

CI at `e052a2c` (`35482989628`) exposed an application bootstrap version
mismatch: the database reached migration 82 while `EXPECTED_DATABASE_MIGRATION`
still required 81. Both Rust golden bootstrap and the browser golden path
failed on this mismatch; Cooking applications passed. The application gate
advanced to 82 at that checkpoint (manual integration below advances it to 83),
and retention's minimum schema requirement remains 81.
The targeted production bearer-push bootstrap and real application retention
tests each passed against disposable PostgreSQL/NATS in
`/home/a/heph-schema82-bootstrap-retention-20260920.log`, including
`REAL_APP_SERVICE_LOG_MAINTENANCE=1 max_migration=82`. Scoped app formatting,
strict Clippy, and rustdoc passed in
`/home/a/heph-schema82-app-fmt-20260920-v2.log`,
`/home/a/heph-schema82-app-clippy-20260920-v3.log`, and
`/home/a/heph-schema82-app-doc-20260920.log`. This corrects the startup schema
gate; it is not a new complete UI or full-quality result.

Subsequent CI passed at `d0def95`, `7ccb01d`, and `44fe0c3`; the successful
run at that historical checkpoint was `35481880564`. This did not establish the cause of the
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

At the initial deployment review, local Phoenix used loopback port 4000 and
production took `PHX_HOST`; its session cookie had no Domain attribute. Caddy
reconciliation replaced one `hephaestus.gateway` subroute and forwarded to the
trusted dispatcher, without a UI namespace. The later routing checkpoint adds
an opt-in namespace guard, and the cookie checkpoint adds production `__Host-`
isolation. Production wildcard DNS/certificate provisioning and the actual UI
upstream remain outstanding.

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

### Installation and static-serving prerequisites (2026-09-20)

Existing project installations already have stable agent-instance and gateway
identities, immutable release-agent/revision links, and lifecycle/revocation
checks. Repository attachment remains explicit. UI installation should reuse
these boundaries rather than introduce a parallel runtime installation model.
A durable UI binding still needs an exact immutable release/UI key and target
scope. Global declarations will use the user-approved organization owner. Do not
represent global authority merely by null project/repository IDs.

`LocalArtifactStore` already handles bounded imports, content hashing, and safe
opaque-key lookup. `ArtifactApplication` supplies actor-authorized artifact-ID
lookup and bounded preview/stream paths. Existing read validation checks regular
file type and stored length. The later verified-byte checkpoint adds a separate
bounded read that recomputes the content hash before returning bytes.
The UI serving path must verify content integrity before emitting bytes and
reject files beyond the 16 MiB UI bound; its 64 MiB aggregate counts unique
referenced artifact IDs. Existing generic streaming can truncate at a caller
limit, so it is not sufficient unchanged. Reuse storage and authorization
boundaries; these findings do not constitute implemented UI serving.

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
The receive writer's application-role reusable-build path and manual/receive
ordering remain separate verification work.

The expanded manual matrix now also passes historical compatibility: a build
created without capture retains its ID, legacy hash, and lack of UI link after
capture is added; the next request creates a separate derived build, and retries
reuse only that new identity. Each build has one outbox event. The matrix checks
the derived event hash and that wrong-hash rejection does not add events. The
real PostgreSQL matrix passed (1/1) in
`/home/a/heph-manual-ui-build-realpg-20260920.log`; scoped Clippy passed in
`/home/a/heph-manual-ui-build-control-clippy-v4-20260920.log`.

### Application-role receive checkpoint (2026-09-20)

Reusable UI builds now pass through `accept_receive_as` under `hephaestus_app`
with a real authorized identity. The regression exposed catalog `FOR SHARE`
requiring unavailable UPDATE privileges, then a second receive exposed the
configuration-revision upsert's UPDATE RLS requirement. Catalog reads now use
the same plain SELECT contract as manual builds; repository-image locking is
unchanged. Existing immutable revisions are selected under the repository lock
and newly absent rows are inserted without an UPDATE clause. Build insertion
uses the same preselect/plain-insert/read sequence as the manual path. No grants
or RLS policies were broadened in this checkpoint.

Tests verify the exact derived hash and capture link, one build event, reuse
across distinct receive IDs, and replay of the original receive ID. The full
forge package passed with disposable PostgreSQL and NATS: 12 unit, nine receive,
two Smart HTTP, and one schema test, with no skip markers. Evidence is in
`/home/a/heph-forge-postgres-package-full-20260920.log`; scoped Clippy and
rustdoc passed in `/home/a/heph-forge-app-receive-{clippy-v2,doc}-20260920.log`.
Formatting and architecture passed. Failure evidence is retained in
`/home/a/heph-forge-ui-lock-20260920.log` and the earlier sections of
`/home/a/heph-forge-app-receive-fixed-20260920.log` (which ends with passing runs).
The receive baseline did not separately reproduce the build-insert RLS failure;
that insertion pattern was diagnosed through the manual application-role path.

### Receive/manual ordering checkpoint (2026-09-20)

A real composition test now uses separate named application-role pools for
receive and manual build creation. An existing Git-ref row lock holds receive
after it acquires the repository lock; PostgreSQL blocking-PID observations
prove manual waits on that receive. Once released, valid capture produces one
derived build and event shared by both requests. Invalid capture preserves the
receive, creates no build/event, and causes manual `FailedPrecondition`.

Both cases passed in one test with explicit server-barrier markers:
`/home/a/heph-receive-manual-ui-ordering-20260920-v6.log`. Strict app Clippy,
rustdoc, workspace formatting, and architecture passed in
`/home/a/heph-receive-manual-ui-clippy-20260920-v2.log`,
`/home/a/heph-receive-manual-ui-doc-20260920.log`,
`/home/a/heph-receive-manual-ui-fmt-20260920-v2.log`, and
`/home/a/heph-receive-manual-ui-architecture-20260920.log`.
Immutable capture rows are retained for disposable-database teardown; the
test does not weaken deletion guards or use sleeps as ordering evidence.

### Release binding schema checkpoint (2026-09-20)

Migration 84 adds immutable release UI source links, descriptors, static file
bindings, managed-service bindings, and API bindings. Natural keys and composite
foreign keys enforce exact build/capture identity and same-release artifact or
agent ownership; static rows require file kind and matching allowed MIME.
Source links reuse immutable capture evidence without duplicating configuration
JSON. Bounded routes, declaration enums, and append-only guards are enforced.

Every insert checks and locks its parent release in draft state. The concurrency
test relies on the trigger's own lock: insert-first blocks publication, while
publish-first causes the blocked insert to fail after publication commits.
Both opposing blocker PIDs are observed. Published and revoked releases reject
new rows in all five tables. Forced RLS gives authorized owners visibility and
filters outsiders across all five tables; unauthorized insertion fails closed.

Both real PostgreSQL schema tests passed in
`/home/a/heph-release-ui-schema84-20260920.log`. Production bearer-push bootstrap
passed (1/1) with the application expecting migration 84 in
`/home/a/heph-schema84-bootstrap-production-20260920.log`. Scoped release/app
Clippy, rustdoc, formatting, and architecture passed; final test-review checks
are `/home/a/heph-release-ui-schema84-final-{fmt,clippy}-20260920.log`.
Cross-row entrypoint matching, aggregate declaration limits, and canonical
hash validation remain responsibilities of the typed publication validator.
The schema is not yet populated by `complete_build`; publication wiring and
authorized UI inspection remain pending.

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

- [x] **1. Specify the distribution UI declaration and publication model**
  - [x] Define a versioned release declaration for UI artifact kind, immutable
    artifact reference, scope (`project`, `repository`, or explicitly installed
    `global`), bounded route base, tab label/icon, presentation mode, declared
    gateway APIs, and compatibility behavior.
  - [x] Validate names, paths, route ownership, icon allowlist, MIME types,
    artifact size, entrypoint, cache policy, and duplicate/conflicting tabs.
  - [x] Persist an immutable published UI declaration tied to the release and
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
