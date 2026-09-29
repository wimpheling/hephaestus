use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use identity_domain::AuthenticatedIdentity;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use std::error::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectionTarget {
    Release,
    AgentInstance,
    Secret,
}

impl InspectionTarget {
    pub const fn object_type(self) -> ObjectType {
        match self {
            Self::Release => ObjectType::Release,
            Self::AgentInstance => ObjectType::AgentInstance,
            Self::Secret => ObjectType::Secret,
        }
    }

    pub const fn permission(self) -> Permission {
        match self {
            Self::Release | Self::AgentInstance => Permission::CanRead,
            Self::Secret => Permission::InspectMetadata,
        }
    }
}

pub async fn inspect(
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
    transaction: &mut Transaction<'_, Postgres>,
    target: InspectionTarget,
    object_id: Uuid,
) -> Result<Option<Value>, sqlx::Error> {
    match target {
        InspectionTarget::Release => {
            sqlx::query_scalar::<_, Value>(include_str!("../../../../sql/operator/release.sql"))
                .bind(object_id)
                .fetch_optional(&mut **transaction)
                .await
        }
        InspectionTarget::AgentInstance => {
            sqlx::query_scalar::<_, Value>(include_str!(
                "../../../../sql/operator/agent_instance.sql"
            ))
            .bind(object_id)
            .fetch_optional(&mut **transaction)
            .await
        }
        InspectionTarget::Secret => {
            sqlx::query_scalar::<_, Value>(include_str!("../../../../sql/operator/secret.sql"))
                .bind(object_id)
                .fetch_optional(&mut **transaction)
                .await
        }
    }
}

async fn audit_secret_inspection(
    transaction: &mut Transaction<'_, Postgres>,
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

pub async fn inspect_metrics(
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
               ),
               'mailbox', COALESCE(
                   (SELECT to_jsonb(metric) FROM mailbox_operation_metrics metric),
                   '{}'::jsonb
               ),
               'gateway', COALESCE(
                   (SELECT to_jsonb(metric) FROM gateway_operation_metrics metric),
                   '{}'::jsonb
               )
           )",
    )
    .fetch_one(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(value)
}
