//! Schema-valid contradictory metadata probes with every FK/trigger enabled.
//! Purged version metadata carries no ciphertext; no Secret dispatch is claimed.

use super::fixture::Fixture;
use sqlx::PgPool;
use uuid::Uuid;

pub struct SecretRows {
    pub secret: Uuid,
    pub version: Uuid,
    pub grant: Uuid,
    pub import: Uuid,
    pub binding: Uuid,
    pub rule: Uuid,
    pub session: Uuid,
    pub lease: Uuid,
}
impl SecretRows {
    pub async fn new(f: &Fixture) -> Self {
        let result = Self {
            secret: Uuid::new_v4(),
            version: Uuid::new_v4(),
            grant: Uuid::new_v4(),
            import: Uuid::new_v4(),
            binding: Uuid::new_v4(),
            rule: Uuid::new_v4(),
            session: Uuid::new_v4(),
            lease: Uuid::new_v4(),
        };
        assert_eq!(f.run.id, f.birth.start.run_id);
        assert_eq!(
            (f.run.instance_id, f.run.instance_revision_id),
            (
                f.birth.start.instance_id,
                f.birth.start.instance_revision_id
            )
        );
        let pool = &f.birth.pool;
        let graph = &f.birth.birth.fixture;
        sqlx::query("INSERT INTO secrets(id,owner_organization_id,organization_id,name,status,allowed_delivery_modes) VALUES($1,$2,$2,'empty-contradiction','purged',ARRAY['brokered'])")
            .bind(result.secret).bind(graph.organization).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO secret_versions(id,secret_id,sequence,status,algorithm,key_reference,content_length,purged_at) VALUES($1,$2,1,'purged','AES-256-GCM+AES-256-GCM-KW/v1','purged/metadata',1,now())")
            .bind(result.version).bind(result.secret).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO secret_grants(id,secret_id,owner_organization_id,target_kind,target_id,target_project_id,delivery_modes,phases,status) VALUES($1,$2,$3,'project',$4,$4,ARRAY['brokered'],ARRAY['normal'],'revoked')")
            .bind(result.grant).bind(result.secret).bind(graph.organization)
            .bind(graph.consuming_project).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO secret_imports(id,grant_id,secret_id,target_kind,target_id,alias,status) VALUES($1,$2,$3,'project',$4,'empty-contradiction','revoked')")
            .bind(result.import).bind(result.grant).bind(result.secret)
            .bind(graph.consuming_project).execute(pool).await.unwrap();
        result
    }
    pub async fn binding(&self, f: &Fixture) {
        sqlx::query("INSERT INTO agent_secret_bindings(id,instance_revision_id,import_id,slot_key,delivery_mode,phases,effective_policy,effective_policy_hash,status) VALUES($1,$2,$3,'contradiction','brokered',ARRAY['normal'],'{}',$4,'revoked')")
            .bind(self.binding).bind(f.run.instance_revision_id.as_uuid())
            .bind(self.import).bind([31_u8;32].as_slice())
            .execute(&f.birth.pool).await.unwrap();
    }
    pub async fn rule(&self, f: &Fixture) {
        sqlx::query("INSERT INTO brokered_secret_rules(id,binding_id,instance_revision_id,secret_version_id,direction,destination_origin,location_kind,header_name,normalized_hash) VALUES($1,$2,$3,$4,'outbound','https://example.test','outbound_header_value','authorization',$5)")
            .bind(self.rule).bind(self.binding).bind(f.run.instance_revision_id.as_uuid())
            .bind(self.version).bind([32_u8;32].as_slice())
            .execute(&f.birth.pool).await.unwrap();
    }
    pub async fn instance_provenance(f: &Fixture) {
        sqlx::query("INSERT INTO run_instance_provenance(run_id,instance_id,instance_revision_id,release_id,release_agent_id,parameter_hash,platform_policy_version,phase,authorization_model_version) SELECT id,instance_id,instance_revision_id,release_id,release_agent_id,(SELECT parameter_hash FROM agent_instance_revisions WHERE id=runs.instance_revision_id),'empty-contradiction','update',$2 FROM runs WHERE id=$1")
            .bind(f.run.id.as_uuid()).bind(authz_postgres::AUTHORIZATION_MODEL_VERSION)
            .execute(&f.birth.pool).await.unwrap();
    }
    pub async fn provenance(&self, f: &Fixture) {
        sqlx::query("INSERT INTO run_secret_provenance(run_id,binding_id,secret_id,secret_version_id,grant_id,import_id,authorization_model_version,delivery_policy_hash) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(f.run.id.as_uuid()).bind(self.binding).bind(self.secret)
            .bind(self.version).bind(self.grant).bind(self.import)
            .bind(authz_postgres::AUTHORIZATION_MODEL_VERSION).bind([31_u8;32].as_slice())
            .execute(&f.birth.pool).await.unwrap();
    }
    pub async fn session(&self, f: &Fixture) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO secret_runtime_sessions(id,run_id,instance_id,instance_revision_id,phase,runtime_credential_hash,status,expires_at) VALUES($1,$2,$3,$4,'update',$5,'revoked',now()+interval '1 hour')")
            .bind(self.session).bind(f.run.id.as_uuid())
            .bind(f.run.instance_id.as_uuid()).bind(f.run.instance_revision_id.as_uuid())
            .bind([33_u8;32].as_slice()).execute(&f.birth.pool).await?;
        Ok(())
    }
    pub async fn lease_and_snapshot(&self, f: &Fixture) {
        let pool = &f.birth.pool;
        sqlx::query("INSERT INTO secret_leases(id,session_id,run_id,binding_id,secret_version_id,delivery_mode,slot_key,status,expires_at) VALUES($1,$2,$3,$4,$5,'brokered','contradiction','active',now()+interval '1 hour')")
            .bind(self.lease).bind(self.session).bind(f.run.id.as_uuid())
            .bind(self.binding).bind(self.version).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO brokered_secret_lease_snapshots(id,lease_id,runtime_session_id,run_id,binding_id,secret_version_id,rule_id,destination_origin,location_kind,header_name,rule_hash) VALUES($1,$2,$3,$4,$5,$6,$7,'https://example.test','outbound_header_value','authorization',$8)")
            .bind(Uuid::new_v4()).bind(self.lease).bind(self.session)
            .bind(f.run.id.as_uuid()).bind(self.binding).bind(self.version)
            .bind(self.rule).bind([32_u8;32].as_slice()).execute(pool).await.unwrap();
        sqlx::query("UPDATE secret_leases SET status='released',released_at=now() WHERE id=$1")
            .bind(self.lease)
            .execute(pool)
            .await
            .unwrap();
    }
}

pub async fn count(pool: &PgPool, run: Uuid) -> (i64, i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM run_secret_provenance WHERE run_id=$1),(SELECT count(*) FROM secret_runtime_sessions WHERE run_id=$1),(SELECT count(*) FROM secret_leases WHERE run_id=$1),(SELECT count(*) FROM brokered_secret_lease_snapshots WHERE run_id=$1)")
        .bind(run).fetch_one(pool).await.unwrap()
}
