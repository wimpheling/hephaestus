DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'hephaestus_authz_owner') THEN
        CREATE ROLE hephaestus_authz_owner NOLOGIN BYPASSRLS;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'hephaestus_app') THEN
        CREATE ROLE hephaestus_app NOLOGIN NOBYPASSRLS;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'hephaestus_worker') THEN
        CREATE ROLE hephaestus_worker NOLOGIN BYPASSRLS;
    END IF;
END
$$;

CREATE TABLE melange_migrations (
    id serial PRIMARY KEY,
    migrated_at timestamptz NOT NULL DEFAULT now(),
    melange_version text NOT NULL,
    schema_checksum varchar(64) NOT NULL,
    codegen_version text NOT NULL,
    function_names text[] NOT NULL,
    function_checksums jsonb NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX idx_melange_migrations_checksum
    ON melange_migrations (schema_checksum, codegen_version);

ALTER FUNCTION check_permission(text, text, text, text, text)
    RENAME TO melange_check_permission;
CREATE FUNCTION check_permission(
    p_subject_type text,
    p_subject_id text,
    p_permission text,
    p_object_type text,
    p_object_id text
) RETURNS integer
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public
SET row_security = off
AS $$
    SELECT public.melange_check_permission(
        p_subject_type, p_subject_id, p_permission, p_object_type, p_object_id
    )
$$;
ALTER FUNCTION check_permission(text, text, text, text, text)
    OWNER TO hephaestus_authz_owner;
REVOKE ALL ON FUNCTION check_permission(text, text, text, text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION check_permission(text, text, text, text, text)
    TO hephaestus_app, hephaestus_worker;

CREATE FUNCTION hephaestus_actor_id() RETURNS text
LANGUAGE sql
STABLE
AS $$
    SELECT NULLIF(current_setting('hephaestus.actor_id', true), '')
$$;
REVOKE ALL ON FUNCTION hephaestus_actor_id() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION hephaestus_actor_id() TO hephaestus_app;

GRANT USAGE ON SCHEMA public TO hephaestus_app, hephaestus_worker;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public
    TO hephaestus_app, hephaestus_worker;
GRANT SELECT ON melange_tuples TO hephaestus_authz_owner;
REVOKE ALL ON melange_migrations FROM hephaestus_app, hephaestus_worker;

INSERT INTO melange_migrations
    (melange_version, schema_checksum, codegen_version, function_names)
SELECT
    '0.8.5',
    '4a71ce11770ac14b15711b5e56fcb2007ec2945014a2fabfb759042c9faea822',
    '0.8.5',
    array_agg(pg_proc.proname ORDER BY pg_proc.proname)
FROM pg_proc
JOIN pg_namespace ON pg_namespace.oid = pg_proc.pronamespace
WHERE pg_namespace.nspname = 'public'
  AND pg_proc.proname ~ '^(check_|expand_|explain_|list_)';

ALTER TABLE organizations ENABLE ROW LEVEL SECURITY;
ALTER TABLE organizations FORCE ROW LEVEL SECURITY;
CREATE POLICY organizations_select ON organizations FOR SELECT
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'organization', id::text) = 1);
CREATE POLICY organizations_insert ON organizations FOR INSERT
    WITH CHECK (false);
CREATE POLICY organizations_update ON organizations FOR UPDATE
    USING (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'organization', id::text) = 1)
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'organization', id::text) = 1);
CREATE POLICY organizations_delete ON organizations FOR DELETE
    USING (check_permission('user', hephaestus_actor_id(), 'can_delete',
        'organization', id::text) = 1);

ALTER TABLE organization_members ENABLE ROW LEVEL SECURITY;
ALTER TABLE organization_members FORCE ROW LEVEL SECURITY;
CREATE POLICY organization_members_select ON organization_members FOR SELECT
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'organization', organization_id::text) = 1);
CREATE POLICY organization_members_insert ON organization_members FOR INSERT
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_manage_members',
        'organization', organization_id::text
    ) = 1);
