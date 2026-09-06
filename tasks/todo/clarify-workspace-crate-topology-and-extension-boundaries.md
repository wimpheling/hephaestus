# Reorganize the Rust workspace into `heph-core` and `heph-std`

Owner: unassigned

## Outcome

Replace the flat `crates/` layout with a context-first hierarchy.
`crates/heph-core/` holds the product's portable contracts and its deliberately
opinionated infrastructure: PostgreSQL as the authoritative store and Git as
the repository protocol. `crates/heph-std/` holds genuinely swappable provider
implementations such as a VM engine, a local filesystem backend, OIDC, and
external registry integrations. The product composition root and development
tooling remain separately visible as
`crates/heph-app/` and `crates/heph-dev/`.

Every existing package keeps its Cargo package name and Rust crate name. This
is a directory/workspace-path reorganization, not an API rename. New facade
packages provide the deliberate public seams for secret, runtime, run, forge,
and build code; implementation packages remain internal workspace details.

## Target tree

```text
crates/
  heph-core/
    platform/{agent-config,runtime-types,rpc-proto,control-plane-postgres}
    authorization/{authz-domain,capability-domain,capability-audit,runtime-authority,
                   runtime-git-authority,authz-postgres,capability-audit-postgres,
                   runtime-authority-postgres,runtime-git-authority-postgres}
    identity/{domain,application,postgres,git-credential}
    event/{application,postgres}
    gateway/{domain,postgres}
    mailbox/{domain,dispatch,postgres}
    secret/{Cargo.toml,domain,store,application,service,brokered-egress-domain,
            brokered-egress-client,postgres}
    runtime/{Cargo.toml,vm/trait,volume/{trait,postgres},workspace/{domain,postgres}}
    run/{Cargo.toml,domain,orchestrator,postgres}
    forge/{Cargo.toml,domain,service,git-capability,pat,git-http,
           postgres,pat-postgres,
           build/{Cargo.toml,orchestrator,postgres,oci-builder-postgres,
                  builder-catalog/{domain,application,postgres}},
           release/{domain,service,artifact-store,postgres},
           review/{domain,service,postgres},
           registry/{domain,notification,token,reconciler,release,postgres}}
  heph-std/
    authorization/runtime-handoff-local
    identity/oidc
    gateway/edge
    secret/{runtime,broker}
    runtime/{vm/{libkrun,fake,conformance},volume/local,workspace/local}
    run/runtime-local
    forge/{build/{oci-builder-runtime-local,oci-builder-worker},
           registry/{http,notification-http,publisher,zot}}
  heph-app/{Cargo.toml,bootstrap}
  heph-dev/
```

Names in the tree are directory names. Existing package names remain unchanged
(for example, `vm-trait` moves to `heph-core/runtime/vm/trait`, and
`vm-libkrun` moves to `heph-std/runtime/vm/libkrun`).

## Locked decisions

| Decision | Required implementation |
| --- | --- |
| `heph-core` holds product contracts, behavior, and mandatory infrastructure; `heph-std` holds provider implementations. | Keep PostgreSQL and Git packages in core because the product relies on PostgreSQL RLS, migrations, and transactional authority, and on Git repository semantics. Use architecture metadata to decide the remaining cases: a local/provider/external adapter belongs in standard; `composition` and `development` packages belong in app/dev. |
| Bounded context is the primary grouping within each layer family. | Group packages by their existing `package.metadata.hephaestus.context`, rather than putting all `*-postgres` or all `*-domain` packages in global layer directories. |
| `build`, `release`, `review`, and `registry` are forge subcontexts. | Place their product contracts and PostgreSQL packages beneath `heph-core/forge/`; place only provider-specific build and registry implementations beneath `heph-std/forge/`. This is a workspace-discovery decision; it does not alter architecture-layer metadata or relax cross-context dependency restrictions. |
| Facades define intended public seams. | Add `heph-secret`, `heph-runtime`, `heph-run`, `heph-forge`, and `heph-build` packages at the indicated parent paths. Each re-exports only explicitly approved types/operations; no blanket `pub use child::*`. |
| `heph-core` must not import `heph-std`. | Add the hard-enabled `ARCH-CORE-NO-STD-DEPENDENCIES` rule. It rejects every normal, build, and development dependency from a package under `crates/heph-core/` to one under `crates/heph-std/`; direction from standard to core is permitted. |
| Moving directories must not change behavior. | Preserve package names, package metadata, features, binaries, migration ownership, generated-code locations, and all public Rust paths of existing leaf packages. |
| File length remains the existing architecture-policy concern. | Do not add Dylint. `ARCH-MAX-FILE-LENGTH` already has configured per-layer thresholds; activate or adjust it only in a distinct task after this move is complete. |

