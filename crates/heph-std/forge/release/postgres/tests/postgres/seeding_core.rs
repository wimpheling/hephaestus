use super::*;

pub(crate) const fn selected_policy() -> RuntimePolicy {
    RuntimePolicy {
        vcpus: 2,
        memory_mib: 1024,
        network: NetworkAccess::BrokerOnly,
    }
}

pub(crate) const fn platform_policy() -> RuntimePolicy {
    RuntimePolicy {
        vcpus: 8,
        memory_mib: 8192,
        network: NetworkAccess::Egress,
    }
}

pub(crate) fn key(operation: &str, id: Uuid) -> ReleaseCommandKey {
    ReleaseCommandKey::derive(operation, &[id.as_bytes()])
}

pub(crate) fn gate_race_event(
    mailbox_id: MailboxId,
    instance_id: AgentInstanceId,
    body: &[u8],
    identity: &str,
) -> MailboxEvent {
    MailboxEvent {
        id: MailboxEventId::new(),
        mailbox_id,
        instance_id,
        producer_id: ProducerId::parse(identity).expect("producer identity"),
        deduplication_key: DeduplicationKey::parse(identity).expect("deduplication key"),
        envelope: MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").expect("method"),
            EnvelopeRoute::parse(format!("/{identity}")).expect("route"),
            BTreeMap::new(),
            ContentMetadata::new(
                BodyReference::new(
                    BodyReferenceId::new(),
                    u32::try_from(body.len()).expect("bounded body"),
                    Sha256::digest(body).into(),
                )
                .expect("body reference"),
                Some(String::from("application/octet-stream")),
                None,
            )
            .expect("content metadata"),
            OffsetDateTime::now_utc(),
            None,
        )
        .expect("mailbox envelope"),
    }
}

