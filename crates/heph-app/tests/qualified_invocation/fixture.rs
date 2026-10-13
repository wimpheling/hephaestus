//! Genuine producer, saved rights, canonical full held sets and encrypted handoff.
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};

use heph_run::{Run, RunAuthorityManager, RunRepository, RunState};
use hephaestus_app::{CanonicalRunLaunchAuthorizer, PgRunAuthorityManager};
use run_orchestrator::RunLaunchAuthorizer;
use runtime_authority::{RuntimeSessionRepository, StoredRuntimeSession};
use runtime_authority_postgres::PgRuntimeSessionRepository;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tempfile::TempDir;
use volume_local::LocalVolumeStore;
use volume_trait::{RunVolumeMetadataRepository, RunVolumeStore};

use super::composition;

pub struct Fixture {
    pub birth: crate::fixture::Fixture,
    pub volumes: Arc<LocalVolumeStore>,
    pub repository: PgRuntimeSessionRepository,
    pub handoff: TempDir,
}
impl Fixture {
    pub async fn leasing(pool: &PgPool, count: usize) -> Self {
        crate::authority_schema::verify_inputs();
        let birth = Box::pin(crate::fixture::Fixture::new(pool, count)).await;
        crate::authority_schema::apply(pool).await;
        birth.consume().await.unwrap();
        let run = birth
            .repository()
            .transition(birth.start.run_id, RunState::LeasingVolume, None, None)
            .await
            .unwrap();
        assert_eq!(run.kind, heph_run::RunKind::Invocation);
        assert_eq!(run.vm_id.as_deref(), Some(birth.vm.0.as_str()));
        let volumes = composition::volumes(&birth);
        let repository = PgRuntimeSessionRepository::new(birth.birth.worker.clone())
            .with_canonical_live_snapshots()
            .with_qualified_invocation_configuration(birth.config.clone());
        Self {
            birth,
            volumes,
            repository,
            handoff: TempDir::new().unwrap(),
        }
    }
    pub async fn provisioning(pool: &PgPool, count: usize) -> Self {
        let f = Box::pin(Self::leasing(pool, count)).await;
        f.acquire().await;
        f
    }
    pub async fn acquire(&self) {
        self.acquire_for(time::Duration::hours(2)).await;
    }
    pub async fn acquire_for(&self, ttl: time::Duration) {
        self.hold_for(ttl).await;
        self.provision().await;
    }
    pub async fn hold_for(&self, ttl: time::Duration) {
        let selections = self
            .volumes
            .load_run_selections(self.birth.start.run_id)
            .await
            .unwrap();
        self.volumes.preflight_run(&selections).await.unwrap();
        let now = pg_time();
        let held = self
            .birth
            .volumes()
            .acquire_run(&selections, self.birth.scope.host_id(), now, now + ttl)
            .await
            .unwrap();
        assert_eq!(held.len(), selections.selections().len());
        for (lease, selected) in held.iter().zip(selections.selections()) {
            assert_eq!(lease.selection(), Some(selected));
            assert_eq!(lease.lease().run_id, self.birth.start.run_id);
        }
        assert_eq!(
            self.volumes
                .leases_for_run(self.birth.start.run_id)
                .await
                .unwrap(),
            held
        );
    }
    pub async fn provision(&self) {
        let held = self
            .volumes
            .leases_for_run(self.birth.start.run_id)
            .await
            .unwrap();
        self.birth
            .repository()
            .bind_resources(self.birth.start.run_id, None, None, None, &self.birth.vm.0)
            .await
            .unwrap();
        let current = self
            .birth
            .repository()
            .transition(self.birth.start.run_id, RunState::Provisioning, None, None)
            .await
            .unwrap();
        assert_eq!(current, self.run().await);
        assert_eq!(self.volumes.leases_for_run(current.id).await.unwrap(), held);
    }
    pub async fn run(&self) -> Run {
        self.birth
            .repository()
            .get(self.birth.start.run_id)
            .await
            .unwrap()
    }
    pub fn manager(&self) -> PgRunAuthorityManager {
        self.manager_ttl(Duration::from_secs(7200))
    }
    pub fn manager_ttl(&self, ttl: Duration) -> PgRunAuthorityManager {
        composition::manager(
            self,
            self.birth.birth.worker.clone(),
            self.birth.config.clone(),
            ttl,
        )
    }
    pub fn launcher(&self) -> CanonicalRunLaunchAuthorizer {
        composition::launcher(
            self,
            self.birth.birth.worker.clone(),
            self.birth.config.clone(),
        )
    }
    pub async fn session(&self) -> StoredRuntimeSession {
        self.repository
            .find(capability_domain::RuntimeSessionId::from_uuid(
                self.birth.start.run_id.as_uuid(),
            ))
            .await
            .unwrap()
            .unwrap()
    }
    pub async fn issue(&self) -> heph_runtime::RuntimeAuthorityBootstrap {
        self.manager()
            .prepare(&self.run().await)
            .await
            .unwrap()
            .bootstrap
            .unwrap()
    }
    pub async fn attach(&self) {
        // Actual trusted worker metadata marker; no VM mount/start is claimed.
        let selections = self
            .volumes
            .load_run_selections(self.birth.start.run_id)
            .await
            .unwrap();
        let attached = self.volumes.mark_run_attached(&selections).await.unwrap();
        assert_eq!(attached.len(), selections.selections().len());
        for (lease, selected) in attached.iter().zip(selections.selections()) {
            assert_eq!(lease.selection(), Some(selected));
        }
    }
    pub async fn rows(&self) -> serde_json::Value {
        let graph: serde_json::Value = sqlx::query_scalar("SELECT jsonb_build_object('snapshots',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY id),'[]') FROM run_authorization_snapshots s),'bindings',(SELECT COALESCE(jsonb_agg(to_jsonb(b) ORDER BY snapshot_id,ordinal),'[]') FROM run_authorization_snapshot_bindings b),'sessions',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY id),'[]') FROM runtime_authority_sessions s),'git',(SELECT COALESCE(jsonb_agg(to_jsonb(g) ORDER BY snapshot_id),'[]') FROM run_git_authority_snapshots g),'credentials',(SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY runtime_session_id),'[]') FROM runtime_git_credentials c))")
            .fetch_one(&self.birth.pool).await.unwrap();
        serde_json::json!({"runtime":graph,"prior":self.birth.rows().await})
    }
    pub fn handoff_files(&self) -> BTreeMap<String, String> {
        fn visit(root: &Path, path: &Path, files: &mut BTreeMap<String, String>) {
            for entry in std::fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(root, &path, files);
                } else {
                    files.insert(
                        path.strip_prefix(root)
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .to_owned(),
                        format!("{:x}", Sha256::digest(std::fs::read(path).unwrap())),
                    );
                }
            }
        }
        let mut files = BTreeMap::new();
        visit(self.handoff.path(), self.handoff.path(), &mut files);
        files
    }
    pub async fn denied_unchanged(&self, supplied: &Run) {
        let before = self.rows().await;
        let handoff = self.handoff_files();
        let manager = self.manager();
        assert!(manager.prepare(supplied).await.is_err());
        assert!(manager.reauthorize(supplied).await.is_err());
        assert!(
            manager
                .acknowledge(supplied, supplied.id.as_uuid(), 1)
                .await
                .is_err()
        );
        let launcher = self.launcher();
        assert!(launcher.authorize(supplied).await.is_err());
        assert!(launcher.authorize_with_volumes(supplied).await.is_err());
        assert_eq!(self.rows().await, before);
        assert_eq!(self.handoff_files(), handoff);
        self.birth.physical.assert_unchanged();
    }
}
pub fn pg_time() -> time::OffsetDateTime {
    let now = time::OffsetDateTime::now_utc().unix_timestamp_nanos();
    time::OffsetDateTime::from_unix_timestamp_nanos(now.div_euclid(1000) * 1000).unwrap()
}
