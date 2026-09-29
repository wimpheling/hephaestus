//! Authorized gateway management adapter state and shared authorization.

use authz_domain::{AuthorizationDecision, ObjectRef, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision};
use identity_domain::AuthenticatedIdentity;
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;

use super::GatewayManagementError;

/// Authorized `PostgreSQL` management query and lifecycle adapter.
#[derive(Clone)]
pub struct PostgresGatewayManagement {
    pub(super) pool: PgPool,
    pub(super) authorizer: Arc<PostgresMelangeAuthorizer>,
}

impl PostgresGatewayManagement {
    /// Creates the management adapter over the control-plane connection pool.
    #[must_use]
    pub const fn new(pool: PgPool, authorizer: Arc<PostgresMelangeAuthorizer>) -> Self {
        Self { pool, authorizer }
    }

    pub(super) async fn require(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        permission: Permission,
        object: ObjectRef,
    ) -> Result<(), GatewayManagementError> {
        let decision = self
            .authorizer
            .check(tx, Subject::User(identity.user_id), permission, object)
            .await
            .map_err(|_| GatewayManagementError::Unavailable)?;
        audit_decision(
            tx,
            identity.user_id,
            permission,
            object,
            decision,
            identity.request_id,
        )
        .await?;
        if decision == AuthorizationDecision::Allow {
            Ok(())
        } else {
            Err(GatewayManagementError::Denied)
        }
    }
}