CREATE POLICY organization_members_update ON organization_members FOR UPDATE
    USING (check_permission('user', hephaestus_actor_id(), 'can_manage_members',
        'organization', organization_id::text) = 1)
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_manage_members',
        'organization', organization_id::text) = 1);
CREATE POLICY organization_members_delete ON organization_members FOR DELETE
    USING (check_permission('user', hephaestus_actor_id(), 'can_manage_members',
        'organization', organization_id::text) = 1);

ALTER TABLE projects ENABLE ROW LEVEL SECURITY;
ALTER TABLE projects FORCE ROW LEVEL SECURITY;
CREATE POLICY projects_select ON projects FOR SELECT
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'project', id::text) = 1);
CREATE POLICY projects_insert ON projects FOR INSERT
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_create_project',
        'organization', organization_id::text) = 1);
CREATE POLICY projects_update ON projects FOR UPDATE
    USING (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'project', id::text) = 1)
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'project', id::text) = 1);
CREATE POLICY projects_delete ON projects FOR DELETE
    USING (check_permission('user', hephaestus_actor_id(), 'can_delete',
        'project', id::text) = 1);

ALTER TABLE project_maintainers ENABLE ROW LEVEL SECURITY;
ALTER TABLE project_maintainers FORCE ROW LEVEL SECURITY;
CREATE POLICY project_maintainers_select ON project_maintainers FOR SELECT
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'project', project_id::text) = 1);
CREATE POLICY project_maintainers_write ON project_maintainers
    USING (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'project', project_id::text) = 1)
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'project', project_id::text) = 1);

ALTER TABLE repositories ENABLE ROW LEVEL SECURITY;
ALTER TABLE repositories FORCE ROW LEVEL SECURITY;
CREATE POLICY repositories_select ON repositories FOR SELECT
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'repository', id::text) = 1);
CREATE POLICY repositories_insert ON repositories FOR INSERT
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_write',
        'project', project_id::text) = 1);
CREATE POLICY repositories_update ON repositories FOR UPDATE
    USING (check_permission('user', hephaestus_actor_id(), 'can_write',
        'repository', id::text) = 1)
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_write',
        'repository', id::text) = 1);
CREATE POLICY repositories_delete ON repositories FOR DELETE
    USING (check_permission('user', hephaestus_actor_id(), 'can_delete',
        'repository', id::text) = 1);

ALTER TABLE agents ENABLE ROW LEVEL SECURITY;
ALTER TABLE agents FORCE ROW LEVEL SECURITY;
CREATE POLICY agents_select ON agents FOR SELECT
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'agent', id::text) = 1);
CREATE POLICY agents_insert ON agents FOR INSERT
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_write',
        'project', project_id::text) = 1);
CREATE POLICY agents_update ON agents FOR UPDATE
    USING (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'agent', id::text) = 1)
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'agent', id::text) = 1);
CREATE POLICY agents_delete ON agents FOR DELETE
    USING (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'agent', id::text) = 1);

ALTER TABLE runs ENABLE ROW LEVEL SECURITY;
ALTER TABLE runs FORCE ROW LEVEL SECURITY;
CREATE POLICY runs_select ON runs FOR SELECT
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'run', id::text) = 1);
CREATE POLICY runs_insert ON runs FOR INSERT
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_execute',
        'agent', agent_id::text) = 1);
CREATE POLICY runs_update ON runs FOR UPDATE
    USING (
        check_permission('user', hephaestus_actor_id(), 'can_cancel',
            'run', id::text) = 1
    )
    WITH CHECK (
        check_permission('user', hephaestus_actor_id(), 'can_cancel',
            'run', id::text) = 1
    );
CREATE POLICY runs_delete ON runs FOR DELETE
    USING (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'agent', agent_id::text) = 1);

ALTER TABLE agent_state_volumes ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_state_volumes FORCE ROW LEVEL SECURITY;
CREATE POLICY state_volumes_select ON agent_state_volumes FOR SELECT
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'state_volume', id::text) = 1);
CREATE POLICY state_volumes_insert ON agent_state_volumes FOR INSERT
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_execute',
        'agent', agent_id::text) = 1);
