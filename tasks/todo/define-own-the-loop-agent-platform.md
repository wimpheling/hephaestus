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

Internet → Caddy → fast gateway → durable inbox → Project/installed Agent
                     runtime                           runtime
                        │                                 │
                 protocol-specific                 owns loop, memory,
                  released code                    and project behavior
```

The public gateway and project agent are separate principals. Compromise of a
public protocol parser must not automatically grant repository, project-state,
or management authority.

### Fast gateways

Hephaestus should provide a surface on which users and their agents can expose
small protocol-specific gateways. V8 isolates, WebAssembly, or another
fast-start low-overhead runtime are candidates; the engine choice is not yet
locked.

A gateway may:

- receive bounded public HTTP or WebSocket traffic;
- validate a protocol signature or application token;
- normalize an event using user-owned code;
- acknowledge the remote service promptly;
- publish a durable event to explicitly bound agent inboxes; and
- send responses through narrowly scoped brokered capabilities.

A gateway should not receive a project state volume, repository mount,
organization-wide token, or implicit Project Agent authority.

Two execution contracts require explicit decisions:

```text
event binding
Caddy → persist generic request → acknowledge → wake agent asynchronously

service binding
Caddy → route to a live guest service with health, drain, and restart policy
```

Event bindings fit webhooks and scale-to-zero behavior. Service bindings fit
streaming, WebSockets, preview servers, and applications requiring a live
process. Service bindings expand beyond the current reusable-release task’s
non-goal of long-lived interactive VMs and therefore require their own
architecture and task.

### Caddy integration boundary

Caddy should be the public HTTP edge, not the source of truth for ingress or
an agent-aware control plane. It owns TLS termination, public HTTP protocol
handling, and forwarding to stable Hephaestus ingress endpoints. Hephaestus
owns domain and route bindings, authorization, activation, readiness, durable
delivery, and the mapping from a logical target to an ephemeral guest.

An authorized user or agent requests an ingress binding through the
Hephaestus API. The binding identifies a domain, bounded route and methods,
event or service contract, target instance or inbox, and revision. Hephaestus
validates domain ownership, route conflicts, and the caller's capabilities,
then reconciles derived Caddy configuration. Agents must not receive direct
access to Caddy's administration API. Caddy configuration is reconstructible
deployment state; authoritative bindings and their audit history remain in
the Hephaestus database.

For event bindings, Caddy forwards to an always-available Hephaestus ingress
service, which applies request limits, durably records the generic envelope,
and only then acknowledges and schedules work. For service bindings, Caddy
forwards through a stable activation proxy; Hephaestus starts or wakes the
selected revision, waits for readiness, and resolves its private endpoint.
Caddy should not need to understand VM addresses, leases, snapshots, or agent
credentials.

The edge contract must define trusted proxy headers, request and response size
limits, timeouts, rate limits, WebSocket and streaming behavior, request IDs,
TLS certificate lifecycle, drain during revision cutover, and safe failure
responses. Project code may implement application authentication such as a
Telegram signature, but platform authorization to own a domain or expose a
route remains outside that code.

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

Because memory, prompts, and agent-authored skills may affect behavior, exact
run provenance should eventually include:

```text
immutable release
+ immutable instance revision
+ pre-run state generation/checkpoint
+ authorization snapshot
+ trigger and target
```

and record the post-run state generation when state changes.

Full state snapshots may be expensive and privacy-sensitive. The architecture
must distinguish generation identifiers, integrity hashes, backups, and
reproducible snapshots rather than assuming all four are equivalent.

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

[capabilities.telegram]
kind = "gateway"
permissions = ["receive_events", "send_responses"]

[capabilities.model]
kind = "model"
permissions = ["invoke"]
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

model.invoke
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
3. semantic capabilities where Hephaestus performs the privileged operation.

Model access is a candidate platform capability even though the agent owns its
loop. A model gateway can keep provider credentials outside the guest and
enforce budgets, rate limits, provider policy, revocation, and usage audit
without prescribing prompts or orchestration.

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
- retains exact release, state-generation, authorization, trigger, and result
  provenance; and
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
  may invoke a bounded model/search policy

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
[`reusable-agent-releases-and-instances.md`](./reusable-agent-releases-and-instances.md).
The release/instance task supplies the immutable software and exact-run
foundation. Its authorization workstream should incorporate the accepted
agent-principal, capability-binding, token-ceiling, and authorization-snapshot
decisions produced here.

Secret brokering, interactive sessions, fast gateways, long-lived services,
state checkpoints, distributions, and the curated catalog should become
separate tasks after their boundaries are accepted.

## Implementation checklist

- [ ] **1. Ratify and document the product definition**
  - [ ] **Write the canonical product document**
    - [ ] Add an architecture/product document defining “own the loop” and
      “authority remains outside the loop.”
    - [ ] Document the target users, reference workloads, core promise,
      deliberate tradeoffs, and non-goals.
    - [ ] Document the platform-versus-released-agent placement test with
      concrete examples.
    - [ ] Link the product document from the root README and relevant
      architecture documentation.
  - [ ] **Validate terminology**
    - [ ] Decide the user-facing and domain names for Operator/Admin Agent,
      Project Agent, gateway, distribution, catalog, capability requirement,
      binding, grant, runtime credential, delegation, approval, and
      authorization snapshot.
    - [ ] Ensure product-level agent instances remain distinct from runtime
      guests, gateway isolates, VMs, runs, and service sessions.
    - [ ] Record rejected or overloaded terms and the reason for avoiding them.
  - [ ] **Review the existing release/instance task**
    - [ ] Identify decisions from this task that belong in the existing task’s
      locked decisions.
    - [ ] Add links in both directions without copying competing requirements.
    - [ ] Split any newly independent release/instance work into focused todo
      tasks instead of expanding that epic indefinitely.

- [ ] **2. Specify agent principals, capabilities, and runtime credentials**
  - [ ] **Design the canonical OpenFGA model extension**
    - [ ] Model `agent_instance` as an authorization subject.
    - [ ] Model gateways and any required runtime/delegation objects.
    - [ ] Define explicit agent relations for repositories, gateways,
      instances, models, schedules, secrets/capabilities, and other initial
      resources without granting implicit project-maintainer authority.
    - [ ] Define who may bind, grant, revoke, and delegate each capability.
    - [ ] Define autonomous-agent and user-mediated authorization semantics.
    - [ ] Verify the proposed model with OpenFGA compatibility fixtures and
      Mélange generation before accepting it.
  - [ ] **Design authoritative domain records**
    - [ ] Specify capability requirements declared by a release agent.
    - [ ] Specify immutable instance-revision bindings from symbolic slots to
      concrete resources.
    - [ ] Specify live grants, revocation, and binding lifecycle.
    - [ ] Specify immutable authorization snapshots and their hashes.
    - [ ] Specify runtime credentials, capability ceilings, delegation, and
      one-shot approval records.
    - [ ] Define which records produce authoritative `melange_tuples`.
  - [ ] **Generalize transaction and RLS identity**
    - [ ] Specify typed transaction-local subject, request, mediator,
      run/session, and delegation context.
    - [ ] Define autonomous agent, delegated user, gateway, and trusted-worker
      database-role flows.
    - [ ] Require agent-facing APIs to remain under non-`BYPASSRLS` roles.
    - [ ] Define audit fields sufficient to reconstruct the complete
      delegation and authorization chain.
  - [ ] **Specify credential minting and checking**
    - [ ] Decide opaque versus signed credential formats for each trust
      boundary, using opaque server-side credentials as the initial default.
    - [ ] Define dispatch-time authorization and minting.
    - [ ] Define ceiling checks plus live Mélange checks for every privileged
      API call.
    - [ ] Define expiry, rotation, revocation, renewal, theft response, and
      cleanup.
    - [ ] Define behavior when authorization changes during queued, running,
      mounted, or long-lived service work.
  - [ ] **Create focused implementation tasks**
    - [ ] Create a todo task for domain types, schema, OpenFGA/Mélange
      generation, and real-PostgreSQL tests.
    - [ ] Create a todo task for runtime credential issuance, validation,
      revocation, and audit.
    - [ ] Create a todo task for generalized RLS subject context and migration
      from the current user-only context.

- [ ] **3. Decide the gateway and service model**
  - [ ] **Write a gateway architecture decision**
    - [ ] Define the separation between Caddy, gateway code, durable inboxes,
      agent guests, and outbound responses.
    - [ ] Specify authoritative ingress-binding records and derived,
      atomically reconciled Caddy configuration.
    - [ ] Define domain verification, certificate lifecycle, route-conflict
      handling, revision cutover, and recovery after Caddy restart.
    - [ ] Keep Caddy's administration API private to the trusted reconciler;
      require agents to use capability-checked Hephaestus APIs.
    - [ ] Define gateway identity, capabilities, state, lifecycle, limits, and
      isolation.
    - [ ] Compare V8 isolates, WebAssembly runtimes, microVMs, and other
      candidates using cold-start, density, compatibility, observability, and
      security criteria.
    - [ ] Evaluate Unikraft as an optional `instant-service` backend, keeping
      the general Linux microVM as the compatibility baseline for coding
      agents.
    - [ ] Distinguish functionality available from open-source Unikraft and
      KraftKit from managed Unikraft Cloud routing, wake-up, and scale-to-zero
      behavior.
    - [ ] Define how gateway releases are built, reviewed, installed, updated,
      and rolled back.
  - [ ] **Run an Unikraft gateway proof**
    - [ ] Package the same small Rust webhook gateway for Unikraft, the current
      Linux microVM runtime, and the leading isolate candidate.
    - [ ] Compare image size, cold request latency, steady memory, concurrent
      start density, packaging effort, observability, and cleanup behavior.
    - [ ] Validate a read-only release with writable SQLite state, including
      WAL, `fsync`, forced termination, restart, and crash recovery.
    - [ ] Validate Caddy request buffering or durable acknowledgement while a
      stopped instance starts.
    - [ ] Validate default-deny egress, credential brokering, capability
      revocation, and runtime-token renewal after snapshot resume.
    - [ ] Record unsupported syscalls, libraries, tooling, and architecture
      constraints; define explicit acceptance and rejection criteria.
  - [ ] **Resolve event versus service bindings**
    - [ ] Specify generic event-ingress request, acknowledgement,
      deduplication, inbox, retry, and response semantics.
    - [ ] Specify live-service routing, health, readiness, idle policy,
      WebSockets, streaming, restart, drain, revision cutover, and volume
      ownership.
    - [ ] Decide which mode or modes are required for the first product
      milestone.
    - [ ] Document why neither contract introduces Telegram-specific semantics
      into the core.
  - [ ] **Create focused implementation tasks**
    - [ ] Create a todo task for Caddy routing and the selected fast gateway
      runtime proof.
    - [ ] Create a todo task for generic durable ingress inbox/outbox.
    - [ ] Create a separate todo task for long-lived services if they remain in
      scope after the architecture decision.

- [ ] **4. Specify credential and network capability brokering**
  - [ ] **Write the threat model**
    - [ ] Cover prompt injection, compromised dependencies, raw secret reads,
      alternate egress, DNS rebinding, SSRF, metadata endpoints, tunneling,
      logs, state persistence, model context, and authorized-channel data
      exfiltration.
    - [ ] Distinguish protecting a credential value from constraining the
      authority exercised through that credential.
    - [ ] Define assumptions and limits for microVM, isolate, gateway, build,
      update-hook, and normal-run guests.
  - [ ] **Design the broker**
    - [ ] Specify raw delivery, proxy substitution, and semantic capability
      modes.
    - [ ] Specify placeholder/capability identifiers, host storage, TLS
      handling, destination policy, DNS/firewall enforcement, audit, rotation,
      and revocation.
    - [ ] Specify model-provider brokering, budgets, rate limits, and usage
      attribution without prescribing the agent loop.
    - [ ] Specify how Git, Telegram, search, browser, and arbitrary API
      capabilities can be supported without creating a monolithic integration
      framework.
  - [ ] **Validate with an adversarial proof**
    - [ ] Create a focused task for a fake upstream service and brokered-secret
      prototype.
    - [ ] Require tests showing the guest cannot read the real credential or
      bypass the intended network path.
    - [ ] Require tests showing that authorized requests remain capable of
      data exfiltration, documenting why least privilege and semantic
      capabilities remain necessary.

- [ ] **5. Define state, concurrency, hibernation, and provenance**
  - [ ] **Specify state concurrency**
    - [ ] Define default serialization for an instance with one exclusive
      state volume.
    - [ ] Define stateless parallelism, bounded concurrency, and future state
      sharding or session-volume options.
    - [ ] Define behavior for simultaneous messages, scheduled tasks, repository
      triggers, updates, and service traffic.
  - [ ] **Specify sleep and wake**
    - [ ] Define what the agent must persist before becoming idle.
    - [ ] Define wake triggers, restart context, inbox replay, ephemeral-state
      loss, and crash behavior.
    - [ ] Distinguish durable application state from arbitrary process or
      workflow-stack checkpointing.
  - [ ] **Specify state provenance**
    - [ ] Define volume generations, checkpoints, hashes, backups, and
      snapshots as distinct concepts.
    - [ ] Define pre-run and post-run state provenance.
    - [ ] Define retention, privacy, inspection, and reproducibility policy.
    - [ ] Define how mutable private skills and promoted released skills appear
      in provenance.
  - [ ] **Create focused implementation tasks**
    - [ ] Create a todo task for per-instance durable mailboxes and concurrency
      policy.
    - [ ] Create a todo task for state generations/checkpoints and run
      provenance.
    - [ ] Create an interactive-session task only after its relationship to
      generic event and service bindings is explicit.

- [ ] **6. Define distributions, packaged agents, and the catalog**
  - [ ] **Specify distribution manifests**
    - [ ] Define raw, advanced, and beginner bootstrap contents while keeping
      one core binary/schema and one security update path.
    - [ ] Define how distributions pin, install, update, and remove curated
      releases and policy defaults.
    - [ ] Define ejectability and source visibility for packaged agents.
    - [ ] Define upgrade compatibility across distributions.
  - [ ] **Specify the Hephaestus coding skill**
    - [ ] Document expected architecture and coding philosophy, not only API
      syntax.
    - [ ] Version the skill with the supported Hephaestus contracts.
    - [ ] Pair it with templates, reference adapters, fixtures, conformance
      tests, and `heph doctor`.
    - [ ] Demonstrate generation of a small bespoke loop and gateway without a
      privileged framework dependency.
  - [ ] **Specify packaged agents**
    - [ ] Define the Operator Agent’s mediation, approval, separation, and
      self-management restrictions.
    - [ ] Define the Project Agent’s default project-scoped capabilities,
      memory, workflows, and approval boundaries.
    - [ ] Define how beginner functionality may use XState, LangGraph, and
      existing adapters without making those libraries part of the platform
      ABI.
    - [ ] Define safe removal, replacement, and recovery of packaged agents.
  - [ ] **Specify catalog governance**
    - [ ] Define publisher identity, review, signing, provenance, permission
      manifests, update diffs, revocation, and retention.
    - [ ] Define beginner curated-only behavior and advanced/raw explicit
      untrusted import behavior.
    - [ ] Define organization-private catalogs and allowlists.
    - [ ] Treat skills, dependencies, gateways, and agents consistently as
      executable supply-chain inputs.
  - [ ] **Create focused implementation tasks**
    - [ ] Create separate todo tasks for distribution bootstrapping, the
      Hephaestus skill, packaged agents, and catalog infrastructure after each
      contract is accepted.

- [ ] **7. Validate the golden cooking-agent product journey**
  - [ ] **Write the reference application**
    - [ ] Specify the two-user Telegram policy, gateway code, agent loop,
      persistent recipe state, blog repository, static-site build, and Caddy
      publication flow.
    - [ ] Specify exact capability requirements and denied capabilities.
    - [ ] Specify model/search usage and credential-broker behavior.
    - [ ] Specify normal operation, simultaneous messages, retry, update,
      credential revocation, gateway failure, and state recovery.
  - [ ] **Define the end-to-end acceptance scenario**
    - [ ] Install the cooking-agent release from a curated source.
    - [ ] Bind its Telegram gateway, two authorized users, state volume, model
      policy, and cooking-blog repository.
    - [ ] Receive messages from both users and update the static blog through
      controlled Git publication.
    - [ ] Demonstrate that an unauthorized Telegram user cannot invoke the
      project agent.
    - [ ] Demonstrate that gateway compromise does not confer repository
      authority.
    - [ ] Demonstrate that the agent cannot read brokered raw credentials or
      grant itself another repository.
    - [ ] Update the released loop while retaining state and exact historical
      provenance.
    - [ ] Inspect release, revision, state generations, authorization snapshot,
      ingress, run, result, and audit chain.
  - [ ] **Create the implementation task**
    - [ ] Create a focused todo task for the golden scenario after its
      dependent gateway, capabilities, credentials, and state contracts are
      accepted.

- [ ] **8. Validate product positioning and valuable-customer requirements**
  - [ ] **Write positioning material**
    - [ ] Produce a one-page narrative using “Own the loop” and “Authority
      remains outside the loop.”
    - [ ] Produce a comparison explaining why Hephaestus does not pursue core
      feature parity with OpenClaw, Hermes, Codex, Claude Code, or Pi.
    - [ ] Describe when a personal-agent harness is the better choice and when
      governed released agents are the better choice.
    - [ ] Use the cooking bot and complex-project coding agent as complementary
      demos of the same platform.
  - [ ] **Validate target users**
    - [ ] Interview or obtain structured feedback from technically acute
      individual users, software teams, and at least one security/operations
      stakeholder.
    - [ ] Test whether release/update rigor is understood as valuable control
      or merely experienced as bureaucracy.
    - [ ] Test willingness to adopt bespoke code plus a Hephaestus skill over
      a universal agent harness.
    - [ ] Record which packaged beginner features materially improve adoption
      without contaminating the core boundary.
  - [ ] **Define enterprise readiness gaps**
    - [ ] Create or link focused tasks for SSO, backups, audit export,
      availability, stable upgrades, operational security, documentation, and
      support expectations that are required by the selected first customer.
    - [ ] Separate first-customer blockers from generic “enterprise” feature
      accumulation.

- [ ] **9. Decompose and hand off**
  - [ ] **Create a dependency-ordered roadmap**
    - [ ] Order the focused tasks created above by architectural dependency and
      smallest demonstrable product slice.
    - [ ] Identify which tasks modify the existing reusable-release epic and
      which remain independent.
    - [ ] Ensure no child task requires reconstructing decisions from chat
      history.
    - [ ] Keep implementation ownership and completion evidence in child tasks,
      not this definition task.
  - [ ] **Verify the definition package**
    - [ ] Review every locked or working decision for internal contradictions.
    - [ ] Confirm every unresolved architecture question has an owner or child
      task.
    - [ ] Run Markdown/link checks used by the repository, if any.
    - [ ] Run `git diff --check`.
    - [ ] Record reviewed product documents, architecture decisions, prototype
      evidence, interview notes, and created child-task paths below.

## Completion evidence

Populate this section while the task is in `tasks/in-progress/`.

Record:

- accepted product and architecture document paths;
- accepted terminology and rejected alternatives;
- OpenFGA/Mélange model and compatibility evidence;
- gateway/runtime and credential-broker prototype results;
- state/provenance decisions;
- distribution, packaged-agent, and catalog specifications;
- cooking-agent scenario evidence;
- target-user feedback and resulting positioning changes;
- the dependency-ordered list of child implementation tasks; and
- deliberate deferrals and their todo-task paths.
