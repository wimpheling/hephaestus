# MVP 04: Brokered model and outbound capabilities

Owner: unassigned

## Outcome

Let released agents invoke a model and send one bounded application response
without receiving provider credentials or bypassing the broker through direct
network access.

This task extends the existing secret broker with semantic, capability-checked
model invocation and Telegram-compatible outbound operations. It preserves
agent ownership of prompts, model-loop policy, and Telegram behavior while the
platform owns credentials, budgets, destination policy, revocation, network
enforcement, and exact usage audit.

## Locked decisions

| Area | Decision |
| --- | --- |
| Ownership | Released code owns prompts, context construction, model selection within allowed policy, Telegram payload semantics, retries, and user experience. |
| Credentials | Provider credentials remain in host-controlled encrypted secret storage and are applied only outside the guest. |
| Authority | A runtime must present an exact opaque credential and bound semantic capability; possession of a secret import alone is insufficient. |
| Network | Broker-only bindings run with guest egress that cannot reach the protected destination by DNS, raw IP, IPv6, redirects, metadata routes, or alternate interfaces. |
| Model access | Model invocation is a platform capability with project-selected provider policy, model allowlists, budgets, rate limits, and usage attribution. |
| Outbound MVP | The first outbound adapter supports the bounded send operation needed by the cooking-agent Telegram journey. Telegram protocol logic does not enter platform core. |
| Responses | Broker responses are bounded and sanitized and never echo authorization material, sensitive provider diagnostics, or secret-bearing headers. |
| Integration | Secret creation, versions, grants, imports, bindings, leases, rotation, and purge remain owned by `manage-delegate-and-deliver-secrets.md`. |

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`mvp-03-event-ingress-and-caddy-routing.md`](mvp-03-event-ingress-and-caddy-routing.md)
- [`manage-delegate-and-deliver-secrets.md`](../done/manage-delegate-and-deliver-secrets.md)
- [`define-own-the-loop-agent-platform.md`](define-own-the-loop-agent-platform.md)

## Non-goals

This task does not add a universal LLM abstraction, prompt templates, memory,
tool selection, web search, browser automation, arbitrary GitHub operations,
a transparent general-purpose TLS interception proxy, raw credential fetch,
or arbitrary third-party protocol adapters.

## Implementation checklist

- [ ] **1. Complete the MVP broker threat model**
  - [ ] **Define protected and residual risks**
    - [ ] Cover prompt injection, compromised released dependencies, raw
      secret reads, DNS and IP bypass, redirects, SSRF, metadata endpoints,
      IPv6, tunneling, logs, state persistence, model context, and malicious
      provider responses.
    - [ ] Distinguish hiding a credential value from constraining the
      operations its upstream authority permits.
    - [ ] Document that an authorized outbound channel can exfiltrate data and
      therefore requires narrow operations, destinations, budgets, and
      bindings.
    - [ ] Define trust assumptions for host broker code, microVM networking,
      Caddy, provider TLS, secret storage, and provider availability.

- [ ] **2. Harden the broker protocol and network boundary**
  - [ ] **Use exact runtime authority**
    - [ ] Accept only a bounded request containing the runtime credential,
      symbolic slot, semantic operation, destination identity, idempotency
      key, and operation-specific body.
    - [ ] Authenticate the exact run, instance or gateway, revision, lease,
      capability ceiling, live binding, destination, operation, expiry, and
      current authorization before any provider call.
    - [ ] Bind every call to an immutable authorization and exact secret
      version provenance record without persisting provider credentials in the
      operation record.
    - [ ] Sanitize all successes, denials, provider failures, retries, and
      timeouts into bounded provider-neutral results.
  - [ ] **Prevent direct protected egress**
    - [ ] Enforce the revision's default-deny network policy and make the
      broker endpoint the only route for broker-only operations.
    - [ ] Prevent bypass using alternate DNS answers, literal IPv4 or IPv6,
      redirects, proxy variables, metadata routes, user-controlled SNI or Host,
      tunneling, and additional guest interfaces.
    - [ ] Fail closed when destination resolution, firewall application,
      broker identity, provider TLS, or policy reconciliation is uncertain.
    - [ ] Add adversarial real-guest tests for every documented bypass path.

