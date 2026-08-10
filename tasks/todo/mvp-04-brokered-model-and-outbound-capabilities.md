# MVP 04: Destination-bound HTTPS egress and secret substitution

Owner: unassigned

## Outcome

Let released agents and gateways call ordinary external HTTPS APIs while real
provider credentials never enter their VMs. Released code owns every API
protocol, request, response, retry, and user-experience decision. Hephaestus
owns only destination-bound egress, placeholder substitution, secret authority,
network enforcement, revocation, and audit.

A workload receives non-secret placeholders for its declared brokered slots.
The host egress proxy replaces an allowed placeholder with the real secret only
on a verified TLS connection to that slot's exact allowed destination. For an
authorized public gateway route, the ingress side performs the inverse:
a valid real inbound secret is replaced with the placeholder before the HTTP
handler sees it.

```text
outbound: VM placeholder → verified HTTPS egress proxy → real secret → API
inbound:  API real secret → authorized gateway route → placeholder → VM
```

## Locked decisions

| Area | Decision |
| --- | --- |
| Product boundary | This is generic HTTPS egress and secret substitution, not an LLM, Telegram, or provider-specific platform capability. |
| Placeholder | A brokered binding gives the VM a stable non-secret placeholder, never the secret value. A placeholder is useful only through its bound substitution rule. |
| Outbound substitution | The host proxy replaces a placeholder only at its declared injection location on a verified TLS connection to an exact authorized destination. It otherwise forwards the placeholder unchanged or denies the request. |
| Inbound substitution | MVP 04 defines the reusable brokered placeholder rule for an inbound header. MVP 03 applies that rule at an authorized public gateway route: it compares the received value with the bound secret in constant time, rewrites a match to the placeholder before VM delivery, and leaves a non-match non-matching or rejects it. |
| Protocol ownership | The workload owns all API and webhook protocol semantics. The platform understands only generic HTTP locations and secret substitutions, never provider request schemas. |
| Network | Brokered-egress workloads are default-deny and cannot bypass the proxy through DNS changes, raw IPs, IPv6, redirects, proxy variables, metadata endpoints, or alternate interfaces. |
| TLS | The proxy verifies destination identity and certificate handling before substitution. Because HTTPS request fields are encrypted, the implementation must use a guest-trusted Hephaestus interception CA or an equivalent cooperating transport; opaque CONNECT tunnelling is insufficient. |
| Secret authority | Existing secret imports, immutable bindings, leases, destination ceilings, live authorization, rotation, revocation, and audit remain authoritative. Raw delivery remains a generic existing mode but is not used for provider/webhook credentials in this MVP. |
| Limits | The MVP supports bounded HTTPS request/response bodies, allowlisted headers and injection locations, timeouts, and cancellation. It excludes WebSockets, streaming, arbitrary proxy protocols, and arbitrary credential fetch. |

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](../done/mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`manage-delegate-and-deliver-secrets.md`](../done/manage-delegate-and-deliver-secrets.md)
- [`define-own-the-loop-agent-platform.md`](define-own-the-loop-agent-platform.md)

## Non-goals

This task does not add an LLM abstraction, model policy, prompts, model
adapters, token/cost budgets, Telegram semantics, provider-specific outbound
adapters, arbitrary HTTP methods/credential locations, generic credential
fetch, direct guest egress, WebSockets, streaming, or arbitrary TLS proxy
configuration.

## Implementation status (2026-08-09)

Implemented foundations include typed immutable placeholder rules, outbound
runtime lease snapshots, route-scoped inbound gateway leases, value-free audit
records, `NetworkMode::BrokerOnly`, a strict HTTPS-only brokered adapter, and
constant-time inbound header rewrite before VM delivery. Outbound use rechecks
the exact lease, active binding, version, destination, and rule before and
after upstream use; it fails closed on rotation or revocation. The adapter
also rejects an upstream response that contains the resolved credential.

Focused domain, broker, secret-adapter, dispatcher, and sentinel tests pass.
A fresh disposable-PostgreSQL secret lifecycle test also passes after the
secret service was corrected to preserve `AuthorizationDenied` and related
typed authorization failures instead of misreporting them as `Persistence`.
The daemon now has a fail-closed operator-pinned HTTPS adapter registry and an
RPC/service path for declaring durable immutable outbound rules. Fresh
PostgreSQL coverage proves rule declaration, exact lease snapshots, matching
authorization, cross-run/rule denial, rotation/revocation, and sentinel
non-disclosure. A real KVM/libkrun guest proves ordinary TCP is unavailable in
`BrokerOnly` while the dedicated broker `AF_VSOCK` carries an actual released
`BrokeredHttpsClient` request: it reads the provisioned one-run runtime
credential, calls the real host `BrokerServer`, and receives only the
sanitized response. The TLS transport has a real handshake-failure injection
test, and OpenFGA/Mélange validation passes.

