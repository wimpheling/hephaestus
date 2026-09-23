-- Explicit release UI repository Git authority and its immutable installation
-- approval snapshot. Existing installations remain non-authorized by default.
ALTER TABLE release_ui_descriptors
    ADD COLUMN repository_git_access text NOT NULL DEFAULT 'none',
    ADD CONSTRAINT release_ui_descriptors_repository_git_access_check
        CHECK (repository_git_access IN ('none', 'read', 'read_write'));

ALTER TABLE ui_installation_generations
    ADD COLUMN repository_git_access text NOT NULL DEFAULT 'none',
    ADD CONSTRAINT ui_installation_generations_repository_git_access_check
        CHECK (repository_git_access IN ('none', 'read', 'read_write'));

-- Application-role verifier for the reserved same-origin UI Git path. It
-- returns the actor selected by the live child session and the exact
-- repository installation target; it accepts no caller-supplied identity.
CREATE FUNCTION resolve_ui_browser_repository_git_access(
    p_session_digest bytea,
    p_expected_generation_id uuid,
    p_repository_id uuid,
    p_operation text
) RETURNS TABLE (
    actor_id uuid,
    repository_id uuid,
    access text
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET row_security = off
STABLE
AS $$
WITH auth_instant AS MATERIALIZED (
    SELECT statement_timestamp() AS instant
), candidate AS MATERIALIZED (
    SELECT parent.user_id AS actor_id,
           installation.id AS installation_id,
           installation.repository_id AS installation_repository_id,
           installation.current_generation_id,
           installation.lifecycle,
           session.generation_id,
           session.organization_id AS session_organization_id,
           session.route AS session_route,
           session.issued_at AS session_issued_at,
           session.expires_at AS session_expires_at,
           parent.issued_at AS parent_issued_at,
           parent.expires_at AS parent_expires_at,
           parent.revoked_at AS parent_revoked_at,
           account.status AS account_status,
           generation.repository_git_access AS generation_access,
           descriptor.repository_git_access AS descriptor_access,
           descriptor.content_kind,
           descriptor.route_base,
           release_record.id AS release_id,
           release_record.state AS release_state,
           source_project.id AS source_project_id,
           target_project.id AS target_project_id,
           target_repository.id AS target_repository_id,
           source_project.organization_id AS source_organization_id,
           target_project.organization_id AS target_organization_id
    FROM public.ui_browser_sessions AS session
    JOIN public.human_browser_sessions AS parent
      ON parent.id = session.parent_session_id
    JOIN public.users AS account
      ON account.id = parent.user_id
    JOIN public.ui_installations AS installation
      ON installation.id = session.installation_id
    JOIN public.ui_installation_generations AS generation
      ON generation.id = session.generation_id
     AND generation.installation_id = installation.id
    JOIN public.release_ui_descriptors AS descriptor
      ON descriptor.release_id = generation.release_id
     AND descriptor.ui_key = generation.ui_key
     AND descriptor.scope = generation.ui_scope
    JOIN public.releases AS release_record
      ON release_record.id = generation.release_id
    JOIN public.repositories AS source_repository
      ON source_repository.id = release_record.repository_id
    JOIN public.projects AS source_project
      ON source_project.id = source_repository.project_id
    JOIN public.repositories AS target_repository
      ON target_repository.id = installation.repository_id
    JOIN public.projects AS target_project
      ON target_project.id = target_repository.project_id
    WHERE octet_length(p_session_digest) = 32
      AND session.session_digest = p_session_digest
      AND session.generation_id = p_expected_generation_id
      AND installation.scope = 'repository'
      AND installation.repository_id = p_repository_id
), eligible AS (
    SELECT c.actor_id, c.target_repository_id AS repository_id,
           c.generation_access AS access
    FROM candidate AS c
    CROSS JOIN auth_instant
    CROSS JOIN LATERAL public.authenticate_ui_browser_session(
        p_session_digest,
        p_expected_generation_id,
        CASE WHEN c.content_kind = 'static' THEN 'static' ELSE 'managed_service' END,
        c.route_base,
        'GET'
    ) AS live
    WHERE c.current_generation_id = c.generation_id
      AND live.actor_id = c.actor_id
      AND live.generation_id = c.generation_id
      AND c.lifecycle = 'enabled'
      AND c.account_status = 'active'
      AND c.parent_revoked_at IS NULL
      AND c.parent_issued_at <= auth_instant.instant
      AND c.parent_expires_at > auth_instant.instant
      AND c.session_issued_at <= auth_instant.instant
      AND c.session_expires_at > auth_instant.instant
      AND c.target_repository_id = p_repository_id
      AND c.session_organization_id = c.target_organization_id
      AND c.session_route = c.route_base
      AND c.source_organization_id = c.target_organization_id
      AND c.generation_access = c.descriptor_access
      AND c.generation_access IN ('read', 'read_write')
      AND c.release_state = 'published'
      AND c.release_id IS NOT NULL
      AND c.source_project_id IS NOT NULL
      AND public.check_permission(
          'user', c.actor_id::text, 'can_read',
          'project', c.target_project_id::text
      ) = 1
      AND public.check_permission(
          'user', c.actor_id::text, 'can_read',
          'repository', c.target_repository_id::text
      ) = 1
      AND public.check_permission(
          'user', c.actor_id::text, 'can_read',
          'project', c.source_project_id::text
      ) = 1
      AND public.check_permission(
          'user', c.actor_id::text, 'can_use',
          'release', c.release_id::text
      ) = 1
      AND (
          p_operation = 'read'
          OR (
              p_operation = 'write'
              AND c.generation_access = 'read_write'
              AND public.check_permission(
                  'user', c.actor_id::text, 'can_write',
                  'repository', c.target_repository_id::text
              ) = 1
          )
      )
)
SELECT actor_id, repository_id, access
FROM eligible
WHERE p_operation IN ('read', 'write');
$$;

REVOKE ALL ON FUNCTION resolve_ui_browser_repository_git_access(bytea, uuid, uuid, text)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION resolve_ui_browser_repository_git_access(bytea, uuid, uuid, text)
    TO hephaestus_app;