## Non-goals

- Rename existing Cargo packages or force all consumers to use a facade in this
  migration.
- Merge leaf crates, alter dependency direction, or change PostgreSQL, RPC,
  event, or sensitive-data boundaries.
- Move `migrations/`, `proto/`, `web/`, `platform/`, or deployment assets.
- Introduce a universal 300/350-line Rust-file limit or a Dylint dependency.

## Implementation checklist

- [ ] **Prepare the workspace migration**
  - [ ] Move this task from `tasks/todo/` to `tasks/in-progress/` and record
    the active session in `Owner` before changing source or manifest paths.
  - [ ] Add the target directory structure without copying crate contents; use
    `git mv` for every existing package directory so history follows the move.
  - [ ] Replace `members = ["crates/*"]` with explicit recursive member globs
    covering every target depth under `heph-core` and `heph-std`, plus the
    `heph-app` and `heph-dev` packages.
  - [ ] Update every internal path dependency, workspace dependency path, test
    fixture path, and package-specific command to its new relative location.
  - [ ] Update path references in scripts, CI, documentation, and architecture
    checker fixtures; do not rewrite package-name dependencies unnecessarily.
  - [ ] Regenerate `Cargo.lock` only through Cargo after all manifest paths are
    valid; inspect the diff to confirm package identities and versions are
    unchanged.

- [ ] **Move core packages and opinionated product infrastructure**
  - [ ] Move platform packages: `agent-config`, `runtime-types`, and
    `rpc-proto` to `heph-core/platform/`.
  - [ ] Move authorization packages: `authz-domain`, `capability-domain`,
    `capability-audit`, `runtime-authority`, and `runtime-git-authority` to
    `heph-core/authorization/`.
  - [ ] Move `identity-domain` and `identity-application` to
    `heph-core/identity/`; move `event-application`, `gateway-domain`,
    `mailbox-domain`, and `mailbox-dispatch` to their corresponding core
    context paths.
  - [ ] Move all secret packages listed under `heph-core/secret/`; retain the
    existing `secret-store` domain-layer classification.
  - [ ] Move `vm-trait`, `volume-trait`, and `workspace-domain` under
    `heph-core/runtime/`, and `run-domain` and `run-orchestrator` under
    `heph-core/run/`.
  - [ ] Move all forge, build, release, review, and registry core packages to
    the exact `heph-core/forge/` paths in the target tree.
  - [ ] Move every PostgreSQL adapter to the matching `heph-core` bounded
    context path, including control-plane, authorization, identity, event,
    gateway, mailbox, secret, runtime, run, forge, build, release, review,
    and registry adapters.
  - [ ] Keep Git-specific product packages in core: `git-http`,
    `git-capability-domain`, `git-credential-hephaestus`, and the forge and
    PAT PostgreSQL packages. Do not represent Git as an optional provider
    boundary in this migration.

- [ ] **Move genuinely replaceable standard implementations**
  - [ ] Move local and provider implementations to their target contexts:
    `vm-libkrun`, `vm-fake`, `vm-conformance`, `volume-local`,
    `workspace-local`, `run-runtime-local`, `secret-runtime`, `secret-broker`,
    `gateway-edge`, `identity-oidc`, `oci-builder-runtime-local`, and
    `oci-builder-worker`.
  - [ ] Move external/edge registry implementations (`registry-http`,
    `registry-notification-http`, `registry-publisher`, and `registry-zot`) to
    `heph-std/forge/registry/`.
  - [ ] Keep each adapter's `layer = "adapter"`, bounded context, SQL adapter
    metadata, and cross-context dependency allowlist unchanged; add focused
    architecture fixtures only if the move exposes a path-sensitive checker
    assumption.

