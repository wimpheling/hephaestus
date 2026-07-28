# Minimal Git forge core

Phase 2 accepts authorized Git smart-HTTP traffic, stores bare repositories,
records accepted receives, parses the exact received `agent.toml`, and commits
idempotent run commands through the shared transactional outbox.

## Crates

- `forge-domain`: opaque forge identifiers and validated Git values.
- `agent-config`: versioned `agent.toml` types, parsing, diagnostics, hashing,
  validation, and trigger matching.
- `forge-service`: canonical bare storage, PostgreSQL metadata, exact-commit
  `gix` inspection, receive processing, and JetStream outbox publication.
- `git-http`: Axum routes, authorization, and the bounded streaming
  `git-http-backend` adapter.

The principal, repository ID, operation, receive ID, and accepted refs are
included in structured tracing. `GitAuthorizer` is called before every
discovery or transfer. Phase 3 uses verified OIDC identities and the
PostgreSQL/Mélange authorizer; there is no allow-all production adapter.

## Storage

The configured repository root is created and canonicalized during startup.
Every repository path has exactly this form:

```text
<canonical-root>/<repository-uuid>.git
```

HTTP route components must be canonical hyphenated UUID text. Repository names
and caller-provided paths never enter path construction. Existing paths are
rejected if the repository entry is a symlink, resolves to a different path,
or does not contain bare-repository metadata.

## PostgreSQL

Migration `0002_git_forge.sql` adds:

- `projects` and `repositories`;
- `git_receives`, current `git_refs`, and ordered `git_ref_updates`;
- valid or invalid `agent_config_revisions`;
- `run_requests`.

It reuses the Phase 1 `outbox`. Receive audit, config revisions, run requests,
and publication intent commit in one transaction. A run request is unique on:

```text
(repository_id, commit_sha, git_ref, config_hash, receive_id)
```

The selected `run_id`, `command_id`, and `agent_id` are persisted with that
tuple. Reprocessing the same receive returns the original request and does not
append another start event.

## Native smart HTTP

The router exposes these paths beneath `/{repository_id}`:

```text
GET  /info/refs?service=git-upload-pack
GET  /info/refs?service=git-receive-pack
POST /git-upload-pack
POST /git-receive-pack
```

Axum middleware consumes and removes the `Authorization` header before the
request reaches repository authorization or the backend adapter. The backend
must be configured by absolute path. Its command builder calls `env_clear()`
and supplies only an explicit CGI/Git allowlist; `HTTP_AUTHORIZATION`, cookies,
arbitrary `HTTP_*` values, `PATH`, and daemon environment secrets cannot reach
the subprocess. Request and response pack data is streamed through bounded
channels. Configurable byte ceilings and a wall-clock timeout apply to each
transaction. Concurrent receive transactions for one repository are
serialized so before/after ref snapshots cannot overlap.

The HTTP response body remains open until the native process exits and all
accepted receive effects commit. A persistence failure terminates the response
stream instead of allowing the client to observe a completed push.

## Agent configuration v1

The initial schema uses these required tables:

```toml
version = 1

[agent]
name = "reviewer"

[guest]
command = "/usr/bin/review"
arguments = ["--format=json"]
working_directory = "/workspace"

[resources]
vcpus = 2
memory_mib = 512

[root_image]
reference = "registry.example/agent@sha256:digest"

[workspace]
mount = true
path = "/workspace/repo"
read_only = true

[state_volume]
enabled = true

[results]
declared_files = ["reports/review.json"]

[network]
profile = "disabled" # or "egress"

[triggers]
push = true
refs = ["refs/heads/*"]
```

Guest paths must be absolute without parent traversal. Declared result files
must be relative, traversal-free regular files outside `.git`. CPU and memory
values are bounded. Trigger refs are fully qualified and may use a terminal
`/*`. Unsupported versions and syntax or validation failures are stored as
structured diagnostics.

## JetStream

- `hephaestus.run.start`: durable `StartRun` commands, consumed by the existing
  run orchestrator.
- `hephaestus.git.receive.accepted`: durable accepted-receive events.
- `hephaestus.git.agent_config.invalid`: durable invalid-config events.

The orchestrator retains its original Phase 1 start subject and also consumes
the forge start subject. Outbox IDs are published as `Nats-Msg-Id`.

## Integration tests

Ordinary tests cover domain validation, configuration parsing, canonical
storage, CGI response parsing, and ref differencing. The native Git transport
and PostgreSQL flow is enabled with:

```sh
HEPHAESTUS_POSTGRES_TEST_URL=postgres://... \
cargo test -p git-http --test smart_http

HEPHAESTUS_POSTGRES_TEST_URL=postgres://... \
HEPHAESTUS_NATS_TEST_URL=nats://... \
cargo test -p forge-service --test postgres
```

The smart-HTTP test performs a real push, clone, and fetch through Axum and
`git-http-backend`, then verifies authorization calls, exact commit inspection,
receive/ref audit, the config revision, and one start-command outbox record.
The forge-service suite additionally verifies invalid diagnostics, receive and
JetStream idempotency, durable publication, command consumption, and a
fake-provider VM reaching the running state.

The daemon-level golden test starts the production composition root and proves
Bearer OIDC authentication, PostgreSQL/Mélange push authorization, native
smart HTTP, receive persistence, outbox delivery, command consumption,
provider-neutral `agent.toml` translation, and a persisted running event:

```sh
HEPHAESTUS_POSTGRES_TEST_URL=postgres://... \
HEPHAESTUS_NATS_TEST_URL=nats://... \
cargo test -p hephaestus-app --test golden
```
