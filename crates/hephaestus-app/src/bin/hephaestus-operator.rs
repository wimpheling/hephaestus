//! Authorization-aware read-only inspection and narrowly scoped recovery CLI.

use authz_domain::{AuthorizationDecision, Authorizer, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::{AgentUpdateId, BuildRequestId, ReleaseCommandKey};
use release_service::{RecoverInstanceUpdate, ReleaseService, UpdateRecoveryAction};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, error::Error, str::FromStr, sync::Arc};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let database_url = env::var("HEPHAESTUS_DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await?;
    let output = execute(&pool, &arguments).await?;
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

async fn execute(pool: &PgPool, arguments: &[String]) -> Result<Value, Box<dyn Error>> {
    let Some(command) = arguments.first().map(String::as_str) else {
        return Err(usage().into());
    };
    match command {
        "inspect-release" => {
            let (identity, object_id) = inspection_arguments(arguments)?;
            inspect(
                pool,
                &identity,
                ObjectType::Release,
                object_id,
                release_inspection_statement(),
            )
            .await
        }
        "inspect-instance" => {
            let (identity, object_id) = inspection_arguments(arguments)?;
            inspect(
                pool,
                &identity,
                ObjectType::AgentInstance,
                object_id,
                instance_inspection_statement(),
            )
            .await
        }
        "inspect-secret" => {
            let (identity, object_id) = inspection_arguments(arguments)?;
            inspect(
                pool,
                &identity,
                ObjectType::Secret,
                object_id,
                secret_inspection_statement(),
            )
            .await
        }
        "metrics" => {
            let identity = identity_argument(arguments, 1, Some(2))?;
            inspect_metrics(pool, &identity).await
        }
        "recover-update" => recover_update(pool, arguments).await,
        "abandon-build" => abandon_build(pool, arguments).await,
        _ => Err(usage().into()),
    }
}

fn inspection_arguments(
    arguments: &[String],
) -> Result<(AuthenticatedIdentity, Uuid), Box<dyn Error>> {
    Ok((
        identity_argument(arguments, 1, Some(3))?,
        Uuid::parse_str(argument(arguments, 2)?)?,
    ))
}

fn identity_argument(
    arguments: &[String],
    user_index: usize,
    request_index: Option<usize>,
) -> Result<AuthenticatedIdentity, Box<dyn Error>> {
    let user_id = UserId::from_uuid(Uuid::parse_str(argument(arguments, user_index)?)?);
    let request_id = request_index
        .and_then(|index| arguments.get(index))
        .map(|value| Uuid::parse_str(value))
        .transpose()?
        .map_or_else(RequestId::new, RequestId::from_uuid);
    Ok(AuthenticatedIdentity::new(
        user_id,
        "hephaestus-operator",
        user_id.to_string(),
        json!({"interface": "operator_cli"}),
        request_id,
    ))
}

fn argument(arguments: &[String], index: usize) -> Result<&str, Box<dyn Error>> {
    arguments
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| usage().into())
}

