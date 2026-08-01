# Refactor the control plane around Connect, events, and enforceable UI boundaries

Owner: unassigned

## Outcome

Refactor Hephaestus so the Rust application is the sole application, persistence,
authorization, event, repository, and artifact boundary, while Phoenix remains a
stateful server-rendered presentation client.

The completed architecture must provide:

- a protobuf-first Connect/gRPC application API served by `hephaestusd`;
- typed, resumable server-streaming product events for live UI state;
- a Phoenix application with no PostgreSQL, NATS, repository-storage, or
  artifact-storage access;
- a consistent LiveView route-adapter, state, and pure-presentation split;
- a three-tier UI system of components, composites, and pages;
- no raw HTML outside basic design-system components;
- SQL isolated in explicitly designated PostgreSQL adapter crates;
- mechanically enforced Rust, RPC, event, Phoenix, and design-system
  dependency boundaries; and
- one normative `ARCHITECTURE.md` that explains both mechanically enforceable
  rules and architectural expectations that require engineering judgment.

This is a whole-project migration. Complete and enforce one architectural
constraint across the entire repository before beginning the next constraint.
Do not establish a permanent violation baseline or a “new code only” policy.

## Locked decisions

| Area | Decision |
| --- | --- |
| Rust server | Extend the existing `hephaestusd` process into the complete application API; do not introduce a second Rust application-server process. |
| Deployment shape | Keep API and workers as separate internal modules in one process. Preserve the option to introduce `api`, `worker`, and `all` roles later without requiring it in this refactor. |
| Application protocol | Protobuf is the canonical cross-process contract. Use `connectrpc/connect-rust` on the Rust server. |
| Phoenix transport | Phoenix calls the Rust server with native gRPC over HTTP/2 using generated Elixir messages and stubs. |
| Future browser transport | The same Rust services may later be called through Connect or gRPC-Web; direct browser RPC is not required by this task. |
| Non-application HTTP | Git smart HTTP and minimal health/bootstrap endpoints remain deliberate non-Connect protocols. |
| API style | Expose domain-oriented queries and commands, not generic CRUD or database-shaped endpoints. |
| Rust baseline | Raise the workspace MSRV to the version required by the selected pinned `connect-rust` release and record that version explicitly. |
| Generated code | Generate Rust and Elixir bindings from shared proto sources. Generated files are reproducible and never hand-edited. |
| Live updates | Use typed server-streaming RPCs from the start; polling and direct PostgreSQL notifications are not the target architecture. |
| Event authority | PostgreSQL state remains authoritative. Events are a durable committed change log and push model, not a requirement to event-source every aggregate. |
| Event delivery | Delivery is at least once, ordered by a resumable cursor within its documented scope, and safe under duplication and reconnect. |
| Phoenix role | Phoenix owns routes, browser OIDC/session handling, LiveView lifecycle, page-local presentation state, and rendering. It owns no domain or persistence authority. |
| Phoenix state | The LiveView process hosts page-local state. Do not create a Zustand-equivalent GenServer per page. |
| Page split | Every live page is separated into a small LiveView route adapter, a state/effects module, and a pure page component. |
| UI tiers | Components may use raw HTML. Composites use components. Pages use components and composites. Dependencies never point upward. |
| Raw HTML | Raw HTML, including SVG and form controls, is forbidden outside the basic component tier. Root document markup must also live in the designated component tier. |
| Styling | Pages and composites do not invent CSS classes or accept an unrestricted class escape hatch. Public styling is expressed through bounded component properties and design tokens. |
| SQL ownership | SQL and `sqlx` are allowed only in explicitly designated PostgreSQL adapter crates and migrations. Folder-only conventions are insufficient for Rust application code. |
| Architecture rules | `ARCHITECTURE.md` is normative. Linters and tests enforce only the objective subset of its rules. |
| Migration policy | Migrate globally, one constraint at a time. A constraint becomes a hard gate only after every repository violation has been removed. |
| Compatibility | This is initial development. Database, fixture, generated-code, and internal API compatibility may be broken deliberately; no production state migration is required. |

## Terminology and target dependency directions

### Rust

```text
domain types and invariants
        ↑
application operations and ports
        ↑
adapters: PostgreSQL, NATS, filesystem, VM, OIDC
        ↑
composition root and Connect transport
```

Dependencies may point downward in implementation terms: outer layers may
depend on inner contracts, while inner layers must not import outer adapters or
transport concerns.

### Phoenix page architecture

```text
LiveView route adapter
        ↓
page state/effects module
        ↓ generated RPC client

LiveView route adapter
        ↓ presentation model
pure page
        ↓
composites
        ↓
components
        ↓
raw HTML and CSS implementation
```

The state module owns page state, RPC effects, event reduction, reconnect
state, and presentation-model construction. The LiveView owns Phoenix callback
plumbing and hosts the state in socket assigns. The page is a pure function
component and knows neither sockets nor protobuf messages.

## Architectural invariant registry

`ARCHITECTURE.md` must define these rule families and assign stable IDs to
individual invariants. Exact IDs may be refined while authoring the document,
but their meanings must remain explicit and searchable.

