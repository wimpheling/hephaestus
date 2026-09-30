//! Opt-in `PostgreSQL` coverage for the release-owned UI publication schema.
//!
//! This module covers the release-owned UI publication schema introduced by
//! migration 0084. It uses only bound SQL and the existing disposable
//! `PostgreSQL` fixture.

#[path = "ui_schema_tests/constraints.rs"]
mod constraints;
#[path = "ui_schema_tests/fixtures.rs"]
mod fixtures;
#[path = "ui_schema_tests/immutable.rs"]
mod immutable;
#[path = "ui_schema_tests/rls.rs"]
mod rls;
#[path = "ui_schema_tests/rows.rs"]
mod rows;
#[path = "ui_schema_tests/scenarios.rs"]
mod scenarios;
#[path = "ui_schema_tests/support.rs"]
mod support;
