# Define the “own the loop” agent platform and product

Owner: unassigned

## Outcome

Define, validate, and document Hephaestus as a secure platform for installing
and operating autonomous agents as versioned software.

Hephaestus originates in the requirements of coding agents working on complex
projects, but it is not limited to a built-in coding-agent harness. A released
agent is an ordinary program that owns its model loop, state schema, protocol
adapters, application policy, and user experience. Hephaestus provides the
privileged substrate: builds, releases, instances, isolation, state, ingress,
capabilities, credential brokering, controlled side effects, and exact audit
provenance.

This task turns that thesis into:

- a stable product definition and vocabulary;
- explicit platform-versus-agent responsibility boundaries;
- an agent-principal, capability, and runtime-token model integrated with the
  existing OpenFGA/Mélange and PostgreSQL RLS design;
- gateway, credential, state, distribution, packaged-agent, and catalog
  architecture decisions;
- a golden cooking-agent scenario that exercises the product end to end;
- positioning for technically acute users and organizations delegating
  valuable work to agents; and
- smaller, sequenced implementation tasks rather than one unbounded epic.

This is a definition, validation, and decomposition task. It should not absorb
all implementation into this file. Once a boundary is accepted, create a
focused task in `tasks/todo/` with independently verifiable acceptance
criteria.

## Product thesis

### Motto

> **Own the loop. Authority remains outside the loop.**

“Own the loop” means that agent authors can implement the behavior directly,
using plain code or libraries such as XState, LangGraph, model SDKs, and
protocol-specific adapters. Hephaestus does not require a universal prompt
format, conversation model, memory system, skills framework, or orchestration
DSL.

“Authority remains outside the loop” means that untrusted released code cannot
grant itself access, read raw platform credentials by default, mutate
canonical resources directly, bypass authorization, or silently broaden its
capabilities. Hephaestus authenticates, authorizes, constrains, mediates, and
audits privileged operations.

### Product category

OpenClaw and Hermes are useful product comparisons, but Hephaestus should not
win by accumulating their complete feature lists in the core platform.

The intended distinction is:

```text
OpenClaw
→ personal agent gateway with channels, tools, skills, and memory

Hermes
→ personal/self-improving agent harness with terminal access, memory,
  delegation, skills, scheduling, and messaging

Hephaestus
→ forge and governed runtime where autonomous agents are reusable,
  versioned, installed, isolated, stateful software
```

The initial target is not the least technical personal-assistant user. It is a
technically acute individual or organization that understands the value of
explicit workflows, exact provenance, scoped authority, native execution,
controlled publication, and deliberate software updates.

Complex coding projects remain the reference workload because they exercise
nearly every hard requirement: native toolchains, large repositories,
persistent knowledge, long tasks, dangerous side effects, credentials,
collaboration, reproducibility, and human review.

### Product promise

Hephaestus should let a team:

> Run autonomous agents continuously, let them learn and operate real tools,
> and still treat them as versioned software installed into governed
> capability boundaries.

The platform should make a small bespoke agent easier to understand and govern
than a large one-size-fits-all agent framework. Models can generate or adapt
the application glue; Hephaestus must make the generated program safe to
build, install, expose, authorize, run, update, inspect, and remove.

## Responsibility boundary

### Hephaestus owns

- exact source materialization, isolated builds, and immutable releases;
- project-owned installations and immutable instance revisions;
- microVM, isolate, process, resource, mount, and network boundaries;
- persistent state volumes, exclusive leases, generations, and recovery;
- generic ingress, durable event delivery, timers, and service routing;
- agent and user authentication;
- OpenFGA/Mélange authorization and PostgreSQL RLS enforcement;
- short-lived runtime credentials and capability attenuation;
- credential storage, egress policy, and brokered credential use;
- canonical Git, release, artifact, and other privileged publication;
- lifecycle, update, pause, recovery, retention, and cleanup coordination;
- logs, metrics, health, generic structured events, audit, and provenance; and
- curated distribution and release provenance.

### Released agents own

- their model loop and model-selection strategy;
- prompts, context construction, compression, and reasoning policy;
- application-level conversations and sessions;
- their memory and state schema;
- Telegram, Slack, webhook, browser, or other protocol semantics;
- application-level user policy, such as which family members may use a bot;
- schedules and business rules;
- tool selection, orchestration, and optional delegation;
- self-improvement and skill semantics;
- their HTTP and WebSocket application behavior; and
- their end-user experience.

