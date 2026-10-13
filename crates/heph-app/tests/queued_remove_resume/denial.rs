use crate::fixture::Fixture;
use identity_domain::AuthenticatedIdentity;
use recipe_application::ExistingInstanceClosureRequest;
use run_domain::StartRun;
use runtime_types::VolumeId;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use std::{fs::File, os::unix::fs::MetadataExt, path::PathBuf};
use uuid::Uuid;

pub struct Backing {
    path: PathBuf,
    file: File,
    identity: (u64, u64, u64, [u8; 32]),
    volume: Value,
    id: VolumeId,
}
impl Backing {
    pub async fn capture(fixture: &Fixture, id: VolumeId) -> Self {
        let volume = volume(fixture, id).await;
        let path = PathBuf::from(volume["host_path"].as_str().unwrap());
        let file = File::open(&path).unwrap();
        let identity = physical(&file);
        assert_eq!(identity.2, 16_777_216);
        Self {
            path,
            file,
            identity,
            volume,
            id,
        }
    }
    pub async fn assert_preserved(&self, fixture: &Fixture) {
        assert_eq!(self.identity, physical(&self.file));
        assert_eq!(self.identity, physical(&File::open(&self.path).unwrap()));
        assert_eq!(self.volume, volume(fixture, self.id).await);
        fixture.owner.validate().unwrap();
    }
}
fn physical(file: &File) -> (u64, u64, u64, [u8; 32]) {
    use std::os::unix::fs::FileExt;
    let metadata = file.metadata().unwrap();
    assert!(metadata.is_file());
    let mut digest = Sha256::new();
    let mut offset = 0;
    let mut bytes = vec![0_u8; 65_536];
    loop {
        let size = file.read_at(&mut bytes, offset).unwrap();
        if size == 0 {
            break;
        }
        digest.update(&bytes[..size]);
        offset += u64::try_from(size).unwrap();
    }
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        digest.finalize().into(),
    )
}
async fn volume(fixture: &Fixture, id: VolumeId) -> Value {
    sqlx::query_scalar("SELECT to_jsonb(row) FROM agent_instance_state_volumes row WHERE id=$1")
        .bind(id.as_uuid())
        .fetch_one(&fixture.admin)
        .await
        .unwrap()
}

// The complete originals, transport, authority and cleanup inventories are
// compared losslessly. Current manager admission receipts/audits are separate
// genuine additions and are not physical or Run-cleanup evidence.
const INVENTORY: &str = "SELECT jsonb_build_object(
 'runs',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]') FROM runs row),
 'invocations',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]') FROM agent_instance_execution_invocations row),
 'consumptions',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY run_id),'[]') FROM run_invocation_consumptions row),
 'controls',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]') FROM control_requests row),
 'events',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY run_id,sequence),'[]') FROM run_events row),
 'outbox',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]') FROM outbox row),
 'inbox',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY command_id),'[]') FROM command_inbox row),
 'leases',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]') FROM agent_instance_volume_leases row),
 'targets',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY run_id,generation),'[]') FROM run_cleanup_targets row),
 'target_leases',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY run_id,generation,volume_id),'[]') FROM run_cleanup_target_leases row),
 'receipts',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY run_id,generation),'[]') FROM run_cleanup_receipts row),
 'completions',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY run_id,generation),'[]') FROM run_cleanup_completions row),
 'instances',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]') FROM agent_instances row),
 'bindings',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY instance_revision_id,slot_key),'[]') FROM agent_instance_revision_volume_bindings row),
 'grants',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]') FROM agent_instance_volume_mount_grants row),
 'revocations',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]') FROM agent_instance_volume_mount_revocations row),
 'removals',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]') FROM agent_instance_removal_requests row),
 'release_inbox',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY command_key),'[]') FROM release_command_inbox row),
 'attempts',(SELECT COALESCE(jsonb_agg(to_jsonb(row) ORDER BY id),'[]') FROM recipe_effect_attempts row))";
pub async fn inventory(pool: &PgPool) -> Value {
    sqlx::query_scalar(INVENTORY).fetch_one(pool).await.unwrap()
}
async fn inside(tx: &mut Transaction<'_, Postgres>) -> Value {
    sqlx::query_scalar(INVENTORY)
        .fetch_one(&mut **tx)
        .await
        .unwrap()
}

