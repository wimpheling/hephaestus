-- Forced RLS for reusable releases, instances, and secret metadata. The
-- trusted worker remains BYPASSRLS; agent-facing broker requests use
-- `hephaestus_app` with subject_type=run and an exact run identifier.

ALTER FUNCTION check_permission(text, text, text, text, text)
    SECURITY DEFINER;
ALTER FUNCTION check_permission(text, text, text, text, text)
    SET search_path = pg_catalog, public;
ALTER FUNCTION check_permission(text, text, text, text, text)
    OWNER TO hephaestus_authz_owner;
REVOKE ALL ON FUNCTION check_permission(text, text, text, text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION check_permission(text, text, text, text, text)
    TO hephaestus_app, hephaestus_worker;

CREATE FUNCTION hephaestus_subject_type() RETURNS text
LANGUAGE sql
STABLE
AS $$
    SELECT COALESCE(
        NULLIF(current_setting('hephaestus.subject_type', true), ''),
        'user'
    )
$$;
REVOKE ALL ON FUNCTION hephaestus_subject_type() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION hephaestus_subject_type() TO hephaestus_app;

CREATE POLICY organization_secret_managers_select
    ON organization_secret_managers FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_inspect_secrets',
        'organization', organization_id::text
    ) = 1);
CREATE POLICY organization_secret_managers_write
    ON organization_secret_managers
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'organization', organization_id::text
    ) = 1)
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'organization', organization_id::text
    ) = 1);
ALTER TABLE organization_secret_managers ENABLE ROW LEVEL SECURITY;
ALTER TABLE organization_secret_managers FORCE ROW LEVEL SECURITY;

CREATE POLICY project_secret_roles_select
    ON project_secret_roles FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'project', project_id::text
    ) = 1);
CREATE POLICY project_secret_roles_write
    ON project_secret_roles
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'project', project_id::text
    ) = 1)
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'project', project_id::text
    ) = 1);
ALTER TABLE project_secret_roles ENABLE ROW LEVEL SECURITY;
ALTER TABLE project_secret_roles FORCE ROW LEVEL SECURITY;

CREATE POLICY repository_managers_select ON repository_managers FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'repository', repository_id::text
    ) = 1);
CREATE POLICY repository_managers_write ON repository_managers
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_write',
        'project', (
            SELECT project_id::text FROM repositories
            WHERE repositories.id = repository_managers.repository_id
        )
    ) = 1)
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_write',
        'project', (
            SELECT project_id::text FROM repositories
            WHERE repositories.id = repository_managers.repository_id
        )
    ) = 1);
ALTER TABLE repository_managers ENABLE ROW LEVEL SECURITY;
ALTER TABLE repository_managers FORCE ROW LEVEL SECURITY;

CREATE POLICY repository_secret_roles_select
    ON repository_secret_roles FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_write',
        'repository', repository_id::text
    ) = 1);
CREATE POLICY repository_secret_roles_write
    ON repository_secret_roles
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_write',
        'repository', repository_id::text
    ) = 1)
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_write',
        'repository', repository_id::text
    ) = 1);
ALTER TABLE repository_secret_roles ENABLE ROW LEVEL SECURITY;
ALTER TABLE repository_secret_roles FORCE ROW LEVEL SECURITY;

CREATE POLICY agent_families_select ON agent_families FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'repository', repository_id::text
    ) = 1);
CREATE POLICY agent_families_insert ON agent_families FOR INSERT
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_write',
        'repository', repository_id::text
    ) = 1);

CREATE POLICY build_requests_select ON build_requests FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read', 'build', id::text
    ) = 1);
CREATE POLICY build_requests_insert ON build_requests FOR INSERT
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_write',
        'repository', repository_id::text
    ) = 1);
CREATE POLICY build_requests_update ON build_requests FOR UPDATE
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_execute', 'build', id::text
    ) = 1)
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_execute', 'build', id::text
    ) = 1);

CREATE POLICY build_request_sources_select ON build_request_sources FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'build', build_request_id::text
    ) = 1);
CREATE POLICY build_request_sources_insert ON build_request_sources FOR INSERT
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_execute',
        'build', build_request_id::text
    ) = 1);

CREATE POLICY releases_select ON releases FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read', 'release', id::text
    ) = 1);
CREATE POLICY releases_insert ON releases FOR INSERT
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_write',
        'repository', repository_id::text
    ) = 1);
