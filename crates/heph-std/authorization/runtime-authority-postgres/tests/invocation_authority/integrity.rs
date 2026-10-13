use super::harness::{Harness, code};
use capability_domain::{AuthorizationSnapshot, RuntimeSessionId, RuntimeSessionIdentity};
use runtime_authority::{NewRuntimeSession, RuntimeSessionRepository};
use runtime_authority_postgres::PgRuntimeSessionRepository;
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn default_off_actor_forgery_and_config_substitution_leave_whole_graph_unchanged(
    pool: PgPool,
) {
    let h = Box::pin(Harness::new(&pool, 0)).await;
    let before = h.rows().await;
    let default = PgRuntimeSessionRepository::new(h.birth.birth.worker.clone());
    assert!(matches!(
        default.create(h.session()).await,
        Err(runtime_authority::RuntimeAuthorityError::Unsupported)
    ));
    let actor_error = h
        .raw_writer(&h.birth.birth.app, &h.identity)
        .await
        .unwrap_err();
    assert_eq!(
        code(&actor_error).as_deref(),
        Some("42501"),
        "{actor_error:?}"
    );
    let config = serde_json::to_vec(&h.birth.config.canonical_json()).unwrap();
    let error = sqlx::query("SELECT write_invocation119_session($1,$2,$3,$4,$5,1,$6,$7,$8,$9,$10)")
        .bind(h.birth.start.run_id.as_uuid())
        .bind(&config[..config.len() - 1])
        .bind(h.snapshot.authorization_model_version())
        .bind(h.snapshot.normalized_hash().as_bytes().as_slice())
        .bind(h.identity.normalized_hash().as_bytes().as_slice())
        .bind(h.credential.as_bytes().as_slice())
        .bind(h.identity.issued_at())
        .bind(h.identity.expires_at())
        .bind(h.identity.issued_at().unix_timestamp_nanos().to_string())
        .bind(h.identity.expires_at().unix_timestamp_nanos().to_string())
        .execute(&h.birth.birth.worker)
        .await
        .unwrap_err();
    assert_eq!(
        code(&error).as_deref(),
        Some("P1190"),
        "saved checked configuration cannot be substituted: {error:?}"
    );
    assert_eq!(h.rows().await, before);
    h.issue().await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn foreign_session_and_omitted_binding_ceiling_cannot_create_replacement(pool: PgPool) {
    let mut h = Box::pin(Harness::new(&pool, 0)).await;
    h.add_optional_bindings().await;
    let before = h.rows().await;
    let subset = AuthorizationSnapshot::new(
        h.snapshot.id(),
        h.snapshot.principal(),
        h.snapshot.authorization_model_version(),
        Vec::new(),
    )
    .unwrap();
    let identity = RuntimeSessionIdentity::new(
        h.identity.id(),
        subset.principal(),
        h.identity.invocation(),
        &subset,
        h.identity.issued_at(),
        h.identity.expires_at(),
    )
    .unwrap();
    let input = NewRuntimeSession {
        snapshot: &subset,
        identity: &identity,
        generation: capability_domain::RuntimeCredentialGeneration::INITIAL,
        credential_hash: h.credential,
        attachment_id: None,
    };
    assert!(h.repository.create(input).await.is_err());
    // The identity retains the subset hash although the supplied snapshot is
    // complete. SQL must rederive the identity from the full immutable ceiling.
    let mismatched = NewRuntimeSession {
        snapshot: &h.snapshot,
        identity: &identity,
        generation: capability_domain::RuntimeCredentialGeneration::INITIAL,
        credential_hash: h.credential,
        attachment_id: None,
    };
    assert!(h.repository.create(mismatched).await.is_err());
    let error = h
        .raw_writer(&h.birth.birth.worker, &identity)
        .await
        .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("23000"), "{error:?}");
    assert_eq!(h.rows().await, before);
    let foreign = RuntimeSessionIdentity::new(
        RuntimeSessionId::from_uuid(Uuid::new_v4()),
        h.identity.principal(),
        h.identity.invocation(),
        &h.snapshot,
        h.identity.issued_at(),
        h.identity.expires_at(),
    )
    .unwrap();
    assert!(h.raw_writer(&h.birth.birth.worker, &foreign).await.is_err());
    assert_eq!(h.rows().await, before);
    h.issue().await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn raw_snapshot_without_complete_session_is_deferred_denied_and_git_credentials_refused(
    pool: PgPool,
) {
    let h = Box::pin(Harness::new(&pool, 0)).await;
    let before = h.rows().await;
    let mut raw = h.birth.birth.worker.begin().await.unwrap();
    sqlx::query("INSERT INTO run_authorization_snapshots(id,run_id,instance_id,instance_revision_id,authorization_model_version,normalized_hash) VALUES($1,$1,$2,$3,$4,$5)")
  .bind(h.birth.start.run_id.as_uuid()).bind(h.birth.start.instance_id.as_uuid()).bind(h.birth.start.instance_revision_id.as_uuid())
  .bind(h.snapshot.authorization_model_version()).bind(h.snapshot.normalized_hash().as_bytes().as_slice())
  .execute(&mut *raw).await.unwrap();
    let error = raw.commit().await.unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("23000"), "{error:?}");
    assert_eq!(h.rows().await, before);
    h.issue().await;
    let before = h.rows().await;
    let error=sqlx::query("INSERT INTO runtime_git_credentials(runtime_session_id,authorization_snapshot_id,binding_id,repository_id,scope_hash,issuance_generation,credential_hash,expires_at) VALUES($1,$1,$2,$3,$4,1,$5,$6)")
  .bind(h.birth.start.run_id.as_uuid()).bind(Uuid::new_v4()).bind(h.birth.birth.fixture.private_repository)
  .bind([37_u8;32].as_slice()).bind([38_u8;32].as_slice()).bind(h.identity.expires_at())
  .execute(&h.birth.birth.worker).await.unwrap_err();
    assert_eq!(
        code(&error).as_deref(),
        Some("0A000"),
        "Invocation never copies a Git credential: {error:?}"
    );
    let error=sqlx::query("INSERT INTO run_git_authority_snapshots(snapshot_id,instance_revision_id,binding_id,repository_id,grammar_version,git_operations,ref_globs,changed_path_globs,branch_update_policy,branch_create,branch_delete,tag_create,tag_update,tag_delete,other_create,other_update,other_delete,request_bytes,pack_bytes,object_count,ref_updates,exact_parent_required,expected_parent,normalized_hash) VALUES($1,$2,$3,$4,1,ARRAY['git_read'],'{}','{}','fast_forward_only',false,false,false,false,false,false,false,false,1,1,1,1,false,NULL,$5)")
  .bind(h.birth.start.run_id.as_uuid()).bind(h.birth.start.instance_revision_id.as_uuid())
  .bind(Uuid::new_v4()).bind(h.birth.birth.fixture.consuming_repository).bind([39_u8;32].as_slice())
  .execute(&h.birth.birth.worker).await.unwrap_err();
    assert_eq!(
        code(&error).as_deref(),
        Some("0A000"),
        "Invocation cannot copy a substituted Git scope: {error:?}"
    );
    assert_eq!(h.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn protected_writer_catalog_acl_rls_and_trusted_configuration_are_explicit(pool: PgPool) {
    let h = Box::pin(Harness::new(&pool, 0)).await;
    let catalog:(bool,bool,bool,bool)=sqlx::query_as("SELECT p.prosecdef,EXISTS(SELECT 1 FROM unnest(p.proconfig) item WHERE item='search_path=pg_catalog, public, pg_temp'),owner.rolname NOT IN ('hephaestus_app','hephaestus_worker'),NOT EXISTS(SELECT 1 FROM aclexplode(COALESCE(p.proacl,acldefault('f',p.proowner))) acl WHERE acl.grantee=0 AND acl.privilege_type='EXECUTE') FROM pg_proc p JOIN pg_roles owner ON owner.oid=p.proowner WHERE p.oid='write_invocation119_session(uuid,bytea,text,bytea,bytea,bigint,bytea,timestamptz,timestamptz,text,text)'::regprocedure")
  .fetch_one(&pool).await.unwrap();
    assert_eq!(catalog, (true, true, true, true));
    let acl:(bool,bool)=sqlx::query_as("SELECT has_function_privilege('hephaestus_worker','write_invocation119_session(uuid,bytea,text,bytea,bytea,bigint,bytea,timestamptz,timestamptz,text,text)','EXECUTE'),has_function_privilege('hephaestus_app','write_invocation119_session(uuid,bytea,text,bytea,bytea,bigint,bytea,timestamptz,timestamptz,text,text)','EXECUTE')")
  .fetch_one(&pool).await.unwrap();
    assert_eq!(acl, (true, false));
    let tables:Vec<(String,bool,bool)>=sqlx::query_as("SELECT relname,relrowsecurity,relforcerowsecurity FROM pg_class WHERE oid IN ('run_authorization_snapshots'::regclass,'run_authorization_snapshot_bindings'::regclass,'runtime_authority_sessions'::regclass) ORDER BY relname")
  .fetch_all(&pool).await.unwrap();
    assert_eq!(tables.len(), 3);
    assert!(
        tables
            .iter()
            .all(|(_, enabled, forced)| *enabled && *forced)
    );
    let stored = h.issue().await;
    let config = &h.birth.config;
    let changed = release_service::InstanceExecutionConfiguration::new(
        config.provider_scope().clone(),
        config.volume_root().to_owned(),
        config.volume_namespace(),
        config.platform_policy().clone(),
        "changed/server-platform".to_owned(),
    )
    .unwrap();
    let drift = PgRuntimeSessionRepository::new(h.birth.birth.worker.clone())
        .with_qualified_invocation_configuration(changed);
    let before = h.rows().await;
    assert!(drift.find(stored.id).await.is_err());
    assert!(drift.create(h.session()).await.is_err());
    assert_eq!(
        h.rows().await,
        before,
        "configuration does not recompute/promote the original producer"
    );
}
