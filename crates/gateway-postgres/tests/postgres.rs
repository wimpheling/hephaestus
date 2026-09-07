//! Opt-in real `PostgreSQL` proof for gateway mailbox publication authority.

use authz_postgres::PostgresMelangeAuthorizer;
use forge_domain::{ProjectId, RepositoryId};
use futures_util::StreamExt;
use gateway_postgres::{
    ConfigureGatewayRequest, GatewayConfigureError, GatewayMailboxPublicationRequest,
    GatewayMailboxPublicationResult, InstallGatewayManifest, PostgresGatewayInstaller,
    PostgresGatewayMailboxPublisher, PostgresGatewayManagement,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use mailbox_dispatch::{
    MAILBOX_WAKE_SUBJECT, MailboxDispatchStore, MailboxOutboxPublisher,
    ensure_mailbox_jetstream_topology,
};
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope,
};
use mailbox_postgres::PostgresMailboxRepository;
use release_domain::ReleaseId;
use serial_test::serial;
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::{collections::BTreeMap, env, sync::Arc, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
#[allow(clippy::too_many_lines)]
async fn gateway_mailbox_publication_rechecks_live_grants_slots_and_producer_scope() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway mailbox migrations");

    let fixture = seed_fixture(&pool).await;
    let publisher = PostgresGatewayMailboxPublisher::new(pool.clone());

    let accepted = publisher
        .publish(request(&fixture, "deliver", "first"))
        .await
        .expect("publish with a live exact binding and grant");
    let event_id = match accepted {
        GatewayMailboxPublicationResult::Accepted { event_id } => event_id,
        other => panic!("expected accepted publication, got {other:?}"),
    };
    let duplicate = publisher
        .publish(request(&fixture, "deliver", "first"))
        .await
        .expect("repeat publication is idempotent");
    assert_eq!(
        duplicate,
        GatewayMailboxPublicationResult::Duplicate { event_id },
        "the same invocation/slot/key must retain the accepted event"
    );
    let (events, wakes, publications): (i64, i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM mailbox_events WHERE id = $1),
             (SELECT count(*) FROM outbox WHERE id = $1 AND subject = 'heph.mailbox.v1.wake'),
             (SELECT count(*) FROM gateway_mailbox_publications
                 WHERE invocation_id = $2 AND slot_key = 'deliver' AND deduplication_key = 'first')",
    )
    .bind(event_id.as_uuid())
    .bind(fixture.invocation)
    .fetch_one(&pool)
    .await
    .expect("load durable acceptance evidence");
    assert_eq!((events, wakes, publications), (1, 1, 1));

    // A declared but unbound slot cannot borrow this binding's mailbox grant.
    assert_eq!(
        publisher
            .publish(request(&fixture, "other", "wrong-slot"))
            .await
            .expect("wrong slot is a redacted denial"),
        GatewayMailboxPublicationResult::Denied
    );

    // Runtime sessions are not ambient authority: pausing the gateway after
    // issuance prevents a fresh invocation from publishing through its old
    // immutable revision.
    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(fixture.gateway)
        .execute(&pool)
        .await
        .expect("pause gateway");
    assert_eq!(
        publisher
            .publish(request(&fixture, "deliver", "gateway-paused"))
            .await
            .expect("paused gateway is a redacted denial"),
        GatewayMailboxPublicationResult::Denied
    );
    sqlx::query("UPDATE gateways SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.gateway)
        .execute(&pool)
        .await
        .expect("resume gateway for grant revocation proof");

    revoke_grant(&pool, fixture.grant, fixture.owner).await;
    assert_eq!(
        publisher
            .publish(request(&fixture, "deliver", "after-revoke"))
            .await
            .expect("revoked grant is a redacted denial"),
        GatewayMailboxPublicationResult::Denied
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1 AND producer_id = 'gateway-proof'",
        )
        .bind(fixture.mailbox)
        .fetch_one(&pool)
        .await
        .expect("count only accepted mailbox events"),
        1
    );

    // Producer identity is globally unique per mailbox, so a second gateway
    // binding cannot impersonate or collide with this producer scope.
    let collision = sqlx::query(
        "INSERT INTO gateway_mailbox_bindings
             (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id, producer_id, created_by)
         VALUES ($1, $2, $3, $4, 'other', $5, 'gateway-proof', $6)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.revision)
    .bind(fixture.gateway)
    .bind(fixture.project)
    .bind(fixture.mailbox)
    .bind(fixture.owner)
    .execute(&pool)
    .await;
    assert!(collision.is_err(), "mailbox producer scopes must be unique");
}

#[tokio::test]
#[serial]
async fn gateway_release_installation_is_authorized_published_and_idempotent() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect gateway installation PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway installation migrations");
    let fixture = seed_fixture(&pool).await;
    // Exercise the same restricted role path as the application connection.
    // Role membership is deployment-owned; this test only changes the
    // session role and never performs schema-changing DDL.
    let app_pool = PgPoolOptions::new()
        .max_connections(6)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect as hephaestus application role");
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&app_pool)
        .await
        .expect("application session role");
    assert_eq!(current_user, "hephaestus_app");
    let installer = PostgresGatewayInstaller::new(app_pool, Arc::new(PostgresMelangeAuthorizer));
    let owner = AuthenticatedIdentity::new(
        UserId::from_uuid(fixture.owner),
        "test",
        "gateway-owner",
        serde_json::json!({}),
        RequestId::new(),
    );
    let release_id = ReleaseId::from_uuid(fixture.release);
    let source = installer
        .published_release(&owner, release_id)
        .await
        .expect("owner resolves published release");
    assert_eq!(
        source.repository_id,
        RepositoryId::from_uuid(fixture.repository)
    );
    let manifest = br#"
version = 1

[[gateways]]
name = "installed"
agent_name = "gateway-mailbox"
handler_contract = "http.v1"
exposure = "public"

