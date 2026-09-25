//! Shared authorization and audit transaction helper.

use super::{
    AuthenticatedIdentity, AuthorizationDecision, ObjectRef, Permission, Postgres, ReleaseService,
    ReleaseServiceError, Subject, Transaction, audit_decision, begin_actor_transaction,
};

impl ReleaseService {
    /// Checks and audits a command authorization decision in its transaction.
    ///
    /// # Errors
    ///
    /// Returns authorization, audit, or database failures.
    pub async fn require(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        permission: Permission,
        object: ObjectRef,
    ) -> Result<(), ReleaseServiceError> {
        let decision = self
            .authorizer
            .check(tx, Subject::User(identity.user_id), permission, object)
            .await?;
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
            // The command transaction will roll back on denial. Persist the
            // denial independently so rejected privileged attempts remain
            // observable without committing any command-side state.
            let mut audit_tx = begin_actor_transaction(&self.pool, identity).await?;
            audit_decision(
                &mut audit_tx,
                identity.user_id,
                permission,
                object,
                decision,
                identity.request_id,
            )
            .await?;
            audit_tx.commit().await?;
            Err(ReleaseServiceError::AuthorizationDenied)
        }
    }
}
