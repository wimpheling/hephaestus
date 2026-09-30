# Gateway contracts

The gateway branch connects a released repository declaration to one bounded
HTTP invocation. [`domain/`](domain/) models route intent, service revisions,
ownership, targets, logs, probes, mailbox publication, and the trusted request
boundary used by the edge and control plane.

Gateway reconciliation resolves an immutable released configuration into a
current route binding and service target. An authenticated UI-origin request
then carries a child session, actor, installation generation, canonical path,
method, and bounded body. The edge rechecks current authority, selects the
route, strips browser credentials before guest dispatch, and validates the
response before returning it to the browser.

The same contracts cover service ownership leases, launch materialization,
bounded log retention, expired-claim recovery, and mailbox publication. The
provider adapters under `heph-std` own listeners, persistence, and process
execution while this branch keeps route and authority decisions explicit.
