use super::fixture::Fixture;
use hephaestus_app::ManagedInvocationController;
use recipe_application::PreparedInstanceImport;
use run_domain::{
    CancelRun, FreshRunExecutionProfile, RunCleanupHostId, RunOutcome, RunState, StartRun,
};
use run_orchestrator::RunRepository;
use run_postgres::{
    PgQualifiedInvocationRoutingReader, PgRunRepository, QualifiedInvocationEvidence,
    QualifiedInvocationRoute, QualifiedInvocationRoutingDecision,
};
use runtime_types::{RunId, VolumeId};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    sync::Mutex,
};
use uuid::Uuid;
use vm_trait::VmProviderOwnerScope;

pub struct Snapshot {
    instance: Value,
    bindings: Value,
    grants: Value,
    revocations: Value,
    volume: Value,
    birth: Value,
    originals: [Value; 2],
    inboxes: [Value; 2],
    sibling: Value,
    sibling_route: Box<QualifiedInvocationRoute>,
    backing: Mutex<File>,
    physical: (u64, u64, u64, [u8; 32]),
    volume_id: VolumeId,
}

impl Snapshot {
    pub async fn capture(
        fixture: &Fixture,
        instance: &PreparedInstanceImport,
        starts: &[StartRun; 2],
        volume: VolumeId,
    ) -> Self {
        let (parent, bindings, grants, revocations) = authority(fixture, instance).await;
        let row = volume_row(fixture, volume).await;
        let birth = birth(fixture, volume).await;
        assert_eq!(row["capacity_bytes"], 16_777_216);
        assert_eq!(row["provisioning_state"], "ready");
        assert_eq!(row["lease_generation"], 0);
        let path = std::path::Path::new(row["host_path"].as_str().unwrap());
        assert_eq!(path.parent(), Some(fixture.owner.root()));
        let mut backing = File::open(path).unwrap();
        let physical = physical(&mut backing);
        assert_eq!(physical.2, 16_777_216);
        for start in starts {
            queued(fixture, start).await;
        }
        let originals = [
            original(fixture, starts[0].run_id).await,
            original(fixture, starts[1].run_id).await,
        ];
        let inboxes = [
            inbox(fixture, &starts[0]).await,
            inbox(fixture, &starts[1]).await,
        ];
        let sibling = run(fixture, starts[1].run_id).await;
        let sibling_route = route(fixture, &starts[1]).await;
        assert_eq!(bindings[0]["volume_id"], volume.as_uuid().to_string());
        assert_eq!(
            grants[0]["id"],
            instance.volumes()[0].grant_id().as_uuid().to_string()
        );
        assert_eq!(grants[0]["volume_id"], volume.as_uuid().to_string());
        Self {
            instance: parent,
            bindings,
            grants,
            revocations,
            volume: row,
            birth,
            originals,
            inboxes,
            sibling,
            sibling_route,
            backing: Mutex::new(backing),
            physical,
            volume_id: volume,
        }
    }

    pub fn require_nonempty(&self) {
        assert_eq!(self.bindings.as_array().unwrap().len(), 1);
        assert_eq!(self.grants.as_array().unwrap().len(), 1);
        assert_eq!(self.revocations, serde_json::json!([]));
        assert_eq!(self.instance["run_gate_open"], true);
    }

    pub async fn assert_preserved(
        &self,
        fixture: &Fixture,
        instance: &PreparedInstanceImport,
        starts: &[StartRun; 2],
    ) {
        let (parent, bindings, grants, revocations) = authority(fixture, instance).await;
        assert_eq!(self.instance, parent);
        assert_eq!(self.bindings, bindings);
        assert_eq!(self.grants, grants);
        assert_eq!(self.revocations, revocations);
        assert_eq!(self.volume, volume_row(fixture, self.volume_id).await);
        assert_eq!(self.birth, birth(fixture, self.volume_id).await);
        for (i, start) in starts.iter().enumerate() {
            assert_eq!(self.originals[i], original(fixture, start.run_id).await);
            assert_eq!(self.inboxes[i], inbox(fixture, start).await);
            zero_consumption(fixture, start.run_id).await;
        }
        assert_eq!(self.sibling, run(fixture, starts[1].run_id).await);
        assert_eq!(self.sibling_route, route(fixture, &starts[1]).await);
        queued(fixture, &starts[1]).await;
        // Both the retained opened backing and its current named entry must
        // keep their original device/inode/length/content, not only a path string.
        {
            let mut backing = self.backing.lock().unwrap();
            assert_eq!(self.physical, physical(&mut backing));
        }
        let mut named = File::open(self.volume["host_path"].as_str().unwrap()).unwrap();
        assert_eq!(self.physical, physical(&mut named));
        fixture.owner.validate().unwrap();
        assert!(
            fixture
                .provider
                .attempt_inventory(fixture.config.provider_scope())
                .await
                .unwrap()
                .attempts
                .is_empty()
        );
    }
}

