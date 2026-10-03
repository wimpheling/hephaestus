//! Actual `PostgreSQL` execution-ledger tests; worker receipts are ledger fixtures,
//! not proof of filesystem, provider, or VM integration.
#[path = "execution/claims.rs"]
mod claims;
#[path = "support/fixtures.rs"]
#[allow(dead_code)] // This target shares admission declaration helpers.
mod fixtures;
#[path = "execution/guards.rs"]
mod guards;
#[path = "execution/harness.rs"]
mod harness;
#[path = "execution/outcomes.rs"]
mod outcomes;
#[path = "../../../../authorization/authz-postgres/tests/postgres/seed.rs"]
#[allow(dead_code)] // Shared historical seed includes unrelated authorization cases.
mod seed;
#[path = "../../../../authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Reuse identities; role-specific pools are supplied below.
mod support;
#[path = "execution/takeover.rs"]
mod takeover;
#[path = "execution/terminal.rs"]
mod terminal;
#[path = "execution/upgrade.rs"]
mod upgrade;
