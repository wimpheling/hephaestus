use crate::{ReleaseService, ReleaseServiceError, fixture, support};
use capability_domain::CapabilitySlotKey;
use release_domain::{InstanceName, ParameterName, ParameterValue, ReleaseId};
use release_service::{ImportAgentWithVolumes, VolumeSlotSelection};
use sqlx::PgPool;
use volume_domain::{GuestMountPath, VolumeAccessMode, VolumeMountGrantId};

pub async fn exercise(
    pool: &PgPool,
    seed: &support::Fixture,
    service: &ReleaseService,
) -> ImportAgentWithVolumes {
    let actor = support::identity(seed.maintainer);
    let agent = fixture::export(pool, seed, false).await;
    let data = fixture::volume(pool, seed.consuming_project, 33_554_432).await;
    let mut command = fixture::import(seed, agent, data, false);
    let cache = fixture::volume(pool, seed.consuming_project, 16_777_216).await;
    command.volumes.push(VolumeSlotSelection {
        slot: CapabilitySlotKey::parse("cache").unwrap(),
        guest_path: GuestMountPath::parse("/cache").unwrap(),
        access_mode: VolumeAccessMode::ReadOnly,
        volume_id: cache,
        grant_id: VolumeMountGrantId::new(),
    });
    invalid_selections(pool, seed, service, &command).await;
    crate::unsupported::requirements(pool, seed, service, &command).await;
    let denied = support::identity(seed.member);
    assert!(matches!(
        service
            .import_agent_with_volumes(&denied, command.clone())
            .await,
        Err(ReleaseServiceError::AuthorizationDenied)
    ));
    assert_no_instance(pool, &command).await;
    let id = service
        .import_agent_with_volumes(&actor, command.clone())
        .await
        .expect("atomic typed initial import");
    assert_eq!(id, command.instance_id);
    let mut reversed = command.clone();
    reversed.volumes.reverse();
    reversed.parameters.insert(
        ParameterName::parse("operation").unwrap(),
        ParameterValue::String(String::from("inspect")),
    );
    assert_eq!(
        service
            .import_agent_with_volumes(&actor, reversed)
            .await
            .unwrap(),
        id,
        "normalized selection order and resolved defaults replay"
    );
    assert_catalog(pool, &command, "explicit").await;
    crate::unsupported::inbox_forgery(pool, seed, &command).await;
    changed_inputs(service, &actor, &command).await;
    crate::unsupported::grant_conflict_rolls_back(pool, seed, service, &command).await;
    source_replay_denial(pool, seed, service, &command).await;
    let legacy_agent = fixture::export(pool, seed, true).await;
    let legacy = fixture::import(
        seed,
        legacy_agent,
        fixture::volume(pool, seed.consuming_project, 16_777_216).await,
        true,
    );
    service
        .import_agent_with_volumes(&actor, legacy.clone())
        .await
        .expect("frozen legacy state declaration may select standalone resource");
    assert_catalog(pool, &legacy, "legacy_declaration").await;
    command
}