| Family | Required meaning | Primary enforcement |
| --- | --- | --- |
| `ARCH-*` | Layer ownership, dependency direction, composition rules, and exception policy. | Documentation, Cargo metadata checks, focused architecture tests. |
| `RPC-*` | Protobuf-only application boundary, generated-code ownership, transport isolation, authentication, and typed errors. | Buf, Cargo checks, Rust lints, integration tests. |
| `EVT-*` | Durable event envelope, transactional publication, cursor semantics, idempotency, authorization, and sensitive-data exclusion. | Types, persistence APIs, descriptor checks, integration tests. |
| `WEB-*` | Phoenix isolation, RPC-only backend access, state ownership, and error mapping. | Mix dependency checks, Elixir AST checks, container tests. |
| `UI-*` | Components/composites/pages, raw-HTML boundary, styling, page states, and accessibility. | HEEx-aware Mix linter, CSS checks, component/page tests. |
| `DB-*` | SQL ownership, static queries, migrations, transaction boundaries, and adapter dependencies. | Cargo metadata, Rust semantic linting, filesystem checks. |
| `SEC-*` | Identity propagation, authorization context, secret non-disclosure, and audit provenance. | Type restrictions, descriptor checks, linting, sentinel tests. |

## Required initial enforcement catalogue

These are the minimum named checks that the architecture harness must provide.
`ARCHITECTURE.md` may refine their final IDs and split a check when implementation
reveals genuinely separate invariants, but it must not silently omit one. Checks
that cannot be proved statically are deliberately assigned to structural APIs or
integration tests instead of being approximated by a misleading linter.

### Stable Rust and repository-structure checks

- [x] `ARCH-CRATE-LAYERS` — read declared Cargo package layers and reject every
  forbidden dependency edge, cycle, or undeclared layer.
- [x] `ARCH-CONTROLLED-PUBLIC-MODULES` — reject top-level public modules outside
  the approved architecture areas for each crate layer.
- [x] `ARCH-ENV-ONLY-IN-CONFIG` — reject environment-variable reads outside
  designated configuration modules.
- [x] `ARCH-HTTP-ONLY-IN-INTEGRATIONS` — reject external HTTP clients outside
  designated integration adapters.
- [x] `ARCH-PROCESS-ONLY-IN-ADAPTERS` — reject subprocess creation outside
  designated process, Git, image, or runtime adapters.
- [x] `ARCH-FILESYSTEM-ONLY-IN-ADAPTERS` — reject repository, artifact,
  workspace, volume, and runtime filesystem access outside declared storage
  adapters.
- [x] `ARCH-MAX-FILE-LENGTH` — warn when a source file exceeds the documented
  layer-specific threshold; configure thresholds centrally and allow only
  narrow justified exceptions.
- [x] `DB-SQLX-ONLY-IN-POSTGRES-ADAPTERS` — reject direct or transitive SQLx
  capability in every crate not explicitly marked as a PostgreSQL adapter.
- [x] `DB-MIGRATIONS-ONLY-IN-MIGRATIONS` — reject schema-changing SQL outside
  the root migration ownership boundary.
- [x] `DB-STATIC-SQL` — reject dynamically constructed SQL passed to SQLx unless
  an exact, documented query-builder exception applies.
- [x] `RPC-CONNECT-ONLY-IN-TRANSPORT` — reject Connect, Axum application
  transport, or generated server types outside RPC and composition modules.
- [x] `RPC-METHOD-IN-SEPARATE-FILE` — require every RPC implementation to live
  in its predictable service/method file with the service module acting only as
  the generated-trait bridge.
- [x] `RPC-NON_RPC-HTTP-ALLOWLIST` — permit non-Connect Axum handlers only for
  declared Git smart-HTTP, health, and unavoidable bootstrap endpoints.
- [x] `RPC-GENERATED-FILES-CLEAN` — regenerate all bindings and fail when the
  resulting worktree differs or generated files contain hand edits.
- [x] `EVT-NATS-ONLY-IN-EVENT-ADAPTERS` — reject NATS client construction and
  publication APIs outside declared event adapters and worker composition.

### Semantic Rust checks

- [x] `RPC-ERRORS-MAPPED-AT-BOUNDARY` — reject Connect transport errors in
  domain, application, persistence, event, and worker layers.
- [x] `RPC-NO-DIRECT-CONNECT-ERROR` — reject direct `ConnectError`
  construction outside the central typed application-error adapter.
- [x] `RPC-HANDLER-IS-THIN` — reject direct SQL, NATS, filesystem, VM, external
  HTTP, and subprocess operations from RPC method implementations.
- [x] `RPC-GENERATED-TYPES-DO-NOT-LEAK-INWARD` — reject generated protobuf
  request/response types in domain and application-service public signatures.
- [x] `SEC-SENSITIVE-NO-UNRESTRICTED-FORMAT` — reject unrestricted debug,
  display, serialization, or tracing of sensitive domain and request types.
- [x] `SEC-NO-SENSITIVE-LOG-ARGUMENTS` — reject known sensitive types passed to
  logging, diagnostics, metrics-label, and error-formatting macros.

### Protobuf and descriptor checks

- [x] `RPC-MUTATION-HAS-IDEMPOTENCY-KEY` — require every mutating RPC request to
  carry the standard request/idempotency context.
- [x] `RPC-LIST-HAS-PAGINATION` — require every collection RPC to use the
  standard page-size, page-token, next-token, and stable-ordering contract.
- [x] `RPC-WATCH-HAS-RESUME-CURSOR` — require every watch request to accept the
  standard resume cursor and every initial response/event to expose the
  corresponding committed cursor.
- [x] `RPC-AUTHORIZATION-POLICY-DECLARED` — require every RPC method to declare
  its authentication and authorization policy through the approved protobuf
  option.
- [x] `RPC-NO-ACTOR-IN-REQUEST` — reject ordinary request fields that allow a
  caller to select or spoof the authenticated actor.
- [x] `RPC-QUERY-IDEMPOTENCY-ANNOTATED` — require side-effect-free unary queries
  to use the standard protobuf idempotency annotation.
- [x] `RPC-REMOVED-FIELDS-RESERVED` — combine Buf breaking checks with a
  descriptor rule requiring removed field numbers and names to remain reserved.
