SELECT jsonb_build_object(
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
) FROM secrets secret WHERE secret.id = $1