async fn authority(
    fixture: &Fixture,
    instance: &PreparedInstanceImport,
) -> (Value, Value, Value, Value) {
    let id = instance.instance_id().as_uuid();
    let parent = sqlx::query_scalar("SELECT to_jsonb(row) FROM agent_instances row WHERE id=$1")
        .bind(id)
        .fetch_one(&fixture.admin)
        .await
        .unwrap();
    let bindings=sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY instance_revision_id,slot_key),'[]'::jsonb) FROM agent_instance_revision_volume_bindings row WHERE instance_id=$1").bind(id).fetch_one(&fixture.admin).await.unwrap();
    let grants=sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]'::jsonb) FROM agent_instance_volume_mount_grants row WHERE instance_id=$1").bind(id).fetch_one(&fixture.admin).await.unwrap();
    let revocations=sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY row.id),'[]'::jsonb) FROM agent_instance_volume_mount_revocations row JOIN agent_instance_volume_mount_grants grant_row ON grant_row.id=row.grant_id WHERE grant_row.instance_id=$1").bind(id).fetch_one(&fixture.admin).await.unwrap();
    (parent, bindings, grants, revocations)
}
async fn volume_row(fixture: &Fixture, id: VolumeId) -> Value {
    sqlx::query_scalar("SELECT to_jsonb(row) FROM agent_instance_state_volumes row WHERE id=$1")
        .bind(id.as_uuid())
        .fetch_one(&fixture.admin)
        .await
        .unwrap()
}
async fn birth(fixture: &Fixture, id: VolumeId) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('seal',to_jsonb(seal),'purpose',to_jsonb(purpose),'operations',(SELECT jsonb_agg(to_jsonb(operation) ORDER BY generation) FROM volume_owned_provisioning_operations operation WHERE operation.volume_id=$1),'observations',(SELECT jsonb_agg(to_jsonb(observation) ORDER BY created_at,id) FROM volume_owned_backing_observations observation WHERE observation.volume_id=$1),'heads',(SELECT jsonb_agg(to_jsonb(head) ORDER BY version) FROM volume_owned_observation_heads head WHERE head.volume_id=$1)) FROM volume_creation_seals seal JOIN volume_owned_backing_purposes purpose ON purpose.seal_id=seal.id WHERE seal.volume_id=$1")
        .bind(id.as_uuid()).fetch_one(&fixture.admin).await.unwrap()
}

async fn original(fixture: &Fixture, id: RunId) -> Value {
    sqlx::query_scalar(
        "SELECT to_jsonb(row) FROM agent_instance_execution_invocations row WHERE run_id=$1",
    )
    .bind(id.as_uuid())
    .fetch_one(&fixture.admin)
    .await
    .unwrap()
}
async fn inbox(fixture: &Fixture, start: &StartRun) -> Value {
    let row: Value =
        sqlx::query_scalar("SELECT to_jsonb(row) FROM command_inbox row WHERE command_id=$1")
            .bind(start.command_id.as_uuid())
            .fetch_one(&fixture.admin)
            .await
            .unwrap();
    assert!(row["processed_at"].is_null());
    row
}
async fn run(fixture: &Fixture, id: RunId) -> Value {
    sqlx::query_scalar("SELECT to_jsonb(row) FROM runs row WHERE id=$1")
        .bind(id.as_uuid())
        .fetch_one(&fixture.admin)
        .await
        .unwrap()
}
async fn route(fixture: &Fixture, start: &StartRun) -> Box<QualifiedInvocationRoute> {
    match PgQualifiedInvocationRoutingReader::new(
        fixture.original.worker.clone(),
        fixture.config.clone(),
    )
    .inspect_start(start)
    .await
    .unwrap()
    {
        QualifiedInvocationRoutingDecision::Qualified(route) => route,
        other @ QualifiedInvocationRoutingDecision::Held(_) => {
            panic!("genuine original must qualify: {other:?}")
        }
    }
}
async fn queued(fixture: &Fixture, start: &StartRun) {
    let original = route(fixture, start).await;
    assert_eq!(
        original.evidence(),
        QualifiedInvocationEvidence::QueuedIntent115
    );
    assert_eq!(original.run_state(), RunState::Queued);
    assert!(!original.cancellation_requested());
    assert!(!original.acquisition_closed());
    assert_eq!(original.owner(), fixture.config.provider_scope());
    assert_eq!(original.original_start(), start);
    // This genuine native read checks mutable gate/grant eligibility; the
    // immutable routing projection alone explicitly supplies no dispatch right.
    let eligible: bool =
        sqlx::query_scalar("SELECT qualified_invocation_current_eligible($1,$2,$3)")
            .bind(start.instance_id.as_uuid())
            .bind(start.instance_revision_id.as_uuid())
            .bind(fixture.config.canonical_json())
            .fetch_one(&fixture.admin)
            .await
            .unwrap();
    assert!(eligible);

    let row = run(fixture, start.run_id).await;
    assert_eq!(row["state_version"], 0);
    assert!(row["volume_id"].is_null());
    assert!(row["lease_id"].is_null());
    zero_consumption(fixture, start.run_id).await;
}
async fn zero_consumption(fixture: &Fixture, id: RunId) {
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM run_invocation_consumptions WHERE run_id=$1),(SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=$1)").bind(id.as_uuid()).fetch_one(&fixture.admin).await.unwrap();
    assert_eq!(counts, (0, 0));
}

