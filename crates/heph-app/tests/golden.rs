//! Opt-in daemon-level golden path through every production boundary.

use authz_postgres::PostgresMelangeAuthorizer;
use brokered_egress_domain::{
    BrokeredSecretRule, BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName, HttpInjectionLocation,
};
use forge_domain::{GitRef, OrganizationId, ProjectId};
use forge_postgres::PgForgeRepository;
use forge_service::{CreateRepository, GitStorage};
use hephaestus_app::{
    AppConfig, GatewayEdgeConfig, HephaestusApp, OciBuilderWorkerConfig, OidcConfig,
    RegistryConfig, RunEventKind, UiOriginConfig, VmBackendConfig,
};
use identity_domain::{
    AuthenticatedIdentity, BrowserSessionSid, RequestId, UserId,
    browser_session_identity_binding_digest, browser_session_sid_digest,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope, MailboxEvent, MailboxId, ProducerId,
};
use mailbox_postgres::PostgresMailboxRepository;
use oci_builder_runtime_local::LocalOciRuntimeConfig;
use registry_domain::{RegistryAuthority, SupplyChainPolicy};
use registry_publisher::PublisherConfiguration;
use registry_token::{RegistryTokenIssuer, SigningKey, TokenLifetime};
use release_service::{
    UiNamespace, UiPublicPort, UiRequestAuditDecision, UiRequestAuditOutcome, UiRequestAuditReason,
    UiRequestAuditSurface,
};
use run_runtime_local::LocalRunRuntimeConfig;
use secret_application::{
    BindSecret, CreateSecret, DeclareBrokeredHttpsRule, GrantAndAcceptSecretImport,
};
use secret_broker::{BrokeredHttpsAdapterRegistry, DenyingBrokerAdapter};
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretCommandKey,
    SecretGrantId, SecretId, SecretImportId, SecretName, SecretOwner, SecretSlotKey, SecretTarget,
    SecretUsePolicy, SecretValue, SecretVersionId,
};
use secret_postgres::SecretService;
use secret_runtime::EphemeralSecretConfig;
use secret_store::{EncryptedStore, LocalKeyProvider};
use serial_test::serial;
use sha2::{Digest, Sha256};
use sqlx::{Row, postgres::PgPoolOptions};
use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use time::OffsetDateTime;
use tokio::process::Command;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
};
use tokio_rustls::TlsAcceptor;
use url::Url;
use vm_trait::RootFilesystem;
use volume_local::LocalVolumeConfig;
use workspace_local::{LocalWorkspaceConfig, WorkspaceLimits};

#[cfg(feature = "test-fixtures")]
#[path = "composition/gateway_service_log.rs"]
mod gateway_service_log_rpc;
#[cfg(feature = "test-fixtures")]
use gateway_service_log_rpc::{
    GUEST_SERVICE_LOG_STDERR_MARKER, GUEST_SERVICE_LOG_STDOUT_MARKER, GatewayServiceGuestLogProof,
    GatewayServiceLogRpcFixture, marker_count,
};

// Wait for both bounded preparation branches even if an assertion or an
// expected-result check panics. Dropping the sibling could abandon published
// production work; resume the original panic only after both futures settle.
#[path = "golden/cooking_preparation.rs"]
pub(crate) mod golden_cooking_preparation;
pub(crate) use golden_cooking_preparation::*;
#[path = "golden/timing_layout.rs"]
pub(crate) mod golden_timing_layout;
pub(crate) use golden_timing_layout::*;
#[path = "golden/cooking_oci.rs"]
pub(crate) mod golden_cooking_oci;
pub(crate) use golden_cooking_oci::*;
#[path = "golden/registry_config.rs"]
pub(crate) mod golden_registry_config;
pub(crate) use golden_registry_config::*;

#[path = "golden/scenario_seed.rs"]
pub(crate) mod golden_scenario_seed;
pub(crate) use golden_scenario_seed::*;
#[path = "golden/scenario_backend.rs"]
pub(crate) mod golden_scenario_backend;
pub(crate) use golden_scenario_backend::*;
#[path = "golden/scenario_session_chat.rs"]
pub(crate) mod golden_scenario_session_chat;
pub(crate) use golden_scenario_session_chat::*;
#[path = "golden/scenario_cooking_releases.rs"]
pub(crate) mod golden_scenario_cooking_releases;
pub(crate) use golden_scenario_cooking_releases::*;
#[path = "golden/scenario_cooking_service.rs"]
pub(crate) mod golden_scenario_cooking_service;
pub(crate) use golden_scenario_cooking_service::*;
#[path = "golden/scenario_cooking_instance.rs"]
pub(crate) mod golden_scenario_cooking_instance;
pub(crate) use golden_scenario_cooking_instance::*;
#[path = "golden/scenario_cooking_browser.rs"]
pub(crate) mod golden_scenario_cooking_browser;
pub(crate) use golden_scenario_cooking_browser::*;
#[path = "golden/scenario_adversarial_setup.rs"]
pub(crate) mod golden_scenario_adversarial_setup;
pub(crate) use golden_scenario_adversarial_setup::*;
#[path = "golden/scenario_libkrun_service.rs"]
pub(crate) mod golden_scenario_libkrun_service;
#[path = "golden/scenario_libkrun_service_log.rs"]
pub(crate) mod golden_scenario_libkrun_service_log;
pub(crate) use golden_scenario_libkrun_service::*;
#[path = "golden/scenario_libkrun_requests.rs"]
pub(crate) mod golden_scenario_libkrun_requests;
pub(crate) use golden_scenario_libkrun_requests::*;
#[path = "golden/scenario_setup.rs"]
pub(crate) mod golden_scenario_setup;
pub(crate) use golden_scenario_setup::*;
#[path = "golden/scenario_mailbox.rs"]
pub(crate) mod golden_scenario_mailbox;
pub(crate) use golden_scenario_mailbox::*;
#[path = "golden/common.rs"]
pub(crate) mod golden_common;
pub(crate) use golden_common::*;
#[path = "golden/installed_ui_control.rs"]
pub(crate) mod golden_installed_ui_control;
pub(crate) use golden_installed_ui_control::*;
#[path = "golden/installed_ui_lifecycle.rs"]
pub(crate) mod golden_installed_ui_lifecycle;
pub(crate) use golden_installed_ui_lifecycle::*;
#[path = "golden/installed_ui_replacement.rs"]
pub(crate) mod golden_installed_ui_replacement;
pub(crate) use golden_installed_ui_replacement::*;
#[path = "golden/installed_ui_revocation.rs"]
pub(crate) mod golden_installed_ui_revocation;
pub(crate) use golden_installed_ui_revocation::*;
#[path = "golden/installed_ui_revocation_phase.rs"]
pub(crate) mod golden_installed_ui_revocation_phase;
pub(crate) use golden_installed_ui_revocation_phase::*;
#[path = "golden/installed_ui_browser.rs"]
pub(crate) mod golden_installed_ui_browser;
pub(crate) use golden_installed_ui_browser::*;
#[path = "golden/installed_ui_policy.rs"]
pub(crate) mod golden_installed_ui_policy;
pub(crate) use golden_installed_ui_policy::*;
#[path = "golden/database.rs"]
pub(crate) mod golden_database;
pub(crate) use golden_database::*;
#[path = "golden/gateway_external_warm.rs"]
pub(crate) mod golden_gateway_external_warm;
pub(crate) use golden_gateway_external_warm::*;
#[path = "golden/gateway_external_failed.rs"]
pub(crate) mod golden_gateway_external_failed;
pub(crate) use golden_gateway_external_failed::*;
#[path = "golden/gateway_external_recovery.rs"]
pub(crate) mod golden_gateway_external_recovery;
pub(crate) use golden_gateway_external_recovery::*;
#[path = "golden/gateway_external_capacity.rs"]
pub(crate) mod golden_gateway_external_capacity;
pub(crate) use golden_gateway_external_capacity::*;
#[path = "golden/gateway_external_candidate.rs"]
pub(crate) mod golden_gateway_external_candidate;
pub(crate) use golden_gateway_external_candidate::*;
#[path = "golden/gateway_external_rollback.rs"]
pub(crate) mod golden_gateway_external_rollback;
pub(crate) use golden_gateway_external_rollback::*;
#[path = "golden/gateway_external_cutover.rs"]
pub(crate) mod golden_gateway_external_cutover;
pub(crate) use golden_gateway_external_cutover::*;
#[path = "golden/gateway_external_daemon.rs"]
pub(crate) mod golden_gateway_external_daemon;
pub(crate) use golden_gateway_external_daemon::*;
#[path = "golden/gateway_external_health.rs"]
pub(crate) mod golden_gateway_external_health;
pub(crate) use golden_gateway_external_health::*;
#[path = "golden/mailbox.rs"]
pub(crate) mod golden_mailbox;
pub(crate) use golden_mailbox::*;
#[path = "golden/browser_fixture.rs"]
pub(crate) mod golden_browser_fixture;
pub(crate) use golden_browser_fixture::*;
#[path = "golden/forge_fixture.rs"]
pub(crate) mod golden_forge_fixture;
pub(crate) use golden_forge_fixture::*;
#[path = "golden/instance_fixture.rs"]
pub(crate) mod golden_instance_fixture;
pub(crate) use golden_instance_fixture::*;
#[path = "golden/gateway_seed.rs"]
pub(crate) mod golden_gateway_seed;
pub(crate) use golden_gateway_seed::*;
#[path = "golden/gateway_route.rs"]
pub(crate) mod golden_gateway_route;
pub(crate) use golden_gateway_route::*;
#[path = "golden/gateway_cutover.rs"]
pub(crate) mod golden_gateway_cutover;
pub(crate) use golden_gateway_cutover::*;
#[path = "golden/gateway_runtime.rs"]
pub(crate) mod golden_gateway_runtime;
pub(crate) use golden_gateway_runtime::*;
#[path = "golden/gateway_requests.rs"]
pub(crate) mod golden_gateway_requests;
pub(crate) use golden_gateway_requests::*;
#[path = "golden/gateway_published.rs"]
pub(crate) mod golden_gateway_published;
pub(crate) use golden_gateway_published::*;
#[path = "golden/gateway_logs.rs"]
pub(crate) mod golden_gateway_logs;
pub(crate) use golden_gateway_logs::*;
#[path = "golden/brokered_route.rs"]
pub(crate) mod golden_brokered_route;
pub(crate) use golden_brokered_route::*;
#[path = "golden/brokered_tls.rs"]
pub(crate) mod golden_brokered_tls;
pub(crate) use golden_brokered_tls::*;
#[path = "golden/brokered_barrier.rs"]
pub(crate) mod golden_brokered_barrier;
pub(crate) use golden_brokered_barrier::*;
#[path = "golden/utilities.rs"]
pub(crate) mod golden_utilities;
pub(crate) use golden_utilities::*;

