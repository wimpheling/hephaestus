#![allow(unused_imports)]
//! First composed acceptance for the ordinary session-chat release.

#[path = "composed/browser_assertions.rs"]
mod browser_assertions;
#[path = "composed/browser_concurrency.rs"]
mod browser_concurrency;
#[path = "composed/browser_records.rs"]
mod browser_records;
#[path = "composed/browser_setup.rs"]
mod browser_setup;
#[path = "composed/browser_state.rs"]
mod browser_state;
#[path = "composed/denial.rs"]
mod denial;
#[path = "composed/diagnostics.rs"]
mod diagnostics;
#[path = "composed/exercise.rs"]
mod exercise_impl;
#[path = "fork.rs"]
mod fork;
#[path = "composed/fork_phase.rs"]
mod fork_phase;
#[path = "composed/fork_setup.rs"]
mod fork_setup;
#[path = "composed/git_mutations.rs"]
mod git_mutations;
#[path = "composed/git_queries.rs"]
mod git_queries;
#[path = "composed/model.rs"]
mod model;
#[path = "composed/nonbrowser_assertions.rs"]
mod nonbrowser_assertions;
#[path = "composed/nonbrowser_setup.rs"]
mod nonbrowser_setup;
#[path = "composed/restart.rs"]
mod restart;
#[path = "composed/restart_tail.rs"]
mod restart_tail;
#[path = "composed/rpc_helpers.rs"]
mod rpc_helpers;
#[path = "composed/runtime_assertions.rs"]
mod runtime_assertions;
#[path = "composed/secrets.rs"]
mod secrets;

pub(crate) use forge_domain::{GitRef, OrganizationId, ProjectId};
use uuid::Uuid;

pub(super) const MODEL_RESPONSE_TEXT: &str = "reference answer from deterministic model";
pub const MODEL_RULE: Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000007");
pub const MODEL_SECRET_VALUE: &str = "session-chat-model-fixture-sentinel-007";
pub(super) const SESSION_ID: &str = "11111111-1111-4111-8111-111111111111";
pub(super) const HUMAN_RECORD_ID: &str = "22222222-2222-4222-8222-222222222222";
pub(super) const DENIAL_HUMAN_RECORD_ID: &str = "55555555-5555-4555-8555-555555555555";

#[derive(Debug, sqlx::FromRow)]
#[allow(clippy::struct_field_names)]
struct BrowserSessionObjects {
    repository_id: Uuid,
    instance_id: Uuid,
    revision_id: Uuid,
    attachment_id: Uuid,
}

pub(crate) use browser_assertions::assert_browser_session;
pub(crate) use browser_concurrency::exercise_browser_concurrency;
pub(crate) use browser_setup::{SessionChatBrowserMode, run_session_chat_browser};
pub(crate) use browser_state::BrowserRestartState;
pub(crate) use denial::{
    accepted_receive_count, assert_denial_probe_output, canonical_main_ref,
    prepare_denial_probe_source,
};
pub(crate) use diagnostics::{run_diagnostics, wait_for_run_succeeded};
pub(crate) use exercise_impl::exercise;
pub(crate) use fork_phase::exercise_browser_fork;
pub(crate) use git_mutations::{append_human_and_push, initialize_session_checkout};
pub(crate) use git_queries::{
    accepted_normal_run_id, accepted_runtime_receive_count, canonical_record_commit,
};
pub(crate) use model::SessionBrokerFixture;
pub(crate) use model::start_model_fixture;
pub(crate) use model::{ObservedModelMessage, ObservedModelRequest};
pub(crate) use restart::exercise_browser_restart;
pub(crate) use rpc_helpers::{
    authenticated_git, git, git_output, git_output_bare, git_output_bare_bytes, instance_client,
    mutation_context, opaque, response_id,
};
pub(crate) use runtime_assertions::{assert_runtime_git_turn, assert_runtime_git_turn_at_commit};
pub(crate) use secrets::{seed_model_import, seed_model_secret, seed_model_secret_with_rule};

pub fn enabled() -> bool {
    std::env::var("HEPHAESTUS_APP_SESSION_CHAT_E2E").as_deref() == Ok("1")
}
pub fn denial_probe_enabled() -> bool {
    std::env::var("HEPHAESTUS_APP_SESSION_CHAT_DENIAL_PROBE_E2E").as_deref() == Ok("1")
}
pub fn restart_e2e_enabled() -> bool {
    std::env::var("HEPHAESTUS_APP_SESSION_CHAT_RESTART_E2E").as_deref() == Ok("1")
}
pub fn concurrent_e2e_enabled() -> bool {
    std::env::var("HEPHAESTUS_APP_SESSION_CHAT_CONCURRENT_E2E").as_deref() == Ok("1")
}
pub fn fork_e2e_enabled() -> bool {
    std::env::var("HEPHAESTUS_APP_SESSION_CHAT_FORK_E2E").as_deref() == Ok("1")
}
