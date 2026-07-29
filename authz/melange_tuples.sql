-- Human-reviewed authoritative tuple projection. The initial view is created
-- by migration 0002; migration 0006 replaces it after the release and secret
-- tables exist.
CREATE OR REPLACE VIEW melange_tuples (
    subject_type, subject_id, relation, object_type, object_id
) AS
SELECT
    'user'::text,
    organization_members.user_id::text,
    organization_members.role,
    'organization'::text,
    organization_members.organization_id::text
FROM organization_members
UNION ALL
SELECT
    'user', organization_secret_managers.user_id::text, 'secret_manager',
    'organization', organization_secret_managers.organization_id::text
FROM organization_secret_managers
UNION ALL
SELECT
    'organization', projects.organization_id::text, 'organization',
    'project', projects.id::text
FROM projects
UNION ALL
SELECT
    'user', project_maintainers.user_id::text, 'maintainer',
    'project', project_maintainers.project_id::text
FROM project_maintainers
UNION ALL
SELECT
    'user', project_secret_roles.user_id::text, project_secret_roles.role,
    'project', project_secret_roles.project_id::text
FROM project_secret_roles
UNION ALL
SELECT
    'project', repositories.project_id::text, 'project',
    'repository', repositories.id::text
FROM repositories
UNION ALL
SELECT
    'user', repository_managers.user_id::text, 'manager',
    'repository', repository_managers.repository_id::text
FROM repository_managers
UNION ALL
SELECT
    'user', repository_secret_roles.user_id::text, repository_secret_roles.role,
    'repository', repository_secret_roles.repository_id::text
FROM repository_secret_roles
UNION ALL
SELECT
    'user', '*', 'public', 'repository', repositories.id::text
FROM repositories
WHERE repositories.is_public
UNION ALL
SELECT
    'agent_instance', runs.instance_id::text, 'instance', 'run', runs.id::text
FROM runs
UNION ALL
SELECT
    'agent_instance', volumes.instance_id::text, 'instance',
    'state_volume', volumes.id::text
FROM agent_instance_state_volumes AS volumes
UNION ALL
SELECT
    'repository', build_requests.repository_id::text, 'repository',
    'build', build_requests.id::text
FROM build_requests
UNION ALL
SELECT
    'repository', releases.repository_id::text, 'repository',
    'release', releases.id::text
FROM releases
UNION ALL
SELECT
    'repository', releases.repository_id::text, 'usable_repository',
    'release', releases.id::text
FROM releases
WHERE releases.state = 'published'
UNION ALL
SELECT
    'release', release_agents.release_id::text, 'release',
    'release_agent', release_agents.id::text
FROM release_agents
UNION ALL
SELECT
    'project', agent_instances.project_id::text, 'project',
    'agent_instance', agent_instances.id::text
FROM agent_instances
UNION ALL
SELECT
    'release_agent', revisions.release_agent_id::text, 'release_agent',
    'agent_instance', agent_instances.id::text
FROM agent_instances
JOIN agent_instance_revisions AS revisions
  ON revisions.id = agent_instances.active_revision_id
UNION ALL
SELECT
    'agent_instance', agent_attachments.instance_id::text, 'instance',
    'agent_attachment', agent_attachments.id::text
FROM agent_attachments
UNION ALL
SELECT
    'repository', agent_attachments.repository_id::text, 'repository',
    'agent_attachment', agent_attachments.id::text
FROM agent_attachments
UNION ALL
SELECT
    'agent_instance', agent_updates.instance_id::text, 'instance',
    'agent_update', agent_updates.id::text
FROM agent_updates
UNION ALL
SELECT
    CASE WHEN secrets.organization_id IS NOT NULL
        THEN 'organization' ELSE 'project' END,
    COALESCE(secrets.organization_id, secrets.project_id)::text,
    'owner', 'secret', secrets.id::text
FROM secrets
UNION ALL
SELECT
    'secret', secret_grants.secret_id::text, 'secret',
    'secret_grant', secret_grants.id::text
FROM secret_grants
UNION ALL
SELECT
    secret_grants.target_kind, secret_grants.target_id::text, 'target',
    'secret_grant', secret_grants.id::text
FROM secret_grants
WHERE secret_grants.status = 'active'
  AND (secret_grants.expires_at IS NULL OR secret_grants.expires_at > now())
UNION ALL
SELECT
    'secret_grant', secret_imports.grant_id::text, 'grant',
    'secret_import', secret_imports.id::text
FROM secret_imports
UNION ALL
SELECT
    secret_imports.target_kind, secret_imports.target_id::text, 'target',
    'secret_import', secret_imports.id::text
FROM secret_imports
UNION ALL
SELECT
    secret_imports.target_kind, secret_imports.target_id::text, 'active_target',
    'secret_import', secret_imports.id::text
FROM secret_imports
JOIN secret_grants ON secret_grants.id = secret_imports.grant_id
JOIN secrets ON secrets.id = secret_imports.secret_id
WHERE secret_imports.status = 'active'
  AND secret_grants.status = 'active'
  AND secrets.status = 'active'
  AND (secret_grants.expires_at IS NULL OR secret_grants.expires_at > now())
UNION ALL
SELECT
    'agent_instance', revisions.instance_id::text, 'instance',
    'agent_secret_binding', bindings.id::text
FROM agent_secret_bindings AS bindings
JOIN agent_instance_revisions AS revisions
  ON revisions.id = bindings.instance_revision_id
UNION ALL
SELECT
    'secret_import', bindings.import_id::text, 'secret_import',
    'agent_secret_binding', bindings.id::text
FROM agent_secret_bindings AS bindings
JOIN secret_imports ON secret_imports.id = bindings.import_id
JOIN secret_grants ON secret_grants.id = secret_imports.grant_id
JOIN secrets ON secrets.id = secret_imports.secret_id
WHERE bindings.status = 'active'
  AND secret_imports.status = 'active'
  AND secret_grants.status = 'active'
  AND secrets.status = 'active'
  AND (secret_grants.expires_at IS NULL OR secret_grants.expires_at > now())
UNION ALL
SELECT
    'agent_secret_binding', secret_leases.binding_id::text, 'binding',
    'secret_lease', secret_leases.id::text
FROM secret_leases
UNION ALL
SELECT
    'run', secret_leases.run_id::text, 'run',
    'secret_lease', secret_leases.id::text
FROM secret_leases
UNION ALL
SELECT
    'run', secret_leases.run_id::text,
    CASE secret_leases.delivery_mode
        WHEN 'raw' THEN 'receive_raw' ELSE 'use_brokered' END,
    'secret_lease', secret_leases.id::text
FROM secret_leases
WHERE secret_leases.status = 'active'
  AND secret_leases.expires_at > now();
