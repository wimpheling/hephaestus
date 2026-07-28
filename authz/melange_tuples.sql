-- Human-reviewed tuple view source. Keep aligned with
-- migrations/0002_melange_tuples.sql.
CREATE VIEW melange_tuples (
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
    'project', repositories.project_id::text, 'project',
    'repository', repositories.id::text
FROM repositories
UNION ALL
SELECT
    'user', '*', 'public', 'repository', repositories.id::text
FROM repositories
WHERE repositories.is_public
UNION ALL
SELECT
    'project', agents.project_id::text, 'project', 'agent', agents.id::text
FROM agents
UNION ALL
SELECT
    'agent', runs.agent_id::text, 'agent', 'run', runs.id::text
FROM runs
UNION ALL
SELECT
    'agent', agent_state_volumes.agent_id::text, 'agent',
    'state_volume', agent_state_volumes.id::text
FROM agent_state_volumes;
