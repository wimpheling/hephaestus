use std::collections::BTreeMap;

use hephaestus_app::recipe_instance_import::to_import_command;
use release_domain::{
    InstanceName, ParameterDeclaration, ParameterName, ParameterType, ParameterValue,
    ReleaseAgentId, ReleaseCommandKey, ReleaseId,
};
use release_service::ImportAgentWithVolumes;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::instance_fixture::Harness;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn actual107_receipts_match_private113_hash_for_unicode_escapes_sorted_keys_and_integer_extremes(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false).await;
    let pin = publish_vector_source(&pool, &h).await;
    for integer in [i64::MIN, i64::MAX] {
        let mut command = to_import_command(&h.prospective_slotless_import());
        command.instance_id = release_domain::AgentInstanceId::from_uuid(Uuid::new_v4());
        command.revision_id = release_domain::AgentInstanceRevisionId::from_uuid(Uuid::new_v4());
        command.name = InstanceName::parse(format!("vector-{}", command.instance_id)).unwrap();
        command.command_key =
            ReleaseCommandKey::derive("actual-vector", &[command.instance_id.as_uuid().as_bytes()]);
        command.release_id = ReleaseId::from_uuid(pin.0);
        command.release_agent_id = ReleaseAgentId::from_uuid(pin.1);
        command.parameters = BTreeMap::from([
            (
                ParameterName::parse("zzz").unwrap(),
                ParameterValue::Integer(integer),
            ),
            (
                ParameterName::parse("aa").unwrap(),
                ParameterValue::String("é中🦀\"\\/\n\r\t\u{8}\u{c}\u{1}\u{1f}".to_owned()),
            ),
            (
                ParameterName::parse("boolean").unwrap(),
                ParameterValue::Boolean(true),
            ),
        ]);
        h.release
            .import_agent_with_volumes(&h.actor, command.clone())
            .await
            .unwrap();
        let actual: Vec<u8> =
            sqlx::query_scalar("SELECT input_hash FROM release_command_inbox WHERE command_key=$1")
                .bind(command.command_key.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(actual, trusted_hash(&h.actor, &command));
        assert_eq!(
            actual,
            sql_hash(&pool, &h, &command).await,
            "compare the actual compiled107 receipt, not two copies of SQL logic"
        );
    }
    let mut tx = pool.begin().await.unwrap();
    let error =
        sqlx::query_scalar::<_, String>("SELECT recipe_import107_json_bytes('1.5'::jsonb,0)")
            .fetch_one(&mut *tx)
            .await
            .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("23000")
    );
    tx.rollback().await.unwrap();
}

async fn publish_vector_source(pool: &PgPool, h: &Harness) -> (Uuid, Uuid) {
    let release = Uuid::new_v4();
    let agent = Uuid::new_v4();
    let declarations = [
        declaration(
            "aa",
            ParameterType::String {
                minimum_length: 0,
                maximum_length: 4096,
            },
        ),
        declaration(
            "zzz",
            ParameterType::Integer {
                minimum: i64::MIN,
                maximum: i64::MAX,
            },
        ),
        declaration("boolean", ParameterType::Boolean),
    ];
    sqlx::query("INSERT INTO releases(id,repository_id,version,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,state)
      SELECT $1,repository_id,'hash-vector-v1',source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,'draft' FROM releases WHERE id=$2")
      .bind(release).bind(h.fixture.release).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state,parameter_schema)
      SELECT $1,$2,family_id,'vectors','Vectors',runtime_contract,runtime_contract_hash,false,$3 FROM release_agents WHERE id=$4")
      .bind(agent).bind(release).bind(serde_json::to_value(declarations).unwrap()).bind(h.fixture.release_agent).execute(pool).await.unwrap();
    sqlx::query("UPDATE releases SET state='published',published_at=now() WHERE id=$1")
        .bind(release)
        .execute(pool)
        .await
        .unwrap();
    (release, agent)
}

fn declaration(name: &str, value_type: ParameterType) -> ParameterDeclaration {
    ParameterDeclaration {
        name: ParameterName::parse(name).unwrap(),
        value_type,
        required: true,
        default: None,
        sensitive: false,
    }
}

fn trusted_hash(
    identity: &identity_domain::AuthenticatedIdentity,
    command: &ImportAgentWithVolumes,
) -> Vec<u8> {
    let selections=command.volumes.iter().map(|slot|serde_json::json!({"slot":slot.slot,"guest_path":slot.guest_path,"access_mode":slot.access_mode,"volume_id":slot.volume_id,"grant_id":slot.grant_id})).collect::<Vec<_>>();
    Sha256::digest(serde_json::to_vec(&serde_json::json!({"version":1,"operation":"import_agent_with_volumes","actor":identity.user_id,
      "instance":command.instance_id,"revision":command.revision_id,"release":command.release_id,"agent":command.release_agent_id,
      "project":command.project_id,"name":command.name.as_str(),"parameters":command.parameters,"selected_policy":command.selected_policy,
      "platform_policy":command.platform_policy,"platform_policy_version":command.platform_policy_version,"volumes":selections})).unwrap()).to_vec()
}

async fn sql_hash(pool: &PgPool, h: &Harness, command: &ImportAgentWithVolumes) -> Vec<u8> {
    // Admin-only hash vector projection: this is not a persisted113 child,
    // claim, worker receipt or authority input to an application method.
    let mut child = serde_json::to_value(h.prospective_slotless_import()).unwrap();
    child["name"] = serde_json::json!(command.name.as_str());
    child["parameters"] = serde_json::to_value(&command.parameters).unwrap();
    sqlx::query_scalar("SELECT recipe_import107_input_hash(jsonb_populate_record(NULL::recipe_prepared_instance_imports,
      jsonb_build_object('project_id',$1::uuid,'instance_id',$2::uuid,'revision_id',$3::uuid,
        'release_id',$4::uuid,'release_agent_id',$5::uuid,'canonical_import',E'\\\\x'||encode($6::bytea,'hex'))),$7::uuid)")
      .bind(command.project_id.as_uuid()).bind(command.instance_id.as_uuid()).bind(command.revision_id.as_uuid())
      .bind(command.release_id.as_uuid()).bind(command.release_agent_id.as_uuid()).bind(serde_json::to_vec(&child).unwrap())
      .bind(h.actor.user_id.as_uuid()).fetch_one(pool).await.unwrap()
}
