use super::*;

pub(crate) async fn assert_no_installation(pool: &PgPool, project_id: ProjectId, ui_key: &str) {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installations WHERE project_id = $1 AND ui_key = $2",
    )
    .bind(project_id.as_uuid())
    .bind(ui_key)
    .fetch_one(pool)
    .await
    .expect("installation absence");
    assert_eq!(count, 0);
}

pub(crate) const MANAGED_API_UI: &str = r#"
version = 1

[[uis]]
key = "assistant"
scope = "project"
label = "Assistant"
icon = "chat"
presentation = "iframe"
route_base = "assistant"
ui_kit_version = 1
cache = "no_store"

[[uis.apis]]
key = "service-api"
gateway_name = "ui-service"
method = "GET"
route = "/service/api"

[uis.content]
kind = "managed_service"
gateway_name = "ui-service"
route = "/service/ui"
entrypoint = "index.html"
"#;

pub(crate) const fn managed_api_manifest() -> &'static [u8] {
    MANAGED_API_UI.as_bytes()
}

pub(crate) fn static_ui_manifest_for_scope(scope: &str) -> String {
    assert!(matches!(scope, "global" | "project" | "repository"));
    static_ui_manifest("text/html")
        .replace("scope = \"repository\"", &format!("scope = \"{scope}\""))
}

pub(crate) fn static_ui_api_manifest() -> String {
    static_ui_manifest("text/html").replace(
        "[uis.content]",
        "[[uis.apis]]\nkey = \"service-api\"\ngateway_name = \"ui-service\"\nmethod = \"GET\"\nroute = \"/service\"\n\n[uis.content]",
    )
}

pub(crate) fn static_ui_manifest(media_type: &str) -> String {
    format!(
        r#"
version = 1

[[uis]]
key = "docs"
scope = "repository"
label = "Docs"
icon = "book"
presentation = "iframe"
route_base = "docs"
ui_kit_version = 1
cache = "no_store"

[uis.content]
kind = "static"
entrypoint = "index.html"

[[uis.content.files]]
route = "index.html"
artifact = "dist/index.html"
media_type = "{media_type}"
"#
    )
}

pub(crate) fn managed_gateway_manifest(agent_name: &str) -> String {
    format!(
        r#"
version = 1

[[gateways]]
name = "ui-service"
agent_name = "{agent_name}"
handler_contract = "http.service.v1"
exposure = "heph_authenticated"

[gateways.service]
loopback_port = 8080
readiness_path = "/ready"
health_path = "/health"

[[gateways.routes]]
path = "/service"
methods = ["GET"]
"#
    )
}

pub(crate) fn release_test_artifact(
    path: &str,
    kind: ArtifactKind,
    media_type: &str,
    size_bytes: u64,
) -> ReleaseArtifactInput {
    ReleaseArtifactInput {
        id: ReleaseArtifactId::new(),
        path: ArtifactPath::parse(path).expect("artifact path"),
        kind,
        mode: 0o444,
        content_hash: ContentHash::digest(path.as_bytes()),
        size_bytes,
        media_type: media_type.to_owned(),
        storage_key: Uuid::new_v4(),
    }
}

pub(crate) async fn attach_ui_capture(
    pool: &PgPool,
    build: BuildRequestId,
    ui_source: impl AsRef<[u8]>,
    gateway_source: Option<&[u8]>,
    build_hash_override: Option<[u8; 32]>,
) {
    attach_ui_capture_with_hashes(
        pool,
        build,
        ui_source,
        gateway_source,
        build_hash_override,
        None,
        None,
    )
    .await;
}