- [x] `RPC-NO-UNTYPED-APPLICATION-PAYLOADS` — reject `Struct`, arbitrary JSON,
  and map-shaped escape hatches where a versioned application message is
  required.
- [x] `EVT-CANONICAL-ENVELOPE` — require every product event to use the standard
  event ID, cursor, scope, aggregate, version, timestamp, provenance, schema,
  and payload envelope.
- [x] `EVT-TYPED-ONEOF-PAYLOAD` — require event payloads to be typed `oneof`
  variants rather than arbitrary bytes, JSON, or unbounded maps.
- [x] `SEC-SENSITIVE-REQUEST-ANNOTATED` — require the custom sensitive-field
  option on every permitted plaintext secret, credential, token, or sensitive
  parameter request field.
- [x] `SEC-NO-SENSITIVE-OUTPUT-FIELDS` — reject sensitive fields and suspicious
  secret-bearing names in responses, events, errors, logs, metrics, and durable
  product-event messages.

### Phoenix, HEEx, and design-system checks

- [x] `WEB-NO-INFRASTRUCTURE-DEPENDENCIES` — reject Ecto SQL, Postgrex, NATS,
  Git process, repository-storage, artifact-storage, and infrastructure client
  dependencies from the Phoenix application.
- [x] `WEB-RPC-CLIENTS-ONLY-IN-STATE` — reject generated RPC calls and client
  construction outside the supervised client layer and page state/effects
  modules.
- [x] `WEB-NO-HANDWRITTEN-BACKEND-CLIENT` — reject Req, Finch, raw gRPC framing,
  or other hand-written application-backend calls that bypass generated stubs.
- [x] `WEB-NO-RAW-BACKEND-ERROR` — reject `inspect(reason)`, caught
  `error.message`, and direct backend error text in presentation models or
  rendered notices; require the central typed error presenter.
- [x] `WEB-NO-FILESYSTEM-OR-PROCESS` — reject filesystem and subprocess APIs in
  all Phoenix application modules.
- [x] `UI-RAW-HTML-ONLY-IN-COMPONENTS` — reject every raw HTML and SVG tag
  outside the basic design-system component tier.
- [x] `UI-TIER-DIRECTION` — enforce page-to-composite-to-component dependency
  direction and reject upward, cyclic, or implementation-module imports.
- [x] `UI-PAGE-COMPANIONS` — require every live page to have its route adapter,
  state/effects module, page component, and expected tests.
- [x] `UI-LIVE-RENDERS-ONE-PAGE` — require a LiveView render function to invoke
  exactly its page component and prohibit presentation composition in the
  route adapter.
- [x] `UI-STATE-HAS-NO-HEEX` — reject HEEx, design-system imports, and rendering
  concerns from state/effects modules.
- [x] `UI-PAGE-IS-PURE` — reject socket access, LiveView callbacks, RPC clients,
  protobuf messages, domain services, filesystem access, and state mutation in
  page components.
- [x] `UI-DECLARED-INTERACTIONS-ONLY` — require page interaction event names and
  options to be declared as typed/validated dynamic presentation properties
  rather than scattered arbitrary event strings.
- [x] `UI-NO-CLASS-ESCAPE-HATCH` — reject arbitrary public `class` properties
  and page/composite-authored CSS classes.
- [x] `UI-DESIGN-TOKENS-ONLY` — reject literal colors, fonts, radii, shadows,
  and unapproved spacing values outside design-system implementation files.
- [x] `UI-NO-EXTERNAL-UI-IMPORTS` — reject direct imports or use of external UI
  libraries outside design-system basic components.
- [x] `UI-NO-DOM-INJECTION` — reject `innerHTML`, `insertAdjacentHTML`, raw DOM
  creation, and equivalent markup injection outside designated design-system
  hooks.
- [x] `UI-PUBLIC-FACADE-COMPLETE` — require every public component/composite to
  have matching implementation, facade export, bounded attributes/slots, and
  no undeclared implementation export.
- [x] `UI-SHOWCASE-AND-TEST-PARITY` — require every public component/composite to
  have its designated rendering example and accessibility-focused test.
- [x] `UI-PAGE-STATE-COVERAGE` — require render coverage for every state variant
  declared by a page state module.

### Event rules enforced structurally or behaviorally

- [x] `EVT-STATE-AND-EVENT-COMMIT-ATOMICALLY` — make the unit-of-work API and
  PostgreSQL integration tests prove that authoritative state and durable event
  records commit or roll back together.
- [x] `EVT-OUTBOX-PUBLISHER-ONLY` — make module visibility, dependency rules,
  and tests prove that only the outbox publisher can publish product events.
- [x] `EVT-CONSUMER-USES-INBOX` — require state-changing consumers to use the
  durable inbox/deduplication abstraction and prove duplicate delivery safety.
- [x] `EVT-SIDE-EFFECT-AFTER-DURABLE-CLAIM` — prove that event handlers cannot
  perform an external side effect before durable ownership/idempotency claim.
- [x] `EVT-REDUCER-COVERAGE` — require every client-facing event variant to have
  a reducer/projection test and reject an event schema addition without one.
- [x] `EVT-STREAM-REAUTHORIZATION` — prove that subscriptions and deliveries
  reauthorize, terminate on revocation, and disclose no post-revocation event.

## Non-goals

- Rewriting the browser application in React or replacing Phoenix LiveView.
- Reproducing Zustand as a separate Phoenix state process.
- Splitting API and worker roles into separately deployed services during this
  refactor.
- Event-sourcing every domain aggregate.
- Replacing Git smart HTTP with Connect.
- Providing a general-purpose public REST API alongside Connect.
- Exposing internal database schemas through protobuf messages.
- Treating generated protobuf types as domain models inside core services.
- Adding an unrestricted CSS or raw-HTML escape hatch for page authors.
- Preserving current development database contents or internal JSON command
  compatibility.