CREATE POLICY releases_update ON releases FOR UPDATE
    USING (
        check_permission(
            'user', hephaestus_actor_id(), 'can_publish', 'release', id::text
        ) = 1
        OR check_permission(
            'user', hephaestus_actor_id(), 'can_revoke', 'release', id::text
        ) = 1
    )
    WITH CHECK (
        check_permission(
            'user', hephaestus_actor_id(), 'can_publish', 'release', id::text
        ) = 1
        OR check_permission(
            'user', hephaestus_actor_id(), 'can_revoke', 'release', id::text
        ) = 1
    );

CREATE POLICY release_artifacts_select ON release_artifacts FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'release', release_id::text
    ) = 1);
CREATE POLICY release_artifacts_insert ON release_artifacts FOR INSERT
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_publish',
        'release', release_id::text
    ) = 1);

CREATE POLICY release_agents_select ON release_agents FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'release_agent', id::text
    ) = 1);
CREATE POLICY release_agents_insert ON release_agents FOR INSERT
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_publish',
        'release', release_id::text
    ) = 1);

CREATE POLICY agent_instances_select ON agent_instances FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'agent_instance', id::text
    ) = 1);
CREATE POLICY agent_instances_insert ON agent_instances FOR INSERT
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'project', project_id::text
    ) = 1);
CREATE POLICY agent_instances_update ON agent_instances FOR UPDATE
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'agent_instance', id::text
    ) = 1)
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'agent_instance', id::text
    ) = 1);

CREATE POLICY agent_instance_revisions_select
    ON agent_instance_revisions FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'agent_instance', instance_id::text
    ) = 1);
CREATE POLICY agent_instance_revisions_insert
    ON agent_instance_revisions FOR INSERT
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'agent_instance', instance_id::text
    ) = 1);

CREATE POLICY agent_instance_state_volumes_select
    ON agent_instance_state_volumes FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'agent_instance', instance_id::text
    ) = 1);
CREATE POLICY agent_instance_state_volumes_write
    ON agent_instance_state_volumes
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_update',
        'agent_instance', instance_id::text
    ) = 1)
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_update',
        'agent_instance', instance_id::text
    ) = 1);

CREATE POLICY agent_instance_volume_leases_select
    ON agent_instance_volume_leases FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'agent_instance', instance_id::text
    ) = 1);
CREATE POLICY agent_instance_volume_leases_write
    ON agent_instance_volume_leases
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_update',
        'agent_instance', instance_id::text
    ) = 1)
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_update',
        'agent_instance', instance_id::text
    ) = 1);

CREATE POLICY agent_attachments_select ON agent_attachments FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'agent_attachment', id::text
    ) = 1);
CREATE POLICY agent_attachments_insert ON agent_attachments FOR INSERT
    WITH CHECK (
        check_permission(
            'user', hephaestus_actor_id(), 'can_manage',
            'agent_instance', instance_id::text
        ) = 1
        AND check_permission(
            'user', hephaestus_actor_id(), 'can_write',
            'repository', repository_id::text
        ) = 1
    );
CREATE POLICY agent_attachments_update ON agent_attachments FOR UPDATE
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'agent_attachment', id::text
    ) = 1)
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'agent_attachment', id::text
    ) = 1);

DROP POLICY run_requests_insert ON run_requests;
CREATE POLICY run_requests_insert ON run_requests FOR INSERT
    WITH CHECK (
        check_permission(
            'user', hephaestus_actor_id(), 'can_write',
            'repository', repository_id::text
        ) = 1
        AND request_kind = 'instance_normal'
        AND check_permission(
            'user', hephaestus_actor_id(), 'can_execute',
            'agent_attachment', attachment_id::text
        ) = 1
        AND check_permission(
            'user', hephaestus_actor_id(), 'can_use',
            'release_agent', release_agent_id::text
        ) = 1
    );

DROP POLICY IF EXISTS runs_insert ON runs;
DROP POLICY IF EXISTS runs_delete ON runs;
CREATE POLICY runs_insert ON runs FOR INSERT
    WITH CHECK (
        check_permission(
            'user', hephaestus_actor_id(), 'can_execute',
            'agent_instance', instance_id::text
        ) = 1
        AND check_permission(
            'user', hephaestus_actor_id(), 'can_use',
            'release_agent', release_agent_id::text
        ) = 1
        AND (
            run_kind = 'update'
            OR (
                run_kind = 'normal'
                AND check_permission(
                    'user', hephaestus_actor_id(), 'can_execute',
                    'agent_attachment', attachment_id::text
                ) = 1
            )
        )
    );
-- Historical runs are immutable audit provenance and are never hard-deleted.
CREATE POLICY runs_delete ON runs FOR DELETE USING (false);

