# MVP-05.1: Complete cooking acceptance

Owner: codex

## Outcome

Complete the remaining [cooking acceptance specification](../../examples/cooking/SCENARIO.md)
using the [existing example](../../examples/cooking/README.md). Extend the
verified first-request journey into a reproducible build/install, multi-request,
upgrade/recovery, security and real-Telegram proof. This task is not an instruction
to start those implementations during the current documentation update.

## Verified baseline

Commit `c5c7a8b` contains the applications and installed-artifact proof. The
real-stack test executes one request through Caddy, separate gateway and agent
VMs, PostgreSQL/NATS, SQLite, brokered model/relay calls, controlled Git proposal
and approval, authenticated provenance queries and optional real Hugo output.
Application unit tests cover replay/restart, the framed broker protocol and
SQLite migration/rollback. The full quality gate passed after consolidation.

The harness seeds release metadata. It does not prove isolated application
builds, ordinary installation, real Telegram delivery, browser operation or
the entire failure/update matrix. Existing generic subsystem tests are useful
prerequisites but do not close the cooking-specific acceptance requirements.

## Feature assessment and boundaries

| Area | Current assessment |
| --- | --- |
| Core execution, releases, mailboxes, authority, secrets, controlled Git and inspection | Implemented foundations with a passing joined first-request proof. No additional broad core subsystem is currently established as necessary. |
| Build/install and Hugo artifact lifecycle | Existing primitives; the example's actual workflow and artifact delivery still need exercising. Any missing public operation must be demonstrated, then implemented narrowly. Seed SQL is not a substitute for product installation. |
| Concurrency, updates, recovery and revocation | Existing mechanisms and focused tests; complete cooking integration remains unverified. Tests may expose further platform defects. |
| Real Telegram relay | Known missing application implementation: relay.py currently uses only deterministic transport. External deployment and real account configuration are also absent. |
| Browser journey | Cooking-specific automation and evidence are missing. Establish actual gaps in existing management pages before adding UI features. A custom chat UI is not required. |

Keep Telegram parsing, family policy, model behavior, SQLite schema and relay
delivery in the example applications. Keep authority, resource ownership,
isolation and exact provenance in Hephaestus. Do not add persistent gateway
service mode, a generic workflow engine, release-owned custom UI hosting or
remote phone-controlled development as prerequisites for this bounded scenario.
The relay runs externally; the existing short-lived gateway contract suffices.

## Remaining work, in execution order

- [ ] **1. Exercise real builds and installation**
  - [ ] Provide repeatable preparation of separate forge source repositories
    from the example folders, preserving their exact source revisions and
    offline dependencies. Keep example source canonical in this repository.
  - [ ] Push gateway and agent sources through the existing forge/build path,
    run isolated builds, publish immutable releases, and record source/build/
    artifact/policy hashes. Do not directly insert release/build rows for this proof.
  - [ ] Import and configure the releases through supported authorized
    operations; bind distinct gateway/agent principals, mailbox, state, route,
    exact blog ref and the three secret uses. Record the operations and IDs.
  - [ ] Build the blog with the declared pinned Hugo image and retain the
    immutable HTML artifact through the existing release/artifact workflow.
    Verify authorized retrieval; the temporary host Hugo check is insufficient
    evidence for that workflow. Do not silently introduce a new hosting service.
  - [ ] Publish v1 and a compatible v2 candidate with the actual update hook,
    plus a deliberate rollback candidate, using ordinary release operations.

- [ ] **2. Extend deterministic multi-request operation**
  - [ ] Send concurrent Alice/Bob updates through the actual gateway and verify
    both acknowledgements, normalized envelopes and serialized state effects.
  - [ ] Replay ingress and broker deliveries; inspect SQLite, relay ledger and
    Git proposals to distinguish one logical recipe from retried physical calls.
  - [ ] Stop/restart the agent and then the supervisor, submit later work, and
    prove recovery uses persisted state and delivery records alone.
  - [ ] Exercise competing recipe proposals and stale branch heads; preserve
    exact input commits and explicit conflicts rather than silently discarding
    another recipe or widening Git authority.