CREATE POLICY state_volumes_update ON agent_state_volumes FOR UPDATE
    USING (check_permission('user', hephaestus_actor_id(), 'can_attach',
        'state_volume', id::text) = 1)
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_attach',
        'state_volume', id::text) = 1);
CREATE POLICY state_volumes_delete ON agent_state_volumes FOR DELETE
    USING (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'state_volume', id::text) = 1);

ALTER TABLE git_receives ENABLE ROW LEVEL SECURITY;
ALTER TABLE git_receives FORCE ROW LEVEL SECURITY;
CREATE POLICY git_receives_select ON git_receives FOR SELECT
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'repository', repository_id::text) = 1);
CREATE POLICY git_receives_insert ON git_receives FOR INSERT
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_write',
        'repository', repository_id::text) = 1);

ALTER TABLE git_ref_updates ENABLE ROW LEVEL SECURITY;
ALTER TABLE git_ref_updates FORCE ROW LEVEL SECURITY;
CREATE POLICY git_ref_updates_select ON git_ref_updates FOR SELECT
    USING (EXISTS (
        SELECT 1 FROM git_receives
        WHERE git_receives.id = git_ref_updates.receive_id
    ));
CREATE POLICY git_ref_updates_insert ON git_ref_updates FOR INSERT
    WITH CHECK (EXISTS (
        SELECT 1 FROM git_receives
        WHERE git_receives.id = git_ref_updates.receive_id
          AND check_permission('user', hephaestus_actor_id(), 'can_write',
              'repository', git_receives.repository_id::text) = 1
    ));

ALTER TABLE git_refs ENABLE ROW LEVEL SECURITY;
ALTER TABLE git_refs FORCE ROW LEVEL SECURITY;
CREATE POLICY git_refs_select ON git_refs FOR SELECT
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'repository', repository_id::text) = 1);
CREATE POLICY git_refs_write ON git_refs
    USING (check_permission('user', hephaestus_actor_id(), 'can_write',
        'repository', repository_id::text) = 1)
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_write',
        'repository', repository_id::text) = 1);

ALTER TABLE agent_config_revisions ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_config_revisions FORCE ROW LEVEL SECURITY;
CREATE POLICY config_revisions_select ON agent_config_revisions FOR SELECT
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'repository', repository_id::text) = 1);
CREATE POLICY config_revisions_insert ON agent_config_revisions FOR INSERT
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_write',
        'repository', repository_id::text) = 1);

ALTER TABLE run_requests ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_requests FORCE ROW LEVEL SECURITY;
CREATE POLICY run_requests_select ON run_requests FOR SELECT
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'repository', repository_id::text) = 1);
CREATE POLICY run_requests_insert ON run_requests FOR INSERT
    WITH CHECK (
        check_permission('user', hephaestus_actor_id(), 'can_write',
            'repository', repository_id::text) = 1
        AND check_permission('user', hephaestus_actor_id(), 'can_execute',
            'agent', agent_id::text) = 1
    );

ALTER TABLE run_workspaces ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_workspaces FORCE ROW LEVEL SECURITY;
CREATE POLICY run_workspaces_select ON run_workspaces FOR SELECT
    USING (
        check_permission(
            'user', hephaestus_actor_id()::text, 'can_read',
            'run', run_id::text
        ) = 1
    );
CREATE POLICY run_workspaces_write ON run_workspaces
    USING (
        check_permission(
            'user', hephaestus_actor_id()::text, 'can_cancel',
            'run', run_id::text
        ) = 1
    )
    WITH CHECK (
        check_permission(
            'user', hephaestus_actor_id()::text, 'can_cancel',
            'run', run_id::text
        ) = 1
    );

ALTER TABLE run_results ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_results FORCE ROW LEVEL SECURITY;
CREATE POLICY run_results_select ON run_results FOR SELECT
    USING (
        check_permission(
            'user', hephaestus_actor_id()::text, 'can_read',
            'run', run_id::text
        ) = 1
    );
