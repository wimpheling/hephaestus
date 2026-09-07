# Deterministic cooking journey

Start here for the MVP-05 example. This directory contains the applications,
the automated scenario, its acceptance specification, and the run command.

| File or directory | What it contains |
| --- | --- |
| [SCENARIO.md](SCENARIO.md) | Full MVP-05 acceptance specification and remaining checklist. |
| [TEST-MATRIX.md](TEST-MATRIX.md) | Required E2E triggers and observable outcomes, including local/CI execution. |
| [Remaining-work task](../../tasks/in-progress/mvp-05.1-complete-cooking-acceptance.md) | Sequenced implementation and verification needed to finish MVP-05. |
| [run.sh](run.sh) | Command to run the real-stack automated example. |
| [CI.md](CI.md) | Dedicated KVM workflow, runner configuration and retained diagnostics. |
| [tests/scenario.rs](tests/scenario.rs) | Executable scenario: fixture setup, inbound requests, cooking run, Git result and Hugo checks. Start with `exercise`. |
| [tests/inspection.rs](tests/inspection.rs) | Authenticated provenance queries, denied queries and proposal approval. |
| [tests/updates.rs](tests/updates.rs) | Draining, migration, rollback, recovery and model-credential rotation assertions. |
| [tests/blog_artifact.rs](tests/blog_artifact.rs) | Exact-source blog build, immutable publication and authenticated HTML retrieval. |
| [tests/confinement.rs](tests/confinement.rs) | Raw application-table credential scan with schema coverage checks. |
| [cooking-gateway/](cooking-gateway/) | Released Rust handler: verification placeholder, two-user policy and normalized mailbox publication. |
| [cooking-agent/](cooking-agent/) | Released Python agent: SQLite memory, broker calls, recipe output and update hook. |
| [telegram-relay/](telegram-relay/) | External relay application with deterministic Telegram transport. |
| [cooking-blog/](cooking-blog/) | Hugo source, templates and source checker. |

The Rust scenario modules are compiled by the shared
[`golden` integration test](../../crates/hephaestus-app/tests/golden.rs).
That test provides daemon setup and selects this scenario when `run.sh` sets
`HEPHAESTUS_APP_COOKING_E2E=1`. The shared
[Caddy/libkrun launcher](../../scripts/run-gateway-libkrun-e2e.sh) owns the
temporary infrastructure. These shared platform helpers remain outside examples.

This terminal acceptance test sends concurrent simulated Alice/Bob requests,
replays ingress, restarts the supervisor, and submits a later Alice request. It
runs the actual applications, inspects the results, approves Alice's initial
proposal, and deletes its temporary services and data. Model and relay responses
are deterministic; no real Telegram message is sent.

The cooking fixture runs the application-owned gateway and cooking agent in
separate network-disabled libkrun guests, with Caddy ingress, PostgreSQL
authority and durable mailbox storage, NATS dispatch, persistent SQLite state,
brokered HTTPS, and controlled Git results.

The application sources live in this directory and are tracked by Hephaestus.
Override `HEPHAESTUS_COOKING_SOURCE_ROOT` only to select another complete example
checkout with the same application directory names. The gateway
is compiled for `x86_64-unknown-linux-musl`; the Python agent is imported byte
for byte. The selected Python image must provide `/usr/local/bin/python3`
with SQLite support. Both guests execute immutable imported release artifacts.

```sh
# From the Hephaestus repository root:
# Set this to the local digest reference printed by provision-builder-image.sh.
HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE='localhost/python-ubuntu@sha256:<manifest-digest>' \
HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE='localhost/rust-ubuntu@sha256:<manifest-digest>' \
  examples/cooking/run.sh
```

The first image above is the locally provisioned platform Python image and the
second is the digest-pinned Rust builder image. Provision both first or supply
other exact compatible digests. The wrapper requires the same
non-root KVM, delegated cgroup, Podman, Rust musl target and host tools as
`scripts/run-gateway-libkrun-e2e.sh`. It creates disposable PostgreSQL, NATS,
Caddy, guest filesystems and application workspaces, and cleans them up on exit.
The real build path also requires prepared OCI builder, verifier and base-image
inputs from the [repository-image workflow](../../docs/repository-image-builds.md).
Its enabled `repository-images/workflow.env` lives under `HEPHAESTUS_LOCAL_ROOT`
(default `.local/hephaestus`). Missing reviewed inputs fail preflight. The blog
image must pass the platform's independent vulnerability policy before its Hugo
build can run.

To prepare a reviewed Python or Rust builder from the canonical platform
Dockerfile, run the helper with immutable provenance inputs:

```sh
examples/cooking/provision-builder-image.sh \
  --builder python-ubuntu \
  --source https://forge.example/hephaestus \
  --revision "$(git rev-parse HEAD)" \
  --created 2026-09-06T12:00:00Z
```

It writes an OCI archive, loads it into the local Podman store, and prints its
exact manifest digest plus a usable local digest reference. A CI job can use
that reference on the same runner. Another runner can prepare the same reviewed
builder, load a transferred OCI archive, or use a published image through the
reviewed platform-image operation. The browser acceptance journey runs by
default. Set `HEPHAESTUS_COOKING_BROWSER_E2E=0` only when deliberately running
the non-acceptance build and VM slice. Set `HEPHAESTUS_COOKING_TIMEOUT_SECONDS` to
change the default 900-second deadline; the deadline includes preflight,
compilation and guest execution, followed by up to 30 seconds of cleanup grace.
Set `HEPHAESTUS_COOKING_DIAGNOSTICS_DIR` to an absolute directory to retain
the execution log and, on failure, host diagnostics. Logs have mode `0600`;
the wrapper redacts the known fixture sentinels and credential header patterns.
This redaction does not replace the acceptance suite's secret-confinement checks.

