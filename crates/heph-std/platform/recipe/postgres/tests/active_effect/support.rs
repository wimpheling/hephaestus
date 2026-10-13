use capability_domain::CapabilitySlotKey;
use identity_domain::{AuthenticatedIdentity, RequestId};
use recipe_application::{
    ActiveDeploymentEffectRepository, AttemptProvenance, CommandIdentity, DeploymentAttemptId,
    DeploymentError, DeploymentOperation, EffectClaim, RemoveDeployment, ResourceAction,
    ResourceEffect,
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{fixtures, harness::Harness, support};

pub async fn claim(h: &Harness, name: &str, action: ResourceAction) -> EffectClaim {
    h.prepare().await;
    let snapshot = h.repository.inspect(&h.actor, h.intent.id()).await.unwrap();
    h.repository
        .claim_resource_effect(
            &h.actor,
            ResourceEffect {
                command: h.command,
                deployment_id: h.intent.id(),
                resource: fixtures::key(name),
                expected_deployment_version: snapshot.version,
                expected_resource_version: snapshot.resources[&fixtures::key(name)].version,
                action,
                provenance: AttemptProvenance::new(
                    &h.actor,
                    DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
                )
                .unwrap(),
            },
        )
        .await
        .unwrap()
}

pub async fn load(
    h: &Harness,
    identity: &AuthenticatedIdentity,
    command: CommandIdentity,
    resource: &CapabilitySlotKey,
) -> Result<Option<EffectClaim>, DeploymentError> {
    h.preparation
        .load_active_effect(identity, command, h.intent.id(), resource)
        .await
}

pub fn fresh(identity: &AuthenticatedIdentity) -> AuthenticatedIdentity {
    let mut result = identity.clone();
    result.request_id = RequestId::new();
    result
}

pub async fn manager(h: &Harness, pool: &PgPool) -> AuthenticatedIdentity {
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(h.fixture.consuming_project)
        .bind(h.fixture.outsider.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    let identity = support::identity(h.fixture.outsider);
    assert!(!h.can_use(&identity).await, "manager has no source use");
    identity
}

pub async fn remove(h: &Harness, identity: &AuthenticatedIdentity) -> CommandIdentity {
    let snapshot = h.repository.inspect(identity, h.intent.id()).await.unwrap();
    let command = CommandIdentity::from_identity(identity, DeploymentOperation::Remove).unwrap();
    h.repository
        .admit_remove(
            identity,
            RemoveDeployment {
                command,
                deployment_id: h.intent.id(),
                expected_version: snapshot.version,
            },
        )
        .await
        .unwrap();
    command
}

pub async fn state(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object(
      'attempts',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_effect_attempts row),
      'resources',(SELECT jsonb_agg(to_jsonb(row) ORDER BY deployment_id,resource_name) FROM recipe_deployment_resources row),
      'deployments',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_deployments row),
      'commands',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_deployment_commands row),
      'verifications',(SELECT count(*) FROM recipe_effect_verifications),
      'grants',(SELECT count(*) FROM agent_instance_volume_mount_grants),
      'transitions',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_execution_transitions row),
      'prepared',(SELECT jsonb_agg(to_jsonb(row) ORDER BY deployment_id) FROM recipe_deployment_execution_preparations row),
      'volumes',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM agent_instance_state_volumes row),
      'instances',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM agent_instances row),
      'events',(SELECT count(*) FROM application_events),
      'outbox',(SELECT count(*) FROM product_event_outbox))")
      .fetch_one(pool).await.unwrap()
}