### Core placement test

Use this test when deciding whether a feature belongs in Hephaestus:

> If the behavior can safely live inside a released program, it belongs to the
> agent. If it requires host privilege, cross-tenant correctness, durable
> resource ownership, or enforcement against compromised agent code, it
> belongs to Hephaestus.

## Working architecture

### Layered runtime

```text
                             management plane

Human ──────────────→ Operator/Admin Agent
                              │
                       typed, audited requests
                              │
                              ▼
                    Hephaestus control API


                               project plane

Internet → shared Caddy → GatewayDispatcher → gateway HTTP handler
                                                runtime
                                                   │
                                           protocol-specific
                                            released code
```

The public gateway and project agent are separate principals. Compromise of a
public protocol parser must not automatically grant repository, project-state,
or management authority.

### Gateways

Hephaestus should provide repository-declared gateway workloads alongside
agents. A gateway maps bounded public routes to a released stateless HTTP
request handler. Its canonical contract is a bounded HTTP request to a bounded
HTTP response: application code owns protocol parsing, validation, and return
semantics, while Hephaestus owns route authority, revision selection,
invocation, and audit.

A gateway may receive bounded public HTTP traffic, validate a protocol
signature or application token, return an application-specific HTTP response,
normalize an event, publish to explicitly bound agent inboxes, and use narrowly
scoped capabilities. It must not receive a project state volume, repository
mount, organization-wide token, Caddy administration, or implicit agent
authority. Streaming, WebSockets, and long-lived service processes are later
contracts, not gateway MVP behavior.

### Caddy integration boundary

Caddy is the shared public HTTP edge, not the source of truth for gateways or
an agent-aware control plane. Platform-owned routes (UI, API, registry, and
health endpoints) remain distinct from the reserved `/gateway/` namespace for
repository-declared routes. Hephaestus validates gateway route conflicts and
authorization, then reconciles derived Caddy configuration. Agents never
receive Caddy administration access; Caddy configuration is reconstructible
deployment state and authoritative gateway records remain in PostgreSQL.

Caddy terminates HTTPS and forwards `/gateway/` requests to a stable host-side
GatewayDispatcher. The dispatcher discards untrusted forwarding headers,
applies limits, resolves the exact revision, invokes the gateway VM over
private HTTP, and relays its bounded response. Caddy does not need VM
addresses, leases, snapshots, or runtime credentials.

A host-side `GatewayProvider` adapter reconciles routes and translates
provider-specific request/response representations to/from this canonical HTTP
contract. The MVP implements only the shared-Caddy adapter. It has no gateway
certificate, domain verification, issuance, renewal, or user-configurable TLS
policy: gateway routes use the preconfigured listener and hostname. Project
code may implement application authentication such as a Telegram signature,
but platform authorization to expose a route remains outside that code.

### Unikraft as an optional instant-service runtime

