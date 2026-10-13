use std::borrow::Cow;

use sqlx::{
    PgPool, Postgres, Transaction,
    migrate::{Migration, MigrationType, Migrator},
};
use uuid::Uuid;

use crate::support::Fixture;

const PUBLIC: Migrator = sqlx::migrate!("../../../../migrations");

pub fn checkpoint(version: i64) -> Migrator {
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
        .into_iter()
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect::<Vec<_>>()
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

pub async fn baseline(pool: &PgPool) -> Fixture {
    checkpoint(100).run(pool).await.unwrap();
    let f = crate::seed::seed(pool).await;
    sqlx::query("UPDATE agent_instances SET state_volume_id=$2 WHERE id=$1")
        .bind(f.instance)
        .bind(f.volume)
        .execute(pool)
        .await
        .unwrap();
    checkpoint(107).run(pool).await.unwrap();
    f
}

pub async fn setup(pool: &PgPool) -> Fixture {
    let f = baseline(pool).await;
    checkpoint(108).run(pool).await.unwrap();
    f
}

#[derive(Clone)]
pub struct Execution {
    pub run: Uuid,
    pub command: Uuid,
    pub instance: Uuid,
    pub revision: Uuid,
    pub agent: Uuid,
    pub release: Uuid,
    pub attachment: Option<Uuid>,
    pub requires_state: bool,
    pub kind: &'static str,
}

impl Execution {
    pub async fn normal(pool: &PgPool, f: &Fixture) -> Self {
        Self {
            run: Uuid::new_v4(),
            command: Uuid::new_v4(),
            instance: f.instance,
            revision: sqlx::query_scalar(
                "SELECT active_revision_id FROM agent_instances WHERE id=$1",
            )
            .bind(f.instance)
            .fetch_one(pool)
            .await
            .unwrap(),
            agent: f.release_agent,
            release: f.release,
            attachment: Some(f.attachment),
            requires_state: true,
            kind: "normal",
        }
    }

    pub fn payload(&self) -> serde_json::Value {
        serde_json::json!({"run_id":self.run,"command_id":self.command,"instance_id":self.instance,"instance_revision_id":self.revision,"release_id":self.release,"release_agent_id":self.agent,"attachment_id":self.attachment,"kind":self.kind,"requires_state":self.requires_state})
    }
}

pub async fn worker(pool: &PgPool) -> Transaction<'_, Postgres> {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx
}

pub async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    e: &Execution,
    profile: &str,
    scope: bool,
    admission: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO runs(id,command_id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,run_kind,requires_state,state,created_at,updated_at,execution_profile,vm_id,vm_provider_namespace,vm_host_id,vm_binding_kind,vm_plan_admission_required) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'queued',now(),now(),$10,$11,$12,$13,$14,$15)")
        .bind(e.run).bind(e.command).bind(e.instance).bind(e.revision).bind(e.release).bind(e.agent).bind(e.attachment).bind(e.kind).bind(e.requires_state).bind(profile)
        .bind(scope.then(||e.run.to_string())).bind(scope.then_some("profile-provider")).bind(scope.then_some("host")).bind(scope.then_some("planned")).bind(admission)
        .execute(&mut **tx).await.map(|_|())
}

// Worker role is established on every checkout; no actor-role substitution.
pub async fn repository(pool: &PgPool) -> runtime_authority_postgres::PgRuntimeSessionRepository {
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
    runtime_authority_postgres::PgRuntimeSessionRepository::new(worker)
        .with_canonical_live_snapshots()
}

pub async fn normal(pool: &PgPool, f: &Fixture, profile: &str) -> Execution {
    let e = Execution::normal(pool, f).await;
    let mut tx = pool.begin().await.unwrap();
    super::producers::git(&mut tx, &e).await.unwrap();
    // The fixture records an actual durable principal on the Git request. It
    // does not prove an authenticated HTTP request or create Invoke admission.
    sqlx::query("UPDATE run_requests SET actor_id=$2,request_id=$3 WHERE run_id=$1")
        .bind(e.run)
        .bind(f.maintainer.as_uuid())
        .bind(Uuid::new_v4())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    insert(&mut tx, &e, profile, profile == "owned_canonical", false)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    advance(pool, e.run, "leasing_volume").await;
    e
}

pub async fn advance(pool: &PgPool, run: Uuid, state: &str) {
    let mut tx = worker(pool).await;
    sqlx::query("UPDATE runs SET state=$2 WHERE id=$1")
        .bind(run)
        .bind(state)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

pub fn pins(e: &Execution) -> runtime_authority::LiveRunSnapshotPins {
    runtime_authority::LiveRunSnapshotPins::new(
        e.run, e.instance, e.revision, e.release, e.agent, e.command,
    )
    .unwrap()
}

pub fn run(e: &Execution, state: run_domain::RunState) -> run_domain::Run {
    let now = time::OffsetDateTime::now_utc();
    run_domain::Run {
        id: e.run.into(),
        instance_id: e.instance.into(),
        instance_revision_id: e.revision.into(),
        release_id: e.release.into(),
        release_agent_id: e.agent.into(),
        attachment_id: e.attachment.map(Into::into),
        kind: if e.kind == "update" {
            run_domain::RunKind::Update
        } else {
            run_domain::RunKind::Normal
        },
        requires_state: e.requires_state,
        command_id: e.command.into(),
        volume_id: None,
        lease_id: None,
        lease_fencing_token: None,
        vm_id: Some(e.run.to_string()),
        state,
        outcome: None,
        exit: None,
        failure: None,
        cancel_requested_at: None,
        created_at: now,
        updated_at: now,
        state_version: 1,
    }
}

pub async fn denied(
    repository: &runtime_authority_postgres::PgRuntimeSessionRepository,
    request: &runtime_authority::LiveRunSnapshotRequest,
) {
    use runtime_authority::LiveRunSnapshotResolver;
    let error = repository
        .resolve_live_snapshot(request, "live-fixture/v1", time::OffsetDateTime::now_utc())
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            runtime_authority::RuntimeAuthorityError::Unsupported
                | runtime_authority::RuntimeAuthorityError::IdentityMismatch
                | runtime_authority::RuntimeAuthorityError::NotFound
        ),
        "expected authority denial, not malformed SQL or infrastructure failure: {error:?}"
    );
}

pub async fn close(pool: &PgPool, run: Uuid) {
    let mut tx = worker(pool).await;
    sqlx::query("UPDATE runs SET volume_acquisition_closed_at=COALESCE(volume_acquisition_closed_at,now()) WHERE id=$1")
        .bind(run)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}