DROP POLICY IF EXISTS control_requests_insert ON control_requests;
CREATE POLICY control_requests_insert ON control_requests FOR INSERT
    WITH CHECK (
        actor_id::text = hephaestus_actor_id()
        AND (
            (
                kind IN ('approve_result', 'reject_result')
                AND check_permission(
                    'user', hephaestus_actor_id(), 'can_write',
                    'repository', repository_id::text
                ) = 1
            )
            OR (
                kind = 'cancel_run'
                AND EXISTS (
                    SELECT 1 FROM runs
                    WHERE runs.id = control_requests.run_id
                      AND check_permission(
                          'user', hephaestus_actor_id(), 'can_cancel',
                          'run', runs.id::text
                      ) = 1
                )
            )
            OR (
                kind = 'retry_run'
                AND EXISTS (
                    SELECT 1 FROM runs
                    WHERE runs.id = control_requests.run_id
                      AND check_permission(
                          'user', hephaestus_actor_id(), 'can_execute',
                          'agent_instance', runs.instance_id::text
                      ) = 1
                )
            )
        )
    );

CREATE POLICY agent_updates_select ON agent_updates FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'agent_update', id::text
    ) = 1);
CREATE POLICY agent_updates_insert ON agent_updates FOR INSERT
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_update',
        'agent_instance', instance_id::text
    ) = 1);
CREATE POLICY agent_updates_update ON agent_updates FOR UPDATE
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_update',
        'agent_instance', instance_id::text
    ) = 1)
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_update',
        'agent_instance', instance_id::text
    ) = 1);

CREATE POLICY deferred_agent_triggers_select
    ON deferred_agent_triggers FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'agent_instance', instance_id::text
    ) = 1);
CREATE POLICY agent_instance_events_select
    ON agent_instance_events FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'agent_instance', instance_id::text
    ) = 1);

CREATE POLICY secrets_select ON secrets FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'inspect_metadata',
        'secret', id::text
    ) = 1);
CREATE POLICY secrets_insert ON secrets FOR INSERT
    WITH CHECK (
        (
            organization_id IS NOT NULL
            AND check_permission(
                'user', hephaestus_actor_id(), 'can_write_secret_value',
                'organization', organization_id::text
            ) = 1
        )
        OR (
            project_id IS NOT NULL
            AND check_permission(
                'user', hephaestus_actor_id(), 'can_write_secret_value',
                'project', project_id::text
            ) = 1
        )
    );
CREATE POLICY secrets_update ON secrets FOR UPDATE
    USING (
        check_permission(
            'user', hephaestus_actor_id(), 'write_value', 'secret', id::text
        ) = 1
        OR check_permission(
            'user', hephaestus_actor_id(), 'rotate', 'secret', id::text
        ) = 1
        OR check_permission(
            'user', hephaestus_actor_id(), 'revoke', 'secret', id::text
        ) = 1
        OR check_permission(
            'user', hephaestus_actor_id(), 'purge', 'secret', id::text
        ) = 1
    )
    WITH CHECK (
        check_permission(
            'user', hephaestus_actor_id(), 'write_value', 'secret', id::text
        ) = 1
        OR check_permission(
            'user', hephaestus_actor_id(), 'rotate', 'secret', id::text
        ) = 1
        OR check_permission(
            'user', hephaestus_actor_id(), 'revoke', 'secret', id::text
        ) = 1
        OR check_permission(
            'user', hephaestus_actor_id(), 'purge', 'secret', id::text
        ) = 1
    );

CREATE POLICY secret_grants_select ON secret_grants FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'inspect_metadata',
        'secret_grant', id::text
    ) = 1);
CREATE POLICY secret_grants_insert ON secret_grants FOR INSERT
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'manage_grants',
        'secret', secret_id::text
    ) = 1);
CREATE POLICY secret_grants_update ON secret_grants FOR UPDATE
    USING (check_permission(
        'user', hephaestus_actor_id(), 'manage',
        'secret_grant', id::text
    ) = 1)
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'manage',
        'secret_grant', id::text
    ) = 1);

CREATE POLICY secret_imports_select ON secret_imports FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'inspect_metadata',
        'secret_import', id::text
    ) = 1);
CREATE POLICY secret_imports_insert ON secret_imports FOR INSERT
    WITH CHECK (
        (
            target_kind = 'project'
            AND check_permission(
                'user', hephaestus_actor_id(), 'can_accept_secret_import',
                'project', target_id::text
            ) = 1
        )
        OR (
            target_kind = 'repository'
            AND check_permission(
                'user', hephaestus_actor_id(), 'can_accept_secret_import',
                'repository', target_id::text
            ) = 1
        )
    );
