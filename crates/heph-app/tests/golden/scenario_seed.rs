use super::*;

/// Durable and provider fixtures shared by every golden execution mode.
// The temporary directory and optional mailbox are retained for fixture
// lifetime and compatibility even when the selected path does not inspect them.
#[allow(dead_code)]
pub struct GoldenSeed {
    pub temporary: tempfile::TempDir,
    pub root: PathBuf,
    pub release_artifact_root: PathBuf,
    pub repository_root: PathBuf,
    pub fixture_repository: PgForgeRepository,
    pub browser_oidc_issuer: String,
    pub user_id: UserId,
    pub organization_id: OrganizationId,
    pub owner_browser_session: BrowserSessionSid,
    pub outsider_id: UserId,
    pub outsider_browser_session: BrowserSessionSid,
    pub project: forge_domain::Project,
    pub repository: forge_domain::Repository,
    pub seeded_instance: SeededInstance,
    pub brokered_fixture: Option<BrokeredFixture>,
    pub session_chat_fixture: Option<session_chat::SessionBrokerFixture>,
    pub gateway_mailbox: Option<MailboxId>,
    pub gateway_service_fixture: Option<GatewayServiceGoldenFixture>,
    pub gateway_edge: Option<(GatewayEdgeConfig, GatewayGoldenFixture)>,
    pub initial_gateway_edge: Option<GatewayEdgeConfig>,
}