[[gateways.routes]]
path = "/installed"
methods = ["POST"]
"#;
    let command = || InstallGatewayManifest {
        project_id: ProjectId::from_uuid(fixture.project),
        repository_id: RepositoryId::from_uuid(fixture.repository),
        release_id: Some(release_id),
        manifest: manifest.to_vec(),
    };
    let first = installer.install(&owner, command()).await.expect("install");
    let (events, pending_outbox): (i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM application_events
               WHERE aggregate_type = 'gateway' AND aggregate_id = $1
                 AND occurrence_id = $2),
             (SELECT count(*) FROM product_event_outbox outbox
               JOIN application_events event ON event.id = outbox.event_id
               WHERE event.aggregate_type = 'gateway' AND event.aggregate_id = $1
                 AND event.occurrence_id = $2)",
    )
    .bind(first.gateways[0].gateway_id.as_uuid())
    .bind(owner.idempotency_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("gateway installation event and outbox");
    assert!(
        events > 0,
        "installation must commit a gateway product event"
    );
    assert_eq!(events, pending_outbox);
    let second = installer
        .install(&owner, command())
        .await
        .expect("repeat install");
    assert_eq!(
        first, second,
        "reinstalling one release must select its existing immutable revision"
    );

    let second_release = Uuid::new_v4();
    let second_build = Uuid::new_v4();
    let second_agent = Uuid::new_v4();
    let hash = [9_u8; 32];
    sqlx::query("INSERT INTO build_requests (id, repository_id, source_commit, source_ref, build_definition_hash, state, completed_at) VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', now())")
        .bind(second_build)
        .bind(fixture.repository)
        .bind("b".repeat(40))
        .bind(hash.as_slice())
        .execute(&pool)
        .await
        .expect("second build");
    sqlx::query("INSERT INTO releases (id, repository_id, version, source_commit, source_ref, build_request_id, build_definition_hash, configuration, configuration_hash, manifest_hash, state, published_at) VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8, 'published', now())")
        .bind(second_release)
        .bind(fixture.repository)
        .bind(format!("gateway-install-{second_release}"))
        .bind("b".repeat(40))
        .bind(second_build)
        .bind(hash.as_slice())
        .bind(hash.as_slice())
        .bind(hash.as_slice())
        .execute(&pool)
        .await
        .expect("second release");
    sqlx::query("INSERT INTO release_agents (id, release_id, family_id, agent_key, display_name, runtime_contract, runtime_contract_hash, parameter_schema, secret_slot_schema, requires_state) VALUES ($1, $2, $3, 'gateway-mailbox', 'Gateway mailbox', '{}', $4, '[]', '[]', false)")
        .bind(second_agent)
        .bind(second_release)
        .bind(fixture.family)
        .bind(hash.as_slice())
        .execute(&pool)
        .await
        .expect("second release agent");
    let second_owner = AuthenticatedIdentity::new(
        UserId::from_uuid(fixture.owner),
        "test",
        "gateway-owner-second-request",
        serde_json::json!({}),
        RequestId::new(),
    );
    let second_install = installer
        .install(
            &second_owner,
            InstallGatewayManifest {
                project_id: ProjectId::from_uuid(fixture.project),
                repository_id: RepositoryId::from_uuid(fixture.repository),
                release_id: Some(ReleaseId::from_uuid(second_release)),
                manifest: manifest.to_vec(),
            },
        )
        .await
        .expect("install second release");
    assert_ne!(
        first.gateways[0].revision_id, second_install.gateways[0].revision_id,
        "the same declaration from a new release must select a new immutable revision"
    );
    assert_eq!(
        installer
            .install(&owner, command())
            .await
            .expect("replay key1"),
        first,
        "retrying key1 must return its original revision after key2 activates a newer release"
    );
    let fresh_key = owner.clone().with_idempotency_id(RequestId::new());
    let fresh_install = installer
        .install(&fresh_key, command())
        .await
        .expect("fresh key same release is a successful no-op install");
    assert_eq!(fresh_install, first);
    assert_reconfigure_after_reinstallation_creates_fresh_authority(
        &pool,
        &installer,
        &owner,
        command(),
        first.gateways[0],
    )
    .await;
    assert!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_events
             WHERE occurrence_id = $1 AND aggregate_type = 'gateway'",
        )
        .bind(fresh_key.idempotency_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("fresh no-op installation receipt event")
            > 0
    );
    assert!(matches!(
        installer
            .install(
                &owner,
                InstallGatewayManifest {
                    project_id: ProjectId::from_uuid(fixture.project),
                    repository_id: RepositoryId::from_uuid(fixture.repository),
                    release_id: Some(ReleaseId::from_uuid(second_release)),
                    manifest: manifest.to_vec(),
                },
            )
            .await,
        Err(gateway_postgres::GatewayInstallError::Conflict)
    ));
    let denied_user = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Gateway Denied Actor')")
        .bind(denied_user)
        .execute(&pool)
        .await
        .expect("denied user");
    let unauthorized = AuthenticatedIdentity::new(
        UserId::from_uuid(denied_user),
        "test",
        "gateway-denied",
        serde_json::json!({}),
        RequestId::new(),
    );
    assert!(matches!(
        installer.install(&unauthorized, command()).await,
        Err(gateway_postgres::GatewayInstallError::AuthorizationDenied)
    ));
    assert!(matches!(
        installer
            .install(
                &owner,
                InstallGatewayManifest {
                    project_id: ProjectId::from_uuid(fixture.project),
                    repository_id: RepositoryId::from_uuid(fixture.repository),
                    release_id: Some(release_id),
                    manifest: b"version = 1\n".to_vec(),
                },
            )
            .await,
        Err(gateway_postgres::GatewayInstallError::InvalidManifest { .. })
    ));

    let draft_release = Uuid::new_v4();
    sqlx::query("INSERT INTO releases (id, repository_id, version, source_commit, source_ref, build_request_id, build_definition_hash, configuration, configuration_hash, manifest_hash, state) VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8, 'draft')")
        .bind(draft_release)
        .bind(fixture.repository)
        .bind(format!("gateway-draft-{draft_release}"))
        .bind("c".repeat(40))
        .bind(second_build)
        .bind(hash.as_slice())
        .bind(hash.as_slice())
        .bind(hash.as_slice())
        .execute(&pool)
        .await
        .expect("draft release");
    assert!(matches!(
        installer
            .published_release(&owner, ReleaseId::from_uuid(draft_release))
            .await,
        Err(gateway_postgres::GatewayInstallError::Unavailable)
    ));
}

