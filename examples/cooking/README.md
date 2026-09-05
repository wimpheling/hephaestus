# Deterministic cooking journey

Start here for the MVP-05 example. This directory contains the applications,
the automated scenario, its acceptance specification, and the run command.

| File or directory | What it contains |
| --- | --- |
| [SCENARIO.md](SCENARIO.md) | Full MVP-05 acceptance specification and remaining checklist. |
| [run.sh](run.sh) | Command to run the real-stack automated example. |
| [tests/scenario.rs](tests/scenario.rs) | Executable scenario: fixture setup, inbound requests, cooking run, Git result and Hugo checks. Start with `exercise`. |
| [tests/inspection.rs](tests/inspection.rs) | Authenticated provenance queries, denied queries and proposal approval. |
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

This is a terminal acceptance test, not a persistent browser demo. It sends a
simulated request from Alice, runs the actual applications, approves the result,
and deletes its temporary services and data. Model responses and Telegram
delivery are deterministic; no real Telegram message is sent.

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
HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE='localhost/hephaestus/python-ubuntu@sha256:cf5f70330594d10e4445178f1371ca63fd8e9e4dae54ddd78612796288c26ae9' \
  examples/cooking/run.sh
```

The image above is the locally provisioned platform Python image. Provision it
first or supply another exact compatible digest. The wrapper requires the same
non-root KVM, delegated cgroup, Podman, Rust musl target and host tools as
`scripts/run-gateway-libkrun-e2e.sh`. It creates disposable PostgreSQL, NATS,
Caddy, guest filesystems and application workspaces, and cleans them up on exit.
An optional `HEPHAESTUS_COOKING_HUGO` absolute executable path builds the approved
recipe through a separately verified pinned Hugo binary.

For application-only tests (no VMs, services or credentials):

```sh
(cd examples/cooking/cooking-gateway && cargo test --all-features)
(cd examples/cooking/cooking-agent && python3 -m unittest -v)
```

The Hugo test is optional in both paths. Supply `HEPHAESTUS_COOKING_HUGO` to
`run.sh`, or `COOKING_HUGO_BINARY` to the Python tests, with the absolute path
of a verified Hugo executable. Successful real-stack output ends with
`daemon golden E2E passed; runtime and cgroup cleanup verified`.

The scenario checks empty `401` authentication failures, a released-policy
`403` for an unknown identity, and a `200` acknowledgement for Alice. That
request produces one normalized durable event, a serial stateful cooking run,
one model call and one relay call through their exact broker rules, a Markdown
recipe result against the frozen incoming Git commit, and host-side publication
after an authorized approval RPC. Inspection uses audience-bound authenticated
RPCs to resolve the mailbox run, result, authorization snapshot, exact HTTPS
leases/rules/secret versions and disposition; an outsider and a wrong-audience
caller are denied. The fixture reports only opaque provenance identifiers.

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
repositories or completion of the entire MVP-05 checklist. Real Telegram smoke,
the broader concurrency/crash/update/recovery matrix, and isolated release
publication remain separate acceptance work.

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