CREATE POLICY run_results_write ON run_results
    USING (
        check_permission(
            'user', hephaestus_actor_id()::text, 'can_cancel',
            'run', run_id::text
        ) = 1
    )
    WITH CHECK (
        check_permission(
            'user', hephaestus_actor_id()::text, 'can_cancel',
            'run', run_id::text
        ) = 1
    );

ALTER TABLE result_artifacts ENABLE ROW LEVEL SECURITY;
ALTER TABLE result_artifacts FORCE ROW LEVEL SECURITY;
CREATE POLICY result_artifacts_select ON result_artifacts FOR SELECT
    USING (
        EXISTS (
            SELECT 1 FROM run_results
            WHERE run_results.id = result_artifacts.result_id
        )
    );
CREATE POLICY result_artifacts_write ON result_artifacts
    USING (
        EXISTS (
            SELECT 1 FROM run_results
            WHERE run_results.id = result_artifacts.result_id
        )
    )
    WITH CHECK (
        EXISTS (
            SELECT 1 FROM run_results
            WHERE run_results.id = result_artifacts.result_id
        )
    );

ALTER TABLE review_proposals ENABLE ROW LEVEL SECURITY;
ALTER TABLE review_proposals FORCE ROW LEVEL SECURITY;
CREATE POLICY review_proposals_select ON review_proposals FOR SELECT
    USING (
        check_permission(
            'user', hephaestus_actor_id(), 'can_read',
            'repository', repository_id::text
        ) = 1
    );

ALTER TABLE control_requests ENABLE ROW LEVEL SECURITY;
ALTER TABLE control_requests FORCE ROW LEVEL SECURITY;
CREATE POLICY control_requests_select ON control_requests FOR SELECT
    USING (
        actor_id::text = hephaestus_actor_id()
        AND check_permission(
            'user', hephaestus_actor_id(), 'can_read',
            'repository', repository_id::text
        ) = 1
    );
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
                          'agent', runs.agent_id::text
                      ) = 1
                )
            )
        )
    );

ALTER TABLE run_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_events FORCE ROW LEVEL SECURITY;
CREATE POLICY run_events_select ON run_events FOR SELECT
    USING (EXISTS (
        SELECT 1 FROM runs WHERE runs.id = run_events.run_id
    ));
CREATE POLICY run_events_write ON run_events
    USING (EXISTS (
        SELECT 1 FROM runs WHERE runs.id = run_events.run_id
    ))
    WITH CHECK (EXISTS (
        SELECT 1 FROM runs WHERE runs.id = run_events.run_id
    ));

ALTER TABLE volume_leases ENABLE ROW LEVEL SECURITY;
ALTER TABLE volume_leases FORCE ROW LEVEL SECURITY;
CREATE POLICY volume_leases_select ON volume_leases FOR SELECT
    USING (EXISTS (
        SELECT 1 FROM agent_state_volumes
        WHERE agent_state_volumes.id = volume_leases.volume_id
    ));
CREATE POLICY volume_leases_write ON volume_leases
    USING (EXISTS (
        SELECT 1 FROM agent_state_volumes
        WHERE agent_state_volumes.id = volume_leases.volume_id
          AND check_permission('user', hephaestus_actor_id(), 'can_attach',
              'state_volume', agent_state_volumes.id::text) = 1
    ))
    WITH CHECK (EXISTS (
        SELECT 1 FROM agent_state_volumes
        WHERE agent_state_volumes.id = volume_leases.volume_id
          AND check_permission('user', hephaestus_actor_id(), 'can_attach',
              'state_volume', agent_state_volumes.id::text) = 1
    ));

ALTER TABLE authorization_audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE authorization_audit_events FORCE ROW LEVEL SECURITY;
CREATE POLICY authorization_audit_insert ON authorization_audit_events FOR INSERT
    WITH CHECK (actor_id::text = hephaestus_actor_id());
CREATE POLICY authorization_audit_select ON authorization_audit_events FOR SELECT
    USING (actor_id::text = hephaestus_actor_id());