async fn assert_reconfigure_after_reinstallation_creates_fresh_authority(
    pool: &sqlx::PgPool,
    installer: &PostgresGatewayInstaller,
    owner: &AuthenticatedIdentity,
    manifest: InstallGatewayManifest,
    gateway: gateway_postgres::InstalledGateway,
) {
    let management =
        PostgresGatewayManagement::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let configure = || ConfigureGatewayRequest {
        gateway_id: gateway.gateway_id.as_uuid(),
        expected_revision_id: gateway.revision_id.as_uuid(),
        parameters: BTreeMap::new(),
        secret_selections: Vec::new(),
    };
    let original_identity = owner.clone().with_idempotency_id(RequestId::new());
    let configured = management
        .configure(&original_identity, configure())
        .await
        .expect("first configuration before reinstall");
    installer
        .install(
            &owner.clone().with_idempotency_id(RequestId::new()),
            manifest,
        )
        .await
        .expect("reinstall reactivates original declaration");
    let fresh_identity = owner.clone().with_idempotency_id(RequestId::new());
    let fresh = management
        .configure(&fresh_identity, configure())
        .await
        .expect("fresh configuration after reinstall");
    assert_ne!(fresh.revision_id, configured.revision_id);
    let replay = management
        .configure(&original_identity, configure())
        .await
        .expect("original successful command remains replayable");
    assert_eq!(replay.revision_id, configured.revision_id);
    let (active, revisions, bindings): (Uuid, i64, i64) = sqlx::query_as(
        "SELECT active_revision_id,
             (SELECT count(*) FROM gateway_revisions WHERE gateway_id = $1),
             (SELECT count(*) FROM gateway_mailbox_bindings WHERE gateway_revision_id = $2)
         FROM gateways WHERE id = $1",
    )
    .bind(gateway.gateway_id.as_uuid())
    .bind(fresh.revision_id)
    .fetch_one(pool)
    .await
    .expect("fresh revision and replay preserve independent authority");
    assert_eq!(
        active, fresh.revision_id,
        "old command replay must not reactivate history"
    );
    // The original release, the later release, and both configured revisions
    // are the only immutable revisions created by the installation fixture.
    assert_eq!(revisions, 4);
    assert_eq!(bindings, 0);
}

