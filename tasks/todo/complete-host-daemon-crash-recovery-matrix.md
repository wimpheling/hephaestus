# Complete host-daemon crash recovery matrix

Owner: unassigned

## Outcome

Prove durable recovery at host-daemon interruption boundaries using the real
Hephaestus stack and deterministic cooking fixture. This work was explicitly
split from [MVP-05 acceptance](../done/mvp-05.1-complete-cooking-acceptance.md)
by the user on 2026-09-07. It is not a prerequisite for completing MVP-05.

## Locked decisions

Existing executable guest-crash, retry, rollback and abnormal-update checks
remain in MVP-05. This task adds the exhaustive host-daemon crash matrix; it
does not replace or disable those checks. Real Telegram remains excluded.

Use deterministic barriers with exact event, update and run identities. A
same-instance run, arbitrary sleep, forced database lease expiry, or worker
cleanup signal is not evidence of the intended crash boundary. Recovery must
use production leases, fencing and durable records. Preserve committed outbox
publication and distinguish logical effects from physical retries.

## Existing preparation

External local drafts exist under `/var/tmp/heph-host-integration/`:
`host-integration.patch`, `parent-lifecycle.patch`, and their audit notes.
`/var/tmp/heph-crash-barriers/crash-barriers-rebased.patch` contains barrier
drafts. These are uncompiled preparation, not completion evidence or portable
repository dependencies. Recheck availability and applicability against the
current source; never restore stale whole files over current work.

## Implementation checklist

- [ ] Integrate and compile the daemon-child bootstrap and feature-gated
  barriers, inactive in ordinary production operation.
- [ ] Implement parent orchestration with private bounded barrier records,
  exact stage/target/operation/PID validation, stopped-process verification,
  daemon-PID-only termination and bounded reaping. Preserve descendants until
  orphan recovery evidence has been collected; clean only owned resources.
- [ ] Cover ingress before durable commit: no accepted acknowledgement without
  durable publication; retry can establish exactly one logical event.
- [ ] Cover ingress after commit but before response: retry resolves to the
  existing publication without a second logical event. The existing drafts do
  not yet cover gateway ingress barriers.
- [ ] Cover dispatch before and after attempt persistence: restart/redelivery
  retains the event and attempts without concurrent state-volume ownership.
- [ ] Cover result import preparation and ref publication: preserve exact
  input/result commits and inspectable recovery; canonical Git changes only
  through authorized approval.
- [ ] Cover host interruption around update-hook completion and revision
  activation: retain decisions, at most one active revision, and dispatch-time
  selection for deferred events; pause on uncertain compatibility.
- [ ] Cover cleanup before runtime destruction: restart reconciles orphaned
  resources and leases before later exclusive ownership is granted.
- [ ] Record exact before/after identities, durable states, retries, conflicts
  or operator recovery for every case; explicitly distinguish daemon crashes
  from the existing guest SIGKILL cases.
- [ ] Expose a reproducible focused runner and execute the complete declared
  matrix locally and in KVM CI, with bounded timeouts, cleanup and redacted
  diagnostics. Missing capabilities must fail rather than skip silently.
- [ ] Preserve and rerun the existing cooking acceptance journey.
- [ ] Run `cargo fmt --all -- --check`, workspace all-target/all-feature
  Clippy, workspace all-feature tests, workspace rustdoc, `cargo dev quality`
  and applicable focused checks for changed components.
- [ ] Update the cooking matrix and documentation with commands, per-case
  results and retained evidence before completing this task.

## Completion evidence

No integrated host-daemon matrix pass is recorded. External patch application
checks and guest-crash results do not close this task.