pub(crate) fn identity(user_id: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user_id,
        "https://issuer.release.test",
        format!("release-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}

pub(crate) async fn pool() -> Option<PgPool> {
    let url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    Some(
        PgPoolOptions::new()
            .max_connections(8)
            .connect(&url)
            .await
            .expect("connect PostgreSQL"),
    )
}

pub(crate) async fn worker_pool() -> Option<PgPool> {
    worker_pool_named("hephaestus-release-test").await
}

pub(crate) async fn worker_pool_named(application_name: &str) -> Option<PgPool> {
    let url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let application_name = application_name.to_owned();
    Some(
        PgPoolOptions::new()
            .max_connections(4)
            .after_connect(move |connection, _metadata| {
                let application_name = application_name.clone();
                Box::pin(async move {
                    sqlx::query("SET ROLE hephaestus_worker")
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('application_name', $1, false)")
                        .bind(application_name)
                        .execute(&mut *connection)
                        .await
                        .map(|_| ())
                })
            })
            .connect(&url)
            .await
            .expect("connect worker PostgreSQL"),
    )
}

pub(crate) fn decode_test_hash(value: &str) -> [u8; 32] {
    assert_eq!(value.len(), 64, "test hash length");
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = u8::from_str_radix(std::str::from_utf8(pair).expect("test hash UTF-8"), 16)
            .expect("test hash hex");
    }
    output
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn seed(pool: &PgPool) -> Fixture {
    seed_with_config(pool, &reusable_config()).await
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn seed_with_config(pool: &PgPool, source: &str) -> Fixture {
    let actor = UserId::new();
    let organization = OrganizationId::new();
    let source_project = ProjectId::new();
    let source_repository = RepositoryId::new();
    let first_project = ProjectId::new();
    let first_repository = RepositoryId::new();
    let first_aux_repository = RepositoryId::new();
    let second_project = ProjectId::new();
    let second_repository = RepositoryId::new();
    let receive_id = Uuid::new_v4();
    let build = BuildRequestId::new();
    let commit = "a".repeat(40);
    let build_image_key = format!("build-{source_repository}");
    let runtime_image_key = format!("runtime-{source_repository}");
    let source = source
        .replace("key = \"build\"", &format!("key = \"{build_image_key}\""))
        .replace(
            "key = \"runtime\"",
            &format!("key = \"{runtime_image_key}\""),
        );
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(actor.as_uuid())
        .bind(format!("release-actor-{actor}"))
        .execute(pool)
        .await
        .expect("seed actor");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization.as_uuid())
        .bind(format!("release-org-{organization}"))
        .execute(pool)
        .await
        .expect("seed organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'member')",
    )
    .bind(organization.as_uuid())
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed member");
    for (project, name) in [
        (source_project, "source"),
        (first_project, "first"),
        (second_project, "second"),
    ] {
        sqlx::query(
            "INSERT INTO projects (id, organization_id, name)
             VALUES ($1, $2, $3)",
        )
        .bind(project.as_uuid())
        .bind(organization.as_uuid())
        .bind(format!("{name}-{project}"))
        .execute(pool)
        .await
        .expect("seed project");
        sqlx::query(
            "INSERT INTO project_maintainers (project_id, user_id)
             VALUES ($1, $2)",
        )
        .bind(project.as_uuid())
        .bind(actor.as_uuid())
        .execute(pool)
        .await
        .expect("seed project maintainer");
    }
    for (repository, project, name) in [
        (source_repository, source_project, "source"),
        (first_repository, first_project, "first"),
        (first_aux_repository, first_project, "first-aux"),
        (second_repository, second_project, "second"),
    ] {
        sqlx::query(
            "INSERT INTO repositories
             (id, project_id, name, default_branch, is_public)
             VALUES ($1, $2, $3, 'refs/heads/main', false)",
        )
        .bind(repository.as_uuid())
        .bind(project.as_uuid())
        .bind(format!("{name}-{repository}"))
        .execute(pool)
        .await
        .expect("seed repository");
    }
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'release-test', 'accepted', now())",
    )
    .bind(receive_id)
    .bind(source_repository.as_uuid())
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed receive");
    let digest = "a".repeat(64);
    for key in [&build_image_key, &runtime_image_key] {
        sqlx::query(
            "INSERT INTO oci_images
             (id, key, display_name, image_reference, toolchains, architectures,
              availability_state, provenance, platform_policy_version)
             VALUES ($1, $2, $2, $3, '[]'::jsonb, ARRAY['x86_64'],
                     'available', '{}'::jsonb, 'test/v1')",
        )
        .bind(Uuid::new_v4())
        .bind(key)
        .bind(format!("{key}@sha256:{digest}"))
        .execute(pool)
        .await
        .expect("seed OCI image");
    }
    let parsed = parse(source.as_bytes());
    let config = parsed.config.expect("fixture configuration should parse");
    sqlx::query(
        "INSERT INTO agent_config_revisions
         (id, repository_id, receive_id, commit_sha, config_hash,
          normalized_config_hash, schema_version, status, config, diagnostics)
         VALUES ($1, $2, $3, $4, $5, $6, 2, 'valid', $7, '[]')",
    )
    .bind(Uuid::new_v4())
    .bind(source_repository.as_uuid())
    .bind(receive_id)
    .bind(&commit)
    .bind(parsed.hash.as_str())
    .bind(
        parsed
            .normalized_hash
            .expect("fixture normalized hash")
            .as_str(),
    )
    .bind(serde_json::to_value(config).expect("serialize fixture config"))
    .execute(pool)
    .await
    .expect("seed reusable configuration");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'importing', $7)",
    )
    .bind(build.as_uuid())
    .bind(source_repository.as_uuid())
    .bind(commit)
    .bind(
        GitRef::parse("refs/heads/main")
            .expect("ref should parse")
            .as_str(),
    )
    .bind(receive_id)
    .bind([9_u8; 32].as_slice())
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed importing build");
    sqlx::query(
        "INSERT INTO build_request_images
         (build_request_id, execution_context, image_id, image_key, image_reference)
         SELECT $1, context.execution_context, image.id, image.key, image.image_reference
           FROM (VALUES ('build'::text, $2::text), ('guest', $3::text))
                    AS context(execution_context, image_key)
           JOIN oci_images AS image ON image.key = context.image_key",
    )
    .bind(build.as_uuid())
    .bind(&build_image_key)
    .bind(&runtime_image_key)
    .execute(pool)
    .await
    .expect("seed build image snapshots");
    Fixture {
        actor,
        organization,
        first_project,
        first_repository,
        first_aux_repository,
        second_project,
        second_repository,
        build,
    }
}
