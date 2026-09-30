# Workspace topology migration evidence

This document preserves the completion evidence for PR [#57](https://github.com/wimpheling/hephaestus/pull/57). The original one-off task was deleted after the migration; the machine-readable inventories are retained beside this document:

- [Before inventory](workspace-topology-migration/workspace-package-inventory.before.json)
- [After inventory](workspace-topology-migration/workspace-package-inventory.after.json)

## Workspace member patterns

The final `[workspace].members` value in `Cargo.toml` is:

```toml
[
    "crates/heph-core/platform/agent-config",
    "crates/heph-core/platform/rpc-proto",
    "crates/heph-core/platform/runtime-types",
    "crates/heph-core/platform/event/*",
    "crates/heph-core/platform/gateway/*",
    "crates/heph-core/auth/authorization/*",
    "crates/heph-core/auth/identity/*",
    "crates/heph-core/auth/secret",
    "crates/heph-core/auth/secret/application",
    "crates/heph-core/auth/secret/brokered-egress-client",
    "crates/heph-core/auth/secret/brokered-egress-domain",
    "crates/heph-core/auth/secret/domain",
    "crates/heph-core/auth/secret/store",
    "crates/heph-core/auth/secret/service",
    "crates/heph-core/image/*",
    "crates/heph-core/forge",
    "crates/heph-core/forge/domain",
    "crates/heph-core/forge/service",
    "crates/heph-core/forge/git-capability",
    "crates/heph-core/forge/pat",
    "crates/heph-core/forge/build",
    "crates/heph-core/forge/release/*",
    "crates/heph-core/forge/review/*",
    "crates/heph-core/forge/registry/*",
    "crates/heph-core/runtime",
    "crates/heph-core/runtime/vm/*",
    "crates/heph-core/runtime/volume/*",
    "crates/heph-core/runtime/workspace/*",
    "crates/heph-core/runtime/run",
    "crates/heph-core/runtime/run/domain",
    "crates/heph-core/runtime/run/orchestrator",
    "crates/heph-core/runtime/mailbox/domain",
    "crates/heph-std/authorization/*",
    "crates/heph-std/control-plane/*",
    "crates/heph-std/event/*",
    "crates/heph-std/identity/*",
    "crates/heph-std/gateway/*",
    "crates/heph-std/image/*",
    "crates/heph-std/secret/*",
    "crates/heph-std/runtime/vm/*",
    "crates/heph-std/runtime/volume/*",
    "crates/heph-std/runtime/workspace/*",
    "crates/heph-std/runtime/mailbox/*",
    "crates/heph-std/run/*",
    "crates/heph-std/forge/build/*",
    "crates/heph-std/forge/git-http",
    "crates/heph-std/forge/storage",
    "crates/heph-std/forge/pat-postgres",
    "crates/heph-std/forge/postgres",
    "crates/heph-std/forge/release/*",
    "crates/heph-std/forge/review/*",
    "crates/heph-std/forge/nats",
    "crates/heph-std/forge/review/nats",
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

It reports a clean reconciliation with 96 current packages and workspace
members: all 85 baseline package names are present, five expected facades are
present (`heph-build`, `heph-forge`, `heph-run`, `heph-runtime`, and
`heph-secret`), and six standard adapters are now explicit package identities:
`forge-nats`, `forge-storage`, `review-git`, `review-nats`, `run-nats`, and
`secret-key-local`. All 22 PostgreSQL adapters remain declared. The inventory
records 85 manifest path changes as the physical crate locations move. The
six approved integration-test relocations are:

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
allows those two exact test targets and the five facade API targets. It also
allows the two `forge-storage` architecture allowlist additions and the
`secret-store` test-fixture feature required by the adapter extraction, while
continuing to compare all other package facts with the historical inventory.

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
