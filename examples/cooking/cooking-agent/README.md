# Cooking reference release

The dependency-free Python 3.11+ artifact owns the SQLite cooking loop. Build in
the platform's pinned `python-ubuntu` image with `build.sh`: this copies only the
executable into `/workspace/output/bin/cooking-agent`; no pip or runtime downloads.
The image must contain Python sqlite3, tomllib and AF_VSOCK support. Resolve the
image key to its immutable platform digest when publishing the release.
The artifact shebang uses `/usr/local/bin/python3`, the pinned python-ubuntu
interpreter path, because isolated guest commands start with a cleared environment.
The blog check reuses `sys.executable` and does not depend on PATH.

The guest reads sealed `/run/hephaestus/{context.json,parameters.json,mailbox-body}`,
stores `/var/lib/hephaestus/cooking.sqlite3`, and renders only Markdown into the
host-prepared `/workspace/work` blog checkout. The blog's `python3 check.py` must
pass before success. A zero exit invokes the existing host controlled result
import from the exact context commit; state records `proposal_ready`, never an
unconfirmed canonical publication. Bind only `refs/heads/main` of cooking-blog.

Mailbox JSON is exactly `{provider_update_id:42,user_id:"alice",command:"recipe",
text:"pasta"}` (valid JSON requires quoted keys). The agent validates it again.
Parameters `model_rule_id` and `relay_rule_id` are exact non-secret rule UUIDs.
Broker calls use the mounted ephemeral secret-runtime credential, current run ID,
framed JSON over AF_VSOCK host CID 2 port 19001, and `https_v1`:

| Slot | Exact destination/path | Authorization placeholder |
| --- | --- | --- |
| model | api.model.example/v1/recipes | Bearer heph-placeholder:v1:<model_rule_id> |
| telegram_relay | relay.cooking.example/v1/messages | Bearer heph-placeholder:v1:<relay_rule_id> |

Model POST body is `{idempotency_key,user_id,text,context}`; response is
`{title,summary,ingredients:[string],steps:[string]}`. Relay POST body is
`{idempotency_key,user_id,text}`; response is `{status:"delivered",message_id}`.
Keys are `recipe-<provider_update_id>`. The model contract must honor deterministic
idempotent responses for that key: a crash before persisting a response can repeat
the physical request. SQLite persists the model outcome before relay delivery.
The external relay atomically owns reply idempotency, including lost responses.
Replay reconstructs the same page in the new ephemeral work tree so a crash before
host result import does not suppress the pending proposal. Multiple physical
proposals after crashes remain visible; no exactly-once canonical publication is
claimed by the application ledger.

Run `python3 -m unittest -v` here for the deterministic local journey, bounds,
conflicting replay, relay lost-response recovery and transactional migration.
Tests use the real sibling relay implementation and blog source check, without
network or real Telegram. They do not substitute for the installed guest test.

Release v1 initializes schema 1. Publish the same reviewed source as a second
immutable version with the declared `--migrate` hook: the hook transactionally adds
`recipe_summary`, backfills and indexes it; schema 2 rendering includes the model
summary. Re-entry is safe. `--rollback-fixture` deliberately raises after the SQL
changes, exits nonzero only after SQLite rollback, and logs no private data.

To prepare the two releases, publish this manifest and its exact source artifact
under distinct immutable release version labels `1.0.0` and `2.0.0`; these are
release-publication metadata, not fields of agent.toml. Install 1.0.0 first: normal
startup initializes schema 1. Updating to 2.0.0 invokes its declared `--migrate`
hook under the platform lease and schema 2 becomes visible only after success.
For a deliberate failing candidate, copy the reviewed manifest into a separate
source revision and replace only update_hook.arguments with
`["--rollback-fixture"]`, then publish a distinct candidate version. Preserve the
original release and recorded build/artifact hashes. This is preparation guidance;
publishing/activating the second release is separate from local unit evidence.

When `COOKING_HUGO_BINARY` points to a verified pinned Hugo executable, the test
suite additionally runs a real Hugo build and asserts the generated recipe HTML.
All test work trees, databases and Hugo output are disposable temporary paths.
