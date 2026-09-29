//! `PostgreSQL` gateway installation command orchestration.

use super::installer_command_ledger::{claim_installation_command, installation_command_key};
use super::installer_declaration::{
    PublishedGatewayReleaseRow, install_declaration, installation_hash, parse_manifest,
    require_repository_boundary, resolve_release_agent,
};
use super::{
    GatewayInstallError, InstallGatewayManifest, InstallGatewayManifestResult,
    PostgresGatewayInstaller, PublishedGatewayRelease,
};
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{audit_decision, begin_actor_transaction};
use forge_domain::{ProjectId, RepositoryId};
use identity_domain::AuthenticatedIdentity;
use release_domain::ReleaseId;
use sqlx::{Postgres, Transaction};

impl PostgresGatewayInstaller {
    /// Resolves a published release after checking project management
    /// authority. The returned commit is the immutable source coordinate the
    /// application layer must use to retrieve the repository gateway manifest.
    ///
    /// # Errors
    ///
    /// Returns a safe authorization, publication-state, or persistence
    /// failure. Draft and revoked releases are intentionally indistinguishable
    /// from unavailable release targets.
    pub async fn published_release(
        &self,
        identity: &AuthenticatedIdentity,
        release_id: ReleaseId,
    ) -> Result<PublishedGatewayRelease, GatewayInstallError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        let target = sqlx::query_as::<_, PublishedGatewayReleaseRow>(
            "SELECT release.id, repositories.project_id, release.repository_id,
                    release.source_commit, release.state
             FROM releases AS release
             JOIN repositories ON repositories.id = release.repository_id
             WHERE release.id = $1",
        )
        .bind(release_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GatewayInstallError::Unavailable)?;
        self.require_manage(&mut tx, identity, ProjectId::from_uuid(target.project_id))
            .await?;
        if target.state != "published" {
            return Err(GatewayInstallError::Unavailable);
        }
        tx.commit().await?;
        Ok(PublishedGatewayRelease {
            release_id: ReleaseId::from_uuid(target.id),
            project_id: ProjectId::from_uuid(target.project_id),
            repository_id: RepositoryId::from_uuid(target.repository_id),
            source_commit: target.source_commit,
        })
    }

    /// Parses, authorizes, and installs the exact source manifest.
    ///
    /// Reinstalling an identical declaration selects its existing immutable
    /// revision. Changed source creates a fresh revision and only then makes
    /// it active, so a failure cannot leave a partially installed gateway.
    ///
    /// # Errors
    ///
    /// Returns a safe manifest, authorization, boundary, or persistence
    /// failure without exposing provider configuration or listener state.
    pub async fn install(
        &self,
        identity: &AuthenticatedIdentity,
        command: InstallGatewayManifest,
    ) -> Result<InstallGatewayManifestResult, GatewayInstallError> {
        let config = parse_manifest(&command.manifest)?;
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        // Gateway rows are the receipt-producing aggregate. Mark this
        // mutation so an idempotent reinstall that keeps the same revision
        // still records one scoped product event for its fresh command key.
        sqlx::query("SET LOCAL hephaestus.gateway_install = 'true'")
            .execute(&mut *tx)
            .await?;
        self.require_manage(&mut tx, identity, command.project_id)
            .await?;
        require_repository_boundary(&mut tx, &command).await?;
        let command_key = installation_command_key(identity);
        if let Some(previous) =
            claim_installation_command(&mut tx, command_key, identity, &command).await?
        {
            tx.commit().await?;
            return Ok(InstallGatewayManifestResult { gateways: previous });
        }

        // Application role performs all authorization and command-ledger
        // writes. Immutable gateway materialization is a separate trusted
        // worker transition with the narrow grants declared by the migration.
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *tx)
            .await?;

        let mut installed = Vec::with_capacity(config.gateways.len());
        for configured in config.gateways {
            let declaration = configured
                .to_declaration()
                .map_err(|_| GatewayInstallError::Unavailable)?;
            let declaration_hash = declaration
                .validate()
                .map_err(|_| GatewayInstallError::Unavailable)?;
            // A gateway is always released code, never a floating repository
            // command.  Resolve the symbolic manifest key before writing the
            // immutable revision so later dispatch cannot silently select a
            // different agent in the same release.
            let release_agent = resolve_release_agent(&mut tx, &command, &declaration).await?;
            let normalized_hash =
                installation_hash(declaration_hash, command.release_id, release_agent.id);
            installed.push(
                install_declaration(
                    &mut tx,
                    identity,
                    command.project_id,
                    command.repository_id,
                    command.release_id,
                    release_agent,
                    declaration,
                    normalized_hash,
                )
                .await?,
            );
        }
        for (ordinal, gateway) in installed.iter().enumerate() {
            sqlx::query(
                "INSERT INTO gateway_install_command_results
                    (command_key, ordinal, gateway_id, revision_id)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(command_key.as_bytes().as_slice())
            .bind(i32::try_from(ordinal + 1).map_err(|_| GatewayInstallError::Unavailable)?)
            .bind(gateway.gateway_id.as_uuid())
            .bind(gateway.revision_id.as_uuid())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(InstallGatewayManifestResult {
            gateways: installed,
        })
    }

    async fn require_manage(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        project_id: ProjectId,
    ) -> Result<(), GatewayInstallError> {
        let object = ObjectRef::new(ObjectType::Project, project_id.as_uuid());
        let decision = self
            .authorizer
            .check(
                tx,
                Subject::User(identity.user_id),
                Permission::CanManage,
                object,
            )
            .await
            .map_err(|_| GatewayInstallError::Unavailable)?;
        audit_decision(
            tx,
            identity.user_id,
            Permission::CanManage,
            object,
            decision,
            identity.request_id,
        )
        .await?;
        if decision == AuthorizationDecision::Allow {
            Ok(())
        } else {
            // Preserve denied authorization evidence even though the command
            // transaction rolls back its in-transaction audit row.
            let mut audit_tx = begin_actor_transaction(&self.pool, identity).await?;
            audit_decision(
                &mut audit_tx,
                identity.user_id,
                Permission::CanManage,
                object,
                decision,
                identity.request_id,
            )
            .await?;
            audit_tx.commit().await?;
            Err(GatewayInstallError::AuthorizationDenied)
        }
    }
}
