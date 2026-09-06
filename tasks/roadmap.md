# Roadmap: From proven core to a usable Hephaestus distribution

This is the product direction agreed on 2026-09-06, not an implementation task
or a competing task-status tracker. Detailed execution and completion evidence
belong in the task directories described in [README.md](README.md).

## Product direction

Ship one useful Hephaestus distribution backed by a reusable, harness-neutral
core. Agents are owned as software projects: source, instructions, skills,
custom tools, dependencies and harness code belong together. Users should start
with a working assistant and be able to customize or replace it without
rebuilding the platform around it.

Keep VM isolation, explicit authority, controlled releases and inspection as
foundations. Minimalism should reduce the concepts users must manage, not
require them to assemble everything before receiving value.

## 1. Close the MVP-05 acceptance gaps

- [ ] Complete [MVP-05.1: cooking acceptance](in-progress/mvp-05.1-complete-cooking-acceptance.md),
  including real builds and installation, multi-request operation, updates,
  recovery, security coverage, real Telegram transport and browser evidence.
- [ ] Preserve the distinction between the verified first-request proof and
  remaining acceptance requirements; record any agreed scope changes explicitly.

Do not introduce broad infrastructure merely to finish the scenario. Its
existing task remains the source of truth for detailed acceptance and tests.

## 2. Audit the existing interactive path

- [ ] Trace the complete path from user input through durable acceptance and
  isolated execution to a visible response and reconnectable history.
- [ ] Classify each transition as already supported, awkward to integrate, or
  genuinely missing, using repository evidence and focused experiments.
- [ ] Identify the smallest necessary changes for response publication,
  subscriptions, authorization and recovery before committing to an API design.
- [ ] Record ownership of the visible transcript separately from agent-owned
  model context and internal workflow state.

This is an audit of Hephaestus's own requirements and existing primitives.
Do not adopt DO/celld concepts, APIs, dependencies or runtime architecture at
this stage. No new actor abstraction, instance API or conversation service is
presupposed by this step. Do not add a second scheduler or delivery system when
the existing mechanisms suffice.

## 3. Ship the first usable distribution

- [ ] **Default agent project**
  - [ ] Supply an ordinary, editable, replaceable Hephaestus project using
    documented platform capabilities and an opinionated starter harness.
  - [ ] Help users create agents from maintained templates, edit their source,
    test them and propose releases rather than promise arbitrary reliable
    agent generation from scratch.
  - [ ] Help inspect and diagnose the installation with scoped authority;
    distinguish reading diagnostics, editing projects, granting access and
    changing the installation.
  - [ ] Keep installation, recovery and privileged approvals available through
    deterministic platform controls without requiring a functioning assistant.

- [ ] **Default web chat**
  - [ ] Provide persistent conversations, text input, incremental responses,
    working/completed/failed states and reconnect handling.
  - [ ] Link artifacts, runs and trusted permission/deployment approval screens.
  - [ ] Keep the frontend replaceable and its conversation contract optional
    for projects; do not prescribe model context, SDK, harness or agent loop.
  - [ ] Verify the full interactive journey identified in step 2, including
    failures and reconnection rather than just a successful live session.

- [ ] **Small iframe integration in the existing web shell**
  - [ ] Register interface pages in shell-owned navigation: project-local pages
    within their project, global entries such as Assistant through explicit
    installation configuration.
  - [ ] Retain shell navigation and project context while the iframe owns its
    content area; use the same integration boundary for the bundled chat.
  - [ ] Define separate-origin isolation and a narrow, versioned, authorized
    bridge without exposing the management session or a generic privileged proxy.
  - [ ] Keep grants and deployment approvals in the trusted shell, with
    server-side authorization; treat conversation-data access as a trust decision.
  - [ ] Cover registration, loading/error states, navigation and isolation
    with focused integration and browser tests.

These are logical boundaries, not a requirement for separate repositories or
new deployment services. Prefer the existing web stack where appropriate.
The starter may choose libraries such as XState and Vercel AI without making
them core requirements. Detailed protocols and hosting decisions follow the
interactive-path audit rather than being treated as already implemented.

## 4. Make the developer journey coherent

- [ ] Provide one documented path to create a project, run it with real
  isolation and authority, open its interface, send sample work, inspect results,
  edit, retest and deploy a version.
- [ ] Supply sensible defaults and helpers over existing operations so normal
  use does not require manually wiring every platform resource.
- [ ] Keep deterministic, scriptable CLI/API operations alongside web-first
  onboarding and chat.
- [ ] Demonstrate the product journey: create a template-based agent, make a
  visible customization, approve its access, run a test and deploy the result.

## 5. Validate usefulness and strengthen operations

- [ ] Put the first distribution in front of a few developers building agents
  for themselves or clients, observing where they need help and using that
  evidence to prioritize subsequent work.
- [ ] Define backup and restoration as a first-class product workstream,
  including the consistent set of code, configuration and persistent state
  needed to recover an installation and the handling of credentials/keys.
- [ ] Demonstrate restoration before promising dependable operation of valuable
  persistent workloads; do not equate replication or durable storage with backups.
- [ ] Document the security threat model, operational limits and recovery
  guarantees alongside the product experience.

## Deliberately deferred

A distribution ecosystem, UI marketplace, arbitrary in-shell JavaScript plugins,
generalized widget framework, polished terminal chat, voice and broad messenger
coverage are not prerequisites for the first distribution. MVP-05's scoped
Telegram proof remains part of its own acceptance work.

DO/celld may remain research references, but borrowing their concepts or adopting
their infrastructure is not part of this roadmap. Broad core restructuring and
new infrastructure require demonstrated needs, not analogy alone.

## Execution and verification

- [ ] Create bounded implementation tasks as each roadmap stage is taken up,
  with ownership, dependencies, acceptance criteria and verification evidence.
- [ ] Apply repository quality policy to implementation changes, including the
  required Rust checks and `cargo dev quality` for the repository-wide handoff,
  plus scenario-specific integration and browser checks where relevant.

The sequence is a priority order, not a claim that later planning must wait for
every earlier test. Do not declare MVP-05 complete or a product capability ready
without its own acceptance evidence.
