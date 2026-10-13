//! Fresh public107 ordinary relationships; no private schema or admission claim.
#[path = "mount_events.rs"]
mod mount_events;

use heph_secret::EphemeralSecretConfig;
use release_domain::{NetworkAccess, RuntimePolicy};
use release_service::InstanceExecutionConfiguration;
use run_domain::RunKind;
use runtime_types::RunId;
use secret_postgres::PostgresSecretMountMetadata;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    collections::BTreeMap,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use uuid::Uuid;
use vm_trait::VmProviderOwnerScope;
use volume_domain::VolumeRootNamespaceId;

pub type Files = BTreeMap<PathBuf, (u64, u64, u32, Option<String>)>;
pub struct Fixture {
    pub pool: PgPool,
    pub worker: PgPool,
    pub run: RunId,
    pub kind: &'static str,
    pub root: TempDir,
    configuration: Option<InstanceExecutionConfiguration>,
}
impl Fixture {
    pub async fn new(pool: &PgPool, kind: RunKind, configured: bool) -> Self {
        let mut migrations = sqlx::migrate!("../../migrations");
        migrations
            .migrations
            .to_mut()
            .retain(|migration| migration.version <= 107);
        assert_eq!(migrations.iter().count(), 107);
        migrations.run(pool).await.unwrap();
        let source = crate::support::seed(pool).await;
        let (instance, revision, _, normal) = crate::support::seed_instance(pool, &source).await;
        let (run, kind) = match kind {
            RunKind::Normal => (normal, "normal"),
            RunKind::Update => {
                let run = RunId::new();
                sqlx::query("INSERT INTO runs (id,instance_id,instance_revision_id,release_id,release_agent_id,run_kind,command_id,state,requires_state,created_at,updated_at) SELECT $1,instance_id,instance_revision_id,release_id,release_agent_id,'update',$2,'queued',false,now(),now() FROM runs WHERE id=$3 AND instance_id=$4 AND instance_revision_id=$5")
                    .bind(run.as_uuid()).bind(Uuid::new_v4()).bind(normal.as_uuid())
                    .bind(instance.as_uuid()).bind(revision.as_uuid())
                    .execute(pool).await.unwrap();
                (run, "update")
            }
            RunKind::Invocation => panic!("ordinary fixture does not construct Invocation"),
        };
        let shape: (String, Option<Uuid>, bool) =
            sqlx::query_as("SELECT run_kind,attachment_id,requires_state FROM runs WHERE id=$1")
                .bind(run.as_uuid())
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(shape.0, kind);
        assert_eq!(shape.1.is_some(), kind == "normal");
        assert_eq!(shape.2, kind == "normal");
        let worker = PgPoolOptions::new()
            .max_connections(2)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET ROLE hephaestus_worker")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect_with(pool.connect_options().as_ref().clone())
            .await
            .unwrap();
        for (pool, role) in [(pool, "postgres"), (&worker, "hephaestus_worker")] {
            let actual: String = sqlx::query_scalar("SELECT current_user::text")
                .fetch_one(pool)
                .await
                .unwrap();
            assert_eq!(actual, role);
        }
        let owner_db: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(pool)
            .await
            .unwrap();
        let worker_db: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&worker)
            .await
            .unwrap();
        assert_eq!(owner_db, worker_db);
        let root = tempfile::Builder::new()
            .prefix("heph-ordinary-cleanup-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in("/dev/shm")
            .expect("private tmpfs test root is required");
        assert_eq!(
            root.path().metadata().unwrap().permissions().mode() & 0o777,
            0o700
        );
        let configuration = configured.then(|| {
            InstanceExecutionConfiguration::new(
                VmProviderOwnerScope::new(
                    "ordinary-cleanup-test".into(),
                    "ordinary-cleanup-host".into(),
                )
                .unwrap(),
                root.path().to_str().unwrap().to_owned(),
                VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).unwrap(),
                RuntimePolicy {
                    vcpus: 1,
                    memory_mib: 128,
                    network: NetworkAccess::Disabled,
                },
                "ordinary-cleanup/platform-v1".into(),
            )
            .unwrap()
        });
        let f = Self {
            pool: pool.clone(),
            worker,
            run,
            kind,
            root,
            configuration,
        };
        f.verify_public().await;
        f
    }
    pub fn metadata(&self, pool: PgPool) -> PostgresSecretMountMetadata {
        let metadata = PostgresSecretMountMetadata::new(pool);
        match &self.configuration {
            Some(config) => metadata.with_qualified_invocation_configuration(config.clone()),
            None => metadata,
        }
    }
    pub fn config(&self) -> EphemeralSecretConfig {
        EphemeralSecretConfig {
            root: self.root.path().into(),
            require_memory_filesystem: true,
        }
    }
    pub async fn verify_public(&self) {
        let stored: Vec<(i64, Vec<u8>)> =
            sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
                .fetch_all(&self.pool)
                .await
                .unwrap();
        let public = sqlx::migrate!("../../migrations");
        let expected = public
            .iter()
            .filter(|m| m.version <= 107)
            .map(|m| (m.version, m.checksum.to_vec()))
            .collect::<Vec<_>>();
        assert_eq!(expected.len(), 107);
        assert_eq!(
            expected.iter().map(|m| m.0).collect::<Vec<_>>(),
            (1..=107).collect::<Vec<_>>()
        );
        assert_eq!(stored, expected);
        let absent: bool = sqlx::query_scalar(
            "SELECT to_regprocedure('public.verify_invocation119_empty_secret_cleanup(uuid,bytea,text,text)') IS NULL")
            .fetch_one(&self.pool).await.unwrap();
        assert!(
            absent,
            "ordinary configured cleanup must not need private functions"
        );
    }
    pub async fn rows(&self) -> Value {
        sqlx::query_scalar("SELECT jsonb_build_object(
          'users',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM users t),
          'organizations',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM organizations t),
          'organization_members',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM organization_members t),
          'organization_secret_managers',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM organization_secret_managers t),
          'projects',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM projects t),
          'project_maintainers',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM project_maintainers t),
          'project_secret_roles',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM project_secret_roles t),
          'repositories',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM repositories t),
          'build_requests',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM build_requests t),
          'agent_families',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM agent_families t),
          'releases',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM releases t),
          'release_agents',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM release_agents t),
          'agent_instances',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM agent_instances t),
          'agent_instance_revisions',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM agent_instance_revisions t),
          'agent_attachments',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM agent_attachments t),
          'runs',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM runs t),
          'agent_updates',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM agent_updates t),
          'run_requests',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM run_requests t),
          'run_instance_provenance',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM run_instance_provenance t),
          'run_secret_provenance',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM run_secret_provenance t),
          'secret_runtime_sessions',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM secret_runtime_sessions t),
          'secret_runtime_mounts',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM secret_runtime_mounts t),
          'secret_leases',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM secret_leases t),
          'agent_secret_bindings',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM agent_secret_bindings t),
          'secrets',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM secrets t),
          'secret_versions',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM secret_versions t),
          'secret_grants',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM secret_grants t),
          'secret_imports',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM secret_imports t),
          'brokered_secret_rules',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM brokered_secret_rules t),
          'brokered_secret_lease_snapshots',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM brokered_secret_lease_snapshots t),
          'secret_audit_events',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM secret_audit_events t),
          'brokered_secret_audit_events',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM brokered_secret_audit_events t),
          'run_events',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM run_events t),
          'command_inbox',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM command_inbox t),
          'outbox',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM outbox t),
          'application_events',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM application_events t),
          'product_event_outbox',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]') FROM product_event_outbox t))")
            .fetch_one(&self.pool).await.unwrap()
    }
    pub fn files(&self) -> Files {
        fn visit(root: &Path, path: &Path, result: &mut Files) {
            let metadata = std::fs::symlink_metadata(path).unwrap();
            assert!(!metadata.file_type().is_symlink());
            let hash = if metadata.is_file() {
                Some(format!(
                    "{:x}",
                    Sha256::digest(std::fs::read(path).unwrap())
                ))
            } else {
                assert!(metadata.is_dir());
                None
            };
            result.insert(
                path.strip_prefix(root).unwrap().into(),
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.permissions().mode(),
                    hash,
                ),
            );
            if metadata.is_dir() {
                for entry in std::fs::read_dir(path).unwrap() {
                    visit(root, &entry.unwrap().path(), result);
                }
            }
        }
        let mut result = BTreeMap::new();
        visit(self.root.path(), self.root.path(), &mut result);
        result
    }
    pub fn assert_created(&self, before: &Value, after: &Value, directory: Uuid) {
        assert!(
            before["secret_runtime_mounts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let rows = after["secret_runtime_mounts"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].as_object().unwrap().len(), 5);
        assert_eq!(rows[0]["run_id"], json!(self.run));
        assert_eq!(rows[0]["opaque_directory"], json!(directory));
        assert_eq!(rows[0]["state"], "materialized");
        assert!(rows[0]["materialized_at"].is_string());
        assert!(rows[0]["destroyed_at"].is_null());
        let mut expected = before.clone();
        expected["secret_runtime_mounts"] = after["secret_runtime_mounts"].clone();
        mount_events::apply(
            before,
            after,
            &mut expected,
            "created",
            "active",
            &rows[0]["materialized_at"],
        );
        assert_eq!(
            *after, expected,
            "only mount persistence and its exact committed event change rows"
        );
    }
    pub fn assert_destroyed(before: &Value, after: &Value) {
        assert!(after["secret_runtime_mounts"][0]["destroyed_at"].is_string());
        let mut expected = before.clone();
        expected["secret_runtime_mounts"][0]["state"] = json!("destroyed");
        expected["secret_runtime_mounts"][0]["destroyed_at"] =
            after["secret_runtime_mounts"][0]["destroyed_at"].clone();
        mount_events::apply(
            before,
            after,
            &mut expected,
            "state_changed",
            "removed",
            &after["secret_runtime_mounts"][0]["destroyed_at"],
        );
        assert_eq!(
            *after, expected,
            "only the original mount and its exact committed event become destroyed"
        );
    }
}
