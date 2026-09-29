use super::*;

pub struct TestSpecFactory {
    pub root: std::path::PathBuf,
}

#[async_trait::async_trait]
impl VmSpecFactory for TestSpecFactory {
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        Ok(VmSpec {
            id: VmId(run.id.to_string()),
            root: RootFilesystem::Directory {
                host_path: self.root.clone(),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 128,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: String::from("/bin/true"),
                args: Vec::new(),
                env: BTreeMap::new(),
                working_dir: None,
            },
            runtime_authority: None,
            runtime_git_bridge: None,
            private_http_service: None,
            labels: BTreeMap::new(),
        })
    }
}

pub async fn wait_for_run_state(pool: &PgPool, run_id: RunId, expected: &str) {
    for _ in 0..200 {
        let state = sqlx::query_scalar::<_, String>("SELECT state FROM runs WHERE id = $1")
            .bind(run_id.as_uuid())
            .fetch_optional(pool)
            .await
            .expect("load run state");
        if state.as_deref() == Some(expected) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("run {run_id} did not reach {expected}");
}

pub async fn cleanup_run(pool: &PgPool, command: &StartRun) {
    let volume_id: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT volume_id FROM runs WHERE id = $1")
            .bind(command.run_id.as_uuid())
            .fetch_one(pool)
            .await
            .expect("run volume");
    sqlx::query("DELETE FROM outbox WHERE aggregate_type = 'run' AND aggregate_id = $1")
        .bind(command.run_id.as_uuid())
        .execute(pool)
        .await
        .expect("delete run outbox");
    sqlx::query("DELETE FROM run_events WHERE run_id = $1")
        .bind(command.run_id.as_uuid())
        .execute(pool)
        .await
        .expect("delete run events");
    sqlx::query("DELETE FROM command_inbox WHERE payload->>'run_id' = $1")
        .bind(command.run_id.to_string())
        .execute(pool)
        .await
        .expect("delete command inbox");
    if let Some(volume_id) = volume_id {
        sqlx::query("DELETE FROM agent_instance_volume_leases WHERE volume_id = $1")
            .bind(volume_id)
            .execute(pool)
            .await
            .expect("delete volume leases");
    }
    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(command.run_id.as_uuid())
        .execute(pool)
        .await
        .expect("delete run");
}

pub async fn fixture() -> Option<(PgPool, PgForgeRepository, Repository, tempfile::TempDir)> {
    let url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("PostgreSQL integration connection");
    let temporary = tempfile::tempdir().expect("temporary directory");
    let storage = Arc::new(
        GitStorage::initialize(temporary.path().join("repositories"))
            .await
            .expect("Git storage"),
    );
    let service = PgForgeRepository::new(pool.clone(), storage);
    service.initialize().await.expect("forge migrations");
    let organization_id = OrganizationId::new();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id.as_uuid())
        .bind("forge-service-integration")
        .execute(&pool)
        .await
        .expect("organization");
    let project = service
        .create_project_trusted(organization_id, "forge-service-integration")
        .await
        .expect("project");
    let repository = service
        .create_repository_trusted(&CreateRepository {
            project_id: project.id,
            name: String::from("repository"),
            default_branch: GitRef::parse("refs/heads/main").expect("default branch"),
            is_public: false,
            agent_runs_enabled: true,
        })
        .await
        .expect("repository");
    seed_catalog_images(&pool, repository.id.as_uuid()).await;
    Some((pool, service, repository, temporary))
}

pub async fn app_role_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .expect("connect application role")
}

pub async fn seed_catalog_images(pool: &PgPool, repository_id: Uuid) {
    for (key, display_name, image_reference) in [
        (
            fixture_image_key("build", repository_id),
            String::from("Forge test build image"),
            fixture_image_reference("forge-build"),
        ),
        (
            fixture_image_key("runtime", repository_id),
            String::from("Forge test runtime image"),
            fixture_image_reference("forge-runtime"),
        ),
    ] {
        sqlx::query(
            "INSERT INTO oci_images
             (id, key, display_name, image_reference, toolchains, architectures,
              availability_state, provenance, platform_policy_version)
             VALUES ($1, $2, $3, $4, '[]'::jsonb, ARRAY['x86_64'],
                     'available', '{}'::jsonb, 'test/v1')",
        )
        .bind(Uuid::new_v4())
        .bind(key)
        .bind(display_name)
        .bind(image_reference)
        .execute(pool)
        .await
        .expect("seed fixture OCI image");
    }
}

pub async fn seed_reusable_attachment(pool: &PgPool, repository: &Repository) {
    let build_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    seed_reusable_release(
        pool,
        repository,
        build_id,
        family_id,
        release_id,
        release_agent_id,
    )
    .await;
    let instance_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let attachment_id = Uuid::new_v4();
    seed_attached_instance(
        pool,
        repository,
        family_id,
        release_agent_id,
        instance_id,
        revision_id,
        attachment_id,
    )
    .await;
}
