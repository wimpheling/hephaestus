# Complete adversarial isolation E2E matrix

Owner: unassigned

## Outcome

Complete adversarial gateway and agent isolation coverage against actual
Hephaestus guests, authority boundaries and reachable fixture services. This
work was explicitly split from
[MVP-05 acceptance](../done/mvp-05.1-complete-cooking-acceptance.md)
by the user on 2026-09-07. It is not a prerequisite for completing MVP-05.

## Locked decisions

Existing executable denial, isolation, rotation/revocation, retirement and
secret-confinement checks remain mandatory in MVP-05. This task owns the
expanded adversarial matrix and missing positive controls. A demonstrated
security defect in the retained journey remains an MVP-05 blocker; this scope
split does not authorize weakened assertions or unresolved known defects.

Exercise real guest attempts against exact owned resources. A host request,
missing executable/repository, unreachable test address or generic timeout is
not proof of guest isolation. Real Telegram accounts and transport are excluded.

## Implementation checklist

- [ ] Inventory existing executable assertions and evidence; identify precisely
  which matrix cells remain unimplemented or unverified without duplicating
  completed cooking checks.
- [ ] Run an adversarial gateway release against cooking state, the blog
  repository, another mailbox and project, route broadening and Caddy
  administration. Assert denial and absence of unauthorized effects.
- [ ] Run adversarial cooking-agent operations against foreign state and
  repositories, undeclared destinations, secret access, authority changes and
  direct canonical Git writes. Retain exact run/resource identities and
  inspectable denial outcomes.
- [ ] Establish a reachable fixture endpoint with a successful authorized
  guest network control, then prove Disabled/BrokerOnly restrictions from the
  corresponding actual guest context. Do not accept TEST-NET timeouts as proof.
- [ ] Provide meaningful direct-Git and Caddy-administration positive controls
  so missing tools, nonexistent repositories or unavailable services cannot
  masquerade as permission denial. Keep successful controls scoped to owned
  disposable resources.
- [ ] Scope broker substitution/denial and side-effect assertions to exact
  adversarial event/run identities, separate from concurrent canonical work.
- [ ] Preserve retained authorized history, fail-closed authority and the
  existing sentinel scans across storage, logs, traces and guest/browser
  surfaces. Keep credential values out of diagnostics and retained evidence.
- [ ] Expose reproducible focused execution and run the declared matrix on
  the real stack locally and in KVM CI, with bounded waits, owned-resource
  cleanup and explicit executed/failed/missing-case reporting.
- [ ] Preserve and rerun the existing cooking acceptance journey.
- [ ] Run `cargo fmt --all -- --check`, workspace all-target/all-feature
  Clippy, workspace all-feature tests, workspace rustdoc, `cargo dev quality`
  and applicable focused checks for changed components.
- [ ] Update the cooking matrix and documentation with exact coverage,
  commands, positive-control results and denial evidence before completing
  this task.

## Completion evidence

No complete expanded matrix pass is recorded. Earlier external sketches under
`/var/tmp/heph-authority-probes/` included invalid direct-Git and host-network
proofs; they must not be applied or counted as verified coverage.