#[path = "../../../examples/cooking/tests/scenario.rs"]
mod cooking;
#[path = "../../../examples/cooking/tests/adversarial_agent.rs"]
mod cooking_adversarial_agent;
#[path = "../../../examples/cooking/tests/authority.rs"]
mod cooking_authority;
#[path = "../../../examples/cooking/tests/blog_artifact.rs"]
mod cooking_blog_artifact;
#[path = "../../../examples/cooking/tests/builds.rs"]
mod cooking_builds;
#[path = "../../../examples/cooking/tests/confinement.rs"]
mod cooking_confinement;
#[path = "../../../examples/cooking/tests/conflicts.rs"]
mod cooking_conflicts;
#[path = "../../../examples/cooking/tests/guest_crash.rs"]
mod cooking_guest_crash;
#[path = "../../../examples/cooking/tests/ingress_loss.rs"]
mod cooking_ingress_loss;
#[path = "../../../examples/cooking/tests/inspection.rs"]
mod cooking_inspection;
#[path = "../../../examples/cooking/tests/retirement.rs"]
mod cooking_retirement;
#[path = "../../../examples/cooking/tests/service_build.rs"]
mod cooking_service_build;
#[path = "../../../examples/cooking/tests/updates.rs"]
mod cooking_updates;
#[path = "service_helpers/revocation.rs"]
mod service_revocation;
#[path = "../../../examples/session-chat/tests/composed.rs"]
mod session_chat;
// The integration-test support tree is private to this test crate; its
// `pub(crate)` child boundaries are required by sibling fixture modules.
#[allow(clippy::redundant_pub_crate)]
mod support;

use support::backend_fixture;

