# Local SQLite application source

## Purpose

Provide a small released application that owns SQLite on one required named
RW `/data` volume. This package is source-ready: its Python tests pass, but it
has not been published or verified through the named-volume native runtime.
Recipe execution, typed import/drain, plural attachment and operator controls
remain separate integration work. No installed deployment is claimed here.

## Responsibilities

The workload owns schema initialization, transactional changes, WAL durability,
input validation, integrity checks and consistent SQLite backup preparation.
Heph owns release identity, attachment authority, provider storage and lifecycle.
An RW mount authorizes file bytes; it does not establish per-query permissions.
There are no host hooks, provider credentials or core database migrations.

## When

Use this example to test local application install/inspect/remove with retained
state after the typed recipe engine and named-volume runtime are wired. It needs
no network service, external Python package, secret or writable source workspace.

## Image and publication

`agent.toml` follows the repository's version 2 release contract. Its build and
guest select the reviewed `python-ubuntu` catalog record. The existing
[Dockerfile](../../platform/builders/python-ubuntu/Dockerfile) pins its Ubuntu
base, builds CPython 3.13.5 with SQLite and installs `/usr/local/bin/python3`.
The image key is resolved to a materialized immutable digest through the
[platform-image publication/catalog workflow](../../docs/builder-catalog-provisioning.md).
This example invents no image digest; a usable published catalog record is a
prerequisite. `build.sh` also checks SQLite support before copying the artifact.

Publish through the existing [release workflow](../../docs/releases-and-instances.md#source-configuration-and-builds):

1. Put this directory's files at a source repository root, so `agent.toml` and
   `build.sh` are at the paths declared by the build contract.
2. Ensure the reviewed `python-ubuntu` image is materialized in the catalog.
3. Push the exact source to `refs/heads/main`. The matching build trigger creates
   an isolated build; wait for its imported release artifacts and inspect them.
4. Publish that actual release through the authenticated release UI/RPC. Save
   its `PublishRelease` or `GetRelease` JSON response, including the release ID,
   published state and the `local-sqlite` export's ID.
5. Generate the pinned static TOML recipe from those returned identities:

   ```sh
   python3 examples/local-sqlite/render_recipe.py publication.json > recipe.toml
   ```

The generated version 1 recipe creates a 16 MiB volume with `removal = "retain"`,
binds slot `data` at `/data` in `read_write` mode and requests deletion only of
its deployment-owned instance. It uses exact release/agent UUIDs from the saved
response. Its bounded `operation`, `key` and `value` inputs take their types,
limits and defaults from `agent.toml`; the instance parameters reference those
inputs. The default operation is `inspect`. Different deployment input values
use the same immutable recipe definition, without editing parameter literals.
No placeholder recipe UUIDs are committed or presented as published.
The generator is a local convenience; installation must independently reload
and authorize the server catalog. Saved JSON is not publication authority.
Changing pinned release identities or the generated definition requires a new
recipe identity/version once the original definition has been registered.

Legacy state allocation is disabled. The package declares no generic capability
slot and no network access. `inspect` also initializes the schema idempotently.
Operations are workload commands selected by bounded deployment inputs. Actual
installation and execution still require the typed recipe/runtime integration.

## Retained-data reuse plan

Generate the distinct static `local-sqlite-reuse` profile from the same saved
publication:

```sh
python3 examples/local-sqlite/render_recipe.py --reuse publication.json > reuse-recipe.toml
```

This profile declares `data` as an external volume and always retains it. Its
release pins, required RW slot and input defaults match the create profile.
At installation, supply the exact retained `VolumeId` separately under resource
name `data`; it is not a TOML host path or an ambient resource lookup. The CLI
flag selects which static manifest to generate. Neither manifest has runtime
conditions or workflow expressions.

The intended lifecycle after runtime integration is:

| Step | Definition and inputs | Required result |
|---|---|---|
| Write deployment | `local-sqlite`; `{"operation":"put","key":"example","value":"durable"}` | Commit the value and record the created volume ID |
| Remove write deployment | Retain `data`; delete the owned instance | Permanently close the consumer, confirm VM destruction and release its exclusive lease before reuse |
| Read deployment | `local-sqlite-reuse`; bind `data` to that retained ID; `{"operation":"get","key":"example","value":""}` | A new independently authorized consumer reads `durable` |
| Remove read deployment | External `data` remains retained | Confirm cleanup and release the lease before the next consumer |
| Integrity deployment | Same reuse definition and volume; `{"operation":"integrity","key":"","value":""}` | A new consumer reports successful SQLite integrity |

Use a distinct deployment identity for each input set; resolved deployment
configuration is immutable. Both profiles request RW access, including reads
that may initialize SQLite or use WAL. Attachments are exclusive initially;
the earlier consumer must be proven detached before another acquires the
volume. Reuse requires exact project ownership, current source authority and a
new consumer grant. Removing a reuse deployment cannot delete the external
volume. None of these native install/remove/reuse steps has been executed yet.

## Workload requests

Production reads bounded ordinary parameters from
`/run/hephaestus/parameters.json`. Each request has exactly `operation`, `key`
and `value`; no request supplies a database or host path. The database is fixed
at `/data/app.sqlite`. `--stdin` reads the same bounded JSON for development or
an explicit released-VM test fixture.

| Operation | `key` / `value` | Result |
|---|---|---|
| `init` or `inspect` | Both empty | Schema version and row count |
| `put` | Nonempty key, bounded value | One committed parameterized upsert |
| `get` | Nonempty key, empty value | Exact value or null |
| `integrity` | Both empty | SQLite `integrity_check` result |
| `backup` | Both empty | Consistent `backup.sqlite` artifact, checksum, row count |

Example request: `{"operation":"put","key":"example","value":"durable"}`.
Requests are capped at 8192 bytes, values at 4096 bytes, responses at 32768 bytes
and the dataset at 256 keys. SQLite uses WAL and `synchronous=FULL`; schema and
writes commit transactionally. Unknown schema versions fail without rewriting
their version. SQL values are bound parameters.

`backup` uses SQLite's backup API, including committed WAL data. It verifies
integrity and fsyncs the prepared artifact and directory. Existing or incomplete
backup artifacts are retained and never overwritten; collect/reconcile them
before another preparation. This is workload preparation, not implemented Heph
volume backup/restore. The tests query a restored copy using SQLite itself;
provider backup storage and restore into a new volume still need implementation.

## Verification

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s examples/local-sqlite -p 'test_*.py' -v
```

The 14 Python tests exercise committed WAL across reopen and process crash, rollback,
parameterized hostile strings, capacity/input limits, unknown schema retention,
consistent backup under an uncommitted writer, restored query/integrity results,
backup preservation, bounded recipe inputs and both static recipe profiles.
The focused Rust integration test runs the actual renderer, parses `agent.toml`
through `agent-config`, and resolves defaults, write inputs and external reuse
through `recipe-domain` using explicitly labeled parser fixture identities:

```sh
cargo test -p agent-config --test local_sqlite_package -- --nocapture
```

This is source and parser evidence;
actual published release, guest mount, install/remove and provider lifecycle
proofs remain pending.
