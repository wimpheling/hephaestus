use super::{
    AuthenticatedIdentity, ObjectRef, ObjectType, Permission, ReleaseCommandKey, ReleaseId,
    ReleaseService, ReleaseServiceError, append_event, begin_actor_transaction, existing_command,
    json, record_command,
};

impl ReleaseService {
    /// Explicitly publishes and freezes one complete draft release.
    ///
    /// # Errors
    ///
    /// Fails for denial, missing/incomplete draft, idempotency conflict, or
    /// database failure.
    #[tracing::instrument(
        skip_all,
        fields(actor_id = %identity.user_id, request_id = %identity.request_id, %release_id)
    )]
    pub async fn publish(
        &self,
        identity: &AuthenticatedIdentity,
        command_key: ReleaseCommandKey,
        release_id: ReleaseId,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanPublish,
            ObjectRef::new(ObjectType::Release, release_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command_key, "publish")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let changed = sqlx::query(
            "UPDATE releases SET state = 'published',
                    publication_actor_id = $2, published_at = now()
             WHERE id = $1 AND state = 'draft'
               AND EXISTS (
                    SELECT 1 FROM release_artifacts WHERE release_id = $1
               )
               AND EXISTS (
                    SELECT 1 FROM release_agents WHERE release_id = $1
               )",
        )
        .bind(release_id.as_uuid())
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(ReleaseServiceError::Unavailable);
        }
        record_command(
            &mut tx,
            command_key,
            "publish",
            release_id.as_uuid(),
            None,
            Some(identity),
        )
        .await?;
        append_event(
            &mut tx,
            release_id.as_uuid(),
            "hephaestus.release.published.v1",
            "release.published.v1",
            json!({"schema_version": 1, "release_id": release_id}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Revokes a published release without deleting immutable provenance.
    ///
    /// Historical instances, revisions, runs, results, and artifacts retain
    /// their exact foreign-key targets, while new imports and guest starts
    /// reject the no-longer-published release.
    ///
    /// # Errors
    ///
    /// Fails for denial, a non-published release, idempotency conflict, or a
    /// database error.
    #[tracing::instrument(
        skip_all,
        fields(actor_id = %identity.user_id, request_id = %identity.request_id, %release_id)
    )]
    pub async fn revoke(
        &self,
        identity: &AuthenticatedIdentity,
        command_key: ReleaseCommandKey,
        release_id: ReleaseId,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRevoke,
            ObjectRef::new(ObjectType::Release, release_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command_key, "revoke")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let changed = sqlx::query(
            "UPDATE releases
             SET state = 'revoked', revoked_at = now()
             WHERE id = $1 AND state = 'published'",
        )
        .bind(release_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(ReleaseServiceError::Unavailable);
        }
        record_command(
            &mut tx,
            command_key,
            "revoke",
            release_id.as_uuid(),
            None,
            Some(identity),
        )
        .await?;
        append_event(
            &mut tx,
            release_id.as_uuid(),
            "hephaestus.release.revoked.v1",
            "release.revoked.v1",
            json!({"release_id": release_id}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
