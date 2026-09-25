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
pub(crate) use gateway_service_log_rpc::{
    GUEST_SERVICE_LOG_STDERR_MARKER, GUEST_SERVICE_LOG_STDOUT_MARKER, GatewayServiceGuestLogProof,
    GatewayServiceLogRpcFixture, marker_count,
};

#[path = "golden/mod.rs"]
pub(crate) mod golden_modules;
pub(crate) use golden_modules::cooking_builds;
pub(crate) use golden_modules::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
#[allow(
    clippy::drop_non_drop,
    clippy::large_futures,
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
            temporary: _temporary,
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
            brokered_fixture,
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
            let input = CookingFlowInput {
                pool: pool.clone(),
                running,
                app_config,
                database_url: database_url.clone(),
                nats_url,
                root,
                fixture_repository,
                browser_oidc_issuer,
                user_id,
                organization_id,
                owner_browser_session,
                outsider_id,
                outsider_browser_session,
                project,
                observer,
                workload_phase_timing,
                release_build_proof,
                libkrun_e2e,
                session_chat_e2e,
                installed_ui_fixture,
                browser_e2e,
                cooking_service_build_proof,
                caddy_tls,
                cooking_wait_timeout,
            };
            let Some(mut preparation) = prepare_cooking_flow(input).await else {
                return;
            };
            run_adversarial_gateway_denial(&mut preparation).await;
            run_cooking_crash_branch(&mut preparation).await;
            return;
        }
        run_standard_flow(StandardFlowInput {
            pool: pool.clone(),
            running,
            app_config,
            root,
            nats_url,
            repository,
            project,
            seeded_instance,
            token,
            libkrun_e2e,
            user_id,
            owner_browser_session,
            outsider_id,
            outsider_browser_session,
            gateway_caddy_e2e,
            gateway_service_e2e,
            gateway_service_log_guest_e2e,
            gateway_service_log_rpc_e2e,
            gateway_edge,
            gateway_service_fixture,
            brokered_fixture,
        })
        .await;
    })
    .await;
    finish_isolated_golden(
        isolated_database,
        pool,
        &database_url,
        &parent_database_url,
        parent_before,
    )
    .await;
}