- Hiding incomplete migration behind a permanent architecture-lint allowlist.
- Expanding unrelated product behavior unless it is required to preserve an
  existing browser workflow through the new boundary.

## Dependencies and affected boundaries

The migration affects at least:

- `crates/hephaestus-app` composition, HTTP serving, authentication mediation,
  internal commands, workers, and runtime startup;
- every domain service currently owning a `PgPool` or embedded SQL;
- PostgreSQL authorization and transaction-context installation;
- forge, release, run, review, secret, volume, workspace, repository, and
  artifact adapters;
- NATS subjects, outbox payloads, consumers, and event publication;
- the Phoenix `Store`, `Repo`, `RunNotifier`, `CommandClient`, repository
  browser, routes, LiveViews, components, tests, and runtime configuration;
- local development startup, state reset, health checks, generated code, and
  watch behavior;
- browser E2E tests and sentinel/non-disclosure checks; and
- contributor documentation, CI, formatting, linting, and architecture checks.

## Implementation checklist

- [x] **Constraint 0 — Specify the target and create the enforcement harness**
  - [x] Add a root `ARCHITECTURE.md` describing the target process topology,
    trust boundaries, Rust layers, Phoenix layers, event semantics, UI tiers,
    prohibited dependencies, and exception policy.
  - [x] Give every normative invariant a stable rule ID and state whether it is
    documentation-only, structurally enforced, linted, or integration-tested.
  - [x] Explain that documentation-only rules are deliberate engineering
    constraints rather than unimplemented lints.
  - [x] Add an architecture-rule index containing rationale, scope,
    enforcement command, valid exceptions, and remediation guidance.
  - [x] Define a narrow exception format requiring rule ID, exact scope,
    rationale, owner, and expiry or tracking task.
  - [x] Reject workspace-wide and directory-wide exceptions unless the
    invariant explicitly defines that scope.
  - [x] Add a stable architecture checker to the development CLI rather than
    relying only on shell `grep` commands.
  - [x] Use `cargo metadata` and filesystem inspection for dependency and path
    rules that do not require compiler internals.
  - [x] Reserve Dylint or another Rust compiler lint for semantic rules that
    cannot be enforced reliably with stable metadata or parsing.
  - [x] Add a custom Mix architecture task for Elixir imports, dependencies,
    module placement, LiveView structure, and HEEx tier rules.
  - [x] Make every architecture diagnostic include its invariant ID and a link
    or path to the relevant `ARCHITECTURE.md` section.
  - [x] Add focused tests for the architecture checkers with valid and invalid
    fixture trees.
  - [x] Add development CLI commands that run architecture, protobuf, Rust,
    Phoenix, UI, and full checks independently.
  - [x] Document the global constraint-by-constraint migration process and the
    rule that no later constraint begins before the active constraint passes
    repository-wide.
  - [x] Gate completion of Constraint 0 on all harness tests passing without
    enabling rules whose migrations have not yet begun.

- [x] **Constraint 1 — Make UI composition entirely design-system-driven**
  - [x] Define the design-system directory and module layout for basic
    components, composites, and the public facade.
  - [x] Move the root document shell, application layout, flash rendering,
    icons, links, typography, controls, forms, tables, lists, dialogs, and
    semantic layout elements into the basic component tier.
  - [x] Ensure basic components are the only modules permitted to emit raw
    lowercase HTML or SVG tags.
  - [x] Define bounded component properties for tone, size, density, spacing,
    alignment, width, state, and interaction instead of exposing arbitrary CSS
    class passthrough.
  - [x] Define composites for every repeated product structure, including
    organization headers, tab navigation, page headings, resource lists,
    empty/error/loading states, secret summaries, build status, release
    provenance, instance summaries, run timelines, and confirmation flows.
  - [x] Require composites to import only the design-system public facade and
    lower-tier composites permitted by an explicit acyclic dependency rule.
  - [x] Migrate every current page so it renders only components and
    composites, including conditional and repeated content.
  - [x] Forbid raw HTML in LiveViews, pages, composites, controllers, layouts,
    and ordinary helper modules.
  - [x] Forbid JavaScript hooks from injecting raw application markup through
    `innerHTML`, `insertAdjacentHTML`, or equivalent APIs outside an explicitly
    designated design-system hook.
  - [x] Prevent pages and composites from importing implementation modules;
    they use only the design-system public facade.
  - [x] Prevent basic components and composites from importing application
    domain, generated protobuf, RPC, state, route, or LiveView modules.
  - [x] Centralize design tokens and reject literal colors, font families,
    radii, shadows, and unapproved spacing values outside the design-system
    implementation.
  - [x] Require a rendering example or showcase entry and accessibility-focused
    component test for every public basic component and composite.
  - [x] Require every page to represent loading, empty, error, reconnecting,
    and ready states when those states are possible.
  - [x] Implement a HEEx-aware checker for raw tags and tier imports; do not use
    a regex that can be bypassed by multiline or dynamic HEEx syntax.
  - [x] Add invalid fixtures covering raw inputs, raw buttons, raw layout tags,
    raw SVG, forbidden imports, upward dependencies, class escape hatches, and
    markup injected by hooks.
  - [x] Remove every repository violation before enabling the `UI-*` checks as
    hard CI failures.
  - [x] Gate completion of Constraint 1 on all existing pages rendering through
    the three tiers and on the browser E2E journey retaining equivalent
    behavior and accessibility.