pub async fn cases(
    fixture: &Fixture,
    manager: &AuthenticatedIdentity,
    request: &ExistingInstanceClosureRequest,
    starts: Option<&[StartRun; 2]>,
) {
    probe(fixture, manager, request, None, true).await;
    probe(
        fixture,
        manager,
        request,
        Some((
            "UPDATE users SET status='suspended' WHERE id=$1",
            manager.user_id.as_uuid(),
        )),
        false,
    )
    .await;
    // Both lease directions, including an otherwise empty instance, are observed.
    // This malformed orphan is NEGATIVE ONLY and never leaves its transaction.
    probe(fixture, manager, request, Some((
        "INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,update_id,host_id,fencing_token,state,expires_at,selection_origin)
         SELECT gen_random_uuid(),binding.volume_id,binding.instance_id,gen_random_uuid(),'orphan-host',1,'active',now()+interval '1 hour','historical_unproven'
         FROM agent_instance_revision_volume_bindings binding WHERE binding.instance_id=$1",
        request.target().prepared().instance_id().as_uuid(),
    )), false).await;
    if let Some(starts) = starts {
        for (sql, id) in [
            (
                "UPDATE runs SET vm_host_id='foreign-host' WHERE id=$1",
                starts[1].run_id.as_uuid(),
            ),
            (
                "UPDATE command_inbox SET processed_at=now() WHERE command_id=$1",
                starts[1].command_id.as_uuid(),
            ),
            (
                "UPDATE run_cleanup_receipts SET vm_id='substituted-completed-vm' WHERE run_id=$1",
                starts[0].run_id.as_uuid(),
            ),
            (
                "UPDATE outbox SET payload=jsonb_set(payload,'{reason}','\"substituted cancellation\"') WHERE aggregate_id=$1 AND event_type='run.cancel_requested.v1'",
                starts[1].run_id.as_uuid(),
            ),
            (
                "WITH changed AS (SELECT id,jsonb_set(config_json,'{vm,namespace}','\"foreign-namespace\"'::jsonb) AS config
                 FROM agent_instance_execution_invocations WHERE run_id=$1)
                 UPDATE agent_instance_execution_invocations original SET config_json=changed.config,
                 config_bytes=convert_to(changed.config::text,'UTF8'),
                 config_hash=sha256(convert_to(changed.config::text,'UTF8')) FROM changed WHERE original.id=changed.id",
                starts[1].run_id.as_uuid(),
            ),
        ] {
            probe(fixture, manager, request, Some((sql, id)), false).await;
        }
    }
}

async fn probe(
    fixture: &Fixture,
    manager: &AuthenticatedIdentity,
    request: &ExistingInstanceClosureRequest,
    mutation: Option<(&str, Uuid)>,
    wrong_namespace: bool,
) {
    let original = inventory(&fixture.admin).await;
    let mut tx = fixture.admin.begin().await.unwrap();
    if let Some((sql, id)) = mutation {
        // Only a disposable negative transaction bypasses mutation triggers.
        // Restore normal guards BEFORE invoking the real actor native front.
        sqlx::query("SET LOCAL session_replication_role=replica")
            .execute(&mut *tx)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query(sql)
                .bind(id)
                .execute(&mut *tx)
                .await
                .unwrap()
                .rows_affected(),
            1
        );
        sqlx::query("SET LOCAL session_replication_role=origin")
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    let held = inside(&mut tx).await;
    let claim: Value =
        sqlx::query_scalar("SELECT claim_json FROM recipe_effect_attempts WHERE id=$1")
            .bind(request.target().original().provenance.attempt_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    sqlx::query("SAVEPOINT qualified_actor_probe")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT set_config('search_path','pg_catalog,public,pg_temp',true),set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.subject_type','user',true),set_config('hephaestus.request_id',$2,true),set_config('hephaestus.occurrence_id',$3,true)")
        .bind(manager.user_id.to_string()).bind(manager.request_id.to_string()).bind(manager.idempotency_id.to_string())
        .execute(&mut *tx).await.unwrap();
    let result = sqlx::query_scalar::<_, Value>(
        "SELECT load_existing_instance_closure118($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(request.command().id().as_uuid())
    .bind(request.intent().id().as_uuid())
    .bind(request.intent().project_id().as_uuid())
    .bind(request.intent().input_hash().as_bytes().as_slice())
    .bind(request.target().original().provenance.attempt_id.as_uuid())
    .bind(claim)
    .bind(serde_json::to_value(request.target().prepared()).unwrap())
    .bind(if wrong_namespace {
        "foreign-namespace"
    } else {
        request.target().provider_namespace()
    })
    .bind(request.target().host_id())
    .bind(serde_json::to_value(request.target().prepared().source().platform()).unwrap())
    .fetch_one(&mut *tx)
    .await;
    // Roll back before asserting the outcome, including an unexpected success.
    // No negative mutation survives a probe or contaminates the next case.
    let reset = sqlx::query("ROLLBACK TO SAVEPOINT qualified_actor_probe")
        .execute(&mut *tx)
        .await;
    let after_front = if reset.is_ok() {
        Some(inside(&mut tx).await)
    } else {
        None
    };
    let rollback = tx.rollback().await;
    let restored = inventory(&fixture.admin).await;
    let error = result.expect_err("contradiction must deny the WHOLE actual actor continuation");
    assert!(
        matches!(
            error
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("P1180" | "P1400" | "42501")
        ),
        "primary denial: {error}"
    );
    reset.unwrap_or_else(|cause| panic!("primary denial: {error}; savepoint rollback: {cause}"));
    rollback
        .unwrap_or_else(|cause| panic!("primary denial: {error}; transaction rollback: {cause}"));
    assert_eq!(
        Some(held),
        after_front,
        "the denied actor front writes no history"
    );
    assert_eq!(
        original, restored,
        "all negative mutations were rolled back"
    );
}