The daemon-supervised golden now combines the live PostgreSQL runtime resolver,
configured HTTPS rule registry, released guest client, and a CA-pinned fake
HTTPS upstream. It proves substituted request delivery and sanitized response
handling without exposing credentials to the guest. The repository-wide
quality gate also passes.

## Implementation checklist

- [x] **1. Define placeholder-substitution contracts**
  - [x] Extend brokered secret slots and immutable bindings with a validated
    placeholder identity, exact destination/origin, injection direction, and
    bounded HTTP location: an outbound allowlisted header value/prefix or one
    inbound gateway header.
  - [x] Define stable non-secret placeholder generation and delivery to the VM;
    placeholders must not be usable as host credentials or confuse raw-secret
    delivery.
  - [x] Reject ambiguous header matching, unsupported body/query injection,
    duplicate substitutions, wildcard destinations, and substitutions outside
    the exact binding/route/revision.
  - [x] Add parser, normalization, serialization, and mismatch tests.

- [x] **2. Extend runtime authority and secret resolution**
  - [x] Bind exact egress destinations, injection rules, secret versions, and
    gateway-route association into immutable authorization snapshots and
    runtime leases.
  - [x] Reuse the existing opaque runtime credential, brokered-use
    authorization, encrypted host-side resolution, live revocation, and audit
    records without exposing plaintext to the VM.
  - [x] Record placeholder and rule identifiers, but never secret values, in
    logs, queues, traces, provenance, or responses.
  - [x] Add RLS/OpenFGA/Mélange tests for cross-run, cross-slot, cross-route,
    destination-broadening, rotation, and revocation denial.

- [x] **3. Implement forced HTTPS egress**
  - [x] Add a brokered-egress VM network mode that routes all permitted HTTPS
    traffic through the host egress proxy and provides no direct `passt` or
    alternate network path.
  - [x] Enforce exact hostname/SNI/certificate identity, DNS pinning and
    rebinding protection, raw-IP/IPv6 denial, private/metadata-address denial,
    redirect policy, proxy-variable denial, and request/response bounds.
  - [x] Establish the guest trust material or cooperating transport required to
    inspect allowed HTTPS request fields, and prove the proxy's own upstream
    certificate validation remains fail closed.
  - [x] Add real-libkrun networking tests for each direct-bypass and TLS failure
    path.

- [x] **4. Substitute secrets without disclosing them**
  - [x] Implement outbound placeholder replacement only after runtime, lease,
    destination, and TLS checks pass. Forward ordinary HTTP responses without
    semantic provider adaptation.
  - [x] Reauthorize before and after upstream use; rotation/revocation blocks
    new substitution immediately and records honest in-flight behavior.
  - [x] Add sentinel tests proving real values never appear in VM environment,
    files, process arguments, guest memory interfaces, logs, traces, queues,
    responses, alternate destinations, or unbound routes.

- [x] **5. Integrate generic workload and gateway use**
  - [x] Let a release declare ordinary HTTPS API usage through brokered secret
    slots and destination-bound placeholder rules, without naming a provider in
    platform domain types.
  - [x] Publish the inbound-header rule contract for MVP 03 to enforce at the
    GatewayDispatcher, including constant-time comparison and non-match
    handling without an oracle.
  - [x] Add fake generic HTTPS upstreams that prove request/response
    pass-through, allowed substitution, destination denial, rotation,
    revocation, timeout, and cancellation.

- [x] **6. Verify and document**
  - [x] Document the placeholder contract, TLS-interception/cooperating-client
    trust boundary, network mode, raw-delivery distinction, inbound/outbound
    rules, revocation, and residual authorized-destination exfiltration risk.
  - [x] Run `cargo fmt --all -- --check`.
  - [x] Run `cargo clippy --workspace --all-targets --all-features`.
  - [x] Run `cargo test --workspace --all-features`.
  - [x] Run `cargo doc --workspace --all-features --no-deps`.
  - [x] Run real-PostgreSQL, real-libkrun, proxy, TLS, DNS-bypass, and
    failure-injection scenarios.
  - [x] Fix the fresh-PostgreSQL secret authorization diagnostic before using
    it as brokered-rule authority evidence: unauthorized secret creation must
    return `AuthorizationDenied`, not a generic persistence failure.
  - [x] Run secret-sentinel scans, `git diff --check`, and `cargo dev quality`.

## Completion evidence

Record secret-slot/binding/lease and placeholder-rule fixtures, interception
trust fixtures, destination and inbound-route fixtures, direct-bypass/TLS
denial evidence, sentinel-scan results, rotation/revocation timing, test
counts, and deliberate follow-up tasks.
