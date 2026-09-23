# Project-defined permission systems and delegated authorization

Owner: unassigned

## Purpose

Explore how a released project can define and enforce its own permission system
without turning Hephaestus into an interpreter for application-specific roles,
resources, or policy language.

The motivating example is a custom IRC-like service implemented by released
code. It defines channels, memberships, roles, moderation, and message
semantics, while authenticating people and agent runtimes through Hephaestus.
An operator may want to grant an agent access to selected channels without
giving it access to every channel or the ability to alter the service's policy.

This is intentionally not an MVP task. It must follow, rather than weaken, the
control-plane boundary established by the capability and runtime-authority work.

## Desired outcome

A project can use Hephaestus identities for people and workloads and choose one
of two authorization models:

1. **Application-local authorization.** The released service verifies a
   Hephaestus-issued identity and applies its own ACL, role, or policy logic
   stored in its repository, SQLite database, or other declared resource.
2. **Optional delegated authorization.** The service asks Hephaestus for an
   allow/deny decision over an application-owned opaque resource and action, or
   validates a short-lived, audience-bound permit issued by Hephaestus.

In both models, application code owns resource meaning and policy semantics;
Hephaestus owns trusted identity, bounded delegation, live revocation where
applicable, audit, and the enforcement of its own control-plane resources.

## Principles

- OIDC authentication answers **who** is connecting; it does not by itself
  answer whether that subject may perform an application action.
- Hephaestus must not add IRC rooms, chat roles, message policy, or arbitrary
  project policy evaluation to its core domain model.
- A project service owns its resource IDs, action vocabulary, relationships,
  role inheritance, membership lifecycle, and authorization semantics.
- A runtime identity is never a human identity: it represents an exact workload
  revision and short-lived runtime session.
- Tokens must be audience-bound to the receiving service and short-lived. A
  service must not be able to replay one service's token to another service.
- A project-defined authorization grant must not become authority to configure
  Hephaestus resources, grant new capabilities, or impersonate a user.
- Live checks and locally validated permits need explicit, honest revocation
  semantics. A cached or self-contained permit cannot promise instantaneous
  revocation.

## Candidate integration model

```text
person or runtime session
  → audience-bound Hephaestus identity token
  → released application service
  → application-local policy check
       or optional delegated allow/deny check
  → application action
```

For an IRC-like service, the application may define its own records:

```text
service: family-irc
room: room/recipes
member: runtime-session or agent-instance identity
role: contributor
actions: join, read, send
```

`room/recipes`, `contributor`, and `send` are application concepts. They are
not new built-in Hephaestus resource kinds. If delegated authorization is used,
Hephaestus sees only a registered service identity, a normalized opaque resource
identifier, a bounded action identifier, and the authenticated subject.

## Questions to resolve before implementation

- [ ] Define the identity-token contract for humans, agent instances, and exact
  runtime sessions: issuer, audience, subject format, project/instance/revision
  attribution, expiry, key rotation, token exchange, and introspection.
- [ ] Decide whether services may accept a runtime token directly, require an
  OIDC token exchange, or support both. Ensure a workload receives no reusable
  user credential.
- [ ] Define how an external/released service is registered as an allowed token
  audience, including ownership, redirect/callback concerns if any, lifecycle,
  rotation, and removal.
- [ ] Decide whether application-local ACLs are the only first version, or
  whether to add a generic delegated allow/deny store.
- [ ] If delegated authorization is added, define the smallest generic model:
  stable service ID, normalized opaque resource ID, bounded action ID, subject,
  grant/revocation lifecycle, and audit record. Do not add arbitrary policy
  code, relation schemas, or provider-specific objects to Hephaestus.
- [ ] Decide who can create resource IDs and grants, how a service proves it
  owns a namespace, and how stale or deleted application resources are handled.
- [ ] Define live-check API semantics, authorization caching, outage behavior,
  rate limits, decision IDs, and audit attribution. State the exact revocation
  guarantee for each mode.
- [ ] Define a compact signed-permit alternative: audience, subject, resource,
  action, issuance/expiry, key ID, and optional proof-of-possession binding.
  Decide how its maximum lifetime and revocation behavior differ from a live
  decision.
- [ ] Define how application policy may use Hephaestus identity attributes
  without treating mutable or user-controlled claims as authorization facts.
- [ ] Define cross-project sharing rules. In particular, decide whether one
  project can ever grant its runtime access to another project's application
  service or resource namespace.
- [ ] Define threat models and tests for confused deputy attacks, audience
  confusion, token replay, runtime/session expiry, compromised service,
  malicious agent, stale membership, grant escalation, and authorization-server
  outage.

## Reference acceptance scenario

- [ ] Build a small released IRC-like service with application-owned room ACLs.
- [ ] Authenticate a human and an agent runtime using distinct Hephaestus
  identities.
- [ ] Permit the agent to join and send only to `room/recipes`; deny
  `room/admin` and all unlisted rooms.
- [ ] Prove that a terminated/revoked runtime can no longer obtain new access.
- [ ] If a delegated check or permit is selected, prove audience isolation,
  exact room/action scoping, grant revocation behavior, and audit provenance.
- [ ] Prove the service cannot use its integration to configure Hephaestus,
  broaden a runtime's platform capabilities, or impersonate a human.

## Non-goals

This task does not add an IRC server, chat protocol, universal RBAC/ABAC/OPA
engine, arbitrary project policy code inside Hephaestus, a new global OpenFGA
model per project, federation, or a replacement for the existing capability
model. It also does not make application-owned authorization authoritative for
Hephaestus control-plane resources.
