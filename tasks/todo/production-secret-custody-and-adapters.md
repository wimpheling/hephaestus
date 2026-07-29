# Production secret custody and semantic adapters

Owner: unassigned

## Outcome

Replace development-local secret custody and test adapters with production
providers while preserving the existing provider-neutral encryption, exact
lease, semantic-operation, revocation, and audit contracts.

## Scope

- [ ] Implement an external KMS/HSM-backed `KeyProvider` with versioned key
  references, availability health, authenticated backup/restore exercises,
  rotation, retirement checks, and no plaintext fallback.
- [ ] Evaluate an external vault adapter only if it preserves immutable
  version provenance, target-scoped live grants, non-transitivity, and
  fail-closed runtime reauthorization.
- [ ] Add production semantic broker adapters one operation at a time with
  exact destination allowlists, budgets, rate limits, sanitized responses,
  and provider-specific integration tests.
- [ ] Decide whether a transparent proxy is needed; it must remain opt-in and
  must not become a generic credential-fetch or unrestricted egress path.
- [ ] Add long-lived credential renewal only with bounded lease identity,
  explicit expiry, revocation, rotation, and crash reconciliation.
- [ ] Run real-provider, restore, revocation-latency, sentinel, and disaster
  recovery exercises before enabling any adapter in production.

## Dependencies

- [`manage-delegate-and-deliver-secrets.md`](../done/manage-delegate-and-deliver-secrets.md)
- [`mvp-04-brokered-model-and-outbound-capabilities.md`](mvp-04-brokered-model-and-outbound-capabilities.md)
