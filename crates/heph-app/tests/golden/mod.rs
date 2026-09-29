// This integration-test facade is private, but its sibling helper modules
// need crate-visible re-exports so the extracted scenarios share one namespace.
#![allow(clippy::redundant_pub_crate)]

use crate::*;

// Wait for both bounded preparation branches even if an assertion or an
// expected-result check panics. Dropping the sibling could abandon published
// production work; resume the original panic only after both futures settle.
#[path = "cooking_preparation.rs"]
pub(crate) mod golden_cooking_preparation;
pub(crate) use golden_cooking_preparation::*;
#[path = "timing_layout.rs"]
pub(crate) mod golden_timing_layout;
pub(crate) use golden_timing_layout::*;
#[path = "cooking_oci.rs"]
pub(crate) mod golden_cooking_oci;
pub(crate) use golden_cooking_oci::*;
#[path = "registry_config.rs"]
pub(crate) mod golden_registry_config;
pub(crate) use golden_registry_config::*;

#[path = "scenario_seed.rs"]
pub(crate) mod golden_scenario_seed;
pub(crate) use golden_scenario_seed::*;
#[path = "scenario_backend.rs"]
pub(crate) mod golden_scenario_backend;
pub(crate) use golden_scenario_backend::*;
#[path = "scenario_session_chat.rs"]
pub(crate) mod golden_scenario_session_chat;
pub(crate) use golden_scenario_session_chat::*;
#[path = "scenario_cooking_releases.rs"]
pub(crate) mod golden_scenario_cooking_releases;
pub(crate) use golden_scenario_cooking_releases::*;
#[path = "scenario_cooking_service.rs"]
pub(crate) mod golden_scenario_cooking_service;
pub(crate) use golden_scenario_cooking_service::*;
#[path = "scenario_cooking_instance.rs"]
pub(crate) mod golden_scenario_cooking_instance;
pub(crate) use golden_scenario_cooking_instance::*;
#[path = "scenario_cooking_browser.rs"]
pub(crate) mod golden_scenario_cooking_browser;
pub(crate) use golden_scenario_cooking_browser::*;
#[path = "scenario_adversarial_setup.rs"]
pub(crate) mod golden_scenario_adversarial_setup;
pub(crate) use golden_scenario_adversarial_setup::*;
#[path = "scenario_cooking_state.rs"]
pub(crate) mod golden_scenario_cooking_state;
pub(crate) use golden_scenario_cooking_state::*;
#[path = "scenario_cooking_prepare.rs"]
pub(crate) mod golden_scenario_cooking_prepare;
pub(crate) use golden_scenario_cooking_prepare::*;
#[path = "scenario_cooking_denial.rs"]
pub(crate) mod golden_scenario_cooking_denial;
pub(crate) use golden_scenario_cooking_denial::*;
#[path = "scenario_cooking_crash.rs"]
pub(crate) mod golden_scenario_cooking_crash;
pub(crate) use golden_scenario_cooking_crash::*;
#[path = "scenario_cooking_updates.rs"]
pub(crate) mod golden_scenario_cooking_updates;
pub(crate) use golden_scenario_cooking_updates::*;
#[path = "scenario_cooking_rotation.rs"]
pub(crate) mod golden_scenario_cooking_rotation;
pub(crate) use golden_scenario_cooking_rotation::*;
#[path = "scenario_cooking_browser_phase.rs"]
pub(crate) mod golden_scenario_cooking_browser_phase;
pub(crate) use golden_scenario_cooking_browser_phase::*;
#[path = "scenario_cooking_cleanup.rs"]
pub(crate) mod golden_scenario_cooking_cleanup;
pub(crate) use golden_scenario_cooking_cleanup::*;
#[path = "scenario_standard_flow.rs"]
pub(crate) mod golden_scenario_standard_flow;
pub(crate) use golden_scenario_standard_flow::*;
#[path = "scenario_libkrun_service.rs"]
pub(crate) mod golden_scenario_libkrun_service;
#[path = "scenario_libkrun_service_log.rs"]
pub(crate) mod golden_scenario_libkrun_service_log;
pub(crate) use golden_scenario_libkrun_service::*;
#[path = "scenario_libkrun_requests.rs"]
pub(crate) mod golden_scenario_libkrun_requests;
pub(crate) use golden_scenario_libkrun_requests::*;
#[path = "scenario_setup.rs"]
pub(crate) mod golden_scenario_setup;
pub(crate) use golden_scenario_setup::*;
#[path = "scenario_mailbox.rs"]
pub(crate) mod golden_scenario_mailbox;
pub(crate) use golden_scenario_mailbox::*;
#[path = "common.rs"]
pub(crate) mod golden_common;
pub(crate) use golden_common::*;
#[path = "installed_ui_control.rs"]
pub(crate) mod golden_installed_ui_control;
pub(crate) use golden_installed_ui_control::*;
#[path = "installed_ui_lifecycle.rs"]
pub(crate) mod golden_installed_ui_lifecycle;
pub(crate) use golden_installed_ui_lifecycle::*;
#[path = "installed_ui_replacement.rs"]
pub(crate) mod golden_installed_ui_replacement;
pub(crate) use golden_installed_ui_replacement::*;
#[path = "installed_ui_revocation.rs"]
pub(crate) mod golden_installed_ui_revocation;
pub(crate) use golden_installed_ui_revocation::*;
#[path = "installed_ui_revocation_phase.rs"]
pub(crate) mod golden_installed_ui_revocation_phase;
pub(crate) use golden_installed_ui_revocation_phase::*;
#[path = "installed_ui_browser.rs"]
pub(crate) mod golden_installed_ui_browser;
pub(crate) use golden_installed_ui_browser::*;
#[path = "installed_ui_policy.rs"]
pub(crate) mod golden_installed_ui_policy;
pub(crate) use golden_installed_ui_policy::*;
#[path = "database.rs"]
pub(crate) mod golden_database;
pub(crate) use golden_database::*;
#[path = "gateway_external_warm.rs"]
pub(crate) mod golden_gateway_external_warm;
pub(crate) use golden_gateway_external_warm::*;
#[path = "gateway_external_failed.rs"]
pub(crate) mod golden_gateway_external_failed;
pub(crate) use golden_gateway_external_failed::*;
#[path = "gateway_external_recovery.rs"]
pub(crate) mod golden_gateway_external_recovery;
pub(crate) use golden_gateway_external_recovery::*;
#[path = "gateway_external_capacity.rs"]
pub(crate) mod golden_gateway_external_capacity;
pub(crate) use golden_gateway_external_capacity::*;
#[path = "gateway_external_candidate.rs"]
pub(crate) mod golden_gateway_external_candidate;
pub(crate) use golden_gateway_external_candidate::*;
#[path = "gateway_external_rollback.rs"]
pub(crate) mod golden_gateway_external_rollback;
pub(crate) use golden_gateway_external_rollback::*;
#[path = "gateway_external_cutover.rs"]
pub(crate) mod golden_gateway_external_cutover;
pub(crate) use golden_gateway_external_cutover::*;
#[path = "gateway_external_daemon.rs"]
pub(crate) mod golden_gateway_external_daemon;
pub(crate) use golden_gateway_external_daemon::*;
#[path = "gateway_external_health.rs"]
pub(crate) mod golden_gateway_external_health;
pub(crate) use golden_gateway_external_health::*;
#[path = "mailbox.rs"]
pub(crate) mod golden_mailbox;
pub(crate) use golden_mailbox::*;
#[path = "browser_fixture.rs"]
pub(crate) mod golden_browser_fixture;
pub(crate) use golden_browser_fixture::*;
#[path = "forge_fixture.rs"]
pub(crate) mod golden_forge_fixture;
pub(crate) use golden_forge_fixture::*;
#[path = "instance_fixture.rs"]
pub(crate) mod golden_instance_fixture;
pub(crate) use golden_instance_fixture::*;
#[path = "gateway_seed.rs"]
pub(crate) mod golden_gateway_seed;
pub(crate) use golden_gateway_seed::*;
#[path = "gateway_route.rs"]
pub(crate) mod golden_gateway_route;
pub(crate) use golden_gateway_route::*;
#[path = "gateway_cutover.rs"]
pub(crate) mod golden_gateway_cutover;
pub(crate) use golden_gateway_cutover::*;
#[path = "gateway_runtime.rs"]
pub(crate) mod golden_gateway_runtime;
pub(crate) use golden_gateway_runtime::*;
#[path = "gateway_requests.rs"]
pub(crate) mod golden_gateway_requests;
pub(crate) use golden_gateway_requests::*;
#[path = "gateway_published.rs"]
pub(crate) mod golden_gateway_published;
pub(crate) use golden_gateway_published::*;
#[path = "gateway_logs.rs"]
#[allow(dead_code)]
pub(crate) mod golden_gateway_logs;
#[cfg(feature = "test-fixtures")]
pub(crate) use golden_gateway_logs::*;
#[path = "brokered_route.rs"]
pub(crate) mod golden_brokered_route;
pub(crate) use golden_brokered_route::*;
#[path = "brokered_tls.rs"]
#[allow(dead_code)]
pub(crate) mod golden_brokered_tls;
pub(crate) use golden_brokered_tls::*;
#[path = "brokered_barrier.rs"]
#[allow(dead_code)]
pub(crate) mod golden_brokered_barrier;
pub(crate) use golden_brokered_barrier::*;
#[path = "utilities.rs"]
pub(crate) mod golden_utilities;
pub(crate) use golden_utilities::*;

