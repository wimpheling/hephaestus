//! Genuine current actor107/118 Drain and immutable original closure ports.
use crate::fixture::Fixture;
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentOperation,
    PreparedInstanceClosureRequest, PreparedInstanceSafetyRepository, ResourceAction,
    ResourceEffect,
};
use release_postgres::PostgresPreparedInstanceSafetyRepository;
use run_orchestrator::{RunCleanupWakeOriginal, RunCleanupWakeRequest};
use uuid::Uuid;

pub fn service(f: &Fixture) -> PostgresPreparedInstanceSafetyRepository {
    PostgresPreparedInstanceSafetyRepository::new(
        f.birth.app.clone(),
        f.birth.worker.clone(),
        f.config.provider_scope(),
        f.birth.input.source().platform().clone(),
    )
}

pub async fn make(fixture: &Fixture) -> PreparedInstanceClosureRequest {
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

pub async fn committed(
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
