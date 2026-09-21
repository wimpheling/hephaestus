# Reference session chat protocol v1

This directory defines a small release-owned protocol for the MVP-06 reference
chat release. It is an example release contract, not a Hephaestus platform
conversation protocol. A distribution adapter may use it after creating a
repository and instance, but the adapter must not move these records into the
core workflow model.

## Repository shape and initialization

Each fresh session has one repository. The release-owned adapter initializes it
on `refs/heads/main` with one baseline commit before asking the runtime to
answer. The baseline commit contains a `session_manifest` and the initial
`participant` records. The platform creates the repository and instance,
binds the symbolic `session` repository capability, and starts the released
adapter; it does not invent an empty-repository transcript commit.

The manifest is a JSON record with this shape:

```json
{
  "protocol": "heph.session-chat",
  "version": 1,
  "record_id": "uuid",
  "kind": "session_manifest",
  "actor": {"id": "release:reference-chat", "role": "release"},
  "participant_id": "release:reference-chat",
  "created_at": "2026-09-21T12:00:00Z",
  "data": {
    "session_id": "uuid",
    "release_id": "release:reference-chat",
    "agent_id": "agent:reference-chat",
    "ref": "refs/heads/main"
  }
}
```

The release may include its own initial agent or human participant records in
that same commit. If it wants a welcome response, it performs a normal
release-owned initial agent run after the baseline exists; the initial run is
not a platform-generated message.

The reference release locks these paths. IDs in path components are
percent-encoded with no safe characters other than the path separators shown:

| Purpose | Exact path |
| --- | --- |
| manifest | `.heph/session/v1/manifest.json` |
| participant declaration | `.heph/session/v1/participants/<encoded-participant-id>.json` |
| human message | `.heph/session/v1/records/human/<encoded-record-id>.json` |
| agent message | `.heph/session/v1/records/agent/<encoded-agent-id>/<encoded-record-id>.json` |
| tombstone | `.heph/session/v1/records/tombstones/<encoded-target-record-id>.json` |
| model context | `.heph/session/v1/context/<encoded-agent-id>/<encoded-key>.json` |
| content metadata | `.heph/session/v1/content/<encoded-content-id>.json` |

The canonical JSON file is the complete record value. Only the release-owned
adapter writes the manifest, participant, tombstone, and content paths. A
human adapter writes human records. The agent runtime is scoped to its own
`.heph/session/v1/records/agent/<encoded-agent-id>/` and
`.heph/session/v1/context/<encoded-agent-id>/` subdirectories; it cannot use
another agent's subdirectory or any human/manifest/participant path. This is a
release path contract; the host still enforces the runtime capability.

The initialization commit contains the manifest followed by participant files
in the locked path order. Each ordinary human, agent, or tombstone commit
contains exactly one changed record path. A fork setup commit contains exactly
one new manifest path. The adapter rejects a commit whose changed protocol
paths are not unique and UTF-8 bytewise sorted.

## Records and participants

Every record has `protocol`, exact integer `version` 1, a canonical UUID
`record_id`, `kind`, an actor `{id, role}`, a `participant_id`, and a UTC
RFC3339 `created_at`. Record IDs are immutable. IDs use these bounded forms:

| Role | ID form | May write |
| --- | --- | --- |
| release | `release:<key>` | manifest, participant, tombstone |
| agent | `agent:<key>` | assistant message |
| human | `user:<uuid>` | user message |

Participant records are release-written declarations. A participant's role is
protocol identity, not a platform grant or inherited authority. The release
must reject records from an unregistered participant or with a role that does
not match the declared ID form.

The Git author, commit identity, and JSON actor fields are presentation claims;
they are not an authenticated signature. The host's trusted Git receive
attribution must be recorded and correlated separately by the distribution
adapter. This example library validates protocol shape and participant
declarations, but it does not authenticate a writer. A human repository writer
with broad Git write access could therefore forge an assistant-shaped record;
the runtime's path-only capability limits released-agent writes but does not
turn transcript records into signed security evidence.

The visible transcript kinds are:

* `user_message`: actor and participant are a human, with one bounded content
  value.
* `assistant_message`: actor and participant are an agent, with one bounded
  content value, `in_reply_to` naming an existing user-message record, and a
  required UUID `correlation_id` for the response/run.
* `tombstone`: release-written record with `tombstone_of` naming a prior
  user/assistant record. A tombstone removes that record from the release's
  view while Git history retains both records.