async fn inspect(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    object_type: ObjectType,
    object_id: Uuid,
    statement: &str,
) -> Result<Value, Box<dyn Error>> {
    let mut transaction = begin_actor_transaction(pool, identity).await?;
    let authorizer = PostgresMelangeAuthorizer;
    let object = ObjectRef::new(object_type, object_id);
    let permission = if object_type == ObjectType::Secret {
        Permission::InspectMetadata
    } else {
        Permission::CanRead
    };
    let decision = authorizer
        .check(
            &mut transaction,
            Subject::User(identity.user_id),
            permission,
            object,
        )
        .await?;
    audit_decision(
        &mut transaction,
        identity.user_id,
        permission,
        object,
        decision,
        identity.request_id,
    )
    .await?;
    if decision == AuthorizationDecision::Deny {
        transaction.commit().await?;
        return Err("inspection authorization denied".into());
    }
    let value = sqlx::query_scalar::<_, Value>(statement)
        .bind(object_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or("object is unavailable")?;
    if object_type == ObjectType::Secret {
        audit_secret_inspection(&mut transaction, identity, object_id).await?;
    }
    transaction.commit().await?;
    Ok(value)
}

async fn audit_secret_inspection(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    identity: &AuthenticatedIdentity,
    secret_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO secret_audit_events
         (id, owner_organization_id, requester_id, secret_id, operation,
          permission, decision, outcome, request_id, command_id,
          authorization_model_version, policy_version)
         SELECT $1, owner_organization_id, $2, id, 'inspect_metadata',
                'secret.inspect_metadata', 'allow', 'metadata_returned',
                $3, $3, $4, 'operator/v1'
         FROM secrets WHERE id = $5",
    )
    .bind(Uuid::new_v4())
    .bind(identity.user_id.as_uuid())
    .bind(identity.request_id.as_uuid())
    .bind(authz_postgres::AUTHORIZATION_MODEL_VERSION)
    .bind(secret_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn inspect_metrics(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
) -> Result<Value, Box<dyn Error>> {
    let mut transaction = begin_actor_transaction(pool, identity).await?;
    let value = sqlx::query_scalar::<_, Value>(
        "SELECT jsonb_build_object(
             'release', COALESCE(
                 (SELECT to_jsonb(metric) FROM release_operation_metrics metric),
                 '{}'::jsonb
             ),
             'instance', COALESCE(
                 (SELECT to_jsonb(metric) FROM instance_operation_metrics metric),
                 '{}'::jsonb
             ),
             'secret', COALESCE(
                 (SELECT to_jsonb(metric) FROM secret_operation_metrics metric),
                 '{}'::jsonb
             )
         )",
    )
    .fetch_one(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(value)
}

async fn recover_update(pool: &PgPool, arguments: &[String]) -> Result<Value, Box<dyn Error>> {
    let identity = identity_argument(arguments, 1, Some(4))?;
    let update_id = AgentUpdateId::from_str(argument(arguments, 2)?)?;
    let action_name = argument(arguments, 3)?;
    let action = match action_name {
        "retry" => UpdateRecoveryAction::RetryHook,
        "reject" => UpdateRecoveryAction::RejectCandidate,
        "resume" => UpdateRecoveryAction::ResumeActivation,
        _ => return Err("recovery action must be retry, reject, or resume".into()),
    };
    let command_key = ReleaseCommandKey::derive(
        "operator-recover-update",
        &[
            update_id.as_uuid().as_bytes(),
            action_name.as_bytes(),
            identity.request_id.as_uuid().as_bytes(),
        ],
    );
    let service = ReleaseService::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let decision = service
        .recover_update(
            &identity,
            RecoverInstanceUpdate {
                command_key,
                update_id,
                action,
            },
        )
        .await?;
    Ok(json!({
        "update_id": update_id,
        "action": action_name,
        "decision": format!("{decision:?}"),
        "request_id": identity.request_id,
    }))
}

async fn abandon_build(pool: &PgPool, arguments: &[String]) -> Result<Value, Box<dyn Error>> {
    let identity = identity_argument(arguments, 1, Some(3))?;
    let build_id = BuildRequestId::from_str(argument(arguments, 2)?)?;
    let object = ObjectRef::new(ObjectType::Build, build_id.as_uuid());
    let authorizer = PostgresMelangeAuthorizer;
    let mut transaction = begin_actor_transaction(pool, &identity).await?;
    let decision = authorizer
        .check(
            &mut transaction,
            Subject::User(identity.user_id),
            Permission::CanCancel,
            object,
        )
        .await?;
    audit_decision(
        &mut transaction,
        identity.user_id,
        Permission::CanCancel,
        object,
        decision,
        identity.request_id,
    )
    .await?;
    if decision == AuthorizationDecision::Deny {
        transaction.commit().await?;
        return Err("build recovery authorization denied".into());
    }
    let changed = sqlx::query(
        "UPDATE build_requests
         SET state = 'cancelled', completed_at = now(),
             diagnostics = jsonb_build_array(jsonb_build_object(
                 'code', 'operator_abandoned',
                 'request_id', $2::text
             ))
         WHERE id = $1 AND state IN ('queued', 'running', 'importing')",
    )
    .bind(build_id.as_uuid())
    .bind(identity.request_id.as_uuid())
    .execute(&mut *transaction)
    .await?;
    if changed.rows_affected() != 1 {
        return Err("build is unavailable or already terminal".into());
    }
    sqlx::query(
        "UPDATE build_executions
         SET state = 'failed', failure_code = 'operator_abandoned',
             completed_at = now(), updated_at = now()
         WHERE build_request_id = $1 AND state <> 'drafted'",
    )
    .bind(build_id.as_uuid())
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO outbox
         (id, aggregate_type, aggregate_id, subject, event_type, payload, occurred_at)
         VALUES ($1, 'release', $2, 'hephaestus.build.failed.v1',
                 'build.failed.v1', $3, now())",
    )
    .bind(Uuid::new_v4())
    .bind(build_id.as_uuid())
    .bind(json!({
        "schema_version": 1,
        "build_request_id": build_id,
        "code": "operator_abandoned",
        "request_id": identity.request_id,
    }))
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(json!({
        "build_request_id": build_id,
        "state": "cancelled",
        "request_id": identity.request_id,
    }))
}

const fn release_inspection_statement() -> &'static str {
    "SELECT jsonb_build_object(
         'release_id', release.id,
         'state', release.state,
         'repository_id', release.repository_id,
         'source_commit', release.source_commit,
         'source_ref', release.source_ref,
         'build_request_id', release.build_request_id,
         'manifest_hash', encode(release.manifest_hash, 'hex'),
         'artifacts', (
             SELECT COALESCE(jsonb_agg(jsonb_build_object(
                 'id', artifact.id, 'path', artifact.path, 'kind', artifact.kind,
                 'size_bytes', artifact.size_bytes,
                 'content_hash', encode(artifact.content_hash, 'hex')
             ) ORDER BY artifact.path), '[]'::jsonb)
             FROM release_artifacts artifact WHERE artifact.release_id = release.id
         ),
         'bindings', (
             SELECT COALESCE(jsonb_agg(to_jsonb(binding)), '[]'::jsonb)
             FROM release_provenance_inspection binding
             WHERE binding.release_id = release.id
         )
     ) FROM releases release WHERE release.id = $1"
}