[Unikraft](https://unikraft.org/) is a promising optional backend for
gateways and compact packaged agents. Its single-application VM model,
millisecond-scale cold starts, small immutable images, KVM isolation, and
support for ordinary ELF or OCI-packaged programs align with “own the loop”:
users still write a program rather than targeting a Hephaestus-specific agent
framework or function interface.

The relevant product roles are:

| Workload | Expected fit |
| --- | --- |
| Public webhook or protocol gateway | High |
| Compact, always-addressable service agent | Medium to high |
| Curated agent in a beginner distribution | Medium, after compatibility validation |
| Arbitrary project or coding agent | Low |
| Release build environment | Low |

Unikraft must not replace the general Linux microVM runtime. Complex coding
agents depend on shells, subprocesses, package managers, browsers, dynamic
libraries, and a long tail of Linux behavior. Unikraft's Linux ABI
compatibility is substantial but incomplete, and its single-address-space
design makes the hypervisor, rather than privilege separation inside the
guest, the principal security boundary. The initial runtime model should
therefore remain:

```text
V8 or WebAssembly
→ smallest deterministic adapters, if their compatibility and isolation win

Unikraft
→ reviewed packaged gateways and compact service agents

general Linux microVM
→ arbitrary coding agents and tool-heavy project workloads
```

The product must also distinguish the open-source Unikraft and KraftKit
runtime/build stack from Unikraft Cloud. Network-triggered wake-up, request
buffering, stateful scale-to-zero, and managed routing are Cloud product
features; they cannot be assumed to exist in a self-hosted Hephaestus
deployment. Hephaestus would need to provide equivalent routing, lifecycle,
snapshot, and recovery control or deliberately depend on that service.

Memory snapshots are an optimization, not canonical agent state. Durable
state remains in the instance's declared state volumes and external systems.
After snapshot resume, Hephaestus must revalidate the exact release, instance
revision, lease, and live authorization; renew or replace expired runtime
credentials; and re-establish broker and network channels. Snapshot and
volume consistency, especially for SQLite WAL and `fsync`, requires explicit
crash testing before stateful resume is accepted.

Expose this initially as an implementation detail or advanced runtime profile,
not as a beginner-facing product concept. A release may select or constrain a
runtime profile, but Hephaestus must not transparently move software between
Linux, Unikraft, and isolate backends until their behavior is proven
equivalent. The most useful first proof is a small Rust webhook gateway with a
read-only release, writable SQLite state, durable inbox publication, default
deny egress, and brokered credentials.

### Durable delivery

Hephaestus should understand a generic ingress envelope, not Telegram or any
other application protocol:

```text
IngressEventId
GatewayId
AgentInstanceId or inbox binding
received timestamp
bounded method, route, headers, and body reference
deduplication key
delivery attempts
disposition
response or outbound-delivery reference
```

The platform owns durable inbox/outbox and at-least-once delivery mechanics.
The released program owns the application interpretation and idempotency
policy.

### Agent-owned state and exact provenance

Persistent SQLite or other files in an instance volume provide durable memory,
but not arbitrary process checkpointing. A sleeping agent restarts its process
and reconstructs ephemeral objects from durable state.

Because memory, prompts, and agent-authored skills may affect behavior, MVP run
provenance includes:

```text
immutable release
+ immutable instance revision
+ exact state volume and fenced lease
+ serialized instance dispatch order
+ terminal state-access outcome
+ authorization snapshot
+ trigger and target
```

This evidence does not claim which application-owned bytes changed. Numbered
state generations, integrity evidence, checkpoints, backups, and reproducible
snapshots remain separate deferred concepts.

### Self-improvement

An agent may keep private mutable learning in instance state. When learning
should become installed software, the preferred promotion path is:

```text
experience
→ proposed source or skill change
→ isolated build and tests
→ immutable release
→ reviewed instance update
```

Hephaestus should not require this promotion for every memory mutation, but it
must make the boundary visible. Agent-authored code that is installed globally
or shared through a catalog is software and must use the release pipeline.

## Agent capabilities and Hephaestus tokens

### Authorization authority

OpenFGA semantics, Mélange-generated PostgreSQL functions, domain-derived
`melange_tuples`, and PostgreSQL RLS remain the authorization authority.
Runtime tokens must not create a parallel permission system.

```text
token
→ authenticates the caller and identifies the run/session

token capability ceiling
→ prevents an old or narrow runtime from acquiring newly granted authority

Mélange/OpenFGA
→ decides whether the effective principal may perform the operation now

RLS
→ independently constrains data access in the same transaction
```

### Capability lifecycle

```text
release declares symbolic capability requirements
→ instance revision binds requirements to concrete resources
→ authorized domain grants become Mélange tuples
→ run snapshots its maximum binding set
→ Hephaestus mints a short-lived runtime credential
→ every privileged API call checks ceiling and live Mélange permission
```

Release declarations grant nothing. An authorized import, configuration, or
binding command creates the authoritative domain relationship.

Example release declaration:

```toml
[capabilities.blog]
kind = "repository"
permissions = ["read_source", "propose_change"]

[capabilities.cooking_inbox]
kind = "mailbox"
permissions = ["publish"]

[secrets.telegram_api]
delivery = "brokered"
destination = "https://api.telegram.org"
```

The consuming project resolves symbolic slots to concrete resources.

### Authorization subjects

At minimum, the canonical model needs distinct subject types for:

- authenticated users;
- agent instances;
- public gateway instances or releases, if gateways call Hephaestus APIs; and
- optionally runtime/delegation objects when persisted relationship modeling
  provides value.

An agent instance must not inherit all project-maintainer authority merely
because it belongs to a project. Resource access is granted explicitly through
capability bindings.

### Runtime credentials

Start with opaque, random, server-side runtime credentials rather than
self-contained JWT permission claims. Store only a token hash and bind the
credential to:

- exact agent instance;
- exact instance revision;
- exact run or service session;
- immutable authorization snapshot;
- maximum capability-binding IDs;
- issued and expiry times;
- revocation and rotation state; and
- optional delegation or one-shot approval.

Queued work must not contain a previously minted bearer token. Reauthorize and
mint at dispatch. Long-running services need renewable credentials whose
renewal depends on the live service lease and current authorization.

Every call performs:

1. runtime credential authentication;
2. run/session/revision/lease validation;
3. token-ceiling validation for the exact action and resource;
4. live Mélange permission evaluation;
5. RLS-constrained execution; and
6. structured audit.

### PostgreSQL subject context

Generalize the current user-only transaction context to include:

```text
hephaestus.subject_type
hephaestus.subject_id
hephaestus.request_id
hephaestus.run_or_session_id
hephaestus.mediator_agent_id
hephaestus.delegation_id
```

Agent-facing APIs must use a non-`BYPASSRLS` role. Trusted workers may perform
mechanical work after a durable authorized command, but a runtime credential
must never grant access to worker-role privileges.

### Autonomous and delegated authority

Autonomous triggers use durable grants belonging to the agent instance.

Interactive management through the Operator Agent normally uses the
interacting human as the effective principal, with the agent recorded as
mediator and constrained by a short-lived delegation ceiling.

Sensitive operations should use one-shot approvals bound to the exact target,
action, command ID, and expiration. No agent may grant itself broader
permissions.

The audit chain must distinguish:

```text
requested_by_user
mediated_by_agent_instance
executed_by_run_or_session
approved_by
permission and object checked
authorization model version
decision and command result
```

### Semantic permissions

Prefer permissions matching controlled Hephaestus operations over broad
filesystem-like verbs:

```text
repository.read_source
repository.request_run
repository.propose_change
repository.publish_approved_result
repository.manage_attachments

gateway.receive_events
gateway.send_responses
gateway.deploy_release
gateway.configure_route
gateway.bind_domain
gateway.manage_credentials

agent_instance.inspect
agent_instance.configure
agent_instance.update
agent_instance.pause
agent_instance.recover

mailbox.publish
secret.use
schedule.manage
authorization.grant_agent_access
```

`repository.propose_change` does not expose canonical Git write access.
`secret.use` does not imply permission to read the secret value.
`agent_instance.configure` does not imply permission to alter authorization.

Mounted resources require authorization before construction and are frozen
into the run snapshot. Revoking access may require cancellation and lease
withdrawal because revoking an API token cannot remove an existing mount.

## Credentials and egress

### Brokered credential direction

Hephaestus should support a host-side credential broker or egress proxy:

```text
guest sends placeholder or capability reference
→ host verifies runtime identity, destination, and policy
→ host injects the real credential outside the guest
→ host forwards and audits the request
```

For HTTPS this requires either a controlled TLS-intercepting proxy with a
guest-trusted proxy CA or an application-level capability endpoint. The guest
must be unable to bypass the proxy through alternate DNS, raw IP, metadata,
IPv6, tunneling, or another network device.

This prevents direct credential disclosure but does not neutralize the
credential’s authority. An agent allowed to call GitHub may still publish
sensitive data, consume quota, or perform any operation allowed by the
upstream token. High-value operations therefore need narrowly scoped upstream
credentials or semantic Hephaestus capabilities.

The architecture should support an explicit safety ladder:

1. raw secret delivery for deliberately trusted workloads;
2. destination-bound proxy substitution;
3. first-class resource operations such as scoped Git, mailbox, SQLite, or
   object-store access, where Hephaestus governs the primitive but never
   interprets the application's protocol or policy.

Model and provider APIs use the second level in this MVP: released code owns
the protocol, retries, and spend decisions, while Hephaestus enforces only the
declared destination and secret-substitution binding.

## Distributions and packaged agents

### One core, several distributions

Distributions should be bootstrap profiles and curated release sets over one
core platform, not divergent forks with separate migrations and security
patches.

#### Raw

- no LLM or preinstalled agent;
- control API, CLI, releases, runtime, ingress, state, and authorization;
- suitable for users bringing fully bespoke released programs.

#### Advanced

- versioned Hephaestus coding skill;
- API and runtime-contract documentation;
- starter repositories and small optional adapters;
- conformance tests and `heph doctor`;
- examples using direct loops, XState, LangGraph, and common model SDKs.

#### Beginner

- signed and pinned Operator/Admin Agent;
- signed and pinned Project Agent;
- opinionated memory and model configuration;
- web search and selected useful capabilities;
- gateway, schedule, and project-management UX;
- a curated catalog; and
- an ejectable implementation that users can inspect, fork, and replace.

Libraries and templates are ordinary released source dependencies, not
privileged framework extensions. Hephaestus must continue to work when an
agent ignores them.

### Operator/Admin Agent

The packaged Operator Agent is the management UX. It may expose a path to
every management operation but must not possess ambient, permanent,
organization-wide authority.

It:

- calls only typed Hephaestus APIs;
- acts using the current user’s checked and attenuated delegation;
- proposes grants but cannot authorize its own proposal;
- cannot access PostgreSQL, the host shell, or master credentials directly;
- cannot bypass RLS or authorization;
- uses one-shot approval for sensitive actions; and
- keeps management-plane context separate from untrusted project content.

A human-controlled break-glass path remains separate from all LLM agents.

### Project Agent

The packaged Project Agent provides project-scoped continuity and may manage
project workflows, repositories, installed agents, schedules, gateways, and
memory through explicit grants.

“Can do anything in the project” means it can request the project operations
its policy permits. It does not imply raw access to all project secrets or
permission to grant itself authority, delete the project, broaden network
access, publish externally, or update itself without the required review.

### Curated catalog

The beginner trust path uses a curated catalog rather than arbitrary
agent/skill installation. “Curated” must have technical meaning:

- reviewed or reviewable source;
- isolated builds and immutable signed releases;
- publisher identity and provenance;
- dependency and artifact manifests;
- declared capabilities, resources, egress, ingress, and secrets;
- automated conformance and security tests;
- version pinning and explicit update review;
- visible permission diffs;
- revocation without historical erasure; and
- no implicit runtime download of unreviewed executable dependencies.

Raw and advanced installations may allow explicit untrusted imports behind a
clear trust boundary. Organizations should be able to operate private
allowlisted catalogs.

## Golden product scenario: family cooking agent

Use a cooking agent as the cross-cutting demonstration.

The agent:

- communicates with two authorized family members through Telegram;
- implements its Telegram gateway and user mapping as ordinary released code;
- owns a persistent recipe memory;
- owns a Git repository containing a generated static cooking blog;
- proposes or publishes controlled blog updates;
- is served publicly through Caddy;
- can use a model and optional web search without reading provider secrets;
- can be inspected, updated, paused, and recovered;
- retains exact release, state-volume, fenced-lease, dispatch-order,
  authorization, trigger, and result provenance; and
- demonstrates that protocol behavior can remain bespoke and small without
  becoming a Hephaestus core feature.

Expected authority:

```text
Telegram gateway
  may receive bounded public requests
  may enqueue only to the cooking-agent inbox

Cooking agent
  may consume its inbox
  may send through its Telegram capability
  may read and propose changes to the cooking-blog repository
  may use its own state volume
  may call explicitly bound HTTPS destinations through placeholder substitution

Cooking agent may not
  inspect another project
  read raw Telegram or model credentials
  change its authorization
  mutate canonical Git storage directly
  add another repository or gateway without an authorized binding
```

## Marketing and business definition

### Primary audience

- technically acute individuals building bespoke autonomous systems;
- software teams deploying coding and project agents;
- organizations allowing agents to touch valuable source, internal services,
  customer data, or production-adjacent workflows;
- regulated or security-conscious teams requiring isolation, provenance,
  deliberate updates, and audit; and
- agent authors who want reusable distribution without surrendering their
  loop to a framework.

### Value proposition

Hephaestus is not another universal assistant harness. It is the place where
agent programs become installable and governable software.

Potential concise copy:

> **Own the loop.**
>
> Build the agent you actually want. Hephaestus packages it as an immutable
> release, installs it into a project, gives it durable state and explicit
> capabilities, runs it in isolation, and records exactly what it did.

Supporting line:

> **Authority remains outside the loop.**
>
> Agents can learn and operate real tools without owning the credentials,
> policy engine, canonical stores, or permission to grant themselves more
> power.

### Enterprise reason to choose Hephaestus

```text
Personal-agent harness
→ install a powerful process, then constrain and monitor it

Hephaestus
→ install versioned agent software into an explicit governed capability
  boundary
```

A sound architecture is necessary but not sufficient for valuable customers.
Adoption also depends on SSO, backups, stable upgrades, audit export,
availability, operational documentation, support, and a credible security
process. These should be positioned as product requirements rather than
silently assumed to follow from microVM isolation.

### Deliberate tradeoffs

- bespoke agent code may duplicate integration logic;
- models and libraries reduce but do not eliminate protocol bureaucracy;
- opinionated frameworks can deliver a faster first demo;
- native/microVM execution costs more than shared-process actor runtimes;
- strict release and approval workflows add friction to self-modification;
- exact provenance becomes harder when mutable state affects behavior; and
- Hephaestus targets control, clarity, and valuable autonomy rather than the
  largest possible consumer feature checklist.

These are acceptable only if the resulting programs are materially easier to
understand, customize, secure, and govern.

## Dependencies and relationship to existing work

This task refines and extends
[`reusable-agent-releases-and-instances.md`](../done/reusable-agent-releases-and-instances.md).
The release/instance task supplies the immutable software and exact-run
foundation. Before that task is completed, it should preserve extension seams
for the accepted agent-principal, capability-binding, token-ceiling, and
authorization-snapshot decisions without absorbing their implementation.

[`manage-delegate-and-deliver-secrets.md`](../done/manage-delegate-and-deliver-secrets.md)
supplies encrypted secret storage, bindings, runtime leases, raw delivery, and
the base non-disclosing broker. The MVP tasks below extend it for generic
destination-bound HTTPS placeholder substitution. Interactive sessions, optimized fast
gateways, long-lived services, snapshots, distributions, packaged agents, and
the curated catalog remain deferred.

## MVP plan DAG

The MVP plans prove released agents can own their loops while Hephaestus retains
authority. The following graph is the authoritative dependency order; each child
task owns its implementation, tests, documentation, and completion evidence.

```text
MVP 01 authority
├── MVP 01.1 Git transport
│   └── MVP 01.2 runtime Git
├── MVP 02 durable mailboxes
└── MVP 04 destination-bound HTTPS egress and secret substitution
    └── MVP 03 synchronous HTTP gateways

MVP 05 cooking journey
  ← MVP 01, 01.2, 02, 03, 04

MVP 06 chat journey
  ← MVP 01.2, MVP 04
```

- [ ] **0. Ratify the minimum product contract**
  - [ ] Add a concise canonical architecture/product document defining “own
    the loop” and “authority remains outside the loop.”
  - [ ] Document the initial technically acute audience, coding-agent reference
    workload, cooking-agent acceptance workload, core promise, placement test,
    deliberate tradeoffs, and MVP non-goals.
  - [ ] Lock the user-facing and domain vocabulary needed by the MVP:
    agent instance, gateway instance, mailbox, capability requirement, binding,
    grant, authorization snapshot, runtime credential, state-access outcome,
    and ingress binding.
  - [ ] Keep product agent instances distinct from runtime guests, gateway
    guests, VMs, runs, and future service sessions.
  - [ ] Record rejected or overloaded terms and why they are avoided.
  - [ ] Link the accepted product document from the root README and relevant
    architecture documents.
  - [ ] Review the reusable-release task and lock only the extension seams for
    symbolic capability requirements, immutable revision bindings,
    authorization snapshots, and runtime principals.
  - [ ] Add links in both directions without copying competing implementation
    requirements into the reusable-release task.

- [ ] **1. Establish agent principals and runtime authority**
  - [ ] Complete
    [MVP 01: Agent capability requirements and instance permissions](mvp-01-agent-principals-capabilities-and-runtime-authority.md).
  - [ ] Verify the completed task extends the existing OpenFGA/Mélange, RLS,
    release, and secret-runtime models rather than creating parallel authority.

- [ ] **2. Add durable mailboxes and serialized stateful dispatch**
  - [ ] Complete
    [MVP 02: Durable agent mailboxes and stateful dispatch](mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md).
  - [ ] Verify a stopped stateful instance can accept durable work, restart,
    serialize execution under its exclusive volume lease, and recover honestly
    from crashes.

- [ ] **3. Add synchronous HTTP gateways**
  - [ ] Complete
    [MVP 03: Gateway HTTP routing and invocation](mvp-03-event-ingress-and-caddy-routing.md).
  - [ ] Verify Caddy configuration is derived from authoritative bindings and a
    compromised gateway cannot acquire target-agent, repository, project-state,
    or Caddy-administration authority.

- [ ] **4. Add destination-bound HTTPS egress and secret substitution**
  - [ ] Complete
    [MVP 04: Destination-bound HTTPS egress and secret substitution](mvp-04-brokered-model-and-outbound-capabilities.md).
  - [ ] Verify real credentials never enter guests, substitutions occur only on
    exact authorized destinations/routes, and direct egress cannot bypass the
    proxy.

- [ ] **5. Prove the golden cooking-agent journey**
  - [ ] Complete
    [MVP 05: Golden cooking-agent journey](mvp-05-golden-cooking-agent-journey.md).
  - [ ] Verify the deterministic real-system journey exercises authorized and
    denied users, gateway isolation, stateful operation, brokered credentials,
    controlled Git publication, release update, recovery, and complete
    provenance.

- [ ] **6. Prove the Git-backed chat journey**
  - [ ] Complete
    [MVP 06: Git-backed session chat journey](mvp-06-git-backed-session-chat-journey.md)
    after MVP 01.2 and MVP 04 only.
  - [ ] Verify the selected chat release owns its session repository protocol;
    public gateways and outbound delivery are not prerequisites.

- [ ] **7. Verify and hand off the MVP roadmap**
  - [x] Create and maintain dependency-ordered MVP task files with independent
    outcomes, locked decisions, non-goals, checklists, verification, and
    completion-evidence requirements.
  - [ ] Review the umbrella decisions and all MVP tasks for contradictory
    terminology, authority, lifecycle, and commit-point requirements.
  - [ ] Confirm each unresolved MVP architecture question has one authoritative
    child task and no requirement depends on chat history.
  - [ ] Confirm the current reusable-release and secret tasks link or hand off
    every shared boundary without silently duplicating ownership.
  - [ ] Run the repository's Markdown and link checks, if any.
  - [ ] Run git diff --check.
  - [ ] Record accepted documents, child-task completion evidence, and any new
    deliberate deferrals below.

## Deferred

The following work is deliberately outside the first product slice. Keep it in
this umbrella until it is prioritized, then create one focused todo task with
independently verifiable acceptance criteria before implementation.

- [ ] **Fast gateway runtimes and long-lived services**
  - [ ] Compare V8 isolates, WebAssembly, Unikraft, and the general Linux
    microVM using cold start, density, compatibility, observability, packaging,
    isolation, and cleanup evidence.
  - [ ] Distinguish open-source Unikraft/KraftKit capabilities from managed
    Unikraft Cloud routing, wake-up, and scale-to-zero behavior.
  - [ ] Validate Unikraft SQLite WAL, fsync, forced termination, restart,
    snapshot renewal, credential renewal, and crash recovery before accepting
    it for stateful services.
  - [ ] Specify service routing, activation, readiness, health, idle policy,
    WebSockets, streaming, restart, drain, revision cutover, and exclusive
    volume ownership.
  - [ ] Keep Linux microVM behavior as the compatibility baseline and prohibit
    transparent cross-runtime substitution without proven equivalence.
  - [ ] Create separate runtime-proof and long-lived-service tasks if this work
    is prioritized.

- [ ] **Advanced state, concurrency, hibernation, and sessions**
  - [ ] Define numbered state generations, lineage, transition semantics,
    integrity evidence, and authorized state-history inspection.
  - [ ] Define stateless parallelism, bounded concurrency, state sharding,
    session volumes, and interactions among schedules, repository triggers,
    ingress, updates, and services.
  - [ ] Define checkpoints, integrity hashes, backups, reproducible volume
    snapshots, VM snapshots, retention, privacy, and restoration as distinct
    features.
  - [ ] Define stateful hibernation and snapshot resume with live revision,
    lease, authorization, credential, network, and filesystem revalidation.
  - [ ] Define mutable private skill provenance and promotion into immutable
    released software.
  - [ ] Complete or replace
    [state-capability transitions](support-agent-state-capability-transitions.md)
    when transitions beyond the MVP reject-by-default contract are required.
  - [ ] Reconcile interactive-session work with the gateway HTTP contract and
    long-lived-service deferral before adding any shared routing behavior.

- [ ] **Broader credential, egress, and semantic capability coverage**
  - [ ] Specify and validate a transparent destination-bound TLS interception
    proxy only if application-level semantic adapters are insufficient.
  - [ ] Add search, browser, GitHub, arbitrary API, and additional provider
    adapters as separate bounded capabilities rather than a monolithic
    integration framework.
  - [ ] Define long-lived service credential renewal, rotation, revocation, and
    snapshot-resume behavior.
  - [ ] Add production KMS, external-vault synchronization, hardware-backed
    custody, and additional secret delivery modes through focused security
    tasks.
  - [ ] Preserve the explicit safety ladder between raw receipt,
    destination-bound substitution, and semantic host-performed operations.

- [ ] **Operator/Admin Agent, Project Agent, and delegated management**
  - [ ] Define user-mediated authority with the human as effective principal
    and the agent recorded as mediator under a short-lived delegation ceiling.
  - [ ] Define one-shot approvals bound to an exact action, target, command,
    approver, and expiry.
  - [ ] Specify Operator Agent separation, proposal and approval behavior,
    self-management restrictions, and a non-agent break-glass path.
  - [ ] Specify Project Agent defaults, project-scoped continuity, memory,
    workflows, schedules, and explicit self-update and publication boundaries.
  - [ ] Define safe removal, replacement, recovery, and audit of packaged
    management agents.

- [ ] **Distributions and developer experience**
  - [ ] Define raw, advanced, and beginner bootstrap manifests over one core
    binary, schema, security model, and update path.
  - [ ] Define how distributions pin, install, update, remove, and recover
    curated releases and policy defaults without becoming divergent forks.
  - [ ] Produce a versioned Hephaestus coding skill, starter repositories,
    reference adapters, fixtures, conformance tests, and heph doctor.
  - [ ] Demonstrate small direct, XState, LangGraph, and model-SDK loops without
    making any library part of the platform ABI.
  - [ ] Define source visibility, ejectability, and upgrade compatibility
    across distributions.

- [ ] **Catalog and executable supply-chain governance**
  - [ ] Define publisher identity, signing, isolated builds, provenance,
    dependency and artifact manifests, permission diffs, review, revocation,
    retention, and update approval.
  - [ ] Define beginner curated-only behavior and raw/advanced explicit
    untrusted-import behavior.
  - [ ] Define organization-private catalogs and allowlists.
  - [ ] Treat agents, gateways, skills, templates, and executable dependencies
    consistently as supply-chain inputs.
  - [ ] Create separate catalog, signing, distribution-bootstrap, and packaged-
    agent tasks only when their contracts are accepted.

- [ ] **Product positioning and customer validation**
  - [ ] Produce a concise narrative using “Own the loop” and “Authority remains
    outside the loop.”
  - [ ] Compare Hephaestus with personal-agent harnesses and coding agents,
    explaining both when governed released agents win and when another product
    is the better choice.
  - [ ] Obtain structured feedback from technically acute users, software
    teams, and at least one security or operations stakeholder.
  - [ ] Test whether release/update rigor creates valuable control or unwanted
    bureaucracy and which packaged features materially affect adoption.
  - [ ] Identify first-customer blockers separately from generic enterprise
    accumulation.
  - [ ] Create focused tasks for selected-customer requirements such as SSO,
    backups, audit export, availability, stable upgrades, operational security,
    documentation, and support.

## Completion evidence

Populate this section while the task is in `tasks/in-progress/`.

Record:

- accepted product and architecture document paths;
- accepted terminology and rejected alternatives;
- completion evidence from each of the five dependency-ordered MVP tasks;
- OpenFGA/Mélange, RLS, runtime-credential, and revocation evidence;
- mailbox, ingress, gateway-isolation, broker, and network-denial evidence;
- serialized state-volume, fenced-lease, dispatch-order, state-access, and
  exact-provenance evidence;
- cooking-agent normal, denial, update, recovery, and inspection evidence;
- reviewed contradictions and the resolution chosen for each;
- deliberate deferrals retained here; and
- focused todo-task paths created when a deferred workstream is prioritized.