The initialization-only kind is `participant`; `session_manifest` appears in
the baseline and in the single fork setup commit. Unknown
kinds, unknown protocol IDs, unsupported versions, duplicate record IDs, bad
UUIDs, unbounded fields, and malformed content are rejected. Text is UTF-8 and
non-empty, at most 16 KiB. A content reference carries an immutable
`content:<uuid>` ID, media type, byte length at most 1 MiB, and a SHA-256 hash;
the referenced bytes live in a release-owned content store.

## Ordering, responses, and concurrent writers

The transcript order is the first-parent Git history of `refs/heads/main`,
then the UTF-8 bytewise path order of changed record files within each commit.
Every commit writes each protocol path at most once, and the release adapter
must stage changed paths in that same sorted order. A record's timestamp is
descriptive and never replaces Git order. The release adapter displays the
ordered reachable records after applying tombstones.

A human append names its expected parent commit. If another human commit wins
the ref first, the stale writer retries against the new head and reconciles its
record by appending it there. It first checks the reachable history by
`record_id`; a repeated identical record is a no-op, while a different payload
with the same ID is an error. This preserves both concurrent human records.

An assistant response is idempotent per `in_reply_to`: only one response may
become visible for a user record. A duplicate with the same payload returns the
existing response; a different payload is an idempotency conflict. Agent
publication also names an expected parent and a `run_id`. If the runtime
reports a stale expected parent, that run is spent and publication fails with a
retry-required result. The agent must start a new run, reread/reconcile the
current history, and publish with the new run's expected parent. The release
must never bypass the runtime's stale-parent check by force-updating a ref.

## Forks, retention, and model context

A fork copies the reachable Git history and appends a new release-owned
`session_manifest` commit on its own `refs/heads/main`; this is the one
post-baseline exception to the baseline initialization rule. The fork keeps the old records and
tombstones for inspection, but receives a new session ID and an empty platform
authority context. No capability, grant, cookie, runtime lease, or other
platform authority is inherited merely because a Git commit is reachable.

Retention is represented by release-written tombstones. The release's view
omits tombstoned records; Git retains the original record and tombstone for
audit and fork history. A release may define additional retention policy, but
it must state whether a view hides a record or whether a new fork keeps it.

Model context is separate release-owned internal state. The adapter may store
bounded context entries under a declared context path such as
`.heph/session/v1/context/<agent_id>/`; those entries are not transcript
records, are not rendered as user/assistant messages, and do not grant access
to another session. The reference library exposes this separation through its
context store.

## Compatibility

The protocol identifier is `heph.session-chat` and the current version is
exactly `1`. A release advertises the versions it can read before opening a
session. Version 1 readers accept only this identifier and version 1; they
reject a future version rather than guessing field meaning. A compatible
version may add release-owned records only after defining their validation and
view behavior. Changing identity, ordering, correlation, authority, or
tombstone semantics requires a new protocol version and an explicit migration
or fork policy.

The library and tests below model these rules in memory. They do not claim
browser, VM, model, or Hephaestus integration acceptance.

## Reference release agent

`agent.toml` packages the ordinary `reference-session-chat` release. It uses
`publication.mode = "runtime_git"` with one required symbolic `session`
repository capability. The capability reads `refs/heads/main`, requires
fast-forward `update_ref`, and permits writes only below the agent record and
agent context namespaces. `exact_parent_required = true` keeps a runtime run
from publishing against a ref that changed after its immutable snapshot. The
guest has `broker_only` networking and no legacy workspace mount.

`agent.py` reads the platform-provided run context and model rule parameter,
opens the ordinary Git checkout through `git_adapter.py`, and resolves every
pending human record visible at the expected parent. It sends each model
request through the existing framed brokered-egress ABI on vsock port 19001,
using the non-secret placeholder `heph-placeholder:v1:<rule-id>` in the
request body. It has no direct HTTPS fallback and never prints the broker
credential. Responses and the latest bounded internal model context are
committed together once and pushed once. A stale expected parent fails before
the commit; a fresh runtime run must reread the history and retry.

The manifest and agent are ordinary release packaging examples. The focused
tests exercise the real local Git adapter, batching, private broker wire and
sanitized response envelope, stale-parent rejection, and
no-fabricated-response behavior. The broker test uses a local stream and does
not contact a model provider; the composed production VM, model, browser, or
GCP journey remains unclaimed.