const fn instance_inspection_statement() -> &'static str {
    "SELECT jsonb_build_object(
         'instance_id', instance.id,
         'project_id', instance.project_id,
         'state', instance.state,
         'run_gate_open', instance.run_gate_open,
         'active_revision_id', instance.active_revision_id,
         'state_volume_id', instance.state_volume_id,
         'revisions', (
             SELECT COALESCE(jsonb_agg(jsonb_build_object(
                 'id', revision.id,
                 'release_agent_id', revision.release_agent_id,
                 'runnable', revision.runnable,
                 'diagnostics', revision.diagnostics,
                 'platform_policy_version', revision.platform_policy_version
             ) ORDER BY revision.created_at), '[]'::jsonb)
             FROM agent_instance_revisions revision
             WHERE revision.instance_id = instance.id
         ),
         'attachments', (
             SELECT COALESCE(jsonb_agg(jsonb_build_object(
                 'id', attachment.id,
                 'repository_id', attachment.repository_id,
                 'ref_selector', attachment.ref_selector,
                 'enabled', attachment.enabled,
                 'removed_at', attachment.removed_at
             ) ORDER BY attachment.created_at), '[]'::jsonb)
             FROM agent_attachments attachment
             WHERE attachment.instance_id = instance.id
         ),
         'updates', (
             SELECT COALESCE(jsonb_agg(jsonb_build_object(
                 'id', update_record.id,
                 'state', update_record.state,
                 'candidate_revision_id', update_record.candidate_revision_id,
                 'hook_run_id', update_record.hook_run_id,
                 'final_decision', update_record.final_decision
             ) ORDER BY update_record.created_at), '[]'::jsonb)
             FROM agent_updates update_record
             WHERE update_record.instance_id = instance.id
         ),
         'leases', (
             SELECT COALESCE(jsonb_agg(jsonb_build_object(
                 'id', lease.id, 'run_id', lease.run_id,
                 'state', lease.state, 'fencing_token', lease.fencing_token
             ) ORDER BY lease.acquired_at), '[]'::jsonb)
             FROM agent_instance_volume_leases lease
             WHERE lease.instance_id = instance.id
         )
     ) FROM agent_instances instance WHERE instance.id = $1"
}

const fn secret_inspection_statement() -> &'static str {
    "SELECT jsonb_build_object(
         'secret_id', secret.id,
         'owner_organization_id', secret.owner_organization_id,
         'organization_id', secret.organization_id,
         'project_id', secret.project_id,
         'name', secret.name,
         'status', secret.status,
         'allowed_delivery_modes', secret.allowed_delivery_modes,
         'active_version_id', secret.active_version_id,
         'versions', (
             SELECT COALESCE(jsonb_agg(jsonb_build_object(
                 'id', version.id, 'sequence', version.sequence,
                 'status', version.status, 'created_at', version.created_at,
                 'revoked_at', version.revoked_at,
                 'purged_at', version.purged_at
             ) ORDER BY version.sequence), '[]'::jsonb)
             FROM secret_version_metadata version
             WHERE version.secret_id = secret.id
         ),
         'grants', (
             SELECT COALESCE(jsonb_agg(jsonb_build_object(
                 'id', secret_grant.id,
                 'target_kind', secret_grant.target_kind,
                 'target_id', secret_grant.target_id,
                 'status', secret_grant.status,
                 'delivery_modes', secret_grant.delivery_modes,
                 'phases', secret_grant.phases,
                 'expires_at', secret_grant.expires_at
             ) ORDER BY secret_grant.created_at), '[]'::jsonb)
             FROM secret_grants secret_grant
             WHERE secret_grant.secret_id = secret.id
         ),
         'last_use', (
             SELECT jsonb_build_object(
                 'operation', audit.operation,
                 'delivery_mode', audit.delivery_mode,
                 'decision', audit.decision,
                 'outcome', audit.outcome,
                 'runtime_run_id', audit.runtime_run_id,
                 'occurred_at', audit.occurred_at
             )
             FROM secret_audit_events audit
             WHERE audit.secret_id = secret.id
             ORDER BY audit.occurred_at DESC LIMIT 1
         )
     ) FROM secrets secret WHERE secret.id = $1"
}

const fn usage() -> &'static str {
    "usage: hephaestus-operator <inspect-release|inspect-instance|inspect-secret> \
     <actor-uuid> <object-uuid> [request-uuid]\n\
     hephaestus-operator metrics <actor-uuid> [request-uuid]\n\
     hephaestus-operator recover-update <actor-uuid> <update-uuid> \
     <retry|reject|resume> [request-uuid]\n\
     hephaestus-operator abandon-build <actor-uuid> <build-uuid> [request-uuid]"
}