// Keep capture construction linear so every immutable source field is visible.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) async fn attach_ui_capture_with_hashes(
    pool: &PgPool,
    build: BuildRequestId,
    ui_source: impl AsRef<[u8]>,
    gateway_source: Option<&[u8]>,
    build_hash_override: Option<[u8; 32]>,
    stored_ui_hash_override: Option<[u8; 32]>,
    stored_gateway_hash_override: Option<[u8; 32]>,
) {
    let ui_source = ui_source.as_ref();
    let parsed_ui = parse_repository_uis(ui_source);
    let ui_config = parsed_ui.config.expect("valid UI fixture");
    let ui_hash = decode_test_hash(
        parsed_ui
            .normalized_hash
            .expect("normalized UI hash")
            .as_str(),
    );
    let (repository_id, receive_id, source_commit, config_json): (Uuid, Uuid, String, Value) =
        sqlx::query_as(
            "SELECT request.repository_id, request.origin_receive_id,
                    request.source_commit, revision.config
             FROM build_requests AS request
             JOIN agent_config_revisions AS revision
               ON revision.repository_id = request.repository_id
              AND revision.commit_sha = request.source_commit
             WHERE request.id = $1",
        )
        .bind(build.as_uuid())
        .fetch_one(pool)
        .await
        .expect("load UI fixture identity");
    let config: AgentConfig = serde_json::from_value(config_json).expect("fixture config");
    let base_hash = agent_config::build_identity::base_build_definition_hash(
        config.build.as_ref().expect("build fixture"),
    )
    .expect("base build hash");
    let (gateway_json, gateway_hash) = gateway_source.map_or((None, None), |source| {
        let parsed = parse_repository_gateways(source);
        let config = parsed.config.expect("valid gateway fixture");
        let canonical = agent_config::canonical_repository_gateways(&config);
        let bytes = toml::to_string(&canonical).expect("gateway TOML");
        let hash: [u8; 32] = Sha256::digest(bytes.as_bytes()).into();
        (
            Some(serde_json::to_value(config).expect("gateway JSON")),
            Some(hash.to_vec()),
        )
    });
    let requires_gateways = gateway_json.is_some();
    let build_definition_hash = build_hash_override.unwrap_or_else(|| {
        agent_config::build_identity::ui_build_definition_hash(
            base_hash,
            ui_hash,
            gateway_hash
                .as_deref()
                .map(|value| value.try_into().expect("gateway hash")),
        )
    });
    let stored_ui_hash = stored_ui_hash_override.unwrap_or(ui_hash);
    let stored_gateway_hash = gateway_hash
        .as_deref()
        .map(|value| value.try_into().expect("gateway hash"))
        .map(|value: [u8; 32]| stored_gateway_hash_override.unwrap_or(value))
        .map(|value| value.to_vec());
    let source_manifest_revision_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, requires_gateways,
          normalized_ui_config, normalized_ui_hash, gateway_manifest_oid,
          gateway_actual_size_bytes, gateway_source_sha256,
          normalized_gateway_config, normalized_gateway_hash, diagnostics)
         VALUES ($1, $2, $3, $4, 'blob', $5, $6, $7, 'valid', $8, $9, $10,
                 $11, $12, $13, $14, $15, '[]')",
    )
    .bind(source_manifest_revision_id)
    .bind(repository_id)
    .bind(receive_id)
    .bind(&source_commit)
    .bind("b".repeat(40))
    .bind(i64::try_from(ui_source.len()).expect("bounded UI fixture"))
    .bind([1_u8; 32].as_slice())
    .bind(requires_gateways)
    .bind(serde_json::to_value(&ui_config).expect("UI JSON"))
    .bind(stored_ui_hash.as_slice())
    .bind(gateway_json.as_ref().map(|_| "c".repeat(40)))
    .bind(gateway_json.as_ref().map(|_| 128_i64))
    .bind(gateway_json.as_ref().map(|_| vec![2_u8; 32]))
    .bind(gateway_json)
    .bind(stored_gateway_hash)
    .execute(pool)
    .await
    .expect("insert UI source capture");
    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit,
          source_manifest_revision_id, source_status)
         VALUES ($1, $2, $3, $4, 'valid')",
    )
    .bind(build.as_uuid())
    .bind(repository_id)
    .bind(&source_commit)
    .bind(source_manifest_revision_id)
    .execute(pool)
    .await
    .expect("link UI source capture");
    sqlx::query("UPDATE build_requests SET build_definition_hash = $2 WHERE id = $1")
        .bind(build.as_uuid())
        .bind(build_definition_hash.as_slice())
        .execute(pool)
        .await
        .expect("store UI build identity");
}

pub(crate) async fn assert_rejected_ui_build(
    service: &ReleaseService,
    admin_pool: &PgPool,
    build: BuildRequestId,
    release_id: ReleaseId,
    artifacts: Vec<ReleaseArtifactInput>,
) {
    let command_key = key("complete-invalid-ui", release_id.as_uuid());
    let result = service
        .complete_build(CompleteBuild {
            command_key,
            build_request_id: build,
            release_id,
            version: ReleaseVersion::parse("invalid-ui-v1").expect("release version"),
            release_agent_id: ReleaseAgentId::new(),
            artifacts,
        })
        .await;
    assert!(
        matches!(
            result,
            Err(release_postgres::ReleaseServiceError::InvalidStoredData)
        ),
        "expected redacted stored-data rejection, got {result:?}"
    );
    let counts: (i64, i64, i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM releases WHERE id = $1),
             (SELECT count(*) FROM release_artifacts WHERE release_id = $1),
             (SELECT count(*) FROM release_agents WHERE release_id = $1),
             (SELECT count(*) FROM release_ui_source_snapshots WHERE release_id = $1),
             (SELECT count(*) FROM release_ui_descriptors WHERE release_id = $1),
             (SELECT count(*) FROM release_ui_static_files WHERE release_id = $1),
             (SELECT count(*) FROM release_ui_managed_services WHERE release_id = $1),
             (SELECT count(*) FROM release_ui_api_bindings WHERE release_id = $1),
             (SELECT count(*) FROM release_command_inbox WHERE command_key = $2)",
    )
    .bind(release_id.as_uuid())
    .bind(command_key.as_bytes().as_slice())
    .fetch_one(admin_pool)
    .await
    .expect("rejected publication counts");
    assert_eq!(counts, (0, 0, 0, 0, 0, 0, 0, 0, 0));
    let (state,): (String,) = sqlx::query_as("SELECT state FROM build_requests WHERE id = $1")
        .bind(build.as_uuid())
        .fetch_one(admin_pool)
        .await
        .expect("rejected build state");
    assert_eq!(state, "importing");
}
