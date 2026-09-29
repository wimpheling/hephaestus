use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use release_domain::{AgentUpdateId, BuildRequestId, ReleaseCommandKey};
use release_postgres::{RecoverInstanceUpdate, ReleaseService, UpdateRecoveryAction};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{error::Error, str::FromStr, sync::Arc};

pub async fn recover_update(pool: &PgPool, arguments: &[String]) -> Result<Value, Box<dyn Error>> {
    let identity = super::cli::identity_argument(arguments, 1, Some(4))?;
    let update_id = AgentUpdateId::from_str(super::cli::argument(arguments, 2)?)?;
    let action_name = super::cli::argument(arguments, 3)?;
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

pub async fn abandon_build(pool: &PgPool, arguments: &[String]) -> Result<Value, Box<dyn Error>> {
    let identity = super::cli::identity_argument(arguments, 1, Some(3))?;
    let build_id = BuildRequestId::from_str(super::cli::argument(arguments, 2)?)?;
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
