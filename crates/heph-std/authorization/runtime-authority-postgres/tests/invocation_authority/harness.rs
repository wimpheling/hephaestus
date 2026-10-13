use crate::fixture::Fixture;
use capability_domain::{
    AuthorizationSnapshot, RuntimeCredentialGeneration, RuntimeCredentialHash, RuntimeInvocation,
    RuntimeSessionId, RuntimeSessionIdentity,
};
use run_domain::RunState;
use run_orchestrator::RunRepository;
use runtime_authority::{NewRuntimeSession, RuntimeSessionRepository, StoredRuntimeSession};
use runtime_authority_postgres::PgRuntimeSessionRepository;
use sqlx::PgPool;
use time::OffsetDateTime;
use volume_trait::RunVolumeMetadataRepository;

pub struct Harness {
    pub birth: Fixture,
    pub repository: PgRuntimeSessionRepository,
    pub snapshot: AuthorizationSnapshot,
    pub identity: RuntimeSessionIdentity,
    pub credential: RuntimeCredentialHash,
}
impl Harness {
    pub async fn new(pool: &PgPool, count: usize) -> Self {
        let birth = Box::pin(Fixture::new(pool, count)).await;
        birth.consume().await.unwrap();
        birth
            .repository()
            .transition(birth.start.run_id, RunState::LeasingVolume, None, None)
            .await
            .unwrap();
        let selection = birth
            .volumes()
            .load_run_selections(birth.start.run_id)
            .await
            .unwrap();
        let now = pg_time();
        birth
            .volumes()
            .acquire_run(
                &selection,
                birth.scope.host_id(),
                now,
                now + time::Duration::hours(2),
            )
            .await
            .unwrap();
        birth
            .repository()
            .bind_resources(birth.start.run_id, None, None, None, &birth.vm.0)
            .await
            .unwrap();
        birth
            .repository()
            .transition(birth.start.run_id, RunState::Provisioning, None, None)
            .await
            .unwrap();
        sqlx::raw_sql(
            &std::fs::read_to_string("/var/tmp/0119_qualified_invocation_authority.sql").unwrap(),
        )
        .execute(pool)
        .await
        .unwrap();
        let repository = PgRuntimeSessionRepository::new(birth.birth.worker.clone())
            .with_qualified_invocation_configuration(birth.config.clone());
        let run = birth.repository().get(birth.start.run_id).await.unwrap();
        let model = "qualified119/Unicode-λ";
        let snapshot = repository.resolve_snapshot(&run, model).await.unwrap();
        let issued = pg_time();
        let identity = RuntimeSessionIdentity::new(
            RuntimeSessionId::from_uuid(run.id.as_uuid()),
            snapshot.principal(),
            RuntimeInvocation::Run(run.id),
            &snapshot,
            issued,
            issued + time::Duration::seconds(3600),
        )
        .unwrap();
        Self {
            birth,
            repository,
            snapshot,
            identity,
            credential: capability_domain::RuntimeCredential::from_secret([31; 32]).storage_hash(
                RuntimeSessionId::from_uuid(run.id.as_uuid()),
                RuntimeCredentialGeneration::INITIAL,
            ),
        }
    }
    pub const fn session(&self) -> NewRuntimeSession<'_> {
        NewRuntimeSession {
            snapshot: &self.snapshot,
            identity: &self.identity,
            generation: RuntimeCredentialGeneration::INITIAL,
            credential_hash: self.credential,
            attachment_id: None,
        }
    }
    pub async fn issue(&self) -> StoredRuntimeSession {
        self.repository.create(self.session()).await.unwrap()
    }
    pub async fn rows(&self) -> serde_json::Value {
        let graph:serde_json::Value=sqlx::query_scalar("SELECT jsonb_build_object('snapshots',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY id),'[]') FROM run_authorization_snapshots s),'bindings',(SELECT COALESCE(jsonb_agg(to_jsonb(b) ORDER BY snapshot_id,ordinal),'[]') FROM run_authorization_snapshot_bindings b),'sessions',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY id),'[]') FROM runtime_authority_sessions s),'git',(SELECT COALESCE(jsonb_agg(to_jsonb(g) ORDER BY snapshot_id),'[]') FROM run_git_authority_snapshots g),'credentials',(SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY runtime_session_id),'[]') FROM runtime_git_credentials c))")
   .fetch_one(&self.birth.pool).await.unwrap();
        serde_json::json!({"runtime":graph,"prior":self.birth.rows().await})
    }
    pub async fn attach_metadata(&self) {
        // This is the trusted worker's exact fence marker. It tests DB/session phase
        // guards only; no VM mount/start/guest execution is asserted by this fixture.
        let selections = self
            .birth
            .volumes()
            .load_run_selections(self.birth.start.run_id)
            .await
            .unwrap();
        let now = pg_time();
        self.birth
            .volumes()
            .mark_run_attached(
                &selections,
                self.birth.scope.host_id(),
                now,
                now + time::Duration::hours(2),
            )
            .await
            .unwrap();
    }
    pub async fn add_optional_bindings(&mut self) {
        use capability_domain::{
            CapabilityBinding, CapabilityBindingId, CapabilityOperation, CapabilityRequirement,
            CapabilityRequirementId, CapabilityResource, CapabilityResourceKind, CapabilitySlotKey,
        };
        let f = &self.birth.birth.fixture;
        let mut tx =
            authz_postgres::begin_actor_transaction(&self.birth.birth.app, &self.birth.birth.actor)
                .await
                .unwrap();
        sqlx::query("INSERT INTO project_capability_granters(project_id,user_id,created_by) VALUES($1,$2,$2)")
   .bind(f.consuming_project).bind(self.birth.birth.actor.user_id.as_uuid()).execute(&mut *tx).await.unwrap();
        let requirements:Vec<(uuid::Uuid,String)>=sqlx::query_as("SELECT id,slot_key FROM release_capability_requirements WHERE release_agent_id=$1 ORDER BY slot_key")
   .bind(self.birth.start.release_agent_id.as_uuid()).fetch_all(&mut *tx).await.unwrap();
        assert_eq!(requirements.len(), 2);
        for (id, slot) in requirements {
            let requirement = CapabilityRequirement::new(
                CapabilityRequirementId::from_uuid(id),
                CapabilitySlotKey::parse(slot).unwrap(),
                CapabilityResourceKind::Project,
                [],
                [CapabilityOperation::Inspect],
                false,
            )
            .unwrap();
            let binding = CapabilityBinding::bind(
                CapabilityBindingId::new(),
                &requirement,
                CapabilityResource::new(CapabilityResourceKind::Project, f.consuming_project),
                [CapabilityOperation::Inspect],
            )
            .unwrap();
            sqlx::query("INSERT INTO agent_capability_bindings(id,instance_revision_id,release_agent_id,requirement_id,requirement_hash,slot_key,resource_kind,resource_id,granted_operations,normalized_hash,authorization_model_version,created_by) VALUES($1,$2,$3,$4,$5,$6,'project',$7,ARRAY['inspect'],$8,$9,$10)")
    .bind(binding.id().as_uuid()).bind(self.birth.start.instance_revision_id.as_uuid())
    .bind(self.birth.start.release_agent_id.as_uuid()).bind(id).bind(requirement.normalized_hash().as_bytes().as_slice())
    .bind(binding.slot().as_str()).bind(f.consuming_project).bind(binding.normalized_hash().as_bytes().as_slice())
    .bind(self.snapshot.authorization_model_version()).bind(self.birth.birth.actor.user_id.as_uuid()).execute(&mut *tx).await.unwrap();
        }
        tx.commit().await.unwrap();
        let run = self
            .birth
            .repository()
            .get(self.birth.start.run_id)
            .await
            .unwrap();
        self.snapshot = self
            .repository
            .resolve_snapshot(&run, self.snapshot.authorization_model_version())
            .await
            .unwrap();
        self.identity = RuntimeSessionIdentity::new(
            self.identity.id(),
            self.snapshot.principal(),
            self.identity.invocation(),
            &self.snapshot,
            self.identity.issued_at(),
            self.identity.expires_at(),
        )
        .unwrap();
        assert_eq!(self.snapshot.bindings().len(), 2);
    }
    pub fn pins(&self) -> runtime_authority::LiveRunSnapshotPins {
        runtime_authority::LiveRunSnapshotPins::new(
            self.birth.start.run_id.as_uuid(),
            self.birth.start.instance_id.as_uuid(),
            self.birth.start.instance_revision_id.as_uuid(),
            self.birth.start.release_id.as_uuid(),
            self.birth.start.release_agent_id.as_uuid(),
            self.birth.start.command_id.as_uuid(),
        )
        .unwrap()
    }
    pub fn ceiling(
        &self,
        session: &StoredRuntimeSession,
    ) -> runtime_authority::LiveRunSnapshotRequest {
        runtime_authority::LiveRunSnapshotRequest::issued(
            self.pins(),
            runtime_authority::LiveRunSessionCeiling::new(session, self.snapshot.normalized_hash())
                .unwrap(),
        )
        .unwrap()
    }
    pub async fn raw_writer(
        &self,
        role_pool: &PgPool,
        identity: &RuntimeSessionIdentity,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT write_invocation119_session($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
            .bind(self.birth.start.run_id.as_uuid())
            .bind(serde_json::to_vec(&self.birth.config.canonical_json()).unwrap())
            .bind(self.snapshot.authorization_model_version())
            .bind(self.snapshot.normalized_hash().as_bytes().as_slice())
            .bind(identity.normalized_hash().as_bytes().as_slice())
            .bind(1_i64)
            .bind(self.credential.as_bytes().as_slice())
            .bind(identity.issued_at())
            .bind(identity.expires_at())
            .bind(identity.issued_at().unix_timestamp_nanos().to_string())
            .bind(identity.expires_at().unix_timestamp_nanos().to_string())
            .execute(role_pool)
            .await
            .map(|_| ())
    }
}
pub fn pg_time() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc();
    OffsetDateTime::from_unix_timestamp_nanos(now.unix_timestamp_nanos().div_euclid(1000) * 1000)
        .unwrap()
}
pub fn code(error: &sqlx::Error) -> Option<std::borrow::Cow<'_, str>> {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
}

pub fn assert_ceiling_unchanged(before: &serde_json::Value, after: &serde_json::Value) {
    for field in ["snapshots", "bindings", "git", "credentials"] {
        assert_eq!(
            after["runtime"][field], before["runtime"][field],
            "immutable {field} changed during irreversible lifecycle operation"
        );
    }
    for field in [
        "id",
        "snapshot_id",
        "run_id",
        "instance_id",
        "instance_revision_id",
        "attachment_id",
        "identity_hash",
        "snapshot_hash",
        "issuance_generation",
        "credential_hash",
        "issued_at",
        "expires_at",
    ] {
        assert_eq!(
            after["runtime"]["sessions"][0][field], before["runtime"]["sessions"][0][field],
            "immutable session {field} changed"
        );
    }
    assert_eq!(
        after["prior"], before["prior"],
        "revocation/expiry cannot release or reset held fences"
    );
}
