use super::*;

/// Read-only secret query facade.
pub struct SecretApplication {
    pool: PgPool,
}

impl SecretApplication {
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn list_project_secrets(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
        page: Page,
    ) -> Result<PageResult<SecretSummary>, SecretQueryError> {
        validate_page(page)?;
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(SecretQueryError::Persistence)?;
        let rows = sqlx::query_as::<_, SecretRow>(
            "SELECT secret.id, secret.name, secret.status,
                    secret.allowed_delivery_modes, secret.active_version_id,
                    secret.created_at, secret.updated_at,
                    NULL::bigint AS active_version_sequence,
                    NULL::timestamptz AS active_version_created_at,
                    (SELECT count(*)::bigint FROM secret_grants AS secret_grant
                     WHERE secret_grant.secret_id = secret.id) AS grant_count,
                    (SELECT count(*)::bigint FROM secret_imports AS imported
                     WHERE imported.secret_id = secret.id) AS import_count,
                    (SELECT count(*)::bigint FROM agent_secret_bindings AS binding
                     JOIN secret_imports AS imported ON imported.id = binding.import_id
                     WHERE imported.secret_id = secret.id) AS binding_count,
                    EXISTS (SELECT 1 FROM agent_secret_bindings AS binding
                            JOIN secret_imports AS imported ON imported.id = binding.import_id
                            WHERE imported.secret_id = secret.id
                              AND binding.delivery_mode = 'raw') AS has_raw_binding,
                    check_permission('user', hephaestus_actor_id(), 'rotate',
                                     'secret', secret.id::text) = 1 AS can_rotate,
                    check_permission('user', hephaestus_actor_id(), 'manage_grants',
                                     'secret', secret.id::text) = 1 AS can_manage_grants,
                    check_permission('user', hephaestus_actor_id(), 'revoke',
                                     'secret', secret.id::text) = 1 AS can_revoke,
                    check_permission('user', hephaestus_actor_id(), 'purge',
                                     'secret', secret.id::text) = 1 AS can_purge
             FROM secrets AS secret
             WHERE secret.project_id = $1
                    AND ($2::uuid IS NULL OR (secret.name, secret.id) > (
                        SELECT cursor.name, cursor.id FROM secrets AS cursor WHERE cursor.id = $2
                    ))
                 ORDER BY secret.name, secret.id
                 LIMIT $3",
        )
        .bind(project_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(SecretQueryError::Persistence)?;
        tx.commit().await.map_err(SecretQueryError::Persistence)?;
        finish_secret_page(rows, page)
    }
    pub async fn list_organization_secrets(
        &self,
        identity: &AuthenticatedIdentity,
        organization_id: Uuid,
        page: Page,
    ) -> Result<PageResult<SecretSummary>, SecretQueryError> {
        validate_page(page)?;
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(SecretQueryError::Persistence)?;
        let rows = sqlx::query_as::<_, SecretRow>(
            "SELECT secret.id, secret.name, secret.status,
                    secret.allowed_delivery_modes, secret.active_version_id,
                    secret.created_at, secret.updated_at,
                    NULL::bigint AS active_version_sequence,
                    NULL::timestamptz AS active_version_created_at,
                    (SELECT count(*)::bigint FROM secret_grants AS secret_grant
                     WHERE secret_grant.secret_id = secret.id) AS grant_count,
                    (SELECT count(*)::bigint FROM secret_imports AS imported
                     WHERE imported.secret_id = secret.id) AS import_count,
                    (SELECT count(*)::bigint FROM agent_secret_bindings AS binding
                     JOIN secret_imports AS imported ON imported.id = binding.import_id
                     WHERE imported.secret_id = secret.id) AS binding_count,
                    EXISTS (SELECT 1 FROM agent_secret_bindings AS binding
                            JOIN secret_imports AS imported ON imported.id = binding.import_id
                            WHERE imported.secret_id = secret.id
                              AND binding.delivery_mode = 'raw') AS has_raw_binding,
                    check_permission('user', hephaestus_actor_id(), 'rotate',
                                     'secret', secret.id::text) = 1 AS can_rotate,
                    check_permission('user', hephaestus_actor_id(), 'manage_grants',
                                     'secret', secret.id::text) = 1 AS can_manage_grants,
                    check_permission('user', hephaestus_actor_id(), 'revoke',
                                     'secret', secret.id::text) = 1 AS can_revoke,
                    check_permission('user', hephaestus_actor_id(), 'purge',
                                     'secret', secret.id::text) = 1 AS can_purge
             FROM secrets AS secret
             WHERE secret.organization_id = $1
                    AND ($2::uuid IS NULL OR (secret.name, secret.id) > (
                        SELECT cursor.name, cursor.id FROM secrets AS cursor WHERE cursor.id = $2
                    ))
                 ORDER BY secret.name, secret.id
                 LIMIT $3",
        )
        .bind(organization_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(SecretQueryError::Persistence)?;
        tx.commit().await.map_err(SecretQueryError::Persistence)?;
        finish_secret_page(rows, page)
    }
    pub async fn list_organization_grants(
        &self,
        identity: &AuthenticatedIdentity,
        organization_id: Uuid,
        page: Page,
    ) -> Result<PageResult<GrantSummary>, SecretQueryError> {
        validate_page(page)?;
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(SecretQueryError::Persistence)?;
        let rows = sqlx::query_as::<_, GrantRow>(
            "SELECT secret_grant.id, secret_grant.secret_id,
                    secret.name AS secret_name, secret_grant.target_kind, secret_grant.target_id,
                    COALESCE(project.name, repository.name) AS target_name,
                    secret_grant.delivery_modes, secret_grant.phases, secret_grant.destinations,
                    secret_grant.expires_at, secret_grant.status, secret_grant.created_at,
                    (SELECT count(*)::bigint FROM secret_imports AS imported
                     WHERE imported.grant_id = secret_grant.id) AS import_count,
                    latest_import.id AS import_id, latest_import.alias AS import_alias,
                    latest_import.status AS import_status
             FROM secret_grants AS secret_grant
             JOIN secrets AS secret ON secret.id = secret_grant.secret_id
             LEFT JOIN projects AS project
                    ON secret_grant.target_kind = 'project' AND project.id = secret_grant.target_id
             LEFT JOIN repositories AS repository
                    ON secret_grant.target_kind = 'repository' AND repository.id = secret_grant.target_id
             LEFT JOIN LATERAL (
                    SELECT imported.id, imported.alias, imported.status
                    FROM secret_imports AS imported
                    WHERE imported.grant_id = secret_grant.id
                    ORDER BY imported.accepted_at DESC, imported.id DESC
                    LIMIT 1
             ) AS latest_import ON true
             WHERE secret_grant.owner_organization_id = $1
                    AND ($2::uuid IS NULL OR (secret.name, secret_grant.created_at, secret_grant.id) > (
                        SELECT cursor_secret.name, cursor.created_at, cursor.id
                        FROM secret_grants AS cursor
                        JOIN secrets AS cursor_secret ON cursor_secret.id = cursor.secret_id
                        WHERE cursor.id = $2
                    ))
                 ORDER BY secret.name, secret_grant.created_at, secret_grant.id
                 LIMIT $3",
        )
        .bind(organization_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(SecretQueryError::Persistence)?;
        tx.commit().await.map_err(SecretQueryError::Persistence)?;
        finish_grant_page(rows, page)
    }
    pub async fn project_authority(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
        grants_page: Page,
        imports_page: Page,
    ) -> Result<ProjectAuthority, SecretQueryError> {
        validate_page(grants_page)?;
        validate_page(imports_page)?;
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(SecretQueryError::Persistence)?;
        let grants = sqlx::query_as::<_, GrantRow>(
            "SELECT secret_grant.id, secret_grant.secret_id,
                    secret.name AS secret_name, secret_grant.target_kind, secret_grant.target_id,
                    COALESCE(project.name, repository.name) AS target_name,
                    secret_grant.delivery_modes, secret_grant.phases, secret_grant.destinations,
                    secret_grant.expires_at, secret_grant.status, secret_grant.created_at,
                    (SELECT count(*)::bigint FROM secret_imports AS imported
                     WHERE imported.grant_id = secret_grant.id) AS import_count,
                    latest_import.id AS import_id, latest_import.alias AS import_alias,
                    latest_import.status AS import_status
             FROM secret_grants AS secret_grant
             JOIN secrets AS secret ON secret.id = secret_grant.secret_id
             LEFT JOIN projects AS project
                    ON secret_grant.target_kind = 'project' AND project.id = secret_grant.target_id
             LEFT JOIN repositories AS repository
                    ON secret_grant.target_kind = 'repository' AND repository.id = secret_grant.target_id
             LEFT JOIN LATERAL (
                    SELECT imported.id, imported.alias, imported.status
                    FROM secret_imports AS imported
                    WHERE imported.grant_id = secret_grant.id
                    ORDER BY imported.accepted_at DESC, imported.id DESC
                    LIMIT 1
             ) AS latest_import ON true
             WHERE secret_grant.target_project_id = $1
                    AND ($2::uuid IS NULL OR (secret.name, secret_grant.created_at, secret_grant.id) > (
                        SELECT cursor_secret.name, cursor.created_at, cursor.id
                        FROM secret_grants AS cursor
                        JOIN secrets AS cursor_secret ON cursor_secret.id = cursor.secret_id
                        WHERE cursor.id = $2
                    ))
                 ORDER BY secret.name, secret_grant.created_at, secret_grant.id
                 LIMIT $3",
        )
        .bind(project_id)
        .bind(grants_page.after)
        .bind(grants_page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(SecretQueryError::Persistence)?;
        let imports = sqlx::query_as::<_, ImportRow>(
            "SELECT imported.id, imported.alias, imported.target_kind,
                    imported.target_id, imported.status, imported.secret_id,
                    secret.name AS secret_name, secret.status AS secret_status,
                    secret.active_version_id,
                    secret.allowed_delivery_modes AS delivery_modes,
                    secret_grant.phases, secret_grant.destinations, secret_grant.expires_at
             FROM secret_imports AS imported
             JOIN secret_grants AS secret_grant ON secret_grant.id = imported.grant_id
             JOIN secrets AS secret ON secret.id = imported.secret_id
             WHERE imported.target_kind = 'project' AND imported.target_id = $1
               AND ($2::uuid IS NULL OR (imported.alias, imported.id) > (
                   SELECT cursor.alias, cursor.id FROM secret_imports AS cursor
                   WHERE cursor.id = $2
               ))
             ORDER BY imported.alias, imported.id
             LIMIT $3",
        )
        .bind(project_id)
        .bind(imports_page.after)
        .bind(imports_page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(SecretQueryError::Persistence)?;
        tx.commit().await.map_err(SecretQueryError::Persistence)?;
        Ok(ProjectAuthority {
            grants: finish_grant_page(grants, grants_page)?,
            imports: finish_import_page(imports, imports_page)?,
        })
    }
}
