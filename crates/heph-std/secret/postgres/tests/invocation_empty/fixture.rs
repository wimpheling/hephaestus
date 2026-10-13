use crate::authority_harness::Harness;
use heph_secret::SecretMountMetadata;
use run_domain::Run;
use run_orchestrator::{RunRepository, RunSecretManager};
use secret_postgres::PostgresSecretMountMetadata;
use sqlx::PgPool;

pub struct Fixture {
    pub authority: Harness,
    pub metadata: PostgresSecretMountMetadata,
}
impl Fixture {
    pub async fn new(pool: &PgPool, count: usize) -> Self {
        let authority = Box::pin(Harness::new(pool, count)).await;
        sqlx::raw_sql(
            &std::fs::read_to_string("/var/tmp/0119_qualified_invocation_empty_secret.sql")
                .unwrap(),
        )
        .execute(pool)
        .await
        .unwrap();
        let metadata = PostgresSecretMountMetadata::new(authority.birth.birth.worker.clone())
            .with_qualified_invocation_configuration(authority.birth.config.clone());
        Self {
            authority,
            metadata,
        }
    }
    pub async fn run(&self) -> Run {
        self.authority
            .birth
            .repository()
            .get(self.authority.birth.start.run_id)
            .await
            .unwrap()
    }
    pub async fn rows(&self) -> serde_json::Value {
        let run = self.authority.birth.start.run_id.as_uuid();
        let revision = self.authority.birth.start.instance_revision_id.as_uuid();
        let secret:serde_json::Value=sqlx::query_scalar("SELECT jsonb_build_object('revision',(SELECT secret_bindings FROM agent_instance_revisions WHERE id=$2),'bindings',(SELECT COALESCE(jsonb_agg(to_jsonb(b) ORDER BY id),'[]') FROM agent_secret_bindings b WHERE instance_revision_id=$2),'provenance',(SELECT COALESCE(jsonb_agg(to_jsonb(p) ORDER BY binding_id),'[]') FROM run_secret_provenance p WHERE run_id=$1),'instance',(SELECT to_jsonb(p) FROM run_instance_provenance p WHERE run_id=$1),'sessions',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY id),'[]') FROM secret_runtime_sessions s WHERE run_id=$1),'leases',(SELECT COALESCE(jsonb_agg(to_jsonb(l) ORDER BY id),'[]') FROM secret_leases l WHERE run_id=$1),'mounts',(SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY run_id),'[]') FROM secret_runtime_mounts m WHERE run_id=$1),'rules',(SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY id),'[]') FROM brokered_secret_rules r WHERE instance_revision_id=$2),'brokered',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY id),'[]') FROM brokered_secret_lease_snapshots s WHERE run_id=$1))")
            .bind(run).bind(revision).fetch_one(&self.authority.birth.pool).await.unwrap();
        serde_json::json!({"authority":self.authority.rows().await,"secret":secret})
    }
    pub async fn allowed_unchanged(&self) {
        let before = self.rows().await;
        let manager = super::ports::manager(self.metadata.clone());
        let run = self.run().await;
        assert!(manager.prepare(&run).await.unwrap().mounts.is_empty());
        manager.reauthorize(&run).await.unwrap();
        assert_eq!(self.rows().await, before);
        self.authority.birth.physical.assert_unchanged();
    }
    pub async fn denied_unchanged(&self) {
        self.denied_run_unchanged(&self.run().await).await;
    }
    pub async fn denied_run_unchanged(&self, run: &Run) {
        let before = self.rows().await;
        let manager = super::ports::manager(self.metadata.clone());
        assert!(manager.prepare(run).await.is_err());
        assert!(manager.reauthorize(run).await.is_err());
        assert_eq!(self.rows().await, before);
        self.authority.birth.physical.assert_unchanged();
    }
    pub async fn raw(&self, pool: &PgPool, run: &Run) -> Result<bool, sqlx::Error> {
        self.raw_config(
            pool,
            run,
            serde_json::to_vec(&self.authority.birth.config.canonical_json()).unwrap(),
        )
        .await
    }
    pub async fn raw_config(
        &self,
        pool: &PgPool,
        run: &Run,
        config: Vec<u8>,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT verify_invocation119_empty_secrets($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(run.id.as_uuid())
        .bind(config)
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .bind(run.release_id.as_uuid())
        .bind(run.release_agent_id.as_uuid())
        .bind(run.command_id.as_uuid())
        .bind(run.requires_state)
        .bind(run.vm_id.as_deref())
        .bind("provisioning")
        .fetch_one(pool)
        .await
    }
    pub async fn verify(&self) -> bool {
        self.metadata
            .verify_qualified_invocation_empty_bindings(&self.run().await)
            .await
            .is_ok()
    }
}