#[allow(
    clippy::cognitive_complexity,
    clippy::fn_params_excessive_bools,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn seed_golden_scenario(
    pool: &sqlx::PgPool,
    gateway_service_external_e2e: bool,
    libkrun_e2e: bool,
    cooking_build_proof: bool,
    session_chat_e2e: bool,
    gateway_caddy_e2e: bool,
    gateway_service_e2e: bool,
    installed_ui_fixture: bool,
    session_chat_browser_e2e: bool,
    gateway_service_log_guest_e2e: bool,
) -> GoldenSeed {
    let temporary = tempfile::tempdir().expect("golden temporary root");
    let root = temporary.path().canonicalize().expect("canonical root");
    let release_artifact_root = if gateway_service_external_e2e {
        root.join("artifacts").join("releases")
    } else {
        root.join("release-artifacts")
    };
    let repository_root = root.join("repositories");
    let storage = Arc::new(
        GitStorage::initialize(&repository_root)
            .await
            .expect("fixture Git storage"),
    );
    let fixture_repository = PgForgeRepository::new(pool.clone(), Arc::clone(&storage));
    let browser_oidc_issuer = golden_issuer();
    let user_id = UserId::new();
    let organization_id = OrganizationId::new();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Golden User')")
        .bind(user_id.as_uuid())
        .execute(pool)
        .await
        .expect("seed user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id.as_uuid())
        .bind(format!("golden-{organization_id}"))
        .execute(pool)
        .await
        .expect("seed organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
           VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id.as_uuid())
    .bind(user_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed organization owner");
    let outsider_id = UserId::new();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Cooking Outsider')")
        .bind(outsider_id.as_uuid())
        .execute(pool)
        .await
        .expect("seed cooking outsider");
    sqlx::query(
        "INSERT INTO external_identities
           (user_id, issuer, subject, provider_metadata)
           VALUES ($1, $2, 'golden-subject', '{}')",
    )
    .bind(user_id.as_uuid())
    .bind(&browser_oidc_issuer)
    .execute(pool)
    .await
    .expect("seed external identity");
    sqlx::query(
        "INSERT INTO external_identities
           (user_id, issuer, subject, provider_metadata)
           VALUES ($1, $2, 'outsider', '{}')",
    )
    .bind(outsider_id.as_uuid())
    .bind(&browser_oidc_issuer)
    .execute(pool)
    .await
    .expect("seed cooking outsider identity");
    let owner_browser_session =
        seed_golden_browser_session(pool, user_id, &browser_oidc_issuer, "golden-subject").await;
    let outsider_browser_session =
        seed_golden_browser_session(pool, outsider_id, &browser_oidc_issuer, "outsider").await;
    let project = fixture_repository
        .create_project_trusted(organization_id, "golden-project")
        .await
        .expect("seed project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project.id.as_uuid())
        .bind(user_id.as_uuid())
        .execute(pool)
        .await
        .expect("seed project maintainer");
    // The config is resolved by the production Git-receive path, so seed the
    // same immutable catalog record that the runtime maps to the libkrun root
    // filesystem below. This keeps the composed proof on the real resolver
    // rather than retaining the previous, now-invalid ad-hoc image syntax.
    sqlx::query(
        "INSERT INTO oci_images
             (id, key, display_name, image_reference, toolchains, architectures,
              availability_state, provenance, platform_policy_version)
         VALUES ($1, 'golden-root', 'Golden root', $2, '[]'::jsonb,
                 ARRAY['x86_64'], 'available', '{}'::jsonb, 'golden/v1')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(ROOT_IMAGE)
    .execute(pool)
    .await
    .expect("seed selected OCI image");
    let repository = fixture_repository
        .create_repository_trusted(&CreateRepository {
            project_id: project.id,
            name: String::from("golden-repository"),
            default_branch: GitRef::parse("refs/heads/main").expect("default ref"),
            is_public: false,
            agent_runs_enabled: true,
        })
        .await
        .expect("seed repository");
    let seeded_instance = seed_reusable_instance(
        pool,
        user_id,
        project.id.as_uuid(),
        repository.id.as_uuid(),
        &release_artifact_root,
        libkrun_e2e,
        None,
        "golden-agent",
    )
    .await;
    let brokered_fixture = if libkrun_e2e && !cooking_build_proof && !session_chat_e2e {
        Some(if cooking::enabled() {
            cooking::seed_brokered_fixture(
                pool,
                user_id,
                organization_id,
                project.id.as_uuid(),
                &seeded_instance,
            )
            .await
        } else {
            seed_brokered_https_fixture(
                pool,
                user_id,
                organization_id,
                project.id.as_uuid(),
                &seeded_instance,
            )
            .await
        })
    } else {
        None
    };
    let session_chat_fixture = if session_chat_e2e {
        Some(session_chat::start_model_fixture().await)
    } else {
        None
    };
    // This mailbox is created before the daemon starts so the released
    // gateway revision can be bound to it immutably.  The guest only sees the
    // symbolic `deliver` slot, never this UUID or the producer identity.
    let gateway_mailbox = if gateway_caddy_e2e && !cooking_build_proof {
        let mailbox_id = MailboxId::new();
        PostgresMailboxRepository::new(pool.clone())
            .ensure_mailbox(
                project.id.as_uuid(),
                mailbox_id,
                runtime_types::AgentInstanceId::from_uuid(seeded_instance.instance),
            )
            .await
            .expect("create bound gateway golden mailbox");
        Some(mailbox_id)
    } else {
        None
    };
    let gateway_service_fixture = if gateway_service_e2e && !cooking_build_proof {
        let service_agent =
            seed_gateway_service_release_agent(pool, &seeded_instance, &release_artifact_root)
                .await;
        Some(
            seed_gateway_service_route(
                pool,
                user_id,
                project.id.as_uuid(),
                repository.id.as_uuid(),
                seeded_instance.release,
                service_agent,
                gateway_service_log_guest_e2e,
            )
            .await,
        )
    } else {
        None
    };
    let gateway_edge = if gateway_caddy_e2e && !cooking_build_proof {
        let gateway_agent =
            seed_gateway_release_agent(pool, &seeded_instance, &release_artifact_root).await;
        let fixture = brokered_fixture
            .as_ref()
            .expect("joined gateway proof has brokered fixture authority");
        let gateway_fixture = seed_gateway_brokered_route(
            pool,
            user_id,
            project.id.as_uuid(),
            repository.id.as_uuid(),
            seeded_instance.release,
            gateway_agent,
            fixture.import_id,
            fixture.version_id,
            gateway_mailbox.expect("joined gateway mailbox"),
        )
        .await;
        let dispatcher = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve gateway dispatcher listener");
        let dispatcher_listen = dispatcher
            .local_addr()
            .expect("gateway dispatcher listener address");
        drop(dispatcher);
        Some((
            GatewayEdgeConfig {
                caddy_admin_url: env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL")
                    .expect("joined Caddy admin URL"),
                caddy_configuration_template: caddy_configuration(
                    &env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL"),
                ),
                caddy_server_name: String::from("shared"),
                dispatcher_listen,
                public_authority: String::from("gateway.golden.invalid"),
                ui_origin: None,
            },
            gateway_fixture,
        ))
    } else {
        None
    };
    // The installed UI fixture publishes an HTTP service gateway before the
    // later Cooking-service restart. Give the initial daemon its real gateway
    // supervisor so that desired service revisions can pass readiness and
    // become active before InstallUi resolves the managed route. The UI origin
    // is intentionally added only by the later restart below.
    let initial_gateway_edge = if installed_ui_fixture {
        let dispatcher = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve installed UI gateway dispatcher listener");
        let dispatcher_listen = dispatcher
            .local_addr()
            .expect("installed UI gateway dispatcher listener address");
        drop(dispatcher);
        Some(GatewayEdgeConfig {
            caddy_admin_url: env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL")
                .expect("joined Caddy admin URL"),
            caddy_configuration_template: caddy_configuration(
                &env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL"),
            ),
            caddy_server_name: String::from("shared"),
            dispatcher_listen,
            public_authority: String::from("gateway.golden.invalid"),
            ui_origin: session_chat_browser_e2e
                .then(|| installed_ui_origin_config(reserve_installed_ui_listener())),
        })
    } else {
        gateway_edge.as_ref().map(|(config, _)| config.clone())
    };

    GoldenSeed {
        temporary,
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
        session_chat_fixture,
        gateway_mailbox,
        gateway_service_fixture,
        gateway_edge,
        initial_gateway_edge,
    }
}