CREATE POLICY secret_imports_update ON secret_imports FOR UPDATE
    USING (check_permission(
        'user', hephaestus_actor_id(), 'accept',
        'secret_import', id::text
    ) = 1)
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'accept',
        'secret_import', id::text
    ) = 1);

CREATE POLICY agent_secret_bindings_select
    ON agent_secret_bindings FOR SELECT
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'agent_secret_binding', id::text
    ) = 1);
CREATE POLICY agent_secret_bindings_insert
    ON agent_secret_bindings FOR INSERT
    WITH CHECK (
        (
            delivery_mode = 'brokered'
            AND check_permission(
                'user', hephaestus_actor_id(), 'bind_brokered',
                'secret_import', import_id::text
            ) = 1
        )
        OR (
            delivery_mode = 'raw'
            AND check_permission(
                'user', hephaestus_actor_id(), 'bind_raw',
                'secret_import', import_id::text
            ) = 1
        )
    );
CREATE POLICY agent_secret_bindings_update
    ON agent_secret_bindings FOR UPDATE
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'agent_secret_binding', id::text
    ) = 1)
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'agent_secret_binding', id::text
    ) = 1);

CREATE POLICY run_secret_provenance_select
    ON run_secret_provenance FOR SELECT
    USING (
        (
            hephaestus_subject_type() = 'user'
            AND check_permission(
                'user', hephaestus_actor_id(), 'can_read',
                'run', run_id::text
            ) = 1
        )
        OR (
            hephaestus_subject_type() = 'run'
            AND hephaestus_actor_id() = run_id::text
        )
    );

CREATE POLICY run_instance_provenance_select
    ON run_instance_provenance FOR SELECT
    USING (
        (
            hephaestus_subject_type() = 'user'
            AND check_permission(
                'user', hephaestus_actor_id(), 'can_read',
                'run', run_id::text
            ) = 1
        )
        OR (
            hephaestus_subject_type() = 'run'
            AND hephaestus_actor_id() = run_id::text
        )
    );

CREATE POLICY secret_leases_runtime_select ON secret_leases FOR SELECT
    USING (
        (
            delivery_mode = 'brokered'
            AND check_permission(
                hephaestus_subject_type(), hephaestus_actor_id(),
                'use_brokered', 'secret_lease', id::text
            ) = 1
        )
        OR (
            delivery_mode = 'raw'
            AND check_permission(
                hephaestus_subject_type(), hephaestus_actor_id(),
                'receive_raw', 'secret_lease', id::text
            ) = 1
        )
    );

-- Runtime credential hashes are authentication material, not metadata. The
-- ordinary application role can only validate one presented hash through this
-- narrow security-definer function and cannot list session rows.
REVOKE ALL ON secret_runtime_sessions FROM hephaestus_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON secret_runtime_sessions
    TO hephaestus_worker;
CREATE FUNCTION authenticate_secret_runtime(p_credential_hash bytea)
RETURNS TABLE (
    session_id uuid,
    run_id uuid,
    instance_id uuid,
    instance_revision_id uuid,
    attachment_id uuid,
    phase text,
    expires_at timestamptz
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_temp
STABLE
AS $$
    SELECT session.id, session.run_id, session.instance_id,
           session.instance_revision_id, session.attachment_id,
           session.phase, session.expires_at
    FROM secret_runtime_sessions AS session
    WHERE session.runtime_credential_hash = p_credential_hash
      AND session.status = 'active'
      AND session.expires_at > now()
$$;
REVOKE ALL ON FUNCTION authenticate_secret_runtime(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION authenticate_secret_runtime(bytea)
    TO hephaestus_app, hephaestus_worker;

CREATE POLICY secret_audit_events_select ON secret_audit_events FOR SELECT
    USING (
        secret_id IS NOT NULL
        AND check_permission(
            'user', hephaestus_actor_id(), 'inspect_metadata',
            'secret', secret_id::text
        ) = 1
    );

-- Policies are intentionally absent for application writes to provenance,
-- leases, deferred triggers, instance events, and secret audit. Those durable
-- transitions are performed by trusted workers after explicit authorization.

INSERT INTO melange_migrations
    (melange_version, schema_checksum, codegen_version, function_names)
SELECT
    '0.8.5',
    '76e7043ed8a534103adff658f24be57485646163ab73a8e33c2dc6d56c91d298',
    '0.8.5',
    array_agg(pg_proc.proname ORDER BY pg_proc.proname)
FROM pg_proc
JOIN pg_namespace ON pg_namespace.oid = pg_proc.pronamespace
WHERE pg_namespace.nspname = 'public'
  AND pg_proc.proname ~ '^(check_|expand_|explain_|list_)';