- [ ] **Enforce the core-to-standard dependency direction**
  - [ ] Add `ARCH-CORE-NO-STD-DEPENDENCIES` to the architecture rule registry,
    `architecture.toml`, and `ARCHITECTURE.md`. Its diagnostic must name the
    importing core package, the standard package, and the dependency kind, then
    direct the maintainer to introduce a core port or move concrete wiring to
    `heph-app`.
  - [ ] Implement the rule in the Cargo-metadata architecture checker using
    canonical manifest-directory prefixes, not package-name conventions. Check
    normal, build, and development dependencies so core test targets cannot
    quietly import standard implementations.
  - [ ] Add valid checker fixtures for core-to-core, standard-to-core, and
    app-to-core-and-standard dependencies, plus invalid fixtures for each of
    normal, build, and development core-to-standard dependency edges.
  - [ ] Remove the existing core-to-standard edges before enabling the rule:
    replace `secret-postgres` → `secret-runtime`, `gateway-postgres` →
    `gateway-edge`, and `git-http` → `identity-oidc` with core ports and
    composition-root wiring.
  - [ ] Relocate tests that currently make core packages depend on local
    providers—such as the `run-postgres` VM/volume tests and the
    `workspace-postgres` local-workspace tests—to `heph-app` integration tests
    or to the relevant `heph-std` provider package. Keep core tests dependent
    only on core fakes/contracts.
  - [ ] Audit the complete post-move Cargo metadata graph and remove every
    remaining `heph-core` → `heph-std` edge rather than adding exceptions.

- [ ] **Create controlled context facades**
  - [ ] Create `heph-secret` at `heph-core/secret/`, `heph-runtime` at
    `heph-core/runtime/`, `heph-run` at `heph-core/run/`, `heph-forge` at
    `heph-core/forge/`, and `heph-build` at `heph-core/forge/build/`.
  - [ ] Give each facade an explicit `package.metadata.hephaestus` layer and
    context consistent with its public role; document any narrowly necessary
    architecture exception rather than weakening layer rules.
  - [ ] Re-export only stable, cross-context contracts selected from leaf
    crates. Keep PostgreSQL row types, generated RPC types, raw secrets, and
    provider-specific configurations private to their leaf packages.
  - [ ] Add compile-time API tests proving each facade exposes its intended
    contract and cannot expose prohibited adapter or sensitive representations.
  - [ ] Update composition-root dependencies to use a facade where it needs a
    context-level API; retain direct leaf dependencies only for deliberate
    concrete-adapter wiring.

- [ ] **Move composition and development packages**
  - [ ] Move `hephaestus-app` to `crates/heph-app/`, preserving the
    `hephaestusd` binary and composition-layer metadata.
  - [ ] Move `bootstrap-postgres` to `crates/heph-app/bootstrap/`, preserving
    its operator and E2E-seed binaries and composition-layer metadata.
  - [ ] Move `hephaestus-dev` to `crates/heph-dev/`, including all architecture
    fixtures and path-sensitive tests.
  - [ ] Update root README crate links to the new locations and add a concise
    `heph-core`/`heph-std` navigation section that directs contributors to the
    facades and concrete adapters.

- [ ] **Verify migration integrity**
  - [ ] Run `cargo metadata --format-version 1 --no-deps` and confirm every
    former workspace package is still a workspace member with the same name,
    version, features, and architecture metadata.
  - [ ] Run `cargo dev check architecture` and resolve every path-sensitive
    diagnostic without adding broad exceptions.
  - [ ] Run the `ARCH-CORE-NO-STD-DEPENDENCIES` valid and invalid fixture tests
    and confirm the complete workspace has zero forbidden dependency edges.
  - [ ] Run focused facade API and architecture-fixture tests.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run `cargo dev quality`.
  - [ ] Run `git diff --check` and inspect `git diff --summary` to confirm the
    crate changes are renames/moves rather than accidental deletions.

## Completion evidence

- [ ] Record the before/after `cargo metadata` package inventory and the final
  workspace member patterns.
- [ ] Record the facade API tests and all required verification commands with
  their passing results.
- [ ] Record any intentional direct leaf dependencies remaining in the
  composition root and why each cannot use its context facade.
- [ ] After all checklist items are verified, delete this task file rather
  than retaining it in `tasks/done/`, as explicitly requested for this one-off
  reorganization plan.
