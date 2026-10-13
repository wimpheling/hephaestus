//! Target-local composition of genuine helper births, closure and owning delivery.
use crate::fixture::Fixture;
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentOperation,
    PreparedInstanceClosureRepository, PreparedInstanceClosureRequest,
    PreparedInstanceSafetyRepository, ResourceAction, ResourceEffect,
};
use release_postgres::PostgresPreparedInstanceSafetyRepository;
use run_domain::{CancelRun, RunState};
use run_orchestrator::{RunCleanupWakeOriginal, RunCleanupWakeRequest, RunRepository};
use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::RunVolumeMetadataRepository;

pub struct Wake {
    pub fixture: Fixture,
    pub closure: PreparedInstanceClosureRequest,
    pub request: RunCleanupWakeRequest,
    pub cancel: CancelRun,
}
impl Wake {
    pub async fn new(pool: &PgPool, count: usize) -> Self {
        Self::from_fixture(Fixture::new(pool, count).await, true, true).await
    }
    pub async fn from_fixture(fixture: Fixture, consumed: bool, delivered: bool) -> Self {
        let h = Self::historical(fixture, consumed, delivered).await;
        h.upgrade().await;
        h
    }
    pub async fn historical(fixture: Fixture, consumed: bool, delivered: bool) -> Self {
        if consumed {
            fixture.consume().await.unwrap();
            acquire(&fixture).await;
        }
        load_closure(&fixture.pool).await;
        let closure = make_closure(&fixture).await;
        let birth = &fixture.birth;
        let actor = &birth.actor;
        load_wake(&fixture.pool).await;
        let pending = check_open(&fixture, &closure).await;
        service(&fixture)
            .close_for_remove(actor, &closure)
            .await
            .unwrap();
        let request = committed_request(&fixture, &closure).await;
        assert_eq!(
            request, pending,
            "actual committed closure matches all pending comparison bytes"
        );
        let payload: serde_json::Value = sqlx::query_scalar("SELECT payload FROM outbox WHERE aggregate_id=$1 AND event_type='run.cancel_requested.v1'")
            .bind(fixture.start.run_id.as_uuid()).fetch_one(&fixture.pool).await.unwrap();
        let cancel: CancelRun = serde_json::from_value(payload).unwrap();
        assert_eq!(cancel.run_id, fixture.start.run_id);
        if delivered {
            // Pre117 remains a real delivered Cancel without fabricated consumption.
            let repository = if consumed {
                fixture.repository()
            } else {
                run_postgres::PgRunRepository::new(birth.worker.clone())
            };
            repository.request_cancel(&cancel).await.unwrap();
        }
        Self {
            fixture,
            closure,
            request,
            cancel,
        }
    }
    pub async fn upgrade(&self) {
        // Historical23 assertions precede this one canonical24-to28 upgrade.
        for path in [
            include_str!(
                "../../../../../../migrations/0132_qualified_invocation_remove_resume.sql"
            ),
            include_str!("../../../../../../migrations/0133_queued_invocation_cleanup.sql"),
            include_str!("../../../../../../migrations/0134_queued_invocation_cancel_delivery.sql"),
            include_str!("../../../../../../migrations/0135_queued_invocation_cleanup_context.sql"),
            include_str!(
                "../../../../../../migrations/0136_queued_invocation_cleanup_transaction.sql"
            ),
        ] {
            sqlx::raw_sql(path)
                .execute(&self.fixture.pool)
                .await
                .unwrap();
        }
        self.fixture
            .birth
            .assert_historical_migrations(&self.fixture.pool)
            .await;
    }
    pub fn repository(&self) -> run_postgres::PgRunRepository {
        self.fixture
            .repository()
            .with_qualified_cleanup_wake(self.fixture.scope.clone())
    }
    pub async fn prepare(&self) -> run_orchestrator::RunCleanupWakeSnapshot {
        self.repository()
            .prepare_cleanup_wake(&self.request, &self.fixture.scope)
            .await
            .unwrap()
    }
    pub async fn read(&self) -> run_orchestrator::RunCleanupWakeSnapshot {
        self.repository()
            .load_cleanup_wake(&self.request, &self.fixture.scope)
            .await
            .unwrap()
    }
    pub async fn withdraw(&self) {
        sqlx::query("UPDATE users SET status='disabled' WHERE id=$1")
            .bind(self.fixture.birth.actor.user_id.as_uuid())
            .execute(&self.fixture.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE releases SET state='revoked',revoked_at=now() WHERE id=$1")
            .bind(self.fixture.birth.fixture.release)
            .execute(&self.fixture.pool)
            .await
            .unwrap();
    }
}
pub fn service(f: &Fixture) -> PostgresPreparedInstanceSafetyRepository {
    PostgresPreparedInstanceSafetyRepository::new(
        f.birth.app.clone(),
        f.birth.worker.clone(),
        f.config.provider_scope(),
        f.birth.input.source().platform().clone(),
    )
}
pub async fn acquire(f: &Fixture) {
    f.repository()
        .transition(f.start.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let selected = f
        .volumes()
        .load_run_selections(f.start.run_id)
        .await
        .unwrap();
    let now = time::OffsetDateTime::now_utc();
    let leases = f
        .volumes()
        .acquire_run(
            &selected,
            f.scope.host_id(),
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    assert_eq!(leases.len(), f.physical.files.len());
    f.repository()
        .bind_resources(f.start.run_id, None, None, None, &f.vm.0)
        .await
        .unwrap();
    f.repository()
        .transition(f.start.run_id, RunState::Provisioning, None, None)
        .await
        .unwrap();
}
async fn load_closure(pool: &PgPool) {
    for path in [
        include_str!("../../../../../../migrations/0124_qualified_invocation_authority.sql"),
        include_str!("../../../../../../migrations/0125_qualified_invocation_empty_secret.sql"),
        include_str!("../../../../../../migrations/0127_prepared_instance_safety.sql"),
        include_str!("../../../../../../migrations/0128_prepared_instance_closure.sql"),
        include_str!("../../../../../../migrations/0129_qualified_remove_resume.sql"),
        include_str!("../../../../../../migrations/0130_existing_instance_closure.sql"),
    ] {
        sqlx::raw_sql(path).execute(pool).await.unwrap();
    }
}
async fn load_wake(pool: &PgPool) {
    sqlx::raw_sql(include_str!(
        "../../../../../../migrations/0131_run_cleanup_wake_qualification.sql"
    ))
    .execute(pool)
    .await
    .unwrap();
}

async fn make_closure(fixture: &Fixture) -> PreparedInstanceClosureRequest {
    let birth = &fixture.birth;
    let actor = &birth.actor;
    let snapshot = birth
        .recipe
        .inspect(actor, birth.intent.id())
        .await
        .unwrap();
    let command = CommandIdentity::from_identity(actor, DeploymentOperation::Remove).unwrap();
    birth
        .recipe
        .admit_remove(
            actor,
            recipe_application::RemoveDeployment {
                command,
                deployment_id: birth.intent.id(),
                expected_version: snapshot.version,
            },
        )
        .await
        .unwrap();
    let service = service(fixture);
    let target = service
        .load_original_target(actor, command, &birth.intent, birth.input.resource())
        .await
        .unwrap();
    let snapshot = birth
        .recipe
        .inspect(actor, birth.intent.id())
        .await
        .unwrap();
    let drain = birth
        .recipe
        .claim_resource_effect(
            actor,
            ResourceEffect {
                command,
                deployment_id: birth.intent.id(),
                resource: birth.input.resource().clone(),
                expected_deployment_version: snapshot.version,
                expected_resource_version: snapshot.resources[birth.input.resource()].version,
                action: ResourceAction::Drain,
                provenance: AttemptProvenance::new(
                    actor,
                    DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
                )
                .unwrap(),
            },
        )
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar("SELECT version FROM agent_instances WHERE id=$1")
        .bind(birth.input.instance_id().as_uuid())
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    PreparedInstanceClosureRequest::new(
        birth.intent.clone(),
        target,
        drain,
        u64::try_from(version).unwrap(),
    )
    .unwrap()
}

async fn check_open(
    fixture: &Fixture,
    closure: &PreparedInstanceClosureRequest,
) -> RunCleanupWakeRequest {
    let actor = &fixture.birth.actor;
    let pending_hash=release_domain::ContentHash::digest(&serde_json::to_vec(&serde_json::json!({"version":1,"operation":"request_instance_removal","actor":actor.user_id,"instance":fixture.start.instance_id,"removal":closure.removal_id(),"expected_version":closure.expected_instance_version()})).unwrap());
    let pending = RunCleanupWakeRequest::new(
        fixture.start.run_id,
        fixture.start.instance_id,
        RunCleanupWakeOriginal {
            drain_attempt: closure.drain().provenance.attempt_id.as_uuid(),
            drain_command: closure.drain().command.id().as_uuid(),
            removal_id: closure.removal_id().as_uuid(),
            command_key: *closure.command_key().as_bytes(),
            input_hash: *pending_hash.as_bytes(),
            expected_version: closure.expected_instance_version(),
            creator: actor.user_id.as_uuid(),
            request: actor.request_id.as_uuid(),
        },
    )
    .unwrap();
    let open = super::observations::rows(&fixture.pool).await;
    let prior = fixture.rows().await;
    historical_held(fixture, &pending).await;
    assert_eq!(
        super::observations::rows(&fixture.pool).await,
        open,
        "open instance and absent original permanent closure cannot prepare"
    );
    assert_eq!(fixture.rows().await, prior);
    pending
}

async fn committed_request(
    fixture: &Fixture,
    closure: &PreparedInstanceClosureRequest,
) -> RunCleanupWakeRequest {
    let (key, hash, creator, request): (Vec<u8>, Vec<u8>, Uuid, Uuid) = sqlx::query_as("SELECT inbox.command_key,removal.input_hash,removal.created_by,removal.request_id FROM agent_instance_removal_requests removal JOIN release_command_inbox inbox ON inbox.secondary_id=removal.id AND inbox.aggregate_id=removal.instance_id AND inbox.operation='request_instance_removal' WHERE removal.id=$1")
            .bind(closure.removal_id().as_uuid()).fetch_one(&fixture.pool).await.unwrap();
    RunCleanupWakeRequest::new(
        fixture.start.run_id,
        fixture.start.instance_id,
        RunCleanupWakeOriginal {
            drain_attempt: closure.drain().provenance.attempt_id.as_uuid(),
            drain_command: closure.drain().command.id().as_uuid(),
            removal_id: closure.removal_id().as_uuid(),
            command_key: key.try_into().unwrap(),
            input_hash: hash.try_into().unwrap(),
            expected_version: closure.expected_instance_version(),
            creator,
            request,
        },
    )
    .unwrap()
}

pub async fn historical_held(f: &Fixture, request: &RunCleanupWakeRequest) {
    let before = super::observations::rows(&f.pool).await;
    for load in [true, false] {
        let mut tx = f.birth.worker.begin().await.unwrap();
        let pins = request.original();
        let original = serde_json::json!({"drain_attempt":pins.drain_attempt,"drain_command":pins.drain_command,"removal_id":pins.removal_id,"command_key":pins.command_key,"input_hash":pins.input_hash,"expected_version":pins.expected_version,"creator":pins.creator,"request":pins.request});
        let query = if load {
            sqlx::query("SELECT * FROM load_run_cleanup_wake118($1,$2,$3,$4,$5,$6)")
        } else {
            sqlx::query("SELECT lock_run_cleanup_wake118($1,$2,$3,$4,$5,$6)")
        };
        let error = query
            .bind(request.run_id().as_uuid())
            .bind(request.instance_id().as_uuid())
            .bind(original)
            .bind(f.scope.provider_namespace())
            .bind(f.scope.host_id())
            .bind(serde_json::to_vec(&f.config.canonical_json()).unwrap())
            .fetch_all(&mut *tx)
            .await
            .unwrap_err();
        assert_eq!(super::observations::code(&error).as_deref(), Some("P1180"));
        tx.rollback().await.unwrap();
        assert_eq!(super::observations::rows(&f.pool).await, before);
    }
}