#[tokio::test]
#[serial]
async fn gateway_configuration_is_immutable_authorized_replayable_and_fail_closed() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect configuration PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply configuration migrations");
    let fixture = seed_fixture(&pool).await;
    let owner = AuthenticatedIdentity::new(
        UserId::from_uuid(fixture.owner),
        "test",
        "gateway-configure-owner",
        serde_json::json!({}),
        RequestId::new(),
    );
    let management =
        PostgresGatewayManagement::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let first = management
        .configure(
            &owner,
            ConfigureGatewayRequest {
                gateway_id: fixture.gateway,
                expected_revision_id: fixture.revision,
                parameters: BTreeMap::new(),
                secret_selections: Vec::new(),
            },
        )
        .await
        .expect("owner configures published gateway");
    assert_ne!(first.revision_id, fixture.revision);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM gateway_mailbox_bindings WHERE gateway_revision_id = $1",
        )
        .bind(first.revision_id)
        .fetch_one(&pool)
        .await
        .expect("count new revision bindings"),
        0,
        "configuration must not copy mailbox grants"
    );
    let second = management
        .configure(
            &AuthenticatedIdentity::new(
                UserId::from_uuid(fixture.owner),
                "test",
                "gateway-configure-owner",
                serde_json::json!({}),
                RequestId::new(),
            ),
            ConfigureGatewayRequest {
                gateway_id: fixture.gateway,
                expected_revision_id: first.revision_id,
                parameters: BTreeMap::new(),
                secret_selections: Vec::new(),
            },
        )
        .await
        .expect("second immutable configuration");
    let replay = management
        .configure(
            &owner,
            ConfigureGatewayRequest {
                gateway_id: fixture.gateway,
                expected_revision_id: fixture.revision,
                parameters: BTreeMap::new(),
                secret_selections: Vec::new(),
            },
        )
        .await
        .expect("retry returns original result after later activation");
    assert_eq!(replay.revision_id, first.revision_id);
    assert_ne!(second.revision_id, replay.revision_id);
    let binding_identity = AuthenticatedIdentity::new(
        UserId::from_uuid(fixture.owner),
        "test",
        "gateway-binding-receipt",
        serde_json::json!({}),
        RequestId::new(),
    );
    let binding = management
        .create_mailbox_binding(
            &binding_identity,
            second.revision_id,
            "deliver",
            fixture.mailbox,
            "binding-proof",
        )
        .await
        .expect("create gateway mailbox binding");
    let mut binding_replay_identity = binding_identity.clone();
    binding_replay_identity.request_id = RequestId::new();
    let binding_replay = management
        .create_mailbox_binding(
            &binding_replay_identity,
            second.revision_id,
            "deliver",
            fixture.mailbox,
            "binding-proof",
        )
        .await
        .expect("replay gateway mailbox binding");
    assert_eq!(binding.id, binding_replay.id);
    assert_eq!(binding.grant_id, binding_replay.grant_id);
    assert_eq!(
        binding_identity.idempotency_id,
        binding_replay_identity.idempotency_id
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_events
             WHERE occurrence_id = $1 AND aggregate_type = 'gateway'
               AND aggregate_id = $2",
        )
        .bind(binding_identity.idempotency_id.as_uuid())
        .bind(fixture.gateway)
        .fetch_one(&pool)
        .await
        .expect("gateway binding product event receipt"),
        1
    );
    let revoke_identity = AuthenticatedIdentity::new(
        UserId::from_uuid(fixture.owner),
        "test",
        "gateway-binding-revoke",
        serde_json::json!({}),
        RequestId::new(),
    );
    let revoked = management
        .revoke_mailbox_binding_grant(&revoke_identity, binding.id)
        .await
        .expect("revoke gateway mailbox binding grant");
    let mut revoke_replay_identity = revoke_identity.clone();
    revoke_replay_identity.request_id = RequestId::new();
    let revoke_replay = management
        .revoke_mailbox_binding_grant(&revoke_replay_identity, binding.id)
        .await
        .expect("replay gateway mailbox binding grant revoke");
    assert_eq!(revoked.id, revoke_replay.id);
    assert_eq!(revoked.grant_id, revoke_replay.grant_id);
    assert_eq!(revoked.grant_status, "revoked");
    let outsider = AuthenticatedIdentity::new(
        UserId::from_uuid(Uuid::new_v4()),
        "test",
        "gateway-configure-outsider",
        serde_json::json!({}),
        RequestId::new(),
    );
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Gateway configure outsider')")
        .bind(outsider.user_id.as_uuid())
        .execute(&pool)
        .await
        .expect("configure outsider");
    assert!(matches!(
        management
            .configure(
                &outsider,
                ConfigureGatewayRequest {
                    gateway_id: fixture.gateway,
                    expected_revision_id: second.revision_id,
                    parameters: BTreeMap::new(),
                    secret_selections: Vec::new(),
                },
            )
            .await,
        Err(GatewayConfigureError::Denied)
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
// Keep acceptance, lifecycle denial, and resumed dispatch in one fixture so
// the test proves that the same accepted work survives the closed gate.
#[allow(clippy::too_many_lines)]
async fn gateway_accepts_during_updates_but_denies_disabled_and_recovery_instances() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrate");
    let fixture = seed_fixture(&pool).await;
    seed_dispatch_target(&pool, &fixture).await;
    let publisher = PostgresGatewayMailboxPublisher::new(pool.clone());
    let mut deferred_event = None;
    for state in ["update_draining", "updating"] {
        set_instance_state(&pool, fixture.mailbox, state).await;
        let outcome = publisher
            .publish(request(&fixture, "deliver", state))
            .await
            .expect("accept during update");
        let GatewayMailboxPublicationResult::Accepted { event_id } = outcome else {
            panic!("expected durable acceptance during {state}, got {outcome:?}");
        };
        assert_eq!(
            publisher
                .publish(request(&fixture, "deliver", state))
                .await
                .expect("deduplicate during update"),
            GatewayMailboxPublicationResult::Duplicate { event_id }
        );
        let command: serde_json::Value = sqlx::query_scalar(
            "SELECT payload FROM outbox WHERE id = $1 AND subject = 'heph.mailbox.v1.wake'",
        )
        .bind(event_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("committed wake");
        let command = serde_json::from_value(command).expect("wake command");
        let store = PostgresMailboxRepository::new(pool.clone());
        store
            .apply_command(MAILBOX_WAKE_SUBJECT, &command)
            .await
            .expect("wake accepted work");
        assert!(
            store
                .claim_dispatch(&command)
                .await
                .expect("closed gate defers dispatch")
                .is_none()
        );
        deferred_event = Some(event_id);
        let attempts: i32 = sqlx::query_scalar(
            "SELECT logical_attempt_count FROM mailbox_deliveries WHERE event_id = $1",
        )
        .bind(event_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("deferred delivery");
        assert_eq!(attempts, 0);
    }
    for state in [
        "disabled",
        "paused_unknown_state",
        "paused_activation_recovery",
        "recovering",
        "removed",
    ] {
        set_instance_state(&pool, fixture.mailbox, state).await;
        assert_eq!(
            publisher
                .publish(request(&fixture, "deliver", state))
                .await
                .expect("redacted lifecycle denial"),
            GatewayMailboxPublicationResult::Denied
        );
    }
    set_instance_state(&pool, fixture.mailbox, "updating").await;
    revoke_grant(&pool, fixture.grant, fixture.owner).await;
    assert_eq!(
        publisher
            .publish(request(&fixture, "deliver", "revoked-during-update"))
            .await
            .expect("live grant check during update"),
        GatewayMailboxPublicationResult::Denied
    );
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
            .bind(fixture.mailbox)
            .fetch_one(&pool)
            .await
            .expect("only update requests accepted");
    assert_eq!(events, 2);
    sqlx::query("UPDATE agent_instances SET state = 'active', run_gate_open = true WHERE id = (SELECT instance_id FROM mailboxes WHERE id = $1)")
        .bind(fixture.mailbox).execute(&pool).await.expect("reopen gate");
    let event_id = deferred_event.expect("accepted deferred event");
    let command: serde_json::Value = sqlx::query_scalar("SELECT payload FROM outbox WHERE subject = 'heph.mailbox.v1.dispatch' AND payload->>'mailbox_event_id' = $1")
        .bind(event_id.as_uuid().to_string()).fetch_one(&pool).await.expect("committed dispatch");
    let command = serde_json::from_value(command).expect("dispatch command");
    assert!(
        PostgresMailboxRepository::new(pool)
            .claim_dispatch(&command)
            .await
            .expect("dispatch accepted work after reopening")
            .is_some()
    );
}

async fn seed_dispatch_target(pool: &sqlx::PgPool, fixture: &Fixture) {
    let repository: Uuid = sqlx::query_scalar("SELECT repository_id FROM gateways WHERE id = $1")
        .bind(fixture.gateway)
        .fetch_one(pool)
        .await
        .expect("fixture repository");
    let receive = Uuid::new_v4();
    sqlx::query("INSERT INTO git_receives (id, repository_id, principal, status, accepted_at) VALUES ($1, $2, 'gateway-test', 'accepted', now())")
        .bind(receive).bind(repository).execute(pool).await.expect("receive");
    sqlx::query("INSERT INTO git_refs (repository_id, git_ref, commit_sha, updated_by_receive_id) VALUES ($1, 'refs/heads/main', $2, $3)")
        .bind(repository).bind("a".repeat(40)).bind(receive).execute(pool).await.expect("target ref");
    sqlx::query("INSERT INTO agent_attachments (id, instance_id, project_id, repository_id, ref_selector, trigger_policy) SELECT gen_random_uuid(), instance_id, project_id, $2, 'refs/heads/main', 'manual' FROM mailboxes WHERE id = $1")
        .bind(fixture.mailbox).bind(repository).execute(pool).await.expect("runnable attachment");
}

async fn set_instance_state(pool: &sqlx::PgPool, mailbox: Uuid, state: &str) {
    sqlx::query("UPDATE agent_instances SET state = $2, run_gate_open = false, removed_at = CASE WHEN $2 = 'removed' THEN now() ELSE NULL END WHERE id = (SELECT instance_id FROM mailboxes WHERE id = $1)")
        .bind(mailbox).bind(state).execute(pool).await.expect("set fixture lifecycle");
}

/// Proves the gateway-owned acceptance outbox survives a dispatcher crash
/// after receipt but before acknowledgement. This stays below the daemon's
/// private worker-loop ownership: the application exposes no safe hook to
/// kill/restart just that loop while the joined Caddy/libkrun fixture runs.
#[tokio::test(flavor = "multi_thread")]
#[serial]
#[allow(clippy::too_many_lines)]
async fn gateway_publication_outbox_redelivers_after_dispatcher_crash() {
    let (Ok(database_url), Ok(nats_url)) = (
        env::var("HEPHAESTUS_POSTGRES_TEST_URL"),
        env::var("HEPHAESTUS_NATS_TEST_URL"),
    ) else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway mailbox migrations");
    let fixture = seed_fixture(&pool).await;
    let event_id = match PostgresGatewayMailboxPublisher::new(pool.clone())
        .publish(request(&fixture, "deliver", "crash-recovery"))
        .await
        .expect("accept gateway publication before dispatcher crash")
    {
        GatewayMailboxPublicationResult::Accepted { event_id } => event_id,
        outcome => panic!("expected accepted publication, got {outcome:?}"),
    };

    let nats = async_nats::connect(nats_url)
        .await
        .expect("connect real NATS");
    let jetstream = async_nats::jetstream::new(nats);
    let consumer = ensure_mailbox_jetstream_topology(&jetstream)
        .await
        .expect("create durable mailbox consumer");
    let store = Arc::new(PostgresMailboxRepository::new(pool.clone()));
    let outbox = MailboxOutboxPublisher::new(jetstream.clone(), store.clone());
    assert!(
        outbox
            .publish_pending(1_000)
            .await
            .expect("publish committed gateway mailbox wake")
            >= 1
    );

    let mut messages = consumer.messages().await.expect("open mailbox consumer");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let first = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let message = tokio::time::timeout(remaining, messages.next())
            .await
            .expect("receive initial mailbox wake")
            .expect("mailbox stream item")
            .expect("valid mailbox wake");
        let command: mailbox_dispatch::MailboxDispatchCommand =
            serde_json::from_slice(&message.payload).expect("identifier-only mailbox command");
        if command.event_id == event_id {
            assert_eq!(message.message.subject.as_str(), MAILBOX_WAKE_SUBJECT);
            break (message, command);
        }
        message
            .double_ack()
            .await
            .expect("ack earlier fixture command");
    };
    // A NAK is the exact broker-visible shape of a dispatcher process dying
    // after receive and before it durably applies the wake transition.
    first
        .0
        .ack_with(async_nats::jetstream::AckKind::Nak(None))
        .await
        .expect("model dispatcher crash before acknowledgement");
    let redelivery = tokio::time::timeout(Duration::from_secs(5), messages.next())
        .await
        .expect("receive redelivered gateway mailbox wake")
        .expect("mailbox stream item")
        .expect("valid redelivered wake");
    let redelivered: mailbox_dispatch::MailboxDispatchCommand =
        serde_json::from_slice(&redelivery.payload).expect("identifier-only redelivery");
    assert_eq!(redelivered, first.1);
    assert!(
        !redelivery
            .payload
            .windows(b"gateway-postgres-proof".len())
            .any(|window| window == b"gateway-postgres-proof"),
        "JetStream command must not expose the gateway body"
    );
    store
        .apply_command(MAILBOX_WAKE_SUBJECT, &redelivered)
        .await
        .expect("apply redelivered wake durably");
    store
        .apply_command(MAILBOX_WAKE_SUBJECT, &redelivered)
        .await
        .expect("duplicate redelivery stays idempotent");
    redelivery
        .double_ack()
        .await
        .expect("ack only after durable recovery");

    let (events, publications, deliveries): (i64, i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM mailbox_events WHERE id = $1),
             (SELECT count(*) FROM gateway_mailbox_publications WHERE event_id = $1),
             (SELECT count(*) FROM mailbox_deliveries WHERE event_id = $1)",
    )
    .bind(event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("gateway crash recovery remains one logical event");
    assert_eq!((events, publications, deliveries), (1, 1, 1));
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
#[allow(clippy::too_many_lines)]
async fn gateway_mailbox_management_and_inspection_are_actor_scoped() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway mailbox migrations");

    let fixture = seed_fixture(&pool).await;
    let outsider = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Gateway mailbox outsider')")
        .bind(outsider)
        .execute(&pool)
        .await
        .expect("outsider");

    // First create an accepted publication. It gives the inspection proof a
    // durable, protected row rather than merely checking an empty relation.
    let publisher = PostgresGatewayMailboxPublisher::new(pool.clone());
    assert!(matches!(
        publisher
            .publish(request(&fixture, "deliver", "actor-scoped-inspection"))
            .await
            .expect("accept publication for inspection"),
        GatewayMailboxPublicationResult::Accepted { .. }
    ));

    let managed_binding = Uuid::new_v4();
    let managed_grant = Uuid::new_v4();
    let mut owner_tx = pool.begin().await.expect("owner transaction");
    set_actor_app_role(&mut owner_tx, fixture.owner)
        .await
        .expect("owner application role");
    sqlx::query(
        "INSERT INTO gateway_mailbox_bindings
             (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id, producer_id, created_by)
         VALUES ($1, $2, $3, $4, 'other', $5, 'gateway-managed-proof', $6)",
    )
    .bind(managed_binding)
    .bind(fixture.revision)
    .bind(fixture.gateway)
    .bind(fixture.project)
    .bind(fixture.mailbox)
    .bind(fixture.owner)
    .execute(&mut *owner_tx)
    .await
    .expect("authorized actor creates exact binding");
    sqlx::query(
        "INSERT INTO gateway_mailbox_binding_grants (id, binding_id, status, granted_by)
         VALUES ($1, $2, 'active', $3)",
    )
    .bind(managed_grant)
    .bind(managed_binding)
    .bind(fixture.owner)
    .execute(&mut *owner_tx)
    .await
    .expect("authorized actor creates exact grant");
    let visible_to_owner: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_mailbox_bindings WHERE id = $1")
            .bind(managed_binding)
            .fetch_one(&mut *owner_tx)
            .await
            .expect("owner can inspect binding");
    let publication_visible_to_owner: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_mailbox_publications WHERE invocation_id = $1",
    )
    .bind(fixture.invocation)
    .fetch_one(&mut *owner_tx)
    .await
    .expect("owner can inspect publication provenance");
    assert_eq!(visible_to_owner, 1);
    assert_eq!(publication_visible_to_owner, 1);
    owner_tx.commit().await.expect("commit authorized binding");

    let mut outsider_tx = pool.begin().await.expect("outsider transaction");
    set_actor_app_role(&mut outsider_tx, outsider)
        .await
        .expect("outsider application role");
    let bindings_visible_to_outsider: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_mailbox_bindings WHERE id = $1")
            .bind(managed_binding)
            .fetch_one(&mut *outsider_tx)
            .await
            .expect("outsider binding inspection is filtered");
    let publications_visible_to_outsider: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_mailbox_publications WHERE invocation_id = $1",
    )
    .bind(fixture.invocation)
    .fetch_one(&mut *outsider_tx)
    .await
    .expect("outsider publication inspection is filtered");
    assert_eq!(bindings_visible_to_outsider, 0);
    assert_eq!(publications_visible_to_outsider, 0);
    let forbidden_revoke = sqlx::query(
        "UPDATE gateway_mailbox_binding_grants
             SET status = 'revoked', revoked_at = now(), revoked_by = $2
           WHERE id = $1",
    )
    .bind(managed_grant)
    .bind(outsider)
    .execute(&mut *outsider_tx)
    .await
    .expect("outsider revoke is RLS-filtered");
    assert_eq!(forbidden_revoke.rows_affected(), 0);
    outsider_tx
        .rollback()
        .await
        .expect("rollback outsider checks");

    // A failed insert aborts its PostgreSQL transaction, so keep it separate
    // from the RLS-filtered revocation assertion above.
    let mut outsider_create_tx = pool.begin().await.expect("outsider create transaction");
    set_actor_app_role(&mut outsider_create_tx, outsider)
        .await
        .expect("outsider application role");
    let forbidden_create = sqlx::query(
        "INSERT INTO gateway_mailbox_bindings
             (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id, producer_id, created_by)
         VALUES ($1, $2, $3, $4, 'other', $5, 'outsider-proof', $6)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.revision)
    .bind(fixture.gateway)
    .bind(fixture.project)
    .bind(fixture.mailbox)
    .bind(outsider)
    .execute(&mut *outsider_create_tx)
    .await;
    assert!(
        forbidden_create.is_err(),
        "outsider cannot create a binding"
    );
    outsider_create_tx
        .rollback()
        .await
        .expect("rollback rejected outsider create");

    let mut worker_mint_tx = pool.begin().await.expect("worker mint transaction");
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *worker_mint_tx)
        .await
        .expect("worker role");
    let worker_mint = sqlx::query(
        "INSERT INTO gateway_mailbox_bindings
             (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id, producer_id, created_by)
         VALUES ($1, $2, $3, $4, 'other', $5, 'worker-proof', $6)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.revision)
    .bind(fixture.gateway)
    .bind(fixture.project)
    .bind(fixture.mailbox)
    .bind(fixture.owner)
    .execute(&mut *worker_mint_tx)
    .await;
    assert!(worker_mint.is_err(), "worker cannot mint a binding");
    worker_mint_tx
        .rollback()
        .await
        .expect("rollback rejected worker mint");

    let mut worker_revoke_tx = pool.begin().await.expect("worker revoke transaction");
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *worker_revoke_tx)
        .await
        .expect("worker role");
    let worker_revoke = sqlx::query(
        "UPDATE gateway_mailbox_binding_grants
             SET status = 'revoked', revoked_at = now(), revoked_by = $2
           WHERE id = $1",
    )
    .bind(managed_grant)
    .bind(fixture.owner)
    .execute(&mut *worker_revoke_tx)
    .await;
    assert!(worker_revoke.is_err(), "worker cannot revoke a grant");
    worker_revoke_tx
        .rollback()
        .await
        .expect("rollback rejected worker revoke");

    let mut owner_revoke_tx = pool.begin().await.expect("owner revoke transaction");
    set_actor_app_role(&mut owner_revoke_tx, fixture.owner)
        .await
        .expect("owner application role");
    let authorized_revoke = sqlx::query(
        "UPDATE gateway_mailbox_binding_grants
             SET status = 'revoked', revoked_at = now(), revoked_by = $2
           WHERE id = $1 AND status = 'active'",
    )
    .bind(managed_grant)
    .bind(fixture.owner)
    .execute(&mut *owner_revoke_tx)
    .await
    .expect("authorized actor revokes grant");
    assert_eq!(authorized_revoke.rows_affected(), 1);
    owner_revoke_tx
        .commit()
        .await
        .expect("commit authorized revocation");
}

