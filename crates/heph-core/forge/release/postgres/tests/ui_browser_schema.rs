//! Real-PostgreSQL matrix for UI browser persistence and application auth.

use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, time::Duration};
use tokio::time::timeout;
use uuid::Uuid;

use gateway_domain::{HttpMethod, RoutePath};
use identity_domain::{BrowserSessionId, RequestId, UserId};
use release_domain::{
    UiInstallationGenerationId, UiInstallationId,
    ui_browser::{UiBrowserHandoffSecret, UiBrowserRoute, UiBrowserSessionSecret},
};
use release_postgres::PgUiBrowserSessionStore;
use release_service::{
    AuthenticateUiBrowserSession, CreateUiBrowserHandoff, ExchangeUiBrowserHandoff,
    UiBrowserHandoffError, UiBrowserRequestRoute, UiBrowserSessionContext, UiBrowserSessionError,
};

const EXPECTED_MIGRATION: i64 = 96;

#[path = "ui_browser_schema/authentication.rs"]
mod authentication;
#[path = "ui_browser_schema/authentication_authority.rs"]
mod authentication_authority;
#[path = "ui_browser_schema/authentication_denials.rs"]
mod authentication_denials;
#[path = "ui_browser_schema/authentication_gateway.rs"]
mod authentication_gateway;
#[path = "ui_browser_schema/authentication_valid.rs"]
mod authentication_valid;
#[path = "ui_browser_schema/binding_assertions.rs"]
mod binding_assertions;
#[path = "ui_browser_schema/child_inserts.rs"]
mod child_inserts;
#[path = "ui_browser_schema/fixture.rs"]
mod fixture;
#[path = "ui_browser_schema/fixture_bindings.rs"]
mod fixture_bindings;
#[path = "ui_browser_schema/fixture_gateway.rs"]
mod fixture_gateway;
#[path = "ui_browser_schema/fixture_identity.rs"]
mod fixture_identity;
#[path = "ui_browser_schema/fixture_installations.rs"]
mod fixture_installations;
#[path = "ui_browser_schema/fixture_release.rs"]
mod fixture_release;
#[path = "ui_browser_schema/fixture_seed.rs"]
mod fixture_seed;
#[path = "ui_browser_schema/handoff_inserts.rs"]
mod handoff_inserts;
#[path = "ui_browser_schema/lifecycle_assertions.rs"]
mod lifecycle_assertions;
#[path = "ui_browser_schema/matrix.rs"]
mod matrix;
#[path = "ui_browser_schema/repository.rs"]
mod repository;
#[path = "ui_browser_schema/role_audit.rs"]
mod role_audit;
#[path = "ui_browser_schema/schema_test_support.rs"]
mod schema_test_support;

use authentication::*;
use authentication_authority::*;
use authentication_denials::*;
use authentication_gateway::*;
use authentication_valid::*;
use binding_assertions::*;
use child_inserts::*;
use fixture::insert_canonical_session;
use fixture::{Fixture, digest, scoped_secret, test_secret};
use fixture_seed::seed_fixture_reusing_installation_helpers;
use handoff_inserts::*;
use lifecycle_assertions::*;
use role_audit::*;
use schema_test_support::*;

#[path = "ui_browser_schema/exchange.rs"]
mod exchange;
#[path = "ui_browser_schema/exchange_basic.rs"]
mod exchange_basic;
#[path = "ui_browser_schema/exchange_concurrency.rs"]
mod exchange_concurrency;
#[path = "ui_browser_schema/exchange_denials.rs"]
mod exchange_denials;
#[path = "ui_browser_schema/exchange_rollback.rs"]
mod exchange_rollback;
#[path = "ui_browser_schema/issue.rs"]
mod issue;
#[path = "ui_browser_schema/issue_authority.rs"]
mod issue_authority;
#[path = "ui_browser_schema/issue_barrier.rs"]
mod issue_barrier;
#[path = "ui_browser_schema/issue_source.rs"]
mod issue_source;
#[path = "ui_browser_schema/issue_success.rs"]
mod issue_success;

#[tokio::test]
#[serial]
async fn ui_browser_issue_binds_current_authority_and_fresh_expiry() {
    issue::run().await;
}

#[tokio::test]
#[serial]
async fn ui_browser_exchange_is_atomic_generation_bound_and_parent_capped() {
    exchange::run().await;
}