- [x] **Constraint 2 — Separate every LiveView into state, adapter, and presentation**
  - [x] Define the required file/module convention for each live page: route
    adapter, state/effects module, and page component.
  - [x] Define a page-state struct convention covering initial, loading,
    ready, submitting, error, stale, reconnecting, and access-revoked states.
  - [x] Keep state in LiveView socket assigns; explicitly forbid a page-local
    GenServer or other duplicate Zustand-style runtime.
  - [x] Make state modules own generated-client calls, command effects, stream
    cursors, event reduction, form state, and construction of presentation
    models.
  - [x] Make page modules accept presentation values and named interaction
    events only; forbid socket access, LiveView callbacks, protobuf messages,
    RPC clients, and business/domain services.
  - [x] Make LiveView modules own only Phoenix lifecycle callbacks, effect
    scheduling, stream-task lifecycle, state assignment, and rendering of one
    page component.
  - [x] Define typed or structurally validated interaction properties so pages
    pass event names/options to components in the same role as dynamic callback
    props in the React architecture.
  - [x] Define how forms retain non-sensitive state while never assigning
    plaintext secret values to the socket.
  - [x] Define how Connect stream tasks are supervised, cancelled on LiveView
    termination, resumed from cursors, and prevented from leaking messages to
    a replacement process.
  - [x] Define how genuinely shared streams may be multiplexed through a
    supervised projection process without making Phoenix authoritative.
  - [x] Migrate every existing LiveView, including organization, project,
    repository, release, instance, run, secret, and review pages.
  - [x] Split oversized multi-action LiveViews into route-specific page/state
    modules while retaining stable URLs.
  - [x] Add pure state-reducer tests for every event and command-result
    transition.
  - [x] Add pure page-render tests for all page-state variants.
  - [x] Keep LiveView integration tests focused on callback wiring, navigation,
    effect execution, reconnect, and event delivery.
  - [x] Add architecture checks for required sibling files, allowed imports,
    prohibited callbacks in page modules, and prohibited HEEx in state modules.
  - [x] Remove every repository violation before enabling the LiveView split as
    a hard CI failure.
  - [x] Gate completion of Constraint 2 on all LiveViews following the same
    state/adapter/presentation contract with no grandfathered modules.

- [x] **Constraint 3 — Establish protobuf and Connect as the complete application boundary**
  - [x] Pin a reviewed `connect-rust` release and raise the workspace MSRV to
    its required Rust version.
  - [x] Add shared protobuf source directories with versioned packages and
    ownership documentation.
  - [x] Add Buf configuration for formatting, linting, generation, and breaking
    change detection.
  - [x] Generate Connect Rust server/client code and native gRPC Elixir
    messages/stubs from the same proto definitions.
  - [x] Decide whether generated outputs are checked in; enforce the decision
    with a clean-generation diff check.
  - [x] Define common protobuf types for opaque IDs, pagination, cursors,
    timestamps, field masks where justified, typed diagnostics, and errors.
  - [x] Do not expose free-form maps or JSON payloads where the domain has a
    known shape.
  - [x] Keep secret plaintext fields only in explicitly designated request
    messages and mark them with a custom sensitive-field option.
  - [x] Define domain-oriented services for organizations, projects,
    repositories, builds, releases, agent instances, runs/reviews, secrets,
    repository browsing, and artifact access.
  - [x] Cover every current Phoenix read performed by `Store` or
    `RepositoryBrowser` with an authorized RPC.
  - [x] Cover every current mutation performed by `CommandClient` or direct
    Phoenix SQL with an authorized RPC.
  - [x] Model long-running operations as durable resources returned immediately
    with stable IDs and states rather than blocking RPCs until execution ends.
  - [x] Define list pagination and stable ordering for every collection RPC.
  - [x] Define artifact and repository-file streaming with bounded sizes,
    authorization, content metadata, and cancellation.
  - [x] Implement Connect services as Tower services mounted alongside Git
    smart HTTP and health routes in the existing `hephaestusd` listener.
  - [x] Keep RPC handlers thin: authenticate, convert transport types, invoke
    one application operation, map typed results, and emit transport metadata.
  - [x] Prohibit SQL, NATS publication, filesystem operations, VM operations,
    and domain decision logic inside RPC handlers.
  - [x] Centralize domain-to-Connect error mapping with stable codes and typed
    details; never require Phoenix to parse error strings.
  - [x] Replace the static bearer token plus request-body `actor_id` trust model
    with a short-lived, audience-bound mediator assertion or equivalently
    strong authenticated user context carried in RPC metadata.
  - [x] Ensure RPC request messages cannot select or spoof the authenticated
    actor.
  - [x] Add authorization, validation, idempotency, timeout, cancellation,
    maximum-message-size, and sensitive-error tests for every service.
  - [x] Add Connect/gRPC protocol interoperability and reflection/tooling tests.
  - [x] Replace and delete `/internal/v1/commands` only after every caller has
    migrated.
  - [x] Remove every non-Git application JSON endpoint before enabling the
    `RPC-*` transport-boundary checks.
  - [x] Gate completion of Constraint 3 on protobuf compatibility checks,
    generated-code checks, Rust/Elixir interoperability tests, and parity for
    every existing UI operation.

