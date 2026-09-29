# Workspace topology migration evidence

This document preserves the completion evidence for PR [#57](https://github.com/wimpheling/hephaestus/pull/57). The original one-off task was deleted after the migration; the machine-readable inventories are retained beside this document:

- [Before inventory](workspace-topology-migration/workspace-package-inventory.before.json)
- [After inventory](workspace-topology-migration/workspace-package-inventory.after.json)

## Workspace member patterns

The final `[workspace].members` value in `Cargo.toml` is:

```toml
[
    "crates/heph-core/platform/*",
    "crates/heph-core/control-plane/*",
    "crates/heph-core/authorization/*",
    "crates/heph-core/identity/*",
    "crates/heph-core/event/*",
    "crates/heph-core/gateway/*",
    "crates/heph-core/mailbox/*",
    "crates/heph-core/image/*",
    "crates/heph-core/forge",
    "crates/heph-core/secret",
    "crates/heph-core/secret/application",
    "crates/heph-core/secret/brokered-egress-client",
    "crates/heph-core/secret/brokered-egress-domain",
    "crates/heph-core/secret/domain",
    "crates/heph-core/secret/postgres",
    "crates/heph-core/secret/service",
    "crates/heph-core/secret/store",
    "crates/heph-core/runtime",
    "crates/heph-core/runtime/vm/*",
    "crates/heph-core/runtime/volume/*",
    "crates/heph-core/runtime/workspace/*",
    "crates/heph-core/run",
    "crates/heph-core/run/domain",
    "crates/heph-core/run/orchestrator",
    "crates/heph-core/run/postgres",
    "crates/heph-core/forge/domain",
    "crates/heph-core/forge/service",
    "crates/heph-core/forge/git-capability",
    "crates/heph-core/forge/pat",
    "crates/heph-core/forge/git-http",
    "crates/heph-core/forge/postgres",
    "crates/heph-core/forge/pat-postgres",
    "crates/heph-core/forge/build",
    "crates/heph-core/forge/build/oci-builder-postgres",
    "crates/heph-core/forge/build/orchestrator",
    "crates/heph-core/forge/build/postgres",
    "crates/heph-core/forge/release/*",
    "crates/heph-core/forge/review/*",
    "crates/heph-core/forge/registry/*",
    "crates/heph-std/authorization/*",
    "crates/heph-std/identity/*",
    "crates/heph-std/gateway/*",
    "crates/heph-std/secret/*",
    "crates/heph-std/runtime/vm/*",
    "crates/heph-std/runtime/volume/*",
    "crates/heph-std/runtime/workspace/*",
    "crates/heph-std/run/*",
    "crates/heph-std/forge/build/*",
    "crates/heph-std/forge/registry/*",
    "crates/heph-app",
    "crates/heph-app/bootstrap",
    "crates/heph-app/registry/*",
    "crates/heph-dev",
    "crates/heph-dev/vm/*",
]
```

## Cargo metadata reconciliation

The reconciliation command is:

```sh
python3 scripts/reconcile-workspace-package-inventory.py
```

It reports a clean reconciliation with 90 current packages and workspace
members: all 85 baseline package names are present, five expected facades are
present (`heph-build`, `heph-forge`, `heph-run`, `heph-runtime`, and
`heph-secret`), and all 22 PostgreSQL adapters remain declared. The inventory
records 85 manifest path changes. The six approved integration-test
relocations are:

```text
volume-postgres::postgres -> hephaestus-app::volume_postgres_local
workspace-postgres::postgres_git -> hephaestus-app::workspace_postgres_git
run-postgres::phase1b_libkrun -> hephaestus-app::run_postgres_phase1b_libkrun
forge-postgres::smart_http -> hephaestus-app::forge_postgres_smart_http
forge-postgres::postgres -> hephaestus-app::forge_postgres_receive
gateway-postgres::service_ownership -> hephaestus-app::gateway_service_ownership
```

The post-baseline architecture audit also added the focused targets
`control-plane-postgres::organization_pagination` and
`hephaestus-app::artifact_deadline_cancellation`; the reconciliation script
allows those two exact test targets and the five facade API targets.

## Facade API checks

The focused facade command is:

```sh
cargo test --workspace --test api --test contracts \
  -p heph-build -p heph-forge -p heph-run -p heph-runtime -p heph-secret
```

The current run passed all five facade targets: one contract test for
`heph-build` and one API test for each of `heph-forge`, `heph-run`,
`heph-runtime`, and `heph-secret`.

Direct leaf dependencies retained by the composition root and their rationale
are documented in [docs/application.md](application.md).

## Quality verification

The completion record for the topology migration reports a passing local
`cargo dev quality` gate and exit code 0 ([follow-up completion record](../tasks/done/enforce-350-line-rust-file-limit.md)).

The required Rust checks are:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
cargo dev quality
```

The [CI workflow](../.github/workflows/ci.yml) runs the Rust 1.88 formatting,
Clippy, serialized workspace tests, documentation, and focused integration
scripts. A PR #57 CI run identifier is not recorded in the repository
evidence; the workflow remains the authoritative command definition for CI
verification.