async fn set_actor_app_role(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true)",
    )
    .bind(actor.to_string())
    .execute(&mut **transaction)
    .await?;
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn request(fixture: &Fixture, slot: &str, key: &str) -> GatewayMailboxPublicationRequest {
    let body = b"gateway-postgres-proof".to_vec();
    let body_reference = BodyReference::new(
        BodyReferenceId::new(),
        u32::try_from(body.len()).expect("body fits bounded envelope"),
        Sha256::digest(&body).into(),
    )
    .expect("body reference");
    GatewayMailboxPublicationRequest {
        runtime_session_id: fixture.session,
        invocation_id: fixture.invocation,
        slot_key: slot.to_owned(),
        deduplication_key: DeduplicationKey::parse(key).expect("deduplication key"),
        envelope: MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").expect("method"),
            EnvelopeRoute::parse("/telegram/update").expect("route"),
            BTreeMap::default(),
            ContentMetadata::new(
                body_reference,
                Some("application/json".to_owned()),
                Some("identity".to_owned()),
            )
            .expect("content"),
            OffsetDateTime::now_utc(),
            None,
        )
        .expect("envelope"),
        decoded_length: u32::try_from(body.len()).expect("body length"),
        encoded_body: body,
    }
}

#[derive(Clone, Copy)]
struct Fixture {
    owner: Uuid,
    project: Uuid,
    repository: Uuid,
    family: Uuid,
    gateway: Uuid,
    revision: Uuid,
    mailbox: Uuid,
    grant: Uuid,
    invocation: Uuid,
    session: Uuid,
    release: Uuid,
}

