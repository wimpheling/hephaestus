# Hephaestus architecture

Status: normative

This document defines the target architecture of Hephaestus. A statement that
uses **must**, **must not**, **only**, **required**, or **forbidden** is a
normative constraint. Every mechanically enforceable constraint has a stable
rule ID in the [rule index](#architecture-rule-index). Architectural guidance
that depends on engineering judgment is intentionally documentation-enforced;
that classification does not mean that the guidance is an unimplemented lint.

The repository is migrating to this architecture one constraint family at a
time. The [migration policy](#migration-policy) controls when checks become
hard failures. Until a check is active, its entry still describes the required
end state but the checker must report it as planned rather than establish an
allowlisted violation baseline.

## Process topology and trust boundaries

```text
browser
  | HTTPS, browser session
  v
Phoenix / LiveView presentation process
  | native gRPC over HTTP/2, generated client, short-lived mediator assertion
  v
hephaestusd
  |-- Connect/gRPC API module
  |-- application operations and worker modules
  |-- Git smart-HTTP and health/bootstrap allowlisted HTTP
  |-- PostgreSQL adapters ------> PostgreSQL (authoritative state and event log)
  |-- event adapters -----------> NATS JetStream (delivery, never authority)
  |-- storage/runtime adapters -> repositories, artifacts, volumes, workspaces, VM
  `-- integration adapters -----> OIDC and other external HTTP services
```

`hephaestusd` is the sole application server and the composition root. API and
workers are separate internal modules in that process. A future `api`,
`worker`, or `all` role may select modules without changing their boundaries;
this migration does not add a second Rust application-server process.

The browser and Phoenix are outside the application-authority boundary.
Phoenix owns routes, browser OIDC and session lifecycle, LiveView lifecycle,
page-local presentation state, and rendering. It does not own authorization,
domain decisions, durable state, event publication, repositories, artifacts,
or runtime resources. Its only application connection is generated native
gRPC. Phoenix receives identity from the browser session and exchanges it for
a short-lived, audience-bound assertion; it must never let a request body
select the actor.

The Rust RPC boundary authenticates that assertion and creates trusted request
context. Every query, command, stream subscription and delivery, repository
read, and artifact read is authorized there. PostgreSQL row-level security is
defence in depth and is installed only by PostgreSQL adapters. NATS, local
filesystems, Git processes, VM providers, and external HTTP services are
untrusted I/O boundaries behind adapters.

Git smart HTTP and minimal health or unavoidable bootstrap endpoints are the
only deliberate non-Connect protocols served by `hephaestusd`. They do not
create a general REST or JSON application API.

## Rust layers and dependency direction

Every workspace package must declare a layer and bounded context in Cargo
package metadata. SQL-capable adapters additionally declare their capability;
the checker derives its allowlist from metadata rather than directory names.

```text
domain types and invariants
        ^
application operations and ports
        ^
adapters: PostgreSQL, event, filesystem, VM, OIDC, external integration
        ^
composition root and Connect transport
```

In Cargo dependency terms, outer layers may depend on inner contracts. Inner
layers must not import an adapter, worker, composition root, generated RPC
server, or transport error. Cross-context use goes through an application port,
not another context's concrete adapter. Cycles and undeclared layers are
forbidden.

Approved top-level public modules are declared per package as a bounded list.
The default areas are `domain`, `application`, `ports`, `postgres`, `events`,
`storage`, `integrations`, `rpc`, `workers`, `config`, and `composition`, as
applicable to the declared layer. A package does not gain all areas merely by
naming one; its metadata is the authority.

The graph declaration is explicit and checked from Cargo metadata. Every
workspace package has `layer` (`domain`, `port`, `application`, `adapter`,
`worker`, `transport`, `composition`, or `development`) and a lowercase
bounded `context`. Production and build path dependencies must point inward in
that layer order; development-only test dependencies are excluded from the
production graph. A deliberate cross-context adapter edge names its exact
target in `allow_cross_context_dependencies`; undeclared edges and dependency
cycles fail `cargo dev check architecture`.

Domain packages may use bounded value-object support such as `serde`,
`serde_json` for typed documents, `time`, `uuid`, hashing, and typed error
crates. They may not depend on async runtimes, transport stacks, SQL clients,
or operating-system/process libraries; the checker reports those capability
dependencies as layer violations.

Representation conversion is kept at the corresponding boundary: generated
RPC types cannot be imported by inward layers, SQL row decoding remains in
PostgreSQL adapters, and durable event envelopes are validated by the event
architecture checks. These checks make protobuf, persistence, and event
representations unavailable as reusable domain/application types.

Environment reads belong in `config`; external HTTP clients in `integrations`;
subprocess creation in declared process, Git, image, or runtime adapters; and
repository, artifact, workspace, volume, and runtime filesystem access in
declared storage/runtime adapters. Connect, Axum application transport, and
generated server types belong in `rpc` or `composition`. NATS construction and
publication belong in event adapters and worker composition.

Compiler-aware semantic checks are used only when Cargo metadata, filesystem
inspection, or stable syntax parsing cannot prove a rule. Semantic exceptions
must be narrower than the item that needs them and include the rule ID in the
required exception format.

### Source-size thresholds

Source size is a warning, not a substitute for a cohesion review. The root
`architecture.toml` is the single source of truth for layer-specific
thresholds; values are not duplicated in checker code. Generated files may use
their declared threshold without an exception. All other overrides require a
narrow exception.

## Protobuf and RPC boundary

Versioned shared protobuf sources are the canonical cross-process application
contract. Rust Connect server/client code and native gRPC Elixir messages and
stubs are reproducibly generated from those sources and are never hand-edited.
Buf formatting, linting, generation, and breaking checks protect the contract.

Services expose domain-oriented queries and commands, not generic CRUD or
database-shaped messages. Generated messages are transport models and must be
converted at the boundary. Domain and application public signatures do not use
them. Each RPC method has a predictable implementation file; its service module
is only the generated-trait bridge. A handler authenticates, converts types,
invokes one application operation, maps typed results, and emits metadata. It
does not perform SQL, event publication, filesystem, VM, external HTTP, process,
or domain-decision work.

Mutations use standard request and idempotency context. Collection queries use
bounded pagination and stable ordering. Watch requests accept a resume cursor
and their snapshot or first event reports the committed cursor. Every method
declares authentication and authorization policy through approved protobuf
options. Side-effect-free unary queries carry the standard idempotency option.
Ordinary request messages cannot contain actor selectors.

Application errors are typed below transport and mapped once at the RPC
boundary to stable codes and typed details. Free-form JSON, protobuf `Struct`,
arbitrary bytes, and map-shaped escape hatches do not replace a known versioned
application message. Removed protobuf field names and numbers remain reserved.

## Event semantics

PostgreSQL state is authoritative. Hephaestus distinguishes:

- internal domain events, which express changes inside an operation and never
  cross a process boundary directly;
- durable application events, which are append-only records committed in the
  same PostgreSQL transaction as authoritative state; and
- client-facing product events, which are authorized protobuf projections of
  durable events delivered by scoped watch RPCs.

Every product event uses the canonical envelope: stable event ID, monotonic
cursor within a documented scope, scope identity, aggregate type and ID,
aggregate version, occurrence timestamp, safe actor and request provenance,
schema version, and a typed `oneof` payload. Payload variants contain only a
typed change kind, normalized lifecycle state, and explicitly related opaque
IDs; raw ref names and commit values remain behind ordinary snapshot RPCs. A
product event never carries arbitrary JSON, untyped bytes, secrets,
credentials, tokens, raw command environments, or unbounded diagnostics.

Delivery is at least once and ordered by cursor within its declared scope.
Consumers deduplicate by stable event ID, detect aggregate-version gaps, and
handle reconnect and retention gaps explicitly. A snapshot and its committed
cursor are obtained without a race with subscription. Reconnection resumes
after that cursor; duplicate delivery must produce neither a duplicate visible
transition nor a repeated side effect.

Every successful mutation returns a `MutationReceipt` containing the stable
event ID, aggregate version, and committed cursor in its primary product-event
scope. The primary scope is identity for identity bootstrap, repository for a
build request, the target run for run control, project for agent import,
agent-instance for later instance commands, secret owner for secret lifecycle
commands, and grant/import target for secret sharing commands. When one
transaction invalidates several scopes, the receipt identifies this primary
event; clients refresh other views through their scoped watches and snapshots.

There is no global product-event stream. The protocol exposes separate watches
for the authenticated identity's safe profile and organization-index
invalidations and for an authorized organization, project, repository, run, or
agent instance. Identity scope is derived from the mediator assertion and is
never caller-selectable. Each watch has bounded event-count and byte budgets,
an opaque scope-bound resume cursor, and explicit retention-gap and
access-revoked terminal items.

For a new watch, the server establishes the live subscription first and then,
in one database snapshot, reads the committed scope cursor and visible
aggregate-version references. It emits those facts as the first
`ScopeSnapshotBarrier`. The client buffers events strictly after the barrier
cursor, loads its ordinary typed RPC snapshots, and then applies the buffered
events in cursor order while deduplicating stable event IDs. A resumed watch
delivers events strictly after the supplied cursor. If that cursor predates
retention, the server emits `RetentionGap` and closes so the client restarts the
barrier handshake. Authorization is checked both when the stream starts and
before each delivery; `AccessRevoked` is the last permitted item.

Only an outbox publisher may publish committed product-event records to NATS.
State-changing consumers use a durable inbox or an equivalent idempotency
claim, and an external side effect cannot happen before that durable claim.
Stream creation and every delivery are reauthorized; revocation terminates the
stream without disclosing later events. Logs, metrics, and artifact bytes use
separate bounded streams.

## PostgreSQL ownership

## Authentication evidence

Connect requests are wrapped by the Axum mediator middleware, which validates
the exact URI audience and installs the trusted `AuthenticatedIdentity`
extension. RPC request conversion consumes that extension, while adapters set
transaction-local actor context through `begin_actor_transaction`. Focused
evidence: `cargo test -p hephaestus-app rpc::auth --lib` (mediator forgery,
audience, expiry, bootstrap-binding, non-disclosure, and non-RPC path tests),
the durable watch integration test
`durable_watch_resumes_across_disconnect_gap_duplicate_wake_and_revocation`,
and `cargo run -p hephaestus-dev -- check architecture` (hard-enabled SEC
rules, LiveView architecture checks, authorization, event-stream
reauthorization, and sentinel scan).

SQL, SQLx capability, PostgreSQL pools and transactions, row decoding, RLS
context installation, optimistic concurrency, and persistence error mapping
belong only in packages explicitly declared as PostgreSQL adapters. Application
ports express bounded operations and units of work without leaking SQLx or row
types. Cross-record atomicity, including state plus durable event insertion, is
implemented by those units of work.

Every SQL-capable package uses this exact Cargo metadata declaration:

```toml
[package.metadata.hephaestus]
postgres_adapter = true
database_context = "authorization"
```

`postgres_adapter` accepts only `true`; non-adapters omit it. `database_context`
is a non-empty lowercase identifier naming the bounded persistence context.
Direct SQLx dependencies are forbidden elsewhere. For transitive analysis, a
valid declared adapter is an explicit capability firewall: consumers may depend
on its provider-neutral public API, but traversal may not reach SQLx through an
undeclared intermediate package. Test targets follow the same package rule.

Schema-changing SQL belongs only under the root `migrations/` ownership
boundary. Adapter SQL is static or compile-time checked. Dynamic construction
is forbidden unless an exact query-builder exception documents why a finite
static query cannot express the operation and constrains all interpolated
fragments.

## Phoenix page architecture

```text
LiveView route adapter
  |-- page state/effects module --> generated RPC client
  `-- presentation model --------> pure page
                                      |
                                      v
                                  composites
                                      |
                                      v
                                  components
                                      |
                                      v
                              raw HTML and CSS implementation
```

Every `lib/**/live/<name>_live.ex` route adapter has exactly these companions:

- `lib/**/live/<name>_state.ex`, defining the sibling `...<Name>State`;
- `lib/**/design_system/pages/<name>_page.ex`, defining
  `...DesignSystem.Pages.<Name>Page`;
- `test/**/<name>_state_test.exs`; and
- `test/**/<name>_page_test.exs`.

State/effects modules are application-side LiveView support and never live
under `design_system/pages`; that namespace is presentation-only.

The LiveView owns only Phoenix lifecycle callbacks, effect scheduling,
stream-task lifetime, assignment of the state struct to its socket, and a
`render/1` that invokes exactly one matching `<Name>Page.*>` component. The
socket is the only page-state runtime. A page-local `GenServer`, `Agent`, ETS
table, process registry, or Zustand-like duplicate store is forbidden.

Some Phoenix responsibilities remain review- and judgment-based rather than
linted. Route adapters preserve immediate form usability and session lifecycle;
state modules derive page-local presentation and choose reviewed copy for typed
transport failures; and LiveViews orchestrate those effects without turning
presentation convenience into domain authority. The `WEB-*` checks enforce the
objective consequences—dependencies, calls, I/O, client ownership, and error
text—not a subjective limit on how much orchestration is "too much."

Every state/effects module defines the following common state shape; page-
specific values belong below `data`, and retained non-sensitive form values
belong below `form`. `data` and `form` may supply page-specific initial values;
the lifecycle defaults remain `status: :initial`, `error: nil`, `cursor: nil`,
and `stream_generation: 0`:

```elixir
@statuses [:initial, :loading, :ready, :submitting, :error, :stale,
           :reconnecting, :access_revoked]
defstruct status: :initial,
          data: nil,
          form: %{},
          error: nil,
          cursor: nil,
          stream_generation: 0
```

The module exposes `statuses/0`, `new/1`, a pure `reduce/2` returning
`{state, [effect_spec]}`, `execute/2` for one scheduled effect, and `present/1`
returning an ordinary presentation map. It owns generated-client calls,
command execution, typed backend-error
presentation, event reduction, reconnect cursors, and construction of that
presentation map. An adapter schedules an effect and routes its tagged result
back through `reduce/2`; it does not implement product state transitions.
State modules contain no HEEx, design-system or page imports, LiveView runtime,
or rendering helpers.

The state test declares literal `@covered_statuses` equal to all eight common
statuses and exercises every reducer/result transition. The page render test
either declares the same complete `@covered_statuses`, or declares a literal
`@status_visual_states` map from all eight statuses to its visual
`@covered_states`; every mapped visual state is rendered. File existence alone
does not satisfy the companion contract.

Page components accept presentation values and declared interaction
properties only. Interaction event names use bounded component attributes such
as `attr :on_save, :string, values: ["save"]`; dynamic options use equivalently
bounded scalar attributes rather than arbitrary maps. Pages cannot access a
socket, define LiveView callbacks, or reference protobuf messages, RPC clients,
application/domain services, filesystems, processes, or mutable state. Backend
errors go through the state module's typed presenter; raw exception or backend
text is never rendered.

Adapters declare `@stream_mode :none` or `@stream_mode :page_scoped`. A page-
scoped Connect stream runs under the application `Task.Supervisor`, is
cancelled on termination or replacement, resumes from the last committed
`cursor`, increments `stream_generation` when replaced, and tags every message
with that generation. The adapter drops messages whose generation does not
equal the currently assigned state, so a late task cannot mutate a replacement
page process. Cursor advancement happens only after an event is accepted by
the reducer.

Secret form plaintext is handled transiently between `handle_event/3` and the
state module's RPC encoder. It may be captured by the immediately scheduled
supervised task, but neither the plaintext, encoded request, nor an effect spec
containing it may be assigned to the socket or retained in the state struct.
Validation may retain only non-sensitive fields and redacted error metadata.

A genuinely shared stream may be multiplexed only by an application-supervised
projection process. Such a projection is a scoped, reauthorized, bounded,
read-only derived cache: it may reduce typed events and serve presentation
snapshots, but it may not accept commands, call repositories, persist business
state, mint authorization decisions, or become domain authority. Subscribers
still carry cursors and must stop receiving events after access revocation.

## Design system

The UI has three downward-only tiers:

1. `components` are the only tier that emits raw lowercase HTML, SVG, form
   controls, or CSS implementation classes;
2. `composites` combine only the public component facade and explicitly
   permitted lower composites; and
3. `pages` combine only the public component/composite facade.

Root document markup, application layouts, flash, icons, links, typography,
controls, forms, tables, lists, dialogs, and semantic layout elements therefore
live in the component tier. Pages, composites, layouts, controllers,
LiveViews, and ordinary helpers emit no raw tags. Dependencies never point
upward and application modules are forbidden below pages.

Public styling is expressed through bounded properties such as tone, size,
density, spacing, alignment, width, and state. Public `class` passthrough is
forbidden. Design tokens are centralized; product pages and composites do not
invent literal colors, fonts, radii, shadows, or spacing. External UI libraries
and markup-producing JavaScript hooks are isolated behind basic components.
Every public component/composite has a facade export, bounded attributes and
slots, a showcase example, and an accessibility-focused test. Every possible
page-state variant has a render test.

## Security and sensitive data

Plaintext secrets are permitted only in explicitly designated request fields
marked by the protobuf sensitive-field option. Sensitive fields and suspicious
secret-bearing names are forbidden in responses, product events, durable
events, error details, logs, metrics, diagnostics, artifacts, repositories, and
browser content. Sensitive domain and generated request types cannot derive or
implement unrestricted debug, display, serialization, or tracing behavior.
Known sensitive values and types cannot be passed to formatting, logging,
diagnostic, metric-label, or error-formatting macros.

## Enforcement model and commands

An invariant has one or more of these classifications:

- **documentation**: review and design evidence are the correct enforcement;
- **structural**: types, visibility, metadata, descriptors, or filesystem shape
  make violations impossible or mechanically detectable;
- **lint**: a stable repository, Rust semantic, Mix, HEEx, CSS, or protobuf
  checker rejects violations; and
- **integration**: behavior is proved at an actual process or persistence
  boundary.

The development CLI is the stable entry point. The focused commands are
`cargo dev check architecture`, `cargo dev check protobuf`, `cargo dev check rust`,
`cargo dev check phoenix`, and `cargo dev check ui`. `cargo dev quality` is the
single repository handoff command: it composes generated-code/Buf checks,
architecture, Rust formatting/Clippy/tests/docs, Phoenix, UI, and focused
integration tests. `cargo dev check full` remains a compatibility alias. The
rule index names the focused command even while its state is **planned**; the
quality gate must not silently run planned rules as hard failures. Diagnostic
form is:

```text
<RULE-ID> <path-or-package>: <problem>; see ARCHITECTURE.md#<rule-anchor>;
remediation: <specific next action>
```

Root `architecture.toml` is the machine-readable harness configuration. It
declares the configuration version, currently active harness rules, central
file-length thresholds, package/layer declarations as they are introduced, and
the complete exception list. The normative meaning and remediation of rules
remain in this document; configuration cannot redefine an invariant.

Metadata and filesystem inspection are preferred for package graphs, declared
capabilities, paths, and predictable file layouts. Rust compiler lints are
reserved for type-aware semantic rules. The Phoenix task uses Elixir AST and an
HEEx-aware parser, never a multiline-bypassable tag regex.

## Exception policy

Exceptions are rare, reviewable records stored in root `architecture.toml` so
the enforcement harness can validate them uniformly. Scope uses exactly one of
`path:line` for a single source line or `path#item` for a single named source
item. It must contain all fields:

```toml
[[exceptions]]
rule_id = "<stable rule ID>"
scope = "<repository-relative-path>:<line> | <repository-relative-path>#<item>"
rationale = "<why the invariant is counterproductive or impossible here>"
owner = "<accountable team or person>"
expires = "<ISO date>" # or: tracking_task = "<task path or issue>"
```

An exception without a known rule ID, exact existing scope, rationale, owner,
and expiry or tracking task is invalid. Glob, package-wide, workspace-wide, and
directory-wide scopes are rejected. Generated-code policy is configured at the
generator, not excused file by file. Expired or unresolved exceptions fail the
check.

## Migration policy

Migration is global and constraint-by-constraint. For each numbered constraint:

1. document the target and implement valid/invalid checker fixtures;
2. keep the new repository rule planned while removing every violation;
3. prove the entire repository satisfies the constraint;
4. activate the rule as a hard failure and record the evidence; then
5. begin the next numbered constraint.

No later constraint begins before the active constraint passes repository-wide.
There is no permanent baseline, grandfathered module, or “new code only” mode.
An exception is not a migration baseline. Constraint 0 is complete when the
enforcement harness and its fixture tests pass, while rules for Constraints 1
through 9 remain explicitly planned until their global migration begins.

## Architecture rule index

`State` is **harness** for rules registered as active hard failures and
**migration-gated** for rules awaiting their global migration. `Command` is the
stable focused entry point. Each remediation is the default; a conforming
exception must follow the policy above.

### Enforcement harness

| Rule | Class / state | Rationale and scope | Command | Exceptions and remediation |
| --- | --- | --- | --- | --- |
| `ARCH-RULE-REGISTRY` | Structural + lint / harness | Prevents unknown, duplicated, undocumented, or silently inactive rule IDs; covers the rule declarations in `architecture.toml`, checker registrations, diagnostics, and this index. | `cargo dev check architecture` | None; add one stable indexed ID, classification, state, command, and checker registration. |
| `ARCH-EXCEPTION-FORMAT` | Structural + lint / harness | Prevents broad or ownerless violation baselines; covers every exception in `architecture.toml`, including required fields, exact scope grammar, expiry/tracking, and live targets. | `cargo dev check architecture` | An exception cannot except this rule; narrow the scope to `path:line` or `path#item` and supply all required ownership fields. |
| `ARCH-CARGO-METADATA` | Structural + lint / harness | Ensures dependency/path checks use stable workspace facts rather than shell-text approximations; covers successful `cargo metadata` loading and filesystem-root validation. | `cargo dev check architecture` | None; repair Cargo metadata or the declared repository-relative path. |

### Rust and repository structure

| Rule | Class / state | Rationale and scope | Command | Exceptions and remediation |
| --- | --- | --- | --- | --- |
| `ARCH-CRATE-LAYERS` | Structural + lint / harness | Cargo package layers make the complete workspace dependency graph reviewable; covers every workspace package, edge, cycle, and undeclared layer. | `cargo dev check architecture` | Exact edge only; declare the package metadata or invert/extract the dependency. |
| `ARCH-CONTROLLED-PUBLIC-MODULES` | Structural + lint / migration-gated | Prevents accidental architectural entry points; covers top-level public modules in every layered crate. | `cargo dev check architecture` | One module only; move it under an approved area or add the bounded metadata declaration. |
| `ARCH-ENV-ONLY-IN-CONFIG` | Semantic lint / harness | Makes runtime configuration auditable; covers all Rust environment reads. | `cargo dev check architecture` | One source item only; inject typed config instead. |
| `ARCH-HTTP-ONLY-IN-INTEGRATIONS` | Structural + semantic lint / harness | Keeps outbound protocols behind ports; covers external HTTP client dependencies and construction. | `cargo dev check architecture` | One integration only; create or use a declared integration adapter. |
| `ARCH-PROCESS-ONLY-IN-ADAPTERS` | Semantic lint / harness | Prevents hidden host effects; covers subprocess creation in Rust. | `cargo dev check architecture` | One adapter item only; move it to a declared process, Git, image, or runtime adapter. |
| `ARCH-VM-PROVIDER-ONLY-IN-COMPOSITION` | Semantic lint / harness | Prevents VM provider coupling from leaking into application code. | `cargo dev check architecture` | VM providers may be imported only by composition or adapter crates; application code uses `vm-trait`. |
| `ARCH-FILESYSTEM-ONLY-IN-ADAPTERS` | Semantic lint / harness | Keeps durable and runtime files behind authority boundaries; covers repository, artifact, workspace, volume, and runtime I/O. | `cargo dev check architecture` | One adapter item only; inject the appropriate storage port. |
| `ARCH-MAX-FILE-LENGTH` | Lint warning / migration-gated | Oversized files signal mixed responsibilities; covers source files using the central layer thresholds. | `cargo dev check architecture` | One file with owner and tracking task; split cohesive modules. |
| `DB-SQLX-ONLY-IN-POSTGRES-ADAPTERS` | Structural + lint / harness | Prevents persistence capability leaking inward; covers direct and transitive SQLx capability in the Cargo graph. Dev-only SQLx test fixtures require explicit `hephaestus.sqlx_test_dependency = true` metadata. | `cargo dev check architecture` | No production capability exception; move SQL to a metadata-declared PostgreSQL adapter. |
| `DB-MIGRATIONS-ONLY-IN-MIGRATIONS` | Filesystem lint / harness | Gives schema changes one owner; covers schema-changing SQL repository-wide. | `cargo dev check architecture` | Exact generator-owned schema artifact only; move changes under root `migrations/`. |
| `DB-STATIC-SQL` | Semantic lint / harness | Makes queries auditable and injection-resistant; covers SQL passed to SQLx in PostgreSQL adapters. | `cargo dev check rust` | Exact query builder with finite fragments; replace interpolation with static or checked SQL. |
| `RPC-CONNECT-ONLY-IN-TRANSPORT` | Structural + lint / harness | Prevents transport concerns leaking inward; covers Connect, application Axum, and generated server types. | `cargo dev check architecture` | Exact composition bridge only; move code to `rpc`/`composition` or convert at the boundary. |
| `RPC-METHOD-IN-SEPARATE-FILE` | Filesystem + structural lint / harness | Keeps handlers predictable and reviewable; covers every generated service method and bridge module. | `cargo dev check architecture` | No permanent layout exception; create the expected service/method file. |
| `RPC-NON_RPC-HTTP-ALLOWLIST` | Structural lint / harness | Prevents a shadow REST API; covers every non-Connect Axum handler. | `cargo dev check architecture` | Exact Git, health, or unavoidable bootstrap route with rationale; remove or convert the endpoint. |
| `RPC-GENERATED-FILES-CLEAN` | Generation diff / harness | Proves shared bindings are reproducible and unedited; covers all generated Rust and Elixir outputs. | `cargo dev check protobuf` | None for hand edits; update proto/generator and regenerate. |
| `EVT-NATS-ONLY-IN-EVENT-ADAPTERS` | Structural + semantic lint / harness | Gives product-event publication one controlled boundary; covers NATS construction and publish APIs. | `cargo dev check architecture` | Exact worker composition wiring only; move behavior to an event adapter/outbox publisher. |

### Semantic Rust

| Rule | Class / state | Rationale and scope | Command | Exceptions and remediation |
| --- | --- | --- | --- | --- |
| `RPC-ERRORS-MAPPED-AT-BOUNDARY` | Semantic lint / harness | Keeps application errors transport-independent; covers domain, application, persistence, event, and worker layers. | `cargo dev check rust` | None inward; return a typed application error and map it in RPC. |
| `RPC-NO-DIRECT-CONNECT-ERROR` | Semantic lint / harness | Ensures one stable error vocabulary; covers construction of Connect errors. | `cargo dev check rust` | Central typed error adapter only; call that adapter. |
| `RPC-HANDLER-IS-THIN` | Semantic lint + integration / harness | Keeps transport deterministic; covers all RPC method implementations and forbidden I/O/domain operations. | `cargo dev check rust` | No I/O exception; invoke one application operation through ports. |
| `RPC-GENERATED-TYPES-DO-NOT-LEAK-INWARD` | Semantic lint / harness | Preserves independent domain/application models; covers public signatures below RPC. | `cargo dev check rust` | Boundary conversion item only; introduce an inward type and converter. |
| `SEC-SENSITIVE-NO-UNRESTRICTED-FORMAT` | Type structure + semantic lint / harness | Prevents accidental disclosure by derived behavior; covers known sensitive domain/request types. | `cargo dev check architecture` | Exact redacted implementation only; remove derive or implement fixed redaction. |
| `SEC-NO-SENSITIVE-LOG-ARGUMENTS` | Semantic lint + sentinel integration / harness | Prevents disclosure through observability; covers logging, diagnostics, metrics labels, and error formatting. | `cargo dev check architecture` | No plaintext exception; log opaque IDs or fixed classifications. |
| `SEC-SENTINEL-NO-PLAINTEXT` | Repository sentinel scan / harness | Prevents secret sentinel values from entering production Rust, Phoenix, protobuf, event, artifact, or repository sources. | `cargo dev check architecture` | Test-only fixtures and the dedicated integration-check binary are explicit test boundaries. |

### Protobuf descriptors

| Rule | Class / state | Rationale and scope | Command | Exceptions and remediation |
| --- | --- | --- | --- | --- |
| `RPC-MUTATION-HAS-IDEMPOTENCY-KEY` | Descriptor lint + integration / harness | Makes retries safe; covers every mutating request. | `cargo dev check protobuf` | None; add standard request/idempotency context and operation tests. |
| `RPC-LIST-HAS-PAGINATION` | Descriptor lint + integration / harness | Bounds collections and fixes ordering; covers every collection RPC response/request pair. | `cargo dev check protobuf` | Exact demonstrably bounded singleton only; add page size/token, next token, and stable order. |
| `RPC-WATCH-HAS-RESUME-CURSOR` | Descriptor lint + integration / harness | Makes reconnect lossless and explicit; covers all watch requests and initial/event responses. | `cargo dev check protobuf` | None; add standard resume and committed cursors. |
| `RPC-AUTHORIZATION-POLICY-DECLARED` | Descriptor lint + integration / harness | Makes access policy reviewable; covers every RPC method. | `cargo dev check protobuf` | None; declare approved authentication/authorization options. |
| `RPC-NO-ACTOR-IN-REQUEST` | Descriptor lint + authentication integration / harness | Prevents actor spoofing; covers ordinary request fields and nested messages. | `cargo dev check protobuf` | Explicit administrator subject-selection message only; take actor from trusted context. |
| `RPC-QUERY-IDEMPOTENCY-ANNOTATED` | Descriptor lint / harness | Allows safe query retries and tooling; covers side-effect-free unary queries. | `cargo dev check protobuf` | None; apply the standard protobuf idempotency annotation. |
| `RPC-REMOVED-FIELDS-RESERVED` | Buf breaking + descriptor lint / harness | Prevents wire-field reuse; covers removed message field numbers and names. | `cargo dev check protobuf` | None; reserve both name and number. |
| `RPC-NO-UNTYPED-APPLICATION-PAYLOADS` | Descriptor lint / harness | Preserves an evolvable contract; covers application messages using Struct, arbitrary JSON/bytes, or map escape hatches. | `cargo dev check protobuf` | Exact opaque standardized media body only; define a versioned typed message. |
| `EVT-CANONICAL-ENVELOPE` | Descriptor lint + integration / harness | Gives events stable identity, ordering, provenance, and evolution; covers every product event. | `cargo dev check protobuf` | None; embed the canonical envelope fields. |
| `EVT-TYPED-ONEOF-PAYLOAD` | Descriptor lint / harness | Makes all event variants enumerable and reducible; covers product-event payloads. | `cargo dev check protobuf` | None; replace JSON/bytes/maps with typed `oneof` variants. |
| `SEC-SENSITIVE-REQUEST-ANNOTATED` | Descriptor lint / harness | Identifies the only legal plaintext inputs; covers secret/credential/token/sensitive request fields and requires the custom option on sensitive leaves. | `cargo dev check protobuf` | None; add the custom option or remove the field. |
| `SEC-NO-SENSITIVE-OUTPUT-FIELDS` | Descriptor lint + sentinel integration / harness | Prevents secret egress and durable disclosure; covers responses, events, errors, logs/metric event schemas, and suspicious names. | `cargo dev check protobuf` | No plaintext exception; remove or replace with opaque metadata. |

### Phoenix and design system

| Rule | Class / state | Rationale and scope | Command | Exceptions and remediation |
| --- | --- | --- | --- | --- |
| `WEB-NO-INFRASTRUCTURE-DEPENDENCIES` | Mix dependency/import lint / harness | Keeps Phoenix presentation-only; covers Ecto SQL, Postgrex, NATS, Git, repository/artifact storage, and infrastructure clients. | `cargo dev check phoenix` | None; replace with generated RPC use in state/client layers. |
| `WEB-RPC-CLIENTS-ONLY-IN-STATE` | Elixir AST lint / harness | Centralizes effects and page state; covers generated calls and client construction. | `cargo dev check phoenix` | Supervised client setup or page state/effects item only; move the call. |
| `WEB-NO-HANDWRITTEN-BACKEND-CLIENT` | Dependency + AST lint / harness | Prevents protocol drift and bypassed policy; covers Req, Finch, raw framing, and equivalent backend calls. | `cargo dev check phoenix` | Browser/OIDC integration only; use generated application stubs. |
| `WEB-NO-RAW-BACKEND-ERROR` | Elixir AST + render tests / harness | Prevents unstable or sensitive backend text reaching users; covers presentation models and notices. | `cargo dev check phoenix` | None; map through the typed error presenter. |
| `WEB-NO-FILESYSTEM-OR-PROCESS` | Elixir AST lint / harness | Removes storage/runtime authority from Phoenix; covers filesystem, Port, and subprocess APIs. | `cargo dev check phoenix` | Build-tool modules outside the application only; add an authorized RPC. |
| `UI-RAW-HTML-ONLY-IN-COMPONENTS` | HEEx-aware lint / harness | Makes the basic tier the sole markup authority; covers raw HTML/SVG in all templates and render functions. | `cargo dev check ui` | Design-system basic component only; replace tags with facade components. |
| `UI-TIER-DIRECTION` | Elixir AST dependency lint / harness | Prevents upward/cyclic UI coupling; covers pages, composites, components, and implementation imports. | `cargo dev check ui` | Exact explicitly allowed lower-composite edge only; import the public facade. |
| `UI-PAGE-COMPANIONS` | Filesystem + module lint / harness | Requires exactly one conventionally named sibling state module, design-system page, state test, and render test for every LiveView route. | `cargo dev check ui` | None; add or rename the exact companions. |
| `UI-LIVE-RENDERS-ONE-PAGE` | HEEx/AST lint / harness | Keeps adapters to callbacks, one socket-hosted state, supervised stream lifetime, and exactly one matching page render. | `cargo dev check ui` | None; move product logic/effects into state and composition into the matching page. |
| `UI-STATE-HAS-NO-HEEX` | Elixir AST/HEEx + state-contract lint / harness | Requires the standard state/status/reducer contract and excludes HEEx, UI imports, sockets, and page-local runtimes from state/effects modules. | `cargo dev check ui` | None; return a presentation model and effect specs instead. |
| `UI-PAGE-IS-PURE` | Elixir AST/import lint / harness | Makes rendering deterministic; covers callbacks, socket/runtime calls, protobuf/RPC clients, services, filesystems, processes, and mutable state in pages. | `cargo dev check ui` | None; move effects/state to the state module or callback wiring to the adapter. |
| `UI-DECLARED-INTERACTIONS-ONLY` | Attribute + HEEx lint / harness | Makes dynamic event behavior bounded and reviewable; covers page/component event names and options. | `cargo dev check ui` | None; declare the interaction property and its allowed values. |
| `UI-NO-CLASS-ESCAPE-HATCH` | Attribute + HEEx lint / harness | Preserves the styling boundary; covers public attributes and page/composite classes. | `cargo dev check ui` | Basic implementation item only; expose a bounded property. |
| `UI-DESIGN-TOKENS-ONLY` | CSS/HEEx lint / harness | Prevents product-local visual vocabulary; covers literal colors, fonts, radii, shadows, and spacing. | `cargo dev check ui` | Exact design-system token definition only; add/reuse a token and bounded property. |
| `UI-NO-EXTERNAL-UI-IMPORTS` | Dependency/import lint / harness | Keeps third-party implementation details behind the facade; covers external UI libraries. | `cargo dev check ui` | Basic component wrapper only; import through that wrapper. |
| `UI-NO-DOM-INJECTION` | JavaScript syntax lint / harness | Prevents markup bypass and injection; covers innerHTML, insertAdjacentHTML, raw DOM creation, and equivalents. | `cargo dev check ui` | Designated design-system hook with sanitized bounded content only; render through HEEx/components. |
| `UI-PUBLIC-FACADE-COMPLETE` | Filesystem + AST lint / harness | Keeps the supported UI API explicit; covers component/composite implementations, exports, attributes, and slots. | `cargo dev check ui` | None; add the matching facade declaration or make implementation private. |
| `UI-SHOWCASE-AND-TEST-PARITY` | Filesystem + test manifest lint / harness | Ensures public UI is demonstrable and accessible; covers every facade export. | `cargo dev check ui` | None; add a rendering example and accessibility-focused test. |
| `UI-PAGE-STATE-COVERAGE` | Structural + render-test lint / harness | Prevents untested loading/error/reconnect UI; covers every state variant declared by page state. | `cargo dev check ui` | Impossible variants may be removed from the type; otherwise add render coverage. |

### Event behavior

| Rule | Class / state | Rationale and scope | Command | Exceptions and remediation |
| --- | --- | --- | --- | --- |
| `EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY` | Structural + PostgreSQL integration / harness | Prevents state/event divergence; covers every state-changing unit of work. | `cargo dev check full` | None; append the durable event through the same transaction port and test rollback. |
| `EVT-OUTBOX-PUBLISHER-ONLY` | Visibility + dependency lint + integration / harness | Prevents publication of uncommitted state; covers every product-event publisher. | `cargo dev check full` | Worker composition may construct the publisher only; route publication through committed outbox records. |
| `EVT-CONSUMER-USES-INBOX` | Structural + integration / harness | Makes duplicate delivery harmless; covers every state-changing consumer. | `cargo dev check full` | Proven naturally idempotent operation only; use durable inbox/deduplication and duplicate tests. |
| `EVT-SIDE-EFFECT-AFTER-DURABLE-CLAIM` | Structural + failure-injection integration / harness | Prevents repeated external effects after crashes; covers event handlers that perform external effects. | `cargo dev check full` | None; claim ownership/idempotency durably before the effect. |
| `EVT-REDUCER-COVERAGE` | Descriptor/test parity lint / harness | Keeps every client-facing variant projectable; covers product event `oneof` variants and reducers. | `cargo dev check full` | None; add reducer/projection tests with the schema variant. |
| `EVT-STREAM-REAUTHORIZATION` | Stream integration / harness | Prevents post-revocation disclosure; covers subscription and each streamed delivery. | `cargo dev check full` | None; reauthorize, terminate on revocation, and test no later event arrives. |
