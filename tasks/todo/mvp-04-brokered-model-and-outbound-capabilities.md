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

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`manage-delegate-and-deliver-secrets.md`](../done/manage-delegate-and-deliver-secrets.md)
- [`define-own-the-loop-agent-platform.md`](define-own-the-loop-agent-platform.md)

## Non-goals

This task does not add an LLM abstraction, model policy, prompts, model
adapters, token/cost budgets, Telegram semantics, provider-specific outbound
adapters, arbitrary HTTP methods/credential locations, generic credential
fetch, direct guest egress, WebSockets, streaming, or arbitrary TLS proxy
configuration.

## Implementation checklist

- [ ] **1. Define placeholder-substitution contracts**
  - [ ] Extend brokered secret slots and immutable bindings with a validated
    placeholder identity, exact destination/origin, injection direction, and
    bounded HTTP location: an outbound allowlisted header value/prefix or one
    inbound gateway header.
  - [ ] Define stable non-secret placeholder generation and delivery to the VM;
    placeholders must not be usable as host credentials or confuse raw-secret
    delivery.
  - [ ] Reject ambiguous header matching, unsupported body/query injection,
    duplicate substitutions, wildcard destinations, and substitutions outside
    the exact binding/route/revision.
  - [ ] Add parser, normalization, serialization, and mismatch tests.

- [ ] **2. Extend runtime authority and secret resolution**
  - [ ] Bind exact egress destinations, injection rules, secret versions, and
    gateway-route association into immutable authorization snapshots and
    runtime leases.
  - [ ] Reuse the existing opaque runtime credential, brokered-use
    authorization, encrypted host-side resolution, live revocation, and audit
    records without exposing plaintext to the VM.
  - [ ] Record placeholder and rule identifiers, but never secret values, in
    logs, queues, traces, provenance, or responses.
  - [ ] Add RLS/OpenFGA/Mélange tests for cross-run, cross-slot, cross-route,
    destination-broadening, rotation, and revocation denial.

- [ ] **3. Implement forced HTTPS egress**
  - [ ] Add a brokered-egress VM network mode that routes all permitted HTTPS
    traffic through the host egress proxy and provides no direct `passt` or
    alternate network path.
  - [ ] Enforce exact hostname/SNI/certificate identity, DNS pinning and
    rebinding protection, raw-IP/IPv6 denial, private/metadata-address denial,
    redirect policy, proxy-variable denial, and request/response bounds.
  - [ ] Establish the guest trust material or cooperating transport required to
    inspect allowed HTTPS request fields, and prove the proxy's own upstream
    certificate validation remains fail closed.
  - [ ] Add real-libkrun networking tests for each direct-bypass and TLS failure
    path.

- [ ] **4. Substitute secrets without disclosing them**
  - [ ] Implement outbound placeholder replacement only after runtime, lease,
    destination, and TLS checks pass. Forward ordinary HTTP responses without
    semantic provider adaptation.
  - [ ] Reauthorize before and after upstream use; rotation/revocation blocks
    new substitution immediately and records honest in-flight behavior.
  - [ ] Add sentinel tests proving real values never appear in VM environment,
    files, process arguments, guest memory interfaces, logs, traces, queues,
    responses, alternate destinations, or unbound routes.

- [ ] **5. Integrate generic workload and gateway use**
  - [ ] Let a release declare ordinary HTTPS API usage through brokered secret
    slots and destination-bound placeholder rules, without naming a provider in
    platform domain types.
  - [ ] Publish the inbound-header rule contract for MVP 03 to enforce at the
    GatewayDispatcher, including constant-time comparison and non-match
    handling without an oracle.
  - [ ] Add fake generic HTTPS upstreams that prove request/response
    pass-through, allowed substitution, destination denial, rotation,
    revocation, timeout, and cancellation.

- [ ] **6. Verify and document**
  - [ ] Document the placeholder contract, TLS-interception/cooperating-client
    trust boundary, network mode, raw-delivery distinction, inbound/outbound
    rules, revocation, and residual authorized-destination exfiltration risk.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run real-PostgreSQL, real-libkrun, proxy, TLS, DNS-bypass, and
    failure-injection scenarios.
  - [ ] Run secret-sentinel scans, `git diff --check`, and `cargo dev quality`.

## Completion evidence

Record secret-slot/binding/lease and placeholder-rule fixtures, interception
trust fixtures, destination and inbound-route fixtures, direct-bypass/TLS
denial evidence, sentinel-scan results, rotation/revocation timing, test
counts, and deliberate follow-up tasks.