#[tokio::test(flavor = "multi_thread")]
#[serial]
#[allow(
    clippy::drop_non_drop,
    clippy::too_many_lines,
    clippy::large_stack_frames
)]
async fn bearer_push_starts_run_through_production_bootstrap() {
    let Some(GoldenRunMode {
        workload_phase_timing,
        parent_database_url,
        nats_url,
        parent_before,
        isolated_database,
        database_url,
        libkrun_e2e,
        session_chat_e2e,
        session_chat_denial_probe_e2e: _,
        session_chat_restart_e2e: _,
        session_chat_browser_e2e,
        installed_ui_fixture,
        cooking_build_proof,
        release_build_proof,
        cooking_service_build_proof,
        caddy_tls,
        build_timeout,
        cooking_wait_timeout,
        browser_e2e,
        gateway_caddy_e2e,
        gateway_service_e2e,
        gateway_service_external_e2e,
        gateway_service_revocation_e2e: _,
        gateway_service_cutover_e2e: _,
        gateway_service_failed_candidate_e2e: _,
        gateway_service_candidate_capacity_e2e: _,
        gateway_service_rollback_e2e: _,
        gateway_service_log_rpc_e2e,
        gateway_service_log_guest_e2e,
    }) = prepare_golden_run_mode().await
    else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect golden PostgreSQL");
    Box::pin(async {
    let GoldenSeed {
        temporary: _,
        root,
        release_artifact_root,
        repository_root,
        fixture_repository,
        browser_oidc_issuer,
        user_id,
        organization_id,
        owner_browser_session,
        outsider_id,
        outsider_browser_session,
        project,
        repository,
        seeded_instance,
        mut brokered_fixture,
        mut session_chat_fixture,
        gateway_mailbox: _,
        gateway_service_fixture,
        gateway_edge,
        initial_gateway_edge,
    } = seed_golden_scenario(
        &pool,
        gateway_service_external_e2e,
        libkrun_e2e,
        cooking_build_proof,
        session_chat_e2e,
        gateway_caddy_e2e,
        gateway_service_e2e,
        installed_ui_fixture,
        session_chat_browser_e2e,
        gateway_service_log_guest_e2e,
    )
    .await;
    let GoldenBackendSetup {
        root_image,
        observer,
        app_config,
    } = configure_golden_backend(
        &pool,
        &root,
        &repository_root,
        workload_phase_timing,
        release_build_proof,
        libkrun_e2e,
        session_chat_e2e,
        &database_url,
        &nats_url,
        &browser_oidc_issuer,
        initial_gateway_edge,
        &brokered_fixture,
        &session_chat_fixture,
        build_timeout,
    )
    .await;
    if gateway_service_external_e2e {
        let service_fixture = gateway_service_fixture
            .as_ref()
            .expect("external persistent-service fixture");
        let (gateway_config, _) = gateway_edge
            .as_ref()
            .expect("external persistent-service gateway configuration");
        exercise_external_gateway_service_warm_path(
            &pool,
            service_fixture,
            gateway_config,
            &app_config,
            &root,
            &root_image,
            &release_artifact_root,
            &AuthenticatedIdentity::new(
                user_id,
                &browser_oidc_issuer,
                "golden-subject",
                serde_json::json!({}),
                RequestId::new(),
            ),
        )
        .await;
        cleanup_streams(&nats_url).await;
        return;
    }
    let app = HephaestusApp::build(app_config.clone())
        .await
        .expect("build production application");
    let daemon_readiness_timer =
        WorkloadPhaseTimer::start("gateway-readiness", workload_phase_timing);
    let running = async move { app.start().await }.await;
    daemon_readiness_timer.finish(running.is_ok());
    let running = running.expect("start ready application");
    let token = signed_token(if release_build_proof {
        Duration::from_secs(45 * 60)
    } else {
        Duration::from_secs(5 * 60)
    });
    if session_chat_e2e {
        run_session_chat_phase(
            &pool,
            &database_url,
            running,
            &root,
            project.id,
            organization_id,
            &fixture_repository,
            user_id,
            &browser_oidc_issuer,
            &owner_browser_session,
            &token,
            session_chat_fixture
                .take()
                .expect("session-chat broker fixture"),
            cooking_wait_timeout,
            &app_config,
            observer,
            &nats_url,
        )
        .await;
        return;
    }
    // The Cooking proof deliberately spans several production builds before
    // it creates the separate blog repository. Keep its fixture assertion
    // valid for the bounded 45-minute host trial while ordinary golden tests
    // retain the shorter token lifetime.
    if cooking_build_proof {
        let installed_ui_fixture =
            env::var("HEPHAESTUS_COOKING_INSTALLED_UI_FIXTURE").as_deref() == Ok("1");
        if installed_ui_fixture {
            assert!(
                browser_e2e,
                "installed UI fixture requires the browser E2E phase to be enabled"
            );
        }
        let source_root =
            PathBuf::from(env::var("HEPHAESTUS_COOKING_SOURCE_ROOT").expect("cooking source root"));
        let identity = AuthenticatedIdentity::new(
            user_id,
            &browser_oidc_issuer,
            "golden-subject",
            serde_json::json!({}),
            RequestId::new(),
        );
        let rpc_token = |audience: &str| {
            let now = OffsetDateTime::now_utc().unix_timestamp();
            encode(
                &Header::new(Algorithm::HS256),
                &serde_json::json!({
                    "iss": "hephaestus-web-mediator",
                    "sub": user_id.to_string(),
                    "aud": audience,
                    "iat": now,
                    "nbf": now,
                    "exp": now + 25,
                    "jti": uuid::Uuid::new_v4().to_string(),
                    "sid": owner_browser_session.to_protocol_string()
                }),
                &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(
                    b"golden-internal-command-token-with-sufficient-entropy",
                )),
            )
            .expect("sign cooking build-proof mediator token")
        };
        if env::var("HEPHAESTUS_COOKING_OCI_BASE_IMPORT_DIAGNOSTIC").as_deref() == Ok("1") {
            let context = cooking_builds::CookingBuildContext {
                pool: &pool,
                running: &running,
                root: &root,
                source_root: &source_root,
                project_id: project.id,
                repositories: &fixture_repository,
                identity: cooking_builds::CookingIdentity {
                    actor: &identity,
                    git_token: &token,
                    rpc_token: &rpc_token,
                },
                timeout: cooking_wait_timeout,
            };
            cooking_builds::create_cooking_blog_repository(&context)
                .await
                .expect("targeted cooking OCI base-import diagnostic");
            running
                .shutdown()
                .await
                .expect("targeted OCI diagnostic shutdown");
            cleanup_streams(&nats_url).await;
            return;
        }
        let retry_fixture =
            if browser_e2e || env::var("HEPHAESTUS_COOKING_UPDATE_E2E").as_deref() == Ok("1") {
                let retry_repository = fixture_repository
                    .create_repository_trusted(&CreateRepository {
                        project_id: project.id,
                        name: format!("golden-retry-source-{}", uuid::Uuid::new_v4()),
                        default_branch: GitRef::parse("refs/heads/main").expect("default ref"),
                        is_public: false,
                        agent_runs_enabled: true,
                    })
                    .await
                    .expect("seed browser retry source repository");
                let retry_instance = seed_reusable_instance(
                    &pool,
                    user_id,
                    project.id.as_uuid(),
                    retry_repository.id.as_uuid(),
                    &root.join("release-artifacts"),
                    libkrun_e2e,
                    Some(GOLDEN_AGENT.as_bytes()),
                    "golden-retry-source",
                )
                .await;
                let source = create_forge_source_run(
                    &pool,
                    &running,
                    &root,
                    retry_repository.id.as_uuid(),
                    retry_instance.instance,
                    &token,
                    libkrun_e2e,
                    true,
                    ForgeSourceContent::GoldenAgent,
                )
                .await;
                Some(ForgeRetryFixture {
                    repository_id: retry_repository.id.as_uuid(),
                    instance: retry_instance,
                    source_run_id: source.run_id,
                })
            } else {
                None
            };
        let cooking_context = cooking_builds::CookingBuildContext {
            pool: &pool,
            running: &running,
            root: &root,
            source_root: &source_root,
            project_id: project.id,
            repositories: &fixture_repository,
            identity: cooking_builds::CookingIdentity {
                actor: &identity,
                git_token: &token,
                rpc_token: &rpc_token,
            },
            timeout: cooking_wait_timeout,
        };
        let installed_reference_uis = if installed_ui_fixture {
            Some(
                cooking_builds::build_and_install_reference_uis(&cooking_context, organization_id)
                    .await
                    .expect("build and install reference UIs through production boundaries"),
            )
        } else {
            None
        };
        drop(cooking_context);
        let Some(service_phase) = run_cooking_service_proof(
            &pool,
            running,
            app_config,
            cooking_service_build_proof,
            caddy_tls,
            workload_phase_timing,
            &database_url,
            &nats_url,
            &root,
            &source_root,
            project.id,
            organization_id,
            user_id,
            &fixture_repository,
            &identity,
            &token,
            &rpc_token,
            cooking_wait_timeout,
            installed_reference_uis,
        )
        .await
        else {
            return;
        };
        let running = service_phase.running;
        let mut app_config = service_phase.app_config;
        let installed_reference_uis = service_phase.installed_reference_uis;
        let cooking_context = cooking_builds::CookingBuildContext {
            pool: &pool,
            running: &running,
            root: &root,
            source_root: &source_root,
            project_id: project.id,
            repositories: &fixture_repository,
            identity: cooking_builds::CookingIdentity {
                actor: &identity,
                git_token: &token,
                rpc_token: &rpc_token,
            },
            timeout: cooking_wait_timeout,
        };
        // These repositories share only their project: Python/Rust release builds
        // do not consume the blog's Hugo image. Keep same-family release mutations
        // serial while the independent OCI build and verification make progress.
        let (builds, adversarial_agent_build, adversarial_gateway_build, update_builds, blog_repository) =
            prepare_cooking_releases(
                &pool,
                &running,
                &root,
                &source_root,
                project.id,
                user_id,
                &fixture_repository,
                &identity,
                &token,
                &rpc_token,
                cooking_wait_timeout,
                workload_phase_timing,
            )
            .await;
        cooking_builds::wait_for_cooking_build_quiescence(
            &pool,
            project.id,
            "golden-cooking-oci-materialization",
            cooking_wait_timeout,
        )
        .await;
        let CookingInstanceSetup {
            instance,
            actual_instance,
            actual_brokered,
            cooking_update_rule_ids,
            cooking_inbound_placeholder,
            installed_gateway,
            foreign_instance,
            mut actual_grant_id,
        } = prepare_cooking_instance_setup(
            &pool,
            &cooking_context,
            user_id,
            organization_id,
            project.id,
            browser_e2e,
            &builds,
            &blog_repository,
            update_builds.as_ref(),
        )
        .await;
        actual_grant_id = run_initial_cooking_browser_phase(
            &pool,
            &database_url,
            &running,
            &root,
            browser_e2e,
            installed_reference_uis,
            organization_id,
            project.id,
            user_id,
            &builds,
            &instance,
            &actual_brokered,
            &cooking_inbound_placeholder,
            installed_gateway,
            workload_phase_timing,
            actual_grant_id,
        )
        .await;
        let mut actual_fixture = GatewayGoldenFixture {
            mailbox_id: MailboxId::from_uuid(instance.mailbox_id),
            grant_id: actual_grant_id.expect("cooking gateway mailbox grant after setup"),
        };
        let AdversarialSetup {
            configured: adversarial_configured,
            foreign_mailbox: adversarial_foreign_mailbox,
            canonical_revision_id,
            instance: adversarial_instance,
            rule_id: adversarial_rule_id,
        } = prepare_adversarial_setup(
            &pool,
            &cooking_context,
            &actual_brokered,
            &cooking_inbound_placeholder,
            &foreign_instance,
            &adversarial_gateway_build,
            &actual_instance,
            &adversarial_agent_build,
            &blog_repository,
            project.id,
            cooking_wait_timeout,
            &mut app_config,
        )
        .await;
        running
            .shutdown()
            .await
            .expect("build-proof daemon restart shutdown");
        let running = Box::pin(restart_application(app_config.clone())).await;
        let adversarial_url = format!(
            "{}/gateway/cooking/telegram",
            env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("public Caddy URL")
        );
        let adversarial_response = cooking::caddy_gateway_client()
            .post(adversarial_url)
            .header("x-telegram-bot-api-secret-token", cooking::INBOUND_SENTINEL)
            .json(&serde_json::json!({
                "update_id": 39,
                "message": {"from": {"id": 1001}, "text": "pasta"}
            }))
            .send()
            .await
            .expect("adversarial foreign publication request");
        assert_eq!(
            adversarial_response.status(),
            reqwest::StatusCode::BAD_GATEWAY,
            "undeclared gateway publication slot is denied by the host"
        );
        adversarial_response
            .bytes()
            .await
            .expect("read adversarial denial response");
        let denied_publications: Vec<(String, Option<String>, String)> = sqlx::query_as(
            "SELECT publication.outcome, publication.denial_code, invocation.outcome
               FROM gateway_mailbox_publications publication
               JOIN gateway_invocations invocation ON invocation.id = publication.invocation_id
              WHERE publication.gateway_revision_id = $1
                AND publication.slot_key = $2
                AND publication.deduplication_key = 'telegram-update-39'",
        )
        .bind(adversarial_configured.revision_id)
        .bind(cooking_builds::FOREIGN_PUBLICATION_SLOT)
        .fetch_all(&pool)
        .await
        .expect("exact adversarial publication denial");
        assert_eq!(
            denied_publications,
            vec![(
                String::from("denied"),
                Some(String::from("authority_unavailable")),
                String::from("failed"),
            )],
            "the guest must reach publication and receive a durable authority denial"
        );
        let (foreign_events, foreign_deliveries, foreign_attempts, foreign_runs): (
            i64,
            i64,
            i64,
            i64,
        ) = sqlx::query_as(
            "SELECT
                (SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1),
                (SELECT count(*) FROM mailbox_deliveries WHERE mailbox_id = $1),
                (SELECT count(*) FROM mailbox_delivery_attempts WHERE mailbox_id = $1),
                (SELECT count(*) FROM runs WHERE instance_id = (
                    SELECT instance_id FROM mailboxes WHERE id = $1
                ))",
        )
        .bind(adversarial_foreign_mailbox)
        .fetch_one(&pool)
        .await
        .expect("foreign mailbox denial effects");
        assert_eq!(
            (
                foreign_events,
                foreign_deliveries,
                foreign_attempts,
                foreign_runs
            ),
            (0, 0, 0, 0),
            "foreign mailbox must have no event, delivery, attempt, or run after denial"
        );
        let restored_context = cooking_builds::CookingBuildContext {
            pool: &pool,
            running: &running,
            root: &root,
            source_root: &source_root,
            project_id: project.id,
            repositories: &fixture_repository,
            identity: cooking_builds::CookingIdentity {
                actor: &identity,
                git_token: &token,
                rpc_token: &rpc_token,
            },
            timeout: cooking_wait_timeout,
        };
        let canonical_installed_gateway = cooking_builds::install_cooking_gateway(
            &restored_context,
            builds.gateway.release_id,
            builds.gateway.repository_id,
        )
        .await
        .expect("reinstall canonical gateway after adversarial probe");
        let restored = cooking_builds::configure_cooking_gateway(
            &restored_context,
            canonical_installed_gateway,
            cooking_builds::cooking_gateway_parameters(&cooking_inbound_placeholder, 1001, 1002),
            actual_brokered.import_id,
            actual_brokered.version_id,
            instance.mailbox_id,
        )
        .await
        .expect("restore canonical gateway after adversarial probe");
        actual_fixture.grant_id = restored.grant_id;
        let checkpoint = cooking::exercise_initial(&pool, &actual_fixture).await;
        cooking::wait_for_checkpoint_runs(&pool, &checkpoint, restored_context.timeout).await;
        let canonical_run_ids = checkpoint.run_ids();
        // Recipe 42 above is the canonical positive control. Capture its
        // durable adapter effects before routing one mailbox-local event to a
        // separate adversarial instance. Scope both measurements to these
        // settled run IDs so unrelated canonical activity cannot move the
        // comparison window.
        let (baseline_model_calls, baseline_relay_calls): (i64, i64) = sqlx::query_as(
            "SELECT
                 count(*) FILTER (WHERE audit.rule_id = $2),
                 count(*) FILTER (WHERE audit.rule_id = $3)
               FROM brokered_secret_audit_events AS audit
               JOIN runs AS run ON run.id = audit.run_id
              WHERE run.id = ANY($1)
                AND audit.event_kind = 'substitution_use'",
        )
        .bind(canonical_run_ids.to_vec())
        .bind(cooking::MODEL_RULE)
        .bind(cooking::RELAY_RULE)
        .fetch_one(&pool)
        .await
        .expect("canonical cooking adapter baseline");
        let (gateway_id, gateway_revision_id): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
            "SELECT gateway.id, gateway.active_revision_id
               FROM gateways AS gateway
               JOIN gateway_revisions AS revision
                 ON revision.gateway_id = gateway.id
                AND revision.id = gateway.active_revision_id
               JOIN gateway_mailbox_bindings AS binding
                 ON binding.gateway_revision_id = revision.id
               JOIN gateway_mailbox_binding_grants AS grant_row
                 ON grant_row.binding_id = binding.id
              WHERE grant_row.id = $1
                AND grant_row.status = 'active'",
        )
        .bind(actual_fixture.grant_id)
        .fetch_one(&pool)
        .await
        .expect("canonical active gateway identity");
        let adversarial_probe = cooking_adversarial_agent::exercise_adversarial_agent_probe(
            cooking_adversarial_agent::AdversarialAgentProbeInput {
                context: &restored_context,
                gateway: cooking_builds::InstalledCookingGateway {
                    gateway_id,
                    revision_id: gateway_revision_id,
                },
                adversarial_instance,
                canonical_run_ids,
                baseline_model_calls,
                baseline_relay_calls,
                adversarial_rule_id,
                inbound_import_id: actual_brokered.import_id,
                inbound_version_id: actual_brokered.version_id,
                inbound_placeholder: &cooking_inbound_placeholder,
                inbound_wire_credential: cooking::INBOUND_SENTINEL,
                public_url: &env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
                    .expect("joined Caddy public URL"),
            },
        )
        .await
        .expect("adversarial cooking agent denial probe");
        assert!(!adversarial_probe.event_id.is_nil());
        assert!(!adversarial_probe.run_id.is_nil());
        assert_eq!(adversarial_probe.mismatched_rule_id, adversarial_rule_id);
        assert_eq!(adversarial_probe.deny_decisions, 1);
        assert_eq!(adversarial_probe.substitution_uses, 0);
        let restored_adversarial_gateway = cooking_builds::install_cooking_gateway(
            &restored_context,
            builds.gateway.release_id,
            builds.gateway.repository_id,
        )
        .await
        .expect("reinstall canonical gateway after agent probe");
        let restored_adversarial = cooking_builds::configure_cooking_gateway(
            &restored_context,
            restored_adversarial_gateway,
            cooking_builds::cooking_gateway_parameters(&cooking_inbound_placeholder, 1001, 1002),
            actual_brokered.import_id,
            actual_brokered.version_id,
            instance.mailbox_id,
        )
        .await
        .expect("restore canonical gateway after agent probe");
        actual_fixture.grant_id = restored_adversarial.grant_id;
        // Prepare the transformed guest and its two new rule identities before
        // the existing supervisor restart. This follows the initial and
        // adversarial controls, while canonical/crash adapters are joined by
        // immutable rule UUID and canonical counts remain unchanged.
        let crash_agent_build =
            cooking_builds::build_and_publish_guest_crash_agent(&restored_context, &builds.agent)
                .await
                .expect("publish deterministic guest crash agent");
        let crash_instance = cooking_adversarial_agent::prepare_brokered_instance_with_rule_ids(
            &restored_context,
            crash_agent_build.release_agent_id,
            blog_repository.repository_id,
            canonical_revision_id,
            "cooking-agent-guest-crash",
            "cooking-agent-guest-crash",
            cooking_adversarial_agent::BrokeredRuleIds {
                model: cooking::CRASH_MODEL_RULE,
                relay: cooking::CRASH_RELAY_RULE,
            },
        )
        .await
        .expect("prepare guest crash instance and rule specs");
        let crash_upstream = cooking::cooking_crash_upstreams(vec![
            cooking::brokered_rule_for_spec(&crash_instance.model),
            cooking::brokered_rule_for_spec(&crash_instance.relay),
        ])
        .await;
        let crash_gateway_id: uuid::Uuid =
            sqlx::query_scalar("SELECT gateway_id FROM gateway_revisions WHERE id = $1")
                .bind(restored_adversarial.revision_id)
                .fetch_one(&pool)
                .await
                .expect("guest crash gateway identity");
        let crash_gateway = cooking_builds::configure_cooking_gateway(
            &restored_context,
            cooking_builds::InstalledCookingGateway {
                gateway_id: crash_gateway_id,
                revision_id: restored_adversarial.revision_id,
            },
            cooking_builds::cooking_gateway_parameters(&cooking_inbound_placeholder, 1001, 1002),
            actual_brokered.import_id,
            actual_brokered.version_id,
            crash_instance.instance.mailbox_id,
        )
        .await
        .expect("route cooking gateway to guest crash mailbox");
        app_config.secret_broker_adapter =
            actual_brokered.upstream.combined_adapter(&crash_upstream);
        cooking_builds::wait_for_cooking_build_quiescence(
            &pool,
            project.id,
            "golden-cooking-oci-materialization",
            cooking_wait_timeout,
        )
        .await;
        running
            .shutdown()
            .await
            .expect("cooking daemon graceful restart shutdown");
        let restarted = Box::pin(restart_application(app_config)).await;
        let crash_fixture = GatewayGoldenFixture {
            mailbox_id: mailbox_domain::MailboxId::from_uuid(crash_instance.instance.mailbox_id),
            grant_id: crash_gateway.grant_id,
        };
        cooking_guest_crash::exercise(&pool, &crash_fixture, &crash_instance.instance).await;
        cooking_guest_crash::wait_for_upstream(crash_upstream).await;
        let crash_disk: PathBuf = PathBuf::from(
            sqlx::query_scalar::<_, String>(
                "SELECT host_path FROM agent_instance_state_volumes
              WHERE instance_id = $1",
            )
            .bind(crash_instance.instance.instance_id)
            .fetch_one(&pool)
            .await
            .expect("load guest crash state volume disk path"),
        );
        let restarted_context = cooking_builds::CookingBuildContext {
            pool: &pool,
            running: &restarted,
            root: &root,
            source_root: &source_root,
            project_id: project.id,
            repositories: &fixture_repository,
            identity: cooking_builds::CookingIdentity {
                actor: &identity,
                git_token: &token,
                rpc_token: &rpc_token,
            },
            timeout: cooking_wait_timeout,
        };
        let restored_after_crash = cooking_builds::configure_cooking_gateway(
            &restarted_context,
            cooking_builds::InstalledCookingGateway {
                gateway_id: crash_gateway_id,
                revision_id: crash_gateway.revision_id,
            },
            cooking_builds::cooking_gateway_parameters(&cooking_inbound_placeholder, 1001, 1002),
            actual_brokered.import_id,
            actual_brokered.version_id,
            instance.mailbox_id,
        )
        .await
        .expect("restore canonical gateway after guest crash branch");
        actual_fixture.grant_id = restored_after_crash.grant_id;
        let resolved_head = cooking::exercise_follow_up(
            &pool,
            &restarted,
            &actual_instance,
            &actual_fixture,
            &root,
            blog_repository.repository_id.as_uuid(),
            &blog_repository.source_commit,
            checkpoint,
            &actual_brokered.upstream,
            owner_browser_session,
            outsider_id,
            outsider_browser_session,
        )
        .await;
        let blog_artifact = cooking_blog_artifact::build_publish_and_verify(
            &cooking_builds::CookingBuildContext {
                pool: &pool,
                running: &restarted,
                root: &root,
                source_root: &source_root,
                project_id: project.id,
                repositories: &fixture_repository,
                identity: cooking_builds::CookingIdentity {
                    actor: &identity,
                    git_token: &token,
                    rpc_token: &rpc_token,
                },
                timeout: cooking_wait_timeout,
            },
            &blog_repository,
            &resolved_head,
            "Family pasta",
            outsider_id,
            outsider_browser_session,
        )
        .await
        .expect("build and retrieve published cooking blog artifact");
        assert_eq!(blog_artifact.source_commit, resolved_head);
        assert!(!blog_artifact.build_id.is_nil());
        assert!(!blog_artifact.release_id.is_nil());
        assert!(!blog_artifact.artifact_id.is_nil());
        assert_eq!(blog_artifact.sha256.len(), 64);
        eprintln!(
            "cooking blog artifact: source_commit={}, build={}, release={}, artifact={}, sha256={}",
            blog_artifact.source_commit,
            blog_artifact.build_id,
            blog_artifact.release_id,
            blog_artifact.artifact_id,
            blog_artifact.sha256
        );
        let mut update_state_volume_disk: Option<PathBuf> = None;
        let browser_abnormal_release_agent_id = update_builds
            .as_ref()
            .map(|builds| builds.abnormal.release_agent_id);
        let update_sequence = if let Some(update_builds) = update_builds {
            let current_revision_id: uuid::Uuid =
                sqlx::query_scalar("SELECT active_revision_id FROM agent_instances WHERE id = $1")
                    .bind(actual_instance.instance)
                    .fetch_one(&pool)
                    .await
                    .expect("load configured cooking revision");
            let sequence = cooking_updates::exercise_barrier_update_sequence(
                &cooking_updates::CookingUpdateContext {
                    pool: &pool,
                    running: &restarted,
                    instance_id: actual_instance.instance,
                    gateway: &actual_fixture,
                    current_revision_id,
                    owner: user_id.as_uuid(),
                    rpc_token: &rpc_token,
                    brokered_rule_ids: cooking_updates::BrokeredRuleIds {
                        model: cooking::MODEL_RULE,
                        relay: cooking::RELAY_RULE,
                    },
                    timeout: cooking_wait_timeout,
                },
                &actual_brokered.upstream,
                cooking_updates::CookingUpdateCandidates {
                    migrate_release_agent_id: update_builds.migrate.release_agent_id,
                    rollback_release_agent_id: update_builds.rollback.release_agent_id,
                    abnormal_release_agent_id: update_builds.abnormal.release_agent_id,
                    migrate_rule_ids: cooking_update_rule_ids
                        .expect("update rule IDs allocated with update builds")
                        .0,
                    rollback_rule_ids: cooking_update_rule_ids
                        .expect("update rule IDs allocated with update builds")
                        .1,
                    abnormal_rule_ids: cooking_update_rule_ids
                        .expect("update rule IDs allocated with update builds")
                        .2,
                },
            )
            .await;
            let disk: String = sqlx::query_scalar(
                "SELECT host_path FROM agent_instance_state_volumes
                 WHERE instance_id = $1",
            )
            .bind(actual_instance.instance)
            .fetch_one(&pool)
            .await
            .expect("load cooking state volume disk path");
            update_state_volume_disk = Some(PathBuf::from(disk));
            Some(sequence)
        } else {
            None
        };
        if update_sequence.is_some() {
            // The update helper rotates relay while event 47 still holds the
            // v1 model lease, before CreateUpdate clones candidate rules.
            // Event 46 remains the completed old-version relay proof.
            let sequence = update_sequence.as_ref().expect("cooking update sequence");
            let relay_run_id = sequence.relay_run_id;
            let relay_rotation = sequence.relay_rotation;
            let inbound_rotation = cooking::rotate_inbound_credential(
                &pool,
                user_id,
                actual_brokered.import_id,
                actual_brokered.version_id,
            )
            .await;
            actual_fixture = cooking_authority::configure_rotated_inbound_gateway(
                &pool,
                &restarted,
                &actual_fixture,
                &rpc_token,
                actual_brokered.import_id,
                inbound_rotation.rotated_version_id,
                instance.mailbox_id,
            )
            .await
            .expect("configure rotated cooking inbound credential");
            let public =
                env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL");
            let url = format!("{public}/gateway/cooking/telegram");
            let client = cooking::caddy_gateway_client();
            let old = cooking::send_update_with_credential(
                &client,
                &url,
                49,
                1001,
                "rotation",
                cooking::INBOUND_SENTINEL,
            )
            .await;
            assert_eq!(old.status(), reqwest::StatusCode::UNAUTHORIZED);
            old.bytes().await.expect("old inbound rotation denial");
            let accepted = cooking::send_update_with_credential(
                &client,
                &url,
                49,
                1001,
                "rotation",
                cooking::INBOUND_ROTATED_SENTINEL,
            )
            .await;
            assert_eq!(accepted.status(), reqwest::StatusCode::OK);
            accepted
                .bytes()
                .await
                .expect("rotated inbound acknowledgement");
            let rotated_run = cooking::wait_for_event_run(&pool, &actual_fixture, 49).await;
            cooking::assert_rotated_brokered_lease(
                &pool,
                relay_run_id,
                rotated_run.event_id,
                cooking::RELAY_RULE,
                sequence.migration.relay_rule_id,
                relay_rotation,
            )
            .await;
            cooking_authority::assert_inbound_lease_history(
                &pool,
                &actual_fixture,
                inbound_rotation.pinned_version_id,
                inbound_rotation.rotated_version_id,
            )
            .await
            .expect("inbound lease rotation history");
        }
        if browser_e2e {
            let (completed_run_id, result_commit, result_ref, target_ref): (
                uuid::Uuid,
                String,
                String,
                String,
            ) = sqlx::query_as(
                "SELECT run.id, result.result_commit, result.result_ref,
                        proposal.target_ref
                   FROM runs run
                   JOIN run_results result ON result.run_id = run.id
                   JOIN review_proposals proposal ON proposal.run_id = run.id
                  WHERE run.instance_id = $1 AND result.state = 'completed'
                    AND proposal.state = 'approved'
                    AND result.result_commit IS NOT NULL
                  ORDER BY result.completed_at DESC, result.id DESC
                  LIMIT 1",
            )
            .bind(actual_instance.instance)
            .fetch_one(&pool)
            .await
            .expect("completed cooking result for browser inspection");
            // The browser must exercise the approval command against a real
            // completed proposal.  The fault-recovery requests in the joined
            // scenario intentionally remain open after their provenance
            // inspection, so select the newest such proposal instead of
            // fabricating a pending row for the UI fixture.
            let (pending_run_id, pending_proposal_id): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
                "SELECT run.id, proposal.id
                       FROM runs run
                       JOIN review_proposals proposal ON proposal.run_id = run.id
                      WHERE run.instance_id = $1
                        AND proposal.state IN ('open', 'approval_requested')
                        AND EXISTS (
                            SELECT 1 FROM run_results result
                             WHERE result.run_id = run.id
                               AND result.state = 'completed'
                               AND result.result_commit IS NOT NULL
                        )
                      ORDER BY proposal.created_at DESC, proposal.id DESC
                      LIMIT 1",
            )
            .bind(actual_instance.instance)
            .fetch_one(&pool)
            .await
            .expect("open cooking result proposal for browser approval");
            let proposal_id: uuid::Uuid = sqlx::query_scalar(
                "SELECT id FROM review_proposals WHERE run_id = $1 ORDER BY id LIMIT 1",
            )
            .bind(completed_run_id)
            .fetch_one(&pool)
            .await
            .expect("completed cooking review proposal");
            let retry_run_ids_before: Vec<uuid::Uuid> = sqlx::query_scalar(
                "SELECT run_id FROM run_requests WHERE retry_of_run_id = $1 ORDER BY run_id",
            )
            .bind(
                retry_fixture
                    .as_ref()
                    .expect("browser retry source run fixture")
                    .source_run_id,
            )
            .fetch_all(&pool)
            .await
            .expect("browser retry ancestry before browser actions");
            let post_path = format!(
                "{}.post.json",
                env::var("HEPHAESTUS_COOKING_BROWSER_FIXTURE_OUTPUT")
                    .expect("cooking browser fixture output path")
            );
            let post_fixture = serde_json::json!({
                "organization_id": organization_id,
                "project_id": project.id,
                "repository_id": builds.gateway.repository_id,
                "release_id": builds.gateway.release_id,
                "release_agent_id": builds.agent.release_agent_id,
                "instance_id": instance.instance_id,
                "mailbox_id": instance.mailbox_id,
                "gateway_id": sqlx::query_scalar::<_, uuid::Uuid>(
                    "SELECT binding.gateway_id
                       FROM gateway_mailbox_bindings binding
                       JOIN gateway_mailbox_binding_grants grant_row
                         ON grant_row.binding_id = binding.id
                      WHERE grant_row.id = $1",
                )
                .bind(actual_fixture.grant_id)
                .fetch_one(&pool)
                .await
                .expect("post-operation cooking gateway id"),
                "completed_run_id": completed_run_id,
                "result_commit": result_commit,
                "result_ref": result_ref,
                "target_ref": target_ref,
                "proposal_id": proposal_id,
                "pending_run_id": pending_run_id,
                "pending_proposal_id": pending_proposal_id,
                "retry_source_run_id": retry_fixture
                    .as_ref()
                    .expect("browser retry source run fixture")
                    .source_run_id,
                "migration_update_id": update_sequence.as_ref().map(|s| s.migration.update_id),
                "abnormal_update_id": update_sequence.as_ref().map(|s| s.abnormal.update_id),
                "abnormal_candidate_revision_id": update_sequence
                    .as_ref()
                    .map(|s| s.abnormal.candidate_revision_id),
                "abnormal_release_agent_id": browser_abnormal_release_agent_id,
                "browser_model_rule_id": cooking_update_rule_ids
                    .map(|(_, _, _, browser)| browser.model),
                "browser_relay_rule_id": cooking_update_rule_ids
                    .map(|(_, _, _, browser)| browser.relay),
                "browser_rule_copies": cooking_update_rule_ids.map(|(migration, _, _, browser)| {
                    vec![
                        serde_json::json!({
                            "source_rule_id": migration.model,
                            "candidate_rule_id": browser.model,
                        }),
                        serde_json::json!({
                            "source_rule_id": migration.relay,
                            "candidate_rule_id": browser.relay,
                        }),
                    ]
                }),
            });
            tokio::fs::write(
                &post_path,
                serde_json::to_vec_pretty(&post_fixture)
                    .expect("post-operation browser fixture JSON"),
            )
            .await
            .expect("write post-operation browser fixture JSON");
            let issuer = env::var("HEPHAESTUS_COOKING_BROWSER_OIDC_ISSUER")
                .expect("cooking browser OIDC issuer");
            let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../scripts/run-ui-e2e-external.sh");
            let browser_timer =
                WorkloadPhaseTimer::start("browser-post-operation", workload_phase_timing);
            let status = tokio::process::Command::new(script)
                .env("HEPHAESTUS_E2E_COOKING_FIXTURE", &post_path)
                .env("HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL", &database_url)
                .env(
                    "HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT",
                    restarted.http_addr().to_string(),
                )
                .env(
                    "HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET",
                    "golden-internal-command-token-with-sufficient-entropy",
                )
                .env("HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER", issuer)
                .env("HEPHAESTUS_E2E_COOKING_PHASE", "post-operation")
                .env("HEPHAESTUS_E2E_BROWSER_RUNNER", "legacy")
                .status()
                .await;
            browser_timer.finish(status.as_ref().is_ok_and(std::process::ExitStatus::success));
            let status = status.expect("run cooking post-operation browser E2E");
            assert!(
                status.success(),
                "cooking post-operation browser E2E failed: {status}"
            );
            // Retry is a separate run command. Do not tear down the daemon
            // while its guest is still cleaning up; otherwise the subsequent
            // NATS and state-volume scans could miss a real active resource.
            let retry_run_ids_after: Vec<uuid::Uuid> = sqlx::query_scalar(
                "SELECT run_id FROM run_requests WHERE retry_of_run_id = $1 ORDER BY run_id",
            )
            .bind(
                retry_fixture
                    .as_ref()
                    .expect("browser retry source run fixture")
                    .source_run_id,
            )
            .fetch_all(&pool)
            .await
            .expect("browser retry ancestry after browser actions");
            assert_eq!(
                retry_run_ids_after.len(),
                retry_run_ids_before.len() + 1,
                "browser retry creates exactly one request for the selected source run"
            );
            assert!(
                retry_run_ids_before
                    .iter()
                    .all(|id| retry_run_ids_after.contains(id))
            );
            let new_retry_ids: Vec<_> = retry_run_ids_after
                .iter()
                .filter(|id| !retry_run_ids_before.contains(id))
                .copied()
                .collect();
            assert_eq!(new_retry_ids.len(), 1);
            let retry_run_id = new_retry_ids[0];
            let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
            loop {
                let (state, outcome): (String, Option<String>) =
                    sqlx::query_as("SELECT state, outcome FROM runs WHERE id = $1")
                        .bind(retry_run_id)
                        .fetch_one(&pool)
                        .await
                        .expect("poll exact browser retry run");
                if state == "cleaned_up" {
                    assert_eq!(
                        outcome.as_deref(),
                        Some("succeeded"),
                        "browser retry succeeds for the dedicated forge source"
                    );
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "browser retry run did not reach terminal cleanup"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if let Some(sequence) = update_sequence {
                let abnormal_release_agent_id =
                    browser_abnormal_release_agent_id.expect("browser abnormal release agent");
                let state: (String, String, bool) = sqlx::query_as(
                    "SELECT update_record.state, instance.state, instance.run_gate_open
                       FROM agent_updates update_record
                       JOIN agent_instances instance ON instance.id = update_record.instance_id
                      WHERE update_record.id = $1",
                )
                .bind(sequence.abnormal.update_id)
                .fetch_one(&pool)
                .await
                .expect("browser recovery update state");
                assert_eq!(
                    state,
                    (
                        String::from("rejected"),
                        String::from("update_rejected"),
                        true
                    )
                );
                let browser_updates: i64 = sqlx::query_scalar(
                    "SELECT count(*)
                       FROM agent_updates update_record
                       JOIN agent_instance_revisions candidate
                         ON candidate.id = update_record.candidate_revision_id
                      WHERE update_record.instance_id = $1
                        AND candidate.release_agent_id = $2
                        AND update_record.state = 'rejected'",
                )
                .bind(actual_instance.instance)
                .bind(abnormal_release_agent_id)
                .fetch_one(&pool)
                .await
                .expect("browser-created abnormal update state");
                assert_eq!(
                    browser_updates, 2,
                    "both abnormal updates must be recovered"
                );
            }
        }
        if update_sequence.is_some() {
            cooking::exercise_active_relay_revocation(
                &pool,
                &actual_fixture,
                user_id,
                &actual_brokered.upstream,
                update_sequence
                    .as_ref()
                    .expect("cooking update sequence")
                    .migration
                    .relay_rule_id,
            )
            .await;
        }
        actual_brokered.upstream.assert_substituted_request().await;
        let inbound_credential = if update_sequence.is_some() {
            cooking::INBOUND_ROTATED_SENTINEL
        } else {
            cooking::INBOUND_SENTINEL
        };
        cooking_authority::retire_cooking_gateway_grant(
            &pool,
            &restarted,
            &actual_fixture,
            &rpc_token,
            inbound_credential,
            outsider_id,
            outsider_browser_session,
        )
        .await
        .expect("retire cooking gateway mailbox grant through RPC");
        if update_sequence.is_some() {
            let retained_run_id: uuid::Uuid = sqlx::query_scalar(
                "SELECT id
                   FROM runs
                  WHERE instance_id = $1
                    AND run_kind = 'normal'
                    AND state = 'cleaned_up'
                    AND outcome = 'succeeded'
                  ORDER BY updated_at DESC, id DESC
                  LIMIT 1",
            )
            .bind(actual_instance.instance)
            .fetch_one(&pool)
            .await
            .expect("completed cooking run retained for retirement proof");
            let gateway_id: uuid::Uuid = sqlx::query_scalar(
                "SELECT gateway_id
                   FROM gateway_mailbox_bindings
                  WHERE id = (
                      SELECT binding_id
                        FROM gateway_mailbox_binding_grants
                       WHERE id = $1
                  )",
            )
            .bind(actual_fixture.grant_id)
            .fetch_one(&pool)
            .await
            .expect("cooking gateway identity for retirement proof");
            let outsider_identity = AuthenticatedIdentity::new(
                outsider_id,
                browser_oidc_issuer.clone(),
                "cooking-outsider",
                serde_json::json!({}),
                RequestId::new(),
            );
            let retirement_public = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
                .expect("joined Caddy public URL for retirement proof");
            let retry_fixture = retry_fixture
                .as_ref()
                .expect("retirement retry forge fixture");
            cooking_retirement::exercise(&cooking_retirement::RetirementContext {
                pool: &pool,
                running: &restarted,
                token_factory: &rpc_token,
                owner: &identity,
                outsider: &outsider_identity,
                instance: &actual_instance,
                retained_run_id,
                retry_instance: &retry_fixture.instance,
                retry_source_run_id: retry_fixture.source_run_id,
                retry_repository_id: retry_fixture.repository_id,
                gateway_id,
                project_id: project.id.as_uuid(),
                mailbox_id: instance.mailbox_id,
                public_url: &retirement_public,
                valid_inbound_credential: inbound_credential,
                import_parameters: cooking_builds::cooking_agent_parameters(),
            })
            .await
            .expect("retire cooking attachment, gateway, and release");
            // Source revocation follows grant retirement so the valid rotated
            // inbound credential proves the grant fence itself.
            cooking::revoke_imported_credential(&pool, user_id, actual_brokered.import_id).await;
        }
        cooking_confinement::assert_database_has_no_credentials(&pool).await;
        restarted.shutdown().await.expect("cooking daemon shutdown");
        let observer = observer.expect("cooking build proof VM observer");
        support::vm_observer_assertions::assert_cooking_vm_contracts(
            &pool,
            &observer,
            support::vm_observer_assertions::CookingVmContractIds {
                canonical_mailbox_id: instance.mailbox_id,
                crash_mailbox_id: crash_instance.instance.mailbox_id,
                adversarial_run_id: adversarial_probe.run_id,
                gateway_build_request_id: builds.gateway.build_request_id,
                agent_build_request_id: builds.agent.build_request_id,
                crash_agent_build_request_id: crash_agent_build.build_request_id,
            },
        )
        .await;
        cooking_guest_crash::assert_sqlite_disk(&crash_disk);
        cooking_confinement::assert_nats_has_no_credentials(&nats_url).await;
        if let Some(disk) = update_state_volume_disk {
            cooking_updates::assert_migrated_sqlite_disk(&disk, 9);
        }
        cleanup_streams(&nats_url).await;
        return;
    }
    let forge_source = create_forge_source_run(
        &pool,
        &running,
        &root,
        repository.id.as_uuid(),
        seeded_instance.instance,
        &token,
        libkrun_e2e,
        false,
        if cooking::enabled() {
            ForgeSourceContent::CookingBlog
        } else {
            ForgeSourceContent::GoldenAgent
        },
    )
    .await;
    let input_commit = forge_source.input_commit;
    let run_id = runtime_types::RunId::from_uuid(forge_source.run_id);

    if env::var("HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E").as_deref() == Ok("1") {
        #[cfg(feature = "test-fixtures")]
        {
            let admission_instance = support::rpc::update_admission::UpdateAdmissionInstance {
                instance_id: seeded_instance.instance,
                revision_id: seeded_instance.revision,
                release_id: seeded_instance.release,
                release_agent_id: seeded_instance.release_agent,
                attachment_id: seeded_instance.attachment,
            };
            let race = support::update_admission::exercise_reconciler_wins_race(
                &pool,
                &running,
                &admission_instance,
                user_id.as_uuid(),
                owner_browser_session,
            )
            .await;
            assert_eq!(race.initial_hook_run_id, race.retried_hook_run_id);
            assert_eq!(race.retried_hook_run_id, race.owner_recovery_hook_run_id);
            running
                .shutdown()
                .await
                .expect("app update race regression shutdown");
            cleanup_streams(&nats_url).await;
            return;
        }
        #[cfg(not(feature = "test-fixtures"))]
        assert_ne!(
            env::var("HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E").as_deref(),
            Ok("1"),
            "HEPHAESTUS_APP_UPDATE_ADMISSION_RACE_E2E requires --features hephaestus-app/test-fixtures"
        );
    }

    if env::var("HEPHAESTUS_APP_UPDATE_ADMISSION_E2E").as_deref() == Ok("1") {
        let admission_instance = support::rpc::update_admission::UpdateAdmissionInstance {
            instance_id: seeded_instance.instance,
            revision_id: seeded_instance.revision,
            release_id: seeded_instance.release,
            release_agent_id: seeded_instance.release_agent,
            attachment_id: seeded_instance.attachment,
        };
        let admission = support::update_admission::exercise(
            &pool,
            &running,
            &admission_instance,
            user_id.as_uuid(),
            owner_browser_session,
        )
        .await;
        assert_ne!(admission.update_id, uuid::Uuid::nil());
        assert_ne!(admission.initial_hook_run_id, admission.retried_hook_run_id);
        assert_ne!(
            admission.retried_hook_run_id,
            admission.owner_recovery_hook_run_id
        );
        running
            .shutdown()
            .await
            .expect("app update regression shutdown");
        cleanup_streams(&nats_url).await;
        return;
    }

    if cooking::enabled() {
        running
            .shutdown()
            .await
            .expect("cooking daemon startup recovery shutdown");
        let running = Box::pin(restart_application(app_config.clone())).await;
        let checkpoint = cooking::exercise_initial(
            &pool,
            &gateway_edge.as_ref().expect("cooking gateway").1,
        )
        .await;
        running
            .shutdown()
            .await
            .expect("cooking daemon graceful restart shutdown");
        let restarted = Box::pin(restart_application(app_config)).await;
        let _ = cooking::exercise_follow_up(
            &pool,
            &restarted,
            &seeded_instance,
            &gateway_edge.as_ref().expect("cooking gateway").1,
            &root,
            repository.id.as_uuid(),
            &input_commit,
            checkpoint,
            &brokered_fixture.as_ref().expect("cooking broker").upstream,
            owner_browser_session,
            outsider_id,
            outsider_browser_session,
        )
        .await;
        brokered_fixture
            .take()
            .expect("cooking broker")
            .upstream
            .assert_substituted_request()
            .await;
        restarted.shutdown().await.expect("cooking daemon shutdown");
        cleanup_streams(&nats_url).await;
        return;
    }

    if libkrun_e2e {
        let mut running = running;
        let (mut service_instance_id, mut service_resource_paths) = if gateway_caddy_e2e {
            let service_instance_id =
                if let Some(service_fixture) = gateway_service_fixture.as_ref() {
                    Some(wait_for_gateway_service_ready(&pool, service_fixture).await)
                } else {
                    None
                };
            let service_resource_paths = service_instance_id.map(|instance_id| {
                let vm_id = format!("gateway-service-{instance_id}");
                let provider_runtime_root = PathBuf::from(
                    env::var("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT")
                        .expect("libkrun runtime root for cleanup assertion"),
                );
                let cgroup_root = PathBuf::from(
                    env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT")
                        .expect("libkrun cgroup root for cleanup assertion"),
                );
                (
                    provider_runtime_root.join(&vm_id),
                    cgroup_root.join(&vm_id),
                    root.join("run-runtime")
                        .join("gateway-services")
                        .join(instance_id.to_string()),
                )
            });
            if let Some((provider_runtime, cgroup, materializer)) = &service_resource_paths {
                assert!(
                    provider_runtime.is_dir(),
                    "service VM runtime exists before shutdown"
                );
                assert!(cgroup.is_dir(), "service VM cgroup exists before shutdown");
                assert!(
                    materializer.is_dir(),
                    "service materializer tree exists before shutdown"
                );
            }
            (service_instance_id, service_resource_paths)
        } else {
            (None, None)
        };
        let public_url = if gateway_caddy_e2e {
            let Some(state) = run_gateway_service_lifecycle(
                &pool,
                running,
                &root,
                &app_config,
                gateway_caddy_e2e,
                &gateway_service_fixture,
                gateway_service_e2e,
                gateway_service_log_guest_e2e,
                gateway_service_log_rpc_e2e,
                project.id,
                user_id,
                &owner_browser_session,
                outsider_id,
                &outsider_browser_session,
                &nats_url,
                service_instance_id,
                service_resource_paths,
            )
            .await
            else {
                return;
            };
            running = state.running;
            service_instance_id = state.service_instance_id;
            service_resource_paths = state.service_resource_paths;
            state.public_url
        } else {
            None
        };
        run_libkrun_gateway_requests(
            &pool,
            &nats_url,
            gateway_caddy_e2e,
            &gateway_edge,
            &gateway_service_fixture,
            public_url,
            gateway_service_e2e,
            user_id,
            running,
            &mut brokered_fixture,
            service_instance_id,
            service_resource_paths,
        )
        .await;
        return;
    }

    run_result_and_mailbox_phase(
        &pool,
        &root,
        repository.id.as_uuid(),
        run_id,
        &input_commit,
        user_id,
        project.id.as_uuid(),
        &seeded_instance,
        running,
        &mut brokered_fixture,
        &nats_url,
    )
    .await;
    }).await;
    finish_isolated_golden(
        isolated_database,
        pool,
        &database_url,
        &parent_database_url,
        parent_before,
    )
    .await;
}