async fn assert_catalog(pool: &PgPool, command: &ImportAgentWithVolumes, provenance: &str) {
    let (mode,gate,scalar,runnable):(String,bool,Option<uuid::Uuid>,bool)=sqlx::query_as("SELECT instance.volume_mode,instance.run_gate_open,instance.state_volume_id,revision.runnable FROM agent_instances instance JOIN agent_instance_revisions revision ON revision.id=instance.active_revision_id WHERE instance.id=$1").bind(command.instance_id.as_uuid()).fetch_one(pool).await.unwrap();
    assert_eq!(mode, "named");
    assert!(!gate);
    assert!(scalar.is_none());
    assert!(
        runnable,
        "revision completeness is independent of dispatch support"
    );
    let rows:Vec<(String,String,uuid::Uuid)>=sqlx::query_as("SELECT binding.slot_key,binding.provenance,mount_grant.id FROM agent_instance_revision_volume_bindings binding JOIN agent_instance_volume_mount_grants mount_grant ON mount_grant.instance_revision_id=binding.instance_revision_id AND mount_grant.slot_key=binding.slot_key WHERE binding.instance_revision_id=$1 ORDER BY binding.slot_key").bind(command.revision_id.as_uuid()).fetch_all(pool).await.unwrap();
    assert_eq!(rows.len(), command.volumes.len());
    for (slot, stored, grant) in rows {
        assert_eq!(stored, provenance);
        assert!(
            command
                .volumes
                .iter()
                .any(|selection| selection.slot.as_str() == slot
                    && selection.grant_id.as_uuid() == grant)
        );
    }
    assert!(
        sqlx::query("UPDATE agent_instances SET run_gate_open=true WHERE id=$1")
            .bind(command.instance_id.as_uuid())
            .execute(pool)
            .await
            .is_err(),
        "unsupported runtime profile cannot open gate"
    );
    let revision = uuid::Uuid::new_v4();
    assert!(sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable FROM agent_instance_revisions WHERE id=$2").bind(revision).bind(command.revision_id.as_uuid()).execute(pool).await.is_err(),"legacy revision entrypoints cannot bypass named update guard");
    assert!(sqlx::query("INSERT INTO runs(id,instance_id,instance_revision_id,release_id,release_agent_id,run_kind,command_id,state,requires_state,created_at,updated_at) VALUES(gen_random_uuid(),$1,$2,$3,$4,'update',gen_random_uuid(),'queued',false,now(),now())").bind(command.instance_id.as_uuid()).bind(command.revision_id.as_uuid()).bind(command.release_id.as_uuid()).bind(command.release_agent_id.as_uuid()).execute(pool).await.is_err(),"unsupported typed update dispatch cannot start");
}

async fn invalid_selections(
    pool: &PgPool,
    seed: &support::Fixture,
    service: &ReleaseService,
    command: &ImportAgentWithVolumes,
) {
    let actor = support::identity(seed.maintainer);
    let small = fixture::volume(pool, seed.consuming_project, 16_777_216).await;
    let foreign = fixture::volume(pool, seed.project, 33_554_432).await;
    let mut cases = Vec::new();
    let mut changed = command.clone();
    changed.volumes.remove(0);
    cases.push(changed);
    let mut changed = command.clone();
    changed.volumes[0].access_mode = VolumeAccessMode::ReadOnly;
    cases.push(changed);
    let mut changed = command.clone();
    changed.volumes[0].guest_path = GuestMountPath::parse("/other").unwrap();
    cases.push(changed);
    let mut changed = command.clone();
    changed.volumes[0].volume_id = small;
    cases.push(changed);
    let mut changed = command.clone();
    changed.volumes[0].volume_id = foreign;
    cases.push(changed);
    let mut changed = command.clone();
    changed.volumes[1].volume_id = changed.volumes[0].volume_id;
    cases.push(changed);
    let mut changed = command.clone();
    changed.volumes[1].grant_id = changed.volumes[0].grant_id;
    cases.push(changed);
    for invalid in cases {
        assert!(matches!(
            service.import_agent_with_volumes(&actor, invalid).await,
            Err(ReleaseServiceError::InvalidVolumeSelection)
        ));
        assert_no_instance(pool, command).await;
    }
}

async fn changed_inputs(
    service: &ReleaseService,
    actor: &identity_domain::AuthenticatedIdentity,
    command: &ImportAgentWithVolumes,
) {
    let mut cases = Vec::new();
    let mut changed = command.clone();
    changed.name = InstanceName::parse("changed-name").unwrap();
    cases.push(changed);
    let mut changed = command.clone();
    changed.parameters.insert(
        ParameterName::parse("operation").unwrap(),
        ParameterValue::String(String::from("put")),
    );
    cases.push(changed);
    let mut changed = command.clone();
    changed.platform_policy_version = String::from("fixture/v2");
    cases.push(changed);
    let mut changed = command.clone();
    changed.volumes[0].grant_id = VolumeMountGrantId::new();
    cases.push(changed);
    let mut changed = command.clone();
    changed.instance_id = release_domain::AgentInstanceId::new();
    cases.push(changed);
    let mut changed = command.clone();
    changed.selected_policy.memory_mib = 64;
    cases.push(changed);
    let mut changed = command.clone();
    changed.platform_policy.memory_mib = 256;
    cases.push(changed);
    let mut changed = command.clone();
    changed.revision_id = release_domain::AgentInstanceRevisionId::new();
    cases.push(changed);
    for changed in cases {
        assert!(matches!(
            service.import_agent_with_volumes(actor, changed).await,
            Err(ReleaseServiceError::IdempotencyConflict)
        ));
    }
    let mut wrong = command.clone();
    wrong.release_id = ReleaseId::new();
    assert!(
        matches!(
            service.import_agent_with_volumes(actor, wrong).await,
            Err(ReleaseServiceError::Unavailable)
        ),
        "release/export exact pair required"
    );
}

async fn assert_no_instance(pool: &PgPool, command: &ImportAgentWithVolumes) {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_instances WHERE id=$1")
        .bind(command.instance_id.as_uuid())
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

async fn source_replay_denial(
    pool: &PgPool,
    seed: &support::Fixture,
    service: &ReleaseService,
    command: &ImportAgentWithVolumes,
) {
    let actor = support::identity(seed.maintainer);
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(seed.project)
        .bind(seed.maintainer.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    assert!(
        matches!(
            service
                .import_agent_with_volumes(&actor, command.clone())
                .await,
            Err(ReleaseServiceError::AuthorizationDenied)
        ),
        "source loss before replay"
    );
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(seed.project)
        .bind(seed.maintainer.as_uuid())
        .execute(pool)
        .await
        .unwrap();
}