- [ ] **3. Prove upgrade and recovery through the platform**
  - [ ] Close the v1 run gate during update, accept simultaneous incoming
    events, drain existing work and run the v2 hook under the exclusive lease.
  - [ ] Verify the migrated recipes/schema, activation, unchanged instance/
    mailbox/route identities, and deferred work selecting v2 only on dispatch.
  - [ ] Exercise explicit application rollback and abnormal termination;
    verify the expected runnable versus paused compatibility-unknown states.
  - [ ] Recover through authorized operator/API operations and verify all
    historical runs, revisions, leases, update decisions and results remain
    inspectable. Do not claim host rollback of application-owned state.

- [ ] **4. Complete authority and crash coverage**
  - [ ] Run adversarial gateway and cooking releases against the exact denied
    resources/operations in SCENARIO.md, including state, repository, mailbox,
    destination, direct networking/Git, grants and Caddy administration.
  - [ ] Rotate inbound, model and relay credentials and revoke authority
    during active operations; verify bounded denial, later-version selection
    and historical exact-version inspection without retroactive authority.
  - [ ] Inject crashes around ingress commit, dispatch, SQLite commit, model/
    relay calls, result import, update hook, activation and cleanup. Record
    expected retry, conflict, uncertain outcome or recovery for each boundary.
  - [ ] Tombstone/revoke attachment, release, route, grant and secret resources;
    verify retained authorized history and denial of new unauthorized work.
  - [ ] Scan database/event storage, logs, traces, metrics, guest files/env/
    arguments and browser evidence for secret sentinels. Existing log/outbox
    checks cover only part of the required surfaces.

- [ ] **5. Implement and exercise real Telegram transport**
  - [ ] Implement the external relay's real HTTPS Telegram adapter with bounded
    request/response handling, external token custody and redacted diagnostics;
    retain deterministic transport for automated tests.
  - [ ] Specify and test lost-response/ambiguous-send handling. A local SQLite
    idempotency key alone cannot promise exactly-once delivery across an
    external send and a crash before recording its outcome.
  - [ ] Configure a persistent test installation, two actual family identities,
    the external relay and public HTTPS ingress scoped to the gateway route.
    Keep development OIDC, database, NATS and administration private.
  - [ ] Obtain the user's actual deployment/account choices and explicit
    authorization before configuring external services or sending Telegram
    messages. No real credentials are needed for workstreams 1–4.
  - [ ] Run the two-user Telegram smoke and independent Bot API token rotation;
    retain redacted evidence and verify the raw token never enters a guest.

- [ ] **6. Browser acceptance and final evidence**
  - [ ] Extend the existing Playwright management journey for cooking
    installation/binding, operation, approval, provenance, denial, update and
    recovery. Reuse current pages and APIs; record concrete missing controls
    before implementing any UI extension.
  - [ ] Capture redacted screenshots, exact source/release/runtime IDs, commands,
    counts, expected failures and recovery decisions in the example docs.
  - [ ] Update SCENARIO.md checkboxes only with matching verification evidence;
    document any agreed scope change explicitly instead of dropping assertions.
  - [ ] Run Rust formatting, workspace all-target/all-feature Clippy,
    workspace all-feature tests and rustdoc, then `cargo dev quality`.
  - [ ] Run the explicit real-stack cooking, fault/update, Telegram and browser
    scenarios; record optional test gates as executed or skipped, not simply green.
  - [ ] Run `mix precommit`, Mélange drift/doctor and OpenFGA compatibility
    checks required by SCENARIO.md, and the complete sentinel scan.
  - [ ] Run `git diff --check` for project-authored changes; preserve vendored
    dependency bytes and their Cargo checksums when upstream whitespace differs.

## Completion evidence

Baseline links: [scenario and recorded IDs](../../examples/cooking/README.md),
[application checks](../../examples/cooking/cooking-agent/VERIFICATION.md),
[authorized run inspection](../../docs/run-provenance-inspection.md).

Append evidence by workstream as it completes. Keep this task in progress until
the remaining acceptance requirements are met or explicitly rescoped by the
user. New generic platform omissions discovered while exercising this task
belong here when needed for acceptance; unrelated expansion belongs in separate tasks.
