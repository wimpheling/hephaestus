use crate::support::Fixture;
use capability_domain::CapabilitySlotKey;
use forge_domain::ProjectId;
use release_domain::{
    AgentInstanceId, AgentInstanceRevisionId, InstanceName, NetworkAccess, ReleaseAgentId,
    ReleaseCommandKey, ReleaseId, RuntimePolicy,
};
use release_service::{ImportAgentWithVolumes, VolumeSlotSelection};
use runtime_types::VolumeId;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use volume_domain::{GuestMountPath, VolumeAccessMode, VolumeMountGrantId};

pub const fn policy() -> RuntimePolicy {
    RuntimePolicy {
        vcpus: 1,
        memory_mib: 128,
        network: NetworkAccess::Disabled,
    }
}

pub async fn export(pool: &PgPool, fixture: &Fixture, legacy: bool) -> ReleaseAgentId {
    let id = ReleaseAgentId::new();
    let slots = if legacy {
        json!([])
    } else {
        json!([
            {"slot":"data","guest_path":"/data","access_mode":"read_write","required":true,"minimum_capacity_bytes":33_554_432},
            {"slot":"cache","guest_path":"/cache","access_mode":"read_only","required":false,"minimum_capacity_bytes":1}
        ])
    };
    let contract = json!({"requires_state":legacy,"volume_slots":slots,"policy_ceiling":policy()});
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state,parameter_schema) SELECT $1,release_id,family_id,$3,'Typed', $4,$5,$6,$7 FROM release_agents WHERE id=$2")
        .bind(id.as_uuid()).bind(fixture.release_agent).bind(if legacy { "legacy-typed" } else { "explicit-typed" }).bind(contract).bind([17_u8;32].as_slice()).bind(legacy)
        .bind(json!([{"name":"operation","value_type":{"type":"string","minimum_length":0,"maximum_length":32},"required":true,"default":"inspect","sensitive":false}]))
        .execute(pool).await.expect("new export with authoritative frozen slots and policy");
    id
}

pub async fn volume(pool: &PgPool, project: Uuid, capacity: i64) -> VolumeId {
    let id = VolumeId::new();
    sqlx::query("INSERT INTO agent_instance_state_volumes(id,project_id,state,capacity_bytes,filesystem_uuid) VALUES($1,$2,'uninitialized',$3,gen_random_uuid())")
        .bind(id.as_uuid()).bind(project).bind(capacity).execute(pool).await.expect("retained standalone resource");
    id
}

pub fn import(
    fixture: &Fixture,
    agent: ReleaseAgentId,
    volume: VolumeId,
    legacy: bool,
) -> ImportAgentWithVolumes {
    ImportAgentWithVolumes {
        command_key: ReleaseCommandKey::derive("typed-import", &[Uuid::new_v4().as_bytes()]),
        instance_id: AgentInstanceId::new(),
        revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::from_uuid(fixture.release),
        release_agent_id: agent,
        project_id: ProjectId::from_uuid(fixture.consuming_project),
        name: InstanceName::parse(if legacy {
            "typed-legacy"
        } else {
            "typed-explicit"
        })
        .unwrap(),
        parameters: std::collections::BTreeMap::new(),
        selected_policy: policy(),
        platform_policy: policy(),
        platform_policy_version: String::from("fixture/v1"),
        volumes: vec![VolumeSlotSelection {
            slot: CapabilitySlotKey::parse(if legacy { "state" } else { "data" }).unwrap(),
            guest_path: GuestMountPath::parse(if legacy {
                "/var/lib/hephaestus"
            } else {
                "/data"
            })
            .unwrap(),
            access_mode: VolumeAccessMode::ReadWrite,
            volume_id: volume,
            grant_id: VolumeMountGrantId::new(),
        }],
    }
}