- [x] **Constraint 4 — Make typed durable events the live product interface**
  - [x] Define the distinction between internal domain events, durable
    application events, and client-facing product events in
    `ARCHITECTURE.md`.
  - [x] Define a standard protobuf event envelope containing event ID, cursor,
    scope, aggregate type and ID, aggregate version, occurrence time, actor and
    request provenance where safe, schema version, and typed `oneof` payload.
  - [x] Add an append-only durable application-event log with a monotonic
    resume cursor and explicit retention semantics.
  - [x] Ensure state changes and their durable event records commit in the same
    PostgreSQL transaction.
  - [x] Make outbox publication originate only from committed durable event
    records.
  - [x] Prohibit direct NATS event publication from application operations and
    adapters other than the designated outbox publisher.
  - [x] Define event-scope ordering precisely and include aggregate versions so
    consumers can detect gaps or impossible reordering.
  - [x] Define at-least-once delivery, stable event IDs, deduplication, retry,
    poison-message, and retention-gap behavior.
  - [x] Require idempotent consumers or a shared durable inbox/deduplication
    abstraction for every state-changing event consumer.
  - [x] Define scoped watch RPCs instead of one unbounded global event stream.
  - [x] Implement initial snapshot plus resumable stream behavior without a
    race between snapshot creation and event subscription.
  - [x] Let commands return the committed aggregate version and/or event cursor
    required for read-your-writes UI behavior.
  - [x] Reauthorize stream creation and delivery; terminate safely when access
    is revoked.
  - [x] Keep high-volume logs, metrics, and artifact bytes on separate bounded
    streaming RPCs rather than embedding them in ordinary product events.
  - [x] Exclude secret values, credentials, tokens, sensitive parameters, raw
    command environments, and unbounded diagnostics from all event payloads.
  - [x] Add protobuf descriptor checks that reject sensitive response/event
    fields and require sensitive annotations on permitted request fields.
  - [x] Add reconnect tests covering exact resume, duplicates, disconnects,
    server restarts, cursor expiry, authorization revocation, and concurrent
    aggregate changes.
  - [x] Add consumer tests proving duplicate event delivery causes no duplicate
    state transition or external side effect.
  - [x] Migrate existing JSON NATS payloads when necessary so durable contracts
    are versioned and typed rather than reconstructed independently by each
    consumer.
  - [x] Remove every PostgreSQL-notification and polling-based live-update path
    after its typed stream replacement is verified.
  - [x] Enable `EVT-*` checks only after every state-changing application path
    uses the durable event discipline.
  - [x] Gate completion of Constraint 4 on event replay/reconnect integration
    tests and the browser E2E journey running entirely from RPC snapshots and
    event streams.

- [x] **Constraint 5 — Isolate Phoenix completely from infrastructure and domain authority**
  - [x] Introduce a supervised Phoenix RPC client/channel layer with generated
    clients, deadlines, authentication metadata, retries limited to safe
    operations, and typed error mapping.
  - [x] Migrate all state modules to Connect queries, commands, and event
    streams.
  - [x] Replace `HephaestusWeb.Store` reads and direct control mutations with
    generated RPC calls.
  - [x] Replace `HephaestusWeb.CommandClient` and its untyped maps with generated
    requests and responses.
  - [x] Replace `HephaestusWeb.RunNotifier` PostgreSQL notifications with typed
    Connect stream subscriptions.
  - [x] Replace direct bare-repository browsing with bounded repository RPCs.
  - [x] Replace direct artifact reads with authorized streaming RPCs.
  - [x] Delete the Phoenix Repo, Store, DataCase, SQL helpers, notification
    process, repository browser, and infrastructure-specific test support once
    unused.
  - [x] Remove Ecto SQL, Postgrex, PostgreSQL URL, NATS, Git CLI, and direct
    storage dependencies from the Phoenix application.
  - [x] Remove database credentials, repository mounts, and artifact mounts from
    the Phoenix container and local development configuration.
  - [x] Ensure Phoenix can start, render its login route, authenticate, and run
    mocked page tests with no PostgreSQL, NATS, repository root, or artifact
    root available.
  - [x] Document Phoenix responsibilities that remain judgment-based: page
    orchestration, presentation derivation, immediate form usability, session
    lifecycle, and transport-error presentation.
  - [x] Lint objective consequences: forbidden dependencies, imports, modules,
    environment variables, SQL fragments, filesystem APIs, subprocess APIs,
    and non-generated backend clients.
  - [x] Do not attempt to lint subjective statements such as “too much
    application logic”; enforce the concrete ownership consequences instead.
  - [x] Add negative architecture fixtures for Ecto imports, Postgrex imports,
    SQL, filesystem reads, `System.cmd`, NATS clients, and hand-written HTTP
    application calls.
  - [x] Remove every repository violation before enabling `WEB-*` checks as
    hard failures.
  - [x] Gate completion of Constraint 5 on a development and browser E2E stack
    where Phoenix receives only browser/OIDC and Rust RPC configuration.

- [x] **Constraint 6 — Isolate all Rust SQL in dedicated PostgreSQL adapters**
  - [x] Inventory every `sqlx` dependency, `PgPool`, transaction, query macro,
    SQL string, database row type, RLS-context call, and migration-sensitive
    assumption in the Rust workspace.
  - [x] Define application-facing repository and unit-of-work ports by bounded
    context without leaking SQLx types, PostgreSQL errors, or row structures.
  - [x] Create or rename dedicated PostgreSQL adapter crates for forge,
    identity, authorization, release, review, run, secret, event, volume, and
    any other persistent bounded context.
  - [x] Split mixed filesystem/PostgreSQL adapters where necessary so SQL
    capability belongs to an explicitly designated crate.
  - [x] Move row decoding, queries, RLS actor-context installation, transaction
    implementation, optimistic concurrency, and persistence error mapping into
    PostgreSQL adapters.
  - [x] Preserve atomic cross-record operations through explicit unit-of-work
    ports rather than reintroducing database handles into application services.
  - [x] Preserve transactional state-plus-event commits required by `EVT-*`.
  - [x] Remove `sqlx`, `PgPool`, `Transaction`, PostgreSQL row types, and SQL
    strings from domain, application, transport, worker, and composition code.
  - [x] Require static SQL or compile-time checked SQL in adapter crates; reject
    dynamic query-string interpolation unless a narrowly documented query
    builder is required.
  - [x] Keep migration SQL solely under the root migration ownership boundary
    and document which adapters depend on each schema area.
  - [x] Mark SQL-capable crates explicitly in workspace metadata and make the
    architecture checker derive its allowlist from that declaration rather
    than hard-coded path guesses.
  - [x] Reject transitive `sqlx` dependencies from non-adapter crates when
    feasible and reject direct dependencies unconditionally.
  - [x] Add adapter contract tests reusable across in-memory/fake and
    PostgreSQL implementations where useful.
  - [x] Run all existing PostgreSQL integration, authorization, RLS,
    idempotency, outbox, recovery, and concurrency tests against the extracted
    adapters.
  - [x] Remove every repository violation before enabling `DB-*` checks as hard
    failures.
  - [x] Gate completion of Constraint 6 on `rg`, Cargo metadata, and semantic
    lint evidence that no SQL or SQLx API remains outside designated adapter
    crates and migrations.

  The composition crate retains SQLx only as an explicitly declared
  `dev-dependency` for database-backed test fixtures; production application
  code has no SQLx capability. The architecture checker requires the narrow
  `hephaestus.sqlx_test_dependency = true` metadata marker for this case.