For application-only tests (no VMs, services or credentials):

```sh
(cd examples/cooking/cooking-gateway && cargo test --all-features)
(cd examples/cooking/cooking-agent && python3 -m unittest -v)
```

The Python unit suite optionally accepts `COOKING_HUGO_BINARY`, the absolute path
of a verified Hugo executable; the application CI job supplies it. A host Hugo
check is supporting coverage and does not replace the E2E's isolated image,
build and artifact assertions. Successful real-stack output ends with
`daemon golden E2E passed; runtime and cgroup cleanup verified`.

The focused update-admission regression can run without KVM:

```sh
scripts/test-update-admission-postgres-nats.sh
```

It owns pinned PostgreSQL/NATS containers and exercises the active-run admission,
reconciler race, revoked-actor recovery and explicit missing-feature failure.
The ordinary CI job runs this regression separately from the cooking E2E.

The scenario checks empty `401` authentication failures, a released-policy
`403` for an unknown identity, `400` for malformed/oversized input, and `200`
acknowledgements for Alice and Bob. Five logical recipe requests produce
five durable events, non-overlapping state leases, and separate model
and relay calls through exact broker rules. A duplicate Alice update produces
no second event. A later Alice request after graceful supervisor restart uses
her persisted SQLite context. Competing Git proposals retain their frozen input
commit; only the explicitly approved result changes canonical Git. Bob's stale
approval records a conflict without moving the canonical ref. Malformed model
output and a lost relay response each produce one failed delivery attempt and
one successful retry. Each endpoint receives six physical calls, while the
relay retains five logical ledger entries.

Inspection uses audience-bound authenticated RPCs to resolve each mailbox run,
result, authorization snapshot, exact HTTPS leases/rules/secret versions and
disposition. Outsider and wrong-audience callers are denied. The fixture reports
only opaque provenance identifiers.

The model is a deterministic certificate-verified TLS fixture. The relay call
executes the actual external `telegram-relay/relay.py` application, including
its authentication, validation, SQLite transaction and idempotency behavior,
using its deterministic transport. It sends no message to real Telegram.
Gateway inbound verification uses its own credential/import, separate from
the agent's model and relay authority. Gateway parameters are delivered from
the exact immutable gateway revision in a bounded read-only control mount.

This is an installed-artifact execution proof. The harness seeds immutable
release/build metadata and imports the selected artifacts, following the
existing golden fixture; it does not prove isolated builds of all three source
repositories or completion of the entire MVP-05 checklist. The broader
concurrency/crash/update/recovery matrix, isolated release publication, and
reproducible local and CI execution remain acceptance work. Real Telegram
transport, accounts, Bot API tokens and public Internet deployment are outside
MVP-05 scope; deterministic endpoints exercise Hephaestus capabilities.

## Recorded verification, 2026-09-05

The cooking wrapper passed one real daemon journey and four PostgreSQL/NATS
regressions. The ordinary gateway wrapper also passed its daemon journey and
the four regressions. The optional Hugo step ran with verified Hugo v0.150.1
(binary SHA-256 `1b727f14b57d19f86057310a1a9e0594cddc3ddbb811e5dd9a9ce71c7dbab675`)
and produced `public/recipes/recipe-42/index.html` from the approved commit.
The gateway parameter materialization regression passed with read-only mounts
and a non-writable parameter file. A real PostgreSQL regression advances the
live Git ref and proves runtime context, workspace and review proposal retain
the mailbox attempt's frozen input. Sentinel checks found no fixture credential
in the journey logs or outbox payloads.

These identifiers describe the disposable final run; they are evidence, not
resources that remain installed after fixture cleanup.

| Evidence | Identifier or outcome |
| --- | --- |
| Route | `2e1ce551-7985-4a3d-8231-1954f5af4847` |
| Gateway revision | `b604910b-a208-4b21-a644-ec575b0706ab` |
| Mailbox | `36441a8b-729a-4561-917e-4b9201e1711d` |
| Event | `d8f69d9f-ff04-40c2-96a7-5de5c0b5c092` |
| Run / authorization snapshot | `417922be-fb60-49d2-9962-37a32db83092` |
| Cooking revision | `ddf12f57-967c-4e74-8769-f0f4e1defa0c` |
| State lease | `e346d3ac-c8f3-4a89-8d27-7e4ea022f6e6`, fencing token `2` |
| Dispatch / disposition | Sequence `1`, `delivered` |
| Model secret version | `b5bf8c30-940d-4e3e-a488-661d9588a34a` |
| Relay secret version | `e9ddeb21-1f61-4b3d-9f88-b3e6ce8a7a0c` |
| HTTPS audit | Two exact requests, each with authorization and substitution records |
| Approved canonical result | `1ba8649a6dcf664820dd2d51c0af63498690e20d` |

Application artifact/source SHA-256 digests (relay executes externally):

```text
cooking-agent.py  c78ff489e56ecc74434ad4a77726ec9da31e7c9755499b9e7fa4138f81945a69
cooking-gateway   9827a91c0810213e4e371bede93ae25738491477235d30e5ab6afd3e179cacc5
relay.py          e37bd83b6be0741d4e6506839d451e2eaf3384b83616917e39f0bad2f10fd018
```
