# MVP-05 E2E acceptance matrix

This matrix defines completion requirements, not claims of passing coverage.
The suite exercises real Hephaestus services and isolated guests, with simulated
inbound identities and deterministic model and relay endpoints. No Telegram
account or live provider delivery is required. Application unit and subsystem
integration tests support these cases but do not replace the joined E2E proof.

On 2026-09-07 the user split the exhaustive
[host-daemon crash matrix](../../tasks/todo/complete-host-daemon-crash-recovery-matrix.md)
and [expanded adversarial isolation matrix](../../tasks/todo/complete-adversarial-isolation-e2e-matrix.md)
into follow-up tasks. They are not MVP-05 blockers. Existing executable crash,
denial, isolation, rotation/revocation, retirement and confinement assertions
remain required; demonstrated security defects remain blockers.

| Case | Trigger | Required observable outcome |
| --- | --- | --- |
| Build and install | Submit canonical example sources through ordinary forge/build/release operations | Isolated builds, immutable releases and authorized installation retain exact source, build, artifact and policy provenance; no seeded release/build rows substitute for these operations. |
| Blog artifact | Approve a recipe and build with pinned Hugo | Authorized retrieval of an immutable HTML artifact containing the approved recipe. |
| Valid ingress | Alice and Bob submit simultaneous valid updates | Both acknowledgements, normalized durable events and serialized state effects; exact provenance for each run. |
| Invalid ingress | Missing/invalid verification, unknown identity, malformed or oversized input | Specified 401/403/400 responses with no mailbox, state, Git or outbound effects. |
| Replay | Repeat ingress and broker delivery | One logical recipe; physical attempts and durable deduplication/retry outcomes remain distinguishable. |
| Restart | Stop agent and supervisor, then submit later work | Recovery from persistent state and delivery records without process-local memory. |
| Git conflict | Competing proposals and a changed canonical branch head | Frozen input commits retained; explicit conflict and authorized resolution without lost recipes or widened authority. |
| Compatible update | Request v2 while work is active and ingress continues | Run gate closes, old work drains, exclusive migration lease, stable instance/mailbox/route IDs, deferred work dispatched on v2. |
| Failed update | Explicit hook rollback or abnormal hook termination | Correct runnable or paused compatibility-unknown state; authorized recovery and retained history. |
| Denied authority | Run existing executable invalid-ingress, outsider and adversarial checks | Preserve exact denial and no-unauthorized-effect assertions; expanded isolation coverage and missing positive controls belong to the separate adversarial task. |
| Rotation and revocation | Rotate fixture credentials or revoke authority during operations | Later operations select appropriate exact versions; bounded in-flight semantics and preserved historical provenance. |
| Outbound failure | Deterministic rejection, interrupted call or lost response | Bounded failure/retry/uncertain outcome, durable evidence and explicit recovery; no unsupported exactly-once delivery claim. |
| Guest crashes and abnormal updates | Run existing guest-crash, broker-response-loss, rollback and abnormal-update cases | Preserve exact retry, logical-effect and recovery assertions; exhaustive host-daemon interruption coverage belongs to the separate crash task. |
| Resource retirement | Tombstone/revoke attachment, release, route, grant or secret | New unauthorized work denied; authorized historical inspection retained. |
| Secret confinement | Scan fixture sentinel values across storage and execution/evidence surfaces | No raw secret exposure in unauthorized database/event views, logs, traces, metrics, guest files/env/arguments, or browser evidence. |
| Browser journey | Install, bind, operate, approve, inspect, deny, update and recover through management UI | User-visible controls and outcomes agree with durable platform state; redacted browser evidence retained. |
| Runner and CI | Execute from a prepared clean checkout locally and in CI | Same documented entry point, explicit prerequisites, bounded execution, isolated resources, verified cleanup and retained diagnostics. Missing required capabilities fail rather than skip. |

Implementation sequencing and recorded verification remain in the
[completed acceptance task](../../tasks/done/mvp-05.1-complete-cooking-acceptance.md).
Each completed case must link executable assertions and verification evidence.
CI success requires execution of its declared cases; an opt-in test returning
early is not acceptance evidence.

## Fault boundary inventory and ownership

The inventory below retains the full intended coverage; it is not a claim that
all rows are MVP-05 blockers or already implemented. Existing executable guest
and response-loss checks remain in MVP-05. The linked host-daemon task owns
exhaustive process-interruption coverage for ingress commit, dispatch, result
import/publication, update completion, activation and cleanup. Shared boundaries
such as update hooks retain their existing MVP-05 checks while additional host
interruption cases are deferred explicitly.

Inject faults at observable boundaries using test-controlled barriers or process
termination. Do not use arbitrary sleeps as proof that a transaction has reached
its boundary. Each case must record the triggering event/run and the state seen
before and after recovery.

| Boundary | Required recovery assertion |
| --- | --- |
| Ingress before durable commit | No acknowledged accepted event without durable publication; retry of the same update can establish one event. |
| Ingress after commit, before response | Client may see failure; retry resolves to the existing publication and does not create another logical event. |
| Dispatch before/after attempt persistence | Restart/redelivery preserves the original event and records physical attempts; no concurrent ownership of the same state volume. |
| SQLite transaction before commit | Interrupted changes roll back; retry reconstructs pending recipe work from the event. |
| SQLite commit before broker call | Retry reuses the durable recipe identity and context rather than creating another recipe. |
| Model response before local persistence | A repeated physical model call is permitted; one durable logical recipe and validated output remain. |
| Relay commit before response/local persistence | Caller records failure or uncertainty; retry with the same key resolves to one deterministic relay ledger entry. Conflicting payload reuse is rejected. |
| Recipe marked proposal-ready before result import | Application state must not claim canonical publication; recovery preserves an inspectable result or failure and permits explicit resolution. |
| Result import before approval | Canonical Git remains unchanged until authorized approval; recovery preserves exact input and result commits. |
| Update hook before/after application commit | Explicit application rollback and abnormal termination remain distinct; uncertain compatibility pauses work until authorized recovery. |
| Revision activation | At most one active revision; historical hook decisions persist and deferred events select their revision at dispatch. |
| Cleanup | Restart reconciles orphaned runtime resources and leases; later work can proceed only after exclusive ownership is restored. |

These are assertions to verify against the real stack in their owning task.
Deferred cases remain unverified until that task records evidence. A discrepancy
is a test or platform defect to investigate, not a reason to relabel a failed or
unexecuted case as passing.