- [ ] **3. Add bounded model invocation**
  - [ ] **Define provider-neutral model policy**
    - [ ] Add stable model-policy, provider, allowed-model, budget,
      rate-limit, request, response, and usage-attribution identifiers and
      bounded values.
    - [ ] Bind a release's symbolic model requirement to a project-selected
      policy in the immutable instance revision.
    - [ ] Validate that project selection restricts release requirements and
      current platform policy rather than broadening them.
    - [ ] Keep prompts, messages, tool schemas, and model-loop decisions as
      opaque bounded application payloads except where provider adaptation
      requires parsing.
  - [ ] **Implement one real adapter and one fake upstream**
    - [ ] Implement a provider adapter sufficient for the selected MVP model
      while keeping provider-specific credentials and transport outside the
      guest.
    - [ ] Implement a deterministic fake model service for authorization,
      budget, rate-limit, retry, response-sanitization, and sentinel tests.
    - [ ] Enforce request/response sizes, model allowlists, per-call limits,
      cumulative project/run budgets, rate limits, timeouts, and cancellation.
    - [ ] Record provider, selected model, policy version, token/usage counts,
      cost attribution where available, and outcome without logging protected
      prompt or credential content by default.
    - [ ] Add tests for denied models, exhausted budgets, rotation, revocation,
      retries, duplicated idempotency keys, malformed responses, and provider
      failure.

- [ ] **4. Add the MVP outbound response adapter**
  - [ ] **Support the cooking-agent response operation**
    - [ ] Define a narrow outbound send operation that accepts the bounded
      destination and payload needed by the released Telegram gateway or
      cooking agent.
    - [ ] Require the destination to be derived from an authorized ingress
      conversation binding or an explicitly configured allowlist rather than
      an arbitrary guest-provided identifier.
    - [ ] Apply the exact Telegram credential outside the guest and enforce
      operation, destination, payload, rate, timeout, and retry limits.
    - [ ] Preserve platform-generic broker records while keeping Telegram
      update parsing, message construction, formatting, and user policy in
      released code.
    - [ ] Add a deterministic fake Telegram upstream proving credential
      application, destination binding, rotation, revocation, rate limiting,
      idempotency, and sanitized failures.

- [ ] **5. Audit, observe, and reconcile**
  - [ ] Record runtime, capability binding, secret lease/version, operation,
    destination policy, model policy, request ID, provider outcome, usage, and
    authorization-model version without recording credential values.
  - [ ] Measure permitted and denied operations, provider latency, budget
    exhaustion, rate limiting, revocation latency, retry outcomes, and network
    policy failures.
  - [ ] Reconcile broker calls interrupted before request, after provider
    acceptance, before result persistence, during revocation, and during
    runtime cancellation without claiming exactly-once provider behavior.
  - [ ] Reauthorize live usage subscriptions and redact sensitive request,
    response, destination, and provider fields.
  - [ ] Add sentinel scans across logs, traces, metrics, NATS, PostgreSQL
    non-ciphertext columns, guest-visible files, and captured responses.

- [ ] **6. Verify and document**
  - [ ] Document model and outbound capability contracts, network assumptions,
    budget/rate policy, revocation behavior, residual exfiltration risk, and
    adapter extension rules.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run the real-libkrun, secret broker, fake model, fake Telegram,
    PostgreSQL, NATS, and network-bypass suites.
  - [ ] Run secret-sentinel scans.
  - [ ] Run `git diff --check`.

## Completion evidence

Record broker protocol and policy versions, adapter versions, fake upstream
fixture IDs, credential sentinel values and scan results, network-bypass
evidence, model budget/rate-limit results, rotation/revocation timing, test
counts, and deliberate follow-up tasks.