- [x] **Constraint 7 — Enforce the complete Rust layer graph and boundary safety**
  - [x] Assign every workspace crate a declared architecture layer and bounded
    context in Cargo package metadata.
  - [x] Define allowed dependency edges between domain, application, port,
    adapter, transport, worker, and composition layers.
  - [x] Reject cycles, upward imports, undeclared cross-context adapter access,
    and composition-root logic leaking into reusable crates.
  - [x] Keep Connect, Axum, and transport errors in transport/composition
    modules only.
  - [x] Keep NATS client types in event adapters and worker composition only.
  - [x] Keep environment-variable access in explicit configuration modules.
  - [x] Keep filesystem repository, artifact, workspace, volume, and runtime
    operations in their designated adapters.
  - [x] Keep VM provider implementations behind `vm-trait` and prohibit
    application services from importing provider-specific crates.
  - [x] Enforce VM provider imports with `ARCH-VM-PROVIDER-ONLY-IN-COMPOSITION`
    and a checked invalid architecture fixture.
  - [x] Ensure domain crates remain free from async runtime, transport,
    persistence, serialization-format, and operating-system dependencies unless
    a documented invariant requires one.
  - [x] Centralize conversion between protobuf, domain, persistence, and event
    representations at their corresponding boundaries.
  - [x] Add custom semantic lints for transport-error construction, direct
    environment reads, forbidden client construction, sensitive formatting,
    and other rules that metadata cannot prove.
  - [x] Add valid and invalid crate-graph fixtures and actionable diagnostics.
  - [x] Remove every repository violation before enabling `ARCH-*` layer checks
    as hard failures.
  - [x] Gate completion of Constraint 7 on a repository-wide architecture graph
    report with no undeclared or forbidden edge.

- [x] **Constraint 8 — Enforce authentication, authorization, and secret safety at the new boundary**
  - [x] Define the Phoenix-to-Rust authentication flow, token/assertion issuer,
    audience, signing, expiry, replay resistance, rotation, and local
    development behavior in `ARCHITECTURE.md`.
  - [x] Install authenticated identity through a Connect/Tower interceptor and
    make application operations receive identity from trusted context.
  - [x] Prohibit actor identity fields in ordinary RPC request messages.
  - [x] Reauthorize every query, command, stream subscription, streamed event,
    repository read, and artifact read at the Rust boundary.
  - [x] Preserve PostgreSQL RLS context as defense in depth inside adapters,
    without making Phoenix responsible for setting it.
  - [x] Define a protobuf sensitive-field option and require it for every
    plaintext secret, credential, token, or sensitive parameter request field.
  - [x] Reject sensitive fields in response messages, product events, durable
    event payloads, logs, metrics, diagnostics, and error details.
  - [x] Prevent sensitive domain and generated request types from unrestricted
    debug/display formatting where the selected code generator permits custom
    policy.
  - [x] Keep secret form values transient in LiveView event handling and RPC
    encoding; never store them in socket assigns or presentation models (the
    LiveView architecture checker and sensitive-field tests enforce this).
  - [x] Extend sentinel tests across Phoenix logs, Rust logs, PostgreSQL event
    records, NATS storage, protobuf errors, generated debug output, artifacts,
    repositories, runtime directories, and browser content.
  - [x] Add stream-revocation tests proving a connected client stops receiving
    data immediately after losing access (`durable_watch_resumes_across_disconnect_gap_duplicate_wake_and_revocation`).
  - [x] Add mediator assertion forgery, expiry, audience, replay, actor-spoofing,
    and request-provenance tests (`rpc::auth` mediator suite).
  - [x] Remove every repository violation before enabling `SEC-*` checks as
    hard failures; the SEC rules are hard-enabled in the architecture
    configuration.
  - [x] Gate completion of Constraint 8 on the full authorization suite and
    sentinel scan passing through the Connect/event architecture.

