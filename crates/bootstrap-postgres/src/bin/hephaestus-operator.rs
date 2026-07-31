//! Authorization-aware read-only inspection and narrowly scoped recovery CLI.

use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::{AgentUpdateId, BuildRequestId, ReleaseCommandKey};
use release_postgres::{RecoverInstanceUpdate, ReleaseService, UpdateRecoveryAction};
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
            inspect(pool, &identity, InspectionTarget::Release, object_id).await
        }
        "inspect-instance" => {
            let (identity, object_id) = inspection_arguments(arguments)?;
            inspect(pool, &identity, InspectionTarget::AgentInstance, object_id).await
        }
        "inspect-secret" => {
            let (identity, object_id) = inspection_arguments(arguments)?;
            inspect(pool, &identity, InspectionTarget::Secret, object_id).await
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InspectionTarget {
    Release,
    AgentInstance,
    Secret,
}

impl InspectionTarget {
    const fn object_type(self) -> ObjectType {
        match self {
            Self::Release => ObjectType::Release,
            Self::AgentInstance => ObjectType::AgentInstance,
            Self::Secret => ObjectType::Secret,
        }
    }

    const fn permission(self) -> Permission {
        match self {
            Self::Release | Self::AgentInstance => Permission::CanRead,
            Self::Secret => Permission::InspectMetadata,
        }
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
    target: InspectionTarget,
    object_id: Uuid,
) -> Result<Value, Box<dyn Error>> {
    let mut transaction = begin_actor_transaction(pool, identity).await?;
    let authorizer = PostgresMelangeAuthorizer;
    let object_type = target.object_type();
    let object = ObjectRef::new(object_type, object_id);
    let permission = target.permission();
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
    let value = load_inspection(&mut transaction, target, object_id)
        .await?
        .ok_or("object is unavailable")?;
    if target == InspectionTarget::Secret {
        audit_secret_inspection(&mut transaction, identity, object_id).await?;
    }
    transaction.commit().await?;
    Ok(value)
}

async fn load_inspection(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    target: InspectionTarget,
    object_id: Uuid,
) -> Result<Option<Value>, sqlx::Error> {
    match target {
        InspectionTarget::Release => {
            sqlx::query_scalar::<_, Value>(include_str!(
                "../../../hephaestus-app/sql/operator/release.sql"
            ))
            .bind(object_id)
            .fetch_optional(&mut **transaction)
            .await
        }
        InspectionTarget::AgentInstance => {
            sqlx::query_scalar::<_, Value>(include_str!(
                "../../../hephaestus-app/sql/operator/agent_instance.sql"
            ))
            .bind(object_id)
            .fetch_optional(&mut **transaction)
            .await
        }
        InspectionTarget::Secret => {
            sqlx::query_scalar::<_, Value>(include_str!(
                "../../../hephaestus-app/sql/operator/secret.sql"
            ))
            .bind(object_id)
            .fetch_optional(&mut **transaction)
            .await
        }
    }
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
    transaction.commit().await?;
    Ok(json!({
        "build_request_id": build_id,
        "state": "cancelled",
        "request_id": identity.request_id,
    }))
}

const fn usage() -> &'static str {
    "usage: hephaestus-operator <inspect-release|inspect-instance|inspect-secret> \
       <actor-uuid> <object-uuid> [request-uuid]\n\
       hephaestus-operator metrics <actor-uuid> [request-uuid]\n\
       hephaestus-operator recover-update <actor-uuid> <update-uuid> \
       <retry|reject|resume> [request-uuid]\n\
       hephaestus-operator abandon-build <actor-uuid> <build-uuid> [request-uuid]"
}

#[cfg(test)]
mod tests {
    use super::{InspectionTarget, ObjectType, Permission};

    #[test]
    fn inspection_targets_have_closed_authorization_policies() {
        assert_eq!(InspectionTarget::Release.object_type(), ObjectType::Release);
        assert_eq!(InspectionTarget::Release.permission(), Permission::CanRead);
        assert_eq!(
            InspectionTarget::AgentInstance.object_type(),
            ObjectType::AgentInstance
        );
        assert_eq!(
            InspectionTarget::AgentInstance.permission(),
            Permission::CanRead
        );
        assert_eq!(InspectionTarget::Secret.object_type(), ObjectType::Secret);
        assert_eq!(
            InspectionTarget::Secret.permission(),
            Permission::InspectMetadata
        );
    }
}