pub async fn processed(fixture: &Fixture, command: &CancelRun) {
    let (kind,payload,processed):(String,Value,bool)=sqlx::query_as("SELECT command_type,payload,processed_at IS NOT NULL FROM command_inbox WHERE command_id=$1").bind(command.command_id.as_uuid()).fetch_one(&fixture.admin).await.unwrap();
    assert_eq!(kind, "cancel_run");
    assert_eq!(payload, serde_json::to_value(command).unwrap());
    assert!(processed);
}

pub async fn cleaned(fixture: &Fixture, controller: &ManagedInvocationController, id: RunId) {
    let actual = controller.run(id).await.unwrap();
    assert_eq!(actual.state, RunState::CleanedUp);
    assert_eq!(actual.outcome, Some(RunOutcome::Cancelled));
    assert!(actual.exit.is_none());
    assert!(actual.failure.is_none());
    zero_consumption(fixture, id).await;
    let pins:Value=sqlx::query_scalar("SELECT jsonb_build_object('target',to_jsonb(target),'receipt',to_jsonb(receipt),'completion',to_jsonb(completion)) FROM run_cleanup_targets target JOIN run_cleanup_receipts receipt USING(run_id,generation,target_digest) JOIN run_cleanup_completions completion USING(run_id,generation,target_digest) WHERE target.run_id=$1").bind(id.as_uuid()).fetch_one(&fixture.admin).await.unwrap();
    assert_eq!(pins["target"]["lease_count"], 0);
    assert_eq!(
        pins["target"]["vm_provider_namespace"],
        fixture.config.provider_scope().namespace()
    );
    assert_eq!(
        pins["target"]["vm_host_id"],
        fixture.config.provider_scope().host_id()
    );
    assert_eq!(pins["target"]["vm_id"], id.to_string());
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id=$1 AND event_type='run.cleaned_up'",
    )
    .bind(id.as_uuid())
    .fetch_one(&fixture.admin)
    .await
    .unwrap();
    assert_eq!(events, 1);
}

// Each new admitted Cancel legitimately updates Run.updated_at. Pin its entire
// cleanup records and immutable outcome/closure fields; do not erase that write.
pub async fn completion(fixture: &Fixture, id: RunId) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('run',jsonb_build_object('id',run.id,'state',run.state,'state_version',run.state_version,'outcome',run.outcome,'cancel_requested_at',run.cancel_requested_at,'volume_acquisition_closed_at',run.volume_acquisition_closed_at),'target',to_jsonb(target),'receipt',to_jsonb(receipt),'completion',to_jsonb(completion),'leases',(SELECT COALESCE(jsonb_agg(to_jsonb(lease) ORDER BY volume_id),'[]'::jsonb) FROM run_cleanup_target_leases lease WHERE lease.run_id=$1),'cleaned_events',(SELECT COALESCE(jsonb_agg(to_jsonb(event) ORDER BY sequence),'[]'::jsonb) FROM run_events event WHERE event.run_id=$1 AND event_type='run.cleaned_up')) FROM runs run JOIN run_cleanup_targets target ON target.run_id=run.id JOIN run_cleanup_receipts receipt USING(run_id,generation,target_digest) JOIN run_cleanup_completions completion USING(run_id,generation,target_digest) WHERE run.id=$1").bind(id.as_uuid()).fetch_one(&fixture.admin).await.unwrap()
}

pub async fn denials(fixture: &Fixture, id: RunId, cancel: &CancelRun) {
    let scope = RunCleanupHostId::new(
        fixture.config.provider_scope().namespace().into(),
        fixture.config.provider_scope().host_id().into(),
    )
    .unwrap();
    let runs = PgRunRepository::new(fixture.original.worker.clone())
        .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
        .with_vm_owner_scope(scope.clone())
        .with_qualified_invocation_configuration(fixture.config.clone())
        .with_qualified_invocation_lease_runtime()
        .with_qualified_cleanup_wake(scope);
    let wrong = VmProviderOwnerScope::new(
        Uuid::new_v4().to_string(),
        fixture.config.provider_scope().host_id().into(),
    )
    .unwrap();
    assert!(runs.classify_invocation_cancel(id, &wrong).await.is_err());
    assert!(
        runs.prepare_invocation_cancel_cleanup(id, &wrong)
            .await
            .is_err()
    );
    assert!(
        runs.classify_invocation_cancel(RunId::new(), fixture.config.provider_scope())
            .await
            .is_err()
    );
    let mut malformed = cancel.clone();
    malformed.reason.push_str(" foreign delivery");
    assert!(runs.request_cancel(&malformed).await.is_err());
}

fn physical(file: &mut File) -> (u64, u64, u64, [u8; 32]) {
    let metadata = file.metadata().unwrap();
    assert!(metadata.is_file());
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut digest = Sha256::new();
    let mut bytes = vec![0_u8; 64 * 1024];
    loop {
        let size = file.read(&mut bytes).unwrap();
        if size == 0 {
            break;
        }
        digest.update(&bytes[..size]);
    }
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        digest.finalize().into(),
    )
}