- [x] **Constraint 9 — Remove obsolete architecture and make the target the only supported path**
  - [x] Delete superseded Phoenix SQL, filesystem, notification, and untyped
    command modules rather than retaining dormant compatibility paths; the
    runtime inventory confirms none remain as modules or supervised children.
  - [x] Delete superseded internal JSON request/response types and routes; the
    unused `:api` pipeline and commented `/api` scope were removed from the
    Phoenix router, and no active JSON command route remains.
  - [x] Delete duplicate product-event JSON payload builders and consumers
    after protobuf event migration; the inventory contains only generated
    protobuf product-event messages and the typed `RPC.ProductEvents` adapter.
    The superseded secret lifecycle JSON builders were removed. JSON remains
    intentionally at the durable-adapter boundary for the release-owned
    outbox and actionable internal command envelopes; those are not duplicate
    product-event consumers.
  - [x] Remove obsolete environment variables, container mounts, ports,
    dependencies, tests, fixtures, documentation, and development commands;
    the local-smoke, browser-E2E, daemon, and Phoenix configuration inventory
    found no stale entries (all exported variables are consumed by the current
    composition root or its explicitly documented fixture runner).
  - [x] Update local fixture seeding so seeded entities are valid through the
    new persistence/event contracts and do not bypass invariants that the
    fixture intends to demonstrate.
  - [x] Prefer exercising public application operations for fixture creation;
    document any trusted fixture-only bootstrap boundary that remains (the
    deterministic E2E seed uses forge `*_trusted` operations and documents its
    narrow SQL-only fixture catalog boundary in `docs/application.md`).
  - [x] Update `cargo dev` build, watch, restart, status, logs, state reset, and
    doctor behavior for protobuf generation and Connect health.
  - [x] Add a single repository quality command that runs formatting, generated
    code checks, Buf checks, architecture checks, Rust checks, Phoenix checks,
    component/page checks, and focused integration tests.
  - [x] Update contributor documentation with common architecture violations
    and their intended remediations.
  - [x] Audit README and product documentation for claims about UI/API/build
    capabilities that no longer match the implementation.
  - [x] Gate completion of Constraint 9 on no obsolete module, dependency,
    route, configuration key, or container permission remaining; static
    inventories of the Rust workspace, Phoenix router, daemon configuration,
    local/browser runners, and container mounts found no unsupported target
    path (the documented release and command JSON adapters are intentional).

- [ ] **Final verification — BLOCKED by the restart/reconnect follow-up below**
  - [x] Run `buf format` in check mode.
  - [x] Run `buf lint`.
  - [x] Run the configured Buf breaking-change check against the accepted
    contract baseline.
  - [x] Regenerate Rust and Elixir bindings and prove the worktree remains
    clean.
  - [x] Run the complete architecture checker and record every enabled rule
    family.
  - [x] Run `cargo fmt --all -- --check`.
  - [x] Run `cargo clippy --workspace --all-targets --all-features`.
  - [x] Run `cargo test --workspace --all-features`.
  - [x] Run `cargo doc --workspace --all-features --no-deps`.
  - [x] Run Phoenix formatting in check mode.
  - [x] Run the complete Phoenix test suite.
  - [x] Run design-system component, composite, page-render, and architecture
    fixture tests.
  - [x] Run Connect/gRPC interoperability, authentication, authorization,
    timeout, cancellation, and maximum-message tests.
  - [x] Run durable event ordering, duplication, resume, reconnect, retention,
    revocation, and restart tests.
  - [x] Run all PostgreSQL adapter, RLS, outbox, idempotency, concurrency, and
    recovery integration tests.
  - [x] Run the browser E2E journey with Phoenix denied PostgreSQL, NATS,
    repository-root, and artifact-root access. The authorized secret metadata
    adapters now provide bounded organization/project secret, grant, and
    authority pages; the isolated journey passes all four Playwright tests.
  - [x] Run the real libkrun build/run integration suite through the new API and
    event contracts. Phase 1B run/update persistence and the daemon golden
    build/run path both pass; the latter now aliases the launch-contract
    `release_state` column and no longer emits the retired build-completed
    internal signal.
  - [x] Run the complete secret sentinel/non-disclosure scan.
  - [ ] Stop and restart the complete development environment, reconnect an
    active browser stream from its cursor, and verify no duplicate visible
    transition or side effect. **BLOCKED:** the isolated namespace support is
    now available, but no restart/reconnect harness exists yet. Follow-up:
    [verify browser reconnect and restart semantics](verify-browser-reconnect-and-restart.md).
  - [x] Verify all documented development commands from a clean state. An
    isolated `HEPHAESTUS_LOCAL_ROOT` and `HEPHAESTUS_LOCAL_NAMESPACE` were
    cleaned, initialized, listed, diagnosed, and cleaned again successfully;
    the namespaced PostgreSQL volume/container and rootfs cleanup completed
    without touching the populated local environment.

## Completion evidence

Record before moving this task to `tasks/done/`:

- the final `ARCHITECTURE.md` invariant IDs and enforcement commands;
- the pinned Connect/Buf/protobuf tool versions and workspace MSRV;
- the generated-code clean-check output;
- the final Rust crate-layer graph;
- proof that Phoenix has no database, NATS, repository, or artifact access;
- proof that raw HTML exists only in the basic component tier;
- proof that SQL/SQLx exists only in designated PostgreSQL adapter crates and
  migrations;
- protobuf compatibility results;
- event replay, reconnect, duplicate-delivery, retention-gap, and revocation
  test results;
- Rust formatting, Clippy, test, and documentation results;
- Phoenix formatting and test results;
- browser and libkrun E2E results;
- development-environment command audit: `cargo dev doctor`, `status`,
  `logs`, `state list`, `cache list`, and all subcommand `--help` invocations
  completed successfully. The
  restart/reconnect proof remains deferred to
  [verify browser reconnect and restart semantics](verify-browser-reconnect-and-restart.md)
  because the current local state is populated. The real libkrun
  Phase 1B and daemon golden suites pass, including result persistence and
  cgroup cleanup. The browser E2E now
  completes OIDC callback and the denied-access isolation checks, and all four
  Playwright tests pass. The clean-state command proof uses a disposable
  namespace and local root, leaving the populated developer state unchanged.
- sentinel/non-disclosure results; and
- any deliberately deferred work split into independently deliverable todo
  tasks with links from this document.