#[allow(clippy::too_many_lines)]
async fn seed_fixture(pool: &sqlx::PgPool) -> Fixture {
    let owner_id = Uuid::new_v4();
    let organization_id = Uuid::new_v4();
    let project_id = Uuid::new_v4();
    let repository_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let build_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    let instance_id = Uuid::new_v4();
    let instance_revision_id = Uuid::new_v4();
    let mailbox_id = Uuid::new_v4();
    let gateway_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let route_id = Uuid::new_v4();
    let binding_id = Uuid::new_v4();
    let grant_id = Uuid::new_v4();
    let invocation_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let hash = [9_u8; 32];

    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Gateway mailbox test owner')")
        .bind(owner_id)
        .execute(pool)
        .await
        .expect("owner");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("gateway-mailbox-{organization_id}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query("INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(organization_id)
        .bind(owner_id)
        .execute(pool)
        .await
        .expect("organization owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project_id)
        .bind(organization_id)
        .bind(format!("gateway-mailbox-{project_id}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project_id)
        .bind(owner_id)
        .execute(pool)
        .await
        .expect("project maintainer");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository_id)
        .bind(project_id)
        .bind(format!("gateway-mailbox-{repository_id}"))
        .execute(pool)
        .await
        .expect("repository");
    sqlx::query("INSERT INTO agent_families (id, repository_id, agent_key) VALUES ($1, $2, 'gateway-mailbox')")
        .bind(family_id)
        .bind(repository_id)
        .execute(pool)
        .await
        .expect("agent family");
    sqlx::query("INSERT INTO build_requests (id, repository_id, source_commit, source_ref, build_definition_hash, state, completed_at) VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', now())")
        .bind(build_id)
        .bind(repository_id)
        .bind("a".repeat(40))
        .bind(hash.as_slice())
        .execute(pool)
        .await
        .expect("build request");
    sqlx::query("INSERT INTO releases (id, repository_id, version, source_commit, source_ref, build_request_id, build_definition_hash, configuration, configuration_hash, manifest_hash, state, published_at) VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8, 'published', now())")
        .bind(release_id).bind(repository_id).bind(format!("gateway-mailbox-{release_id}"))
        .bind("a".repeat(40)).bind(build_id).bind(hash.as_slice()).bind(hash.as_slice()).bind(hash.as_slice())
        .execute(pool).await.expect("release");
    sqlx::query("INSERT INTO release_agents (id, release_id, family_id, agent_key, display_name, runtime_contract, runtime_contract_hash, parameter_schema, secret_slot_schema, requires_state) VALUES ($1, $2, $3, 'gateway-mailbox', 'Gateway mailbox', '{}', $4, '[]', '[]', false)")
        .bind(release_agent_id).bind(release_id).bind(family_id).bind(hash.as_slice())
        .execute(pool).await.expect("release agent");
    sqlx::query("INSERT INTO agent_instances (id, project_id, family_id, name, state) VALUES ($1, $2, $3, $4, 'active')")
        .bind(instance_id).bind(project_id).bind(family_id).bind(format!("gateway-mailbox-{instance_id}"))
        .execute(pool).await.expect("agent instance");
    sqlx::query("INSERT INTO agent_instance_revisions (id, instance_id, release_agent_id, parameters, parameter_hash, resource_selection, network_restriction, effective_runtime_policy, effective_policy_hash, platform_policy_version, runnable) VALUES ($1, $2, $3, '{}', $4, '{}', '{}', '{}', $5, 'test/v1', true)")
        .bind(instance_revision_id).bind(instance_id).bind(release_agent_id).bind(hash.as_slice()).bind(hash.as_slice())
        .execute(pool).await.expect("agent revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(instance_revision_id)
        .execute(pool)
        .await
        .expect("activate instance");
    sqlx::query(
        "INSERT INTO mailboxes (id, project_id, instance_id, state) VALUES ($1, $2, $3, 'active')",
    )
    .bind(mailbox_id)
    .bind(project_id)
    .bind(instance_id)
    .execute(pool)
    .await
    .expect("mailbox");
    sqlx::query("INSERT INTO gateways (id, project_id, repository_id, name, lifecycle, created_by) VALUES ($1, $2, $3, $4, 'enabled', $5)")
        .bind(gateway_id).bind(project_id).bind(repository_id).bind(format!("gateway-{gateway_id}"))
        .bind(owner_id).execute(pool).await.expect("gateway");
    sqlx::query("INSERT INTO gateway_revisions (id, gateway_id, project_id, repository_id, release_id, release_agent_id, release_agent_key, handler_contract, exposure, parameters, mailbox_slots, normalized_hash, created_by) VALUES ($1, $2, $3, $4, $5, $6, 'gateway-mailbox', 'http.v1', 'public', '{}', ARRAY['deliver', 'other'], $7, $8)")
        .bind(revision_id).bind(gateway_id).bind(project_id).bind(repository_id).bind(release_id).bind(release_agent_id).bind(hash.as_slice()).bind(owner_id)
        .execute(pool).await.expect("gateway revision");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("activate gateway revision");
    sqlx::query(
        "INSERT INTO project_capability_granters (project_id, user_id, created_by)
         VALUES ($1, $2, $2)",
    )
    .bind(project_id)
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("grant gateway mailbox delegation authority");
    sqlx::query("INSERT INTO gateway_routes (id, gateway_revision_id, gateway_id, project_id, path, methods) VALUES ($1, $2, $3, $4, '/telegram', ARRAY['POST'])")
        .bind(route_id).bind(revision_id).bind(gateway_id).bind(project_id).execute(pool).await.expect("gateway route");
    sqlx::query("INSERT INTO gateway_mailbox_bindings (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id, producer_id, created_by) VALUES ($1, $2, $3, $4, 'deliver', $5, 'gateway-proof', $6)")
        .bind(binding_id).bind(revision_id).bind(gateway_id).bind(project_id).bind(mailbox_id).bind(owner_id)
        .execute(pool).await.expect("gateway mailbox binding");
    sqlx::query("INSERT INTO gateway_mailbox_binding_grants (id, binding_id, status, granted_by) VALUES ($1, $2, 'active', $3)")
        .bind(grant_id).bind(binding_id).bind(owner_id).execute(pool).await.expect("authorized gateway mailbox grant");
    insert_invocation_and_session(
        pool,
        invocation_id,
        session_id,
        snapshot_id,
        gateway_id,
        revision_id,
        route_id,
        project_id,
        mailbox_id,
        grant_id,
        binding_id,
    )
    .await;

    Fixture {
        owner: owner_id,
        project: project_id,
        repository: repository_id,
        family: family_id,
        gateway: gateway_id,
        revision: revision_id,
        mailbox: mailbox_id,
        grant: grant_id,
        invocation: invocation_id,
        session: session_id,
        release: release_id,
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_invocation_and_session(
    pool: &sqlx::PgPool,
    invocation_id: Uuid,
    session_id: Uuid,
    snapshot_id: Uuid,
    gateway_id: Uuid,
    revision_id: Uuid,
    route_id: Uuid,
    project_id: Uuid,
    mailbox_id: Uuid,
    grant_id: Uuid,
    binding_id: Uuid,
) {
    let hash = [7_u8; 32];
    let credential_hash = Sha256::digest(session_id.as_bytes());
    sqlx::query("INSERT INTO gateway_invocations (id, gateway_id, gateway_revision_id, gateway_route_id, project_id, request_id, outcome) VALUES ($1, $2, $3, $4, $5, $6, 'accepted')")
        .bind(invocation_id).bind(gateway_id).bind(revision_id).bind(route_id).bind(project_id).bind(Uuid::new_v4())
        .execute(pool).await.expect("accepted invocation");
    sqlx::query("INSERT INTO gateway_authorization_snapshots (id, invocation_id, gateway_id, gateway_revision_id, authorization_model_version, normalized_hash) VALUES ($1, $2, $3, $4, 'test/v1', $5)")
        .bind(snapshot_id).bind(invocation_id).bind(gateway_id).bind(revision_id).bind(hash.as_slice())
        .execute(pool).await.expect("authorization snapshot");
    sqlx::query("INSERT INTO gateway_runtime_authority_sessions (id, snapshot_id, invocation_id, gateway_id, gateway_revision_id, identity_hash, snapshot_hash, issuance_generation, credential_hash, status, issued_at, expires_at, acknowledged_at) VALUES ($1, $2, $3, $4, $5, $6, $6, 1, $7, 'active', now(), now() + interval '10 minutes', now())")
        .bind(session_id).bind(snapshot_id).bind(invocation_id).bind(gateway_id).bind(revision_id).bind(hash.as_slice()).bind(credential_hash.as_slice())
        .execute(pool).await.expect("active runtime session");
    sqlx::query(
        "INSERT INTO gateway_authorization_snapshot_bindings
             (snapshot_id, gateway_revision_id, ordinal, binding_id, grant_id, binding_hash,
              slot_key, resource_kind, resource_id, granted_operations)
         VALUES ($1, $2, 0, $3, $4, $5, 'deliver', 'mailbox', $6, ARRAY['publish'])",
    )
    .bind(snapshot_id)
    .bind(revision_id)
    .bind(binding_id)
    .bind(grant_id)
    .bind(hash.as_slice())
    .bind(mailbox_id)
    .execute(pool)
    .await
    .expect("exact mailbox authority snapshot binding");
}

async fn revoke_grant(pool: &sqlx::PgPool, grant_id: Uuid, owner_id: Uuid) {
    sqlx::query("UPDATE gateway_mailbox_binding_grants SET status = 'revoked', revoked_at = now(), revoked_by = $2 WHERE id = $1")
        .bind(grant_id).bind(owner_id).execute(pool).await.expect("authorized grant revocation");
}
