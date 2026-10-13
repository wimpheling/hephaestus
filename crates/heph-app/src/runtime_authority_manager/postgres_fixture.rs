use std::borrow::Cow;

use heph_run::{Run, RunKind, RunState};
use sqlx::{
    PgPool,
    migrate::{Migration, MigrationType, Migrator},
};
use tempfile::TempDir;
use uuid::Uuid;

use super::super::PgRunAuthorityManager;
use super::{seed, support};

static PUBLIC: Migrator = sqlx::migrate!("../../migrations");

fn checkpoint(version: i64) -> Migrator {
    let mut migrations = PUBLIC
        .iter()
        .filter(|m| m.version <= version.min(107))
        .cloned()
        .collect::<Vec<_>>();
    if version == 108 {
        assert_eq!(
            std::env::var("HEPHAESTUS_PRIVATE_RECIPE_MIGRATIONS").as_deref(),
            Ok("1")
        );
        let sql = [
            "/var/tmp/0108_run_volume_leases.profile.sql",
            "/var/tmp/0108_run_cleanup_fragment.profile.sql",
        ]
        .map(|path| std::fs::read_to_string(path).unwrap())
        .join("\n");
        migrations.push(Migration::new(
            108,
            Cow::Borrowed("private_profile_foundation"),
            MigrationType::Simple,
            Cow::Owned(sql),
            false,
        ));
    }
    Migrator {
        migrations: Cow::Owned(migrations),
        ..Migrator::DEFAULT
    }
}

pub struct Fixture {
    pub pool: PgPool,
    pub worker: PgPool,
    pub actor: support::Fixture,
    pub root: TempDir,
    pub manager: PgRunAuthorityManager,
    pub run: Run,
}

impl Fixture {
    pub async fn new(pool: PgPool) -> Self {
        checkpoint(100).run(&pool).await.unwrap();
        let actor = seed::seed(&pool).await;
        sqlx::query("UPDATE agent_instances SET state_volume_id=$2 WHERE id=$1")
            .bind(actor.instance)
            .bind(actor.volume)
            .execute(&pool)
            .await
            .unwrap();
        checkpoint(107).run(&pool).await.unwrap();
        checkpoint(108).run(&pool).await.unwrap();
        let worker = sqlx::postgres::PgPoolOptions::new()
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
        let root = tempfile::tempdir().unwrap();
        let manager = manager(&worker, &root, true);
        let run = fresh_run(&pool, &actor).await;
        Self {
            pool,
            worker,
            actor,
            root,
            manager,
            run,
        }
    }

    pub async fn advance(&mut self, state: RunState) {
        let label = serde_json::to_value(state).unwrap();
        sqlx::query("UPDATE runs SET state=$2 WHERE id=$1")
            .bind(self.run.id.as_uuid())
            .bind(label.as_str().unwrap())
            .execute(&self.worker)
            .await
            .unwrap();
        self.run.state = state;
    }

    pub async fn evidence(&self) -> serde_json::Value {
        sqlx::query_scalar("SELECT jsonb_build_object('session',(SELECT to_jsonb(s) FROM runtime_authority_sessions s WHERE s.run_id=$1),'snapshot',(SELECT to_jsonb(s) FROM run_authorization_snapshots s WHERE s.run_id=$1))")
            .bind(self.run.id.as_uuid()).fetch_one(&self.pool).await.unwrap()
    }

    pub fn envelopes(&self) -> Vec<(String, Vec<u8>)> {
        let mut entries = std::fs::read_dir(self.root.path())
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().into_string().unwrap(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }
}

pub fn manager(worker: &PgPool, root: &TempDir, canonical: bool) -> PgRunAuthorityManager {
    let manager = PgRunAuthorityManager::new(
        worker.clone(),
        runtime_git_authority_postgres::PgRuntimeGitCredentialRepository::new(worker.clone()),
        root.path().into(),
        [31; 32],
        std::time::Duration::from_secs(600),
    )
    .unwrap();
    if canonical {
        manager.with_canonical_live_authority()
    } else {
        manager
    }
}

async fn fresh_run(pool: &PgPool, actor: &support::Fixture) -> Run {
    let now = time::OffsetDateTime::now_utc();
    let revision: Uuid =
        sqlx::query_scalar("SELECT active_revision_id FROM agent_instances WHERE id=$1")
            .bind(actor.instance)
            .fetch_one(pool)
            .await
            .unwrap();
    let run = Run {
        id: Uuid::new_v4().into(),
        instance_id: actor.instance.into(),
        instance_revision_id: revision.into(),
        release_id: actor.release.into(),
        release_agent_id: actor.release_agent.into(),
        attachment_id: Some(actor.attachment.into()),
        kind: RunKind::Normal,
        command_id: Uuid::new_v4().into(),
        requires_state: true,
        volume_id: None,
        lease_id: None,
        lease_fencing_token: None,
        vm_id: None,
        state: RunState::LeasingVolume,
        outcome: None,
        exit: None,
        failure: None,
        cancel_requested_at: None,
        created_at: now,
        updated_at: now,
        state_version: 1,
    };
    let payload = serde_json::json!({"run_id":run.id,"command_id":run.command_id,
        "instance_id":run.instance_id,"instance_revision_id":run.instance_revision_id,
        "release_id":run.release_id,"release_agent_id":run.release_agent_id,
        "attachment_id":run.attachment_id,"kind":"normal","requires_state":true});
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO command_inbox(command_id,command_type,payload) VALUES($1,'start_run',$2)",
    )
    .bind(run.command_id.as_uuid())
    .bind(payload)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO run_requests(id,repository_id,commit_sha,git_ref,run_id,command_id,trigger_command_id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,platform_policy_version,request_kind,requires_state,actor_id,request_id) SELECT gen_random_uuid(),repository_id,$1,'refs/heads/main',$2,$3,$3,$4,$5,$6,$7,$8,'fixture/v1','instance_normal',true,$9,$10 FROM agent_attachments WHERE id=$8")
        .bind("a".repeat(40)).bind(run.id.as_uuid()).bind(run.command_id.as_uuid())
        .bind(actor.instance).bind(revision).bind(actor.release).bind(actor.release_agent)
        .bind(actor.attachment).bind(actor.maintainer.as_uuid()).bind(Uuid::new_v4())
        .execute(&mut *tx).await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO runs(id,command_id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,run_kind,requires_state,state,created_at,updated_at,execution_profile,vm_id,vm_provider_namespace,vm_host_id,vm_binding_kind) VALUES($1,$2,$3,$4,$5,$6,$7,'normal',true,'queued',now(),now(),'owned_canonical',$8,'manager-fixture-provider','manager-fixture-host','planned')")
        .bind(run.id.as_uuid()).bind(run.command_id.as_uuid()).bind(actor.instance)
        .bind(revision).bind(actor.release).bind(actor.release_agent).bind(actor.attachment)
        .bind(run.id.to_string()).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE runs SET state='leasing_volume' WHERE id=$1")
        .bind(run.id.as_uuid())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    run
}