#[path = "../../../../examples/cooking/tests/scenario.rs"]
#[allow(dead_code)]
mod cooking;
#[path = "../../../../examples/cooking/tests/adversarial_agent.rs"]
mod cooking_adversarial_agent;
#[path = "../../../../examples/cooking/tests/authority.rs"]
#[allow(dead_code)]
mod cooking_authority;
#[path = "../../../../examples/cooking/tests/blog_artifact.rs"]
mod cooking_blog_artifact;
#[path = "../../../../examples/cooking/tests/builds.rs"]
pub(crate) mod cooking_builds;
#[path = "../../../../examples/cooking/tests/confinement.rs"]
mod cooking_confinement;
#[path = "../../../../examples/cooking/tests/conflicts.rs"]
mod cooking_conflicts;
#[path = "../../../../examples/cooking/tests/guest_crash.rs"]
mod cooking_guest_crash;
#[path = "../../../../examples/cooking/tests/ingress_loss.rs"]
mod cooking_ingress_loss;
#[path = "../../../../examples/cooking/tests/inspection.rs"]
mod cooking_inspection;
#[path = "../../../../examples/cooking/tests/retirement.rs"]
mod cooking_retirement;
#[path = "../../../../examples/cooking/tests/service_build.rs"]
mod cooking_service_build;
#[path = "../../../../examples/cooking/tests/updates.rs"]
mod cooking_updates;
#[path = "../service_helpers/revocation.rs"]
mod service_revocation;
#[path = "../../../../examples/session-chat/tests/composed.rs"]
mod session_chat;
// The integration-test support tree is private to this test crate; its
// `pub(crate)` child boundaries are required by sibling fixture modules.
#[path = "../support/mod.rs"]
#[allow(clippy::redundant_pub_crate)]
mod support;

use support::backend_fixture;
