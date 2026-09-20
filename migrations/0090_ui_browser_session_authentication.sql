-- Application-role verifier for one presented generation-bound UI session.
-- This migration is intentionally separate from 0089's worker-owned writes.
-- The application role receives no table privileges and the function returns
-- only safe session context.

CREATE FUNCTION authenticate_ui_browser_session(
    p_session_digest bytea,
    p_expected_generation_id uuid,
    p_request_kind text,
    p_request_path text,
    p_request_method text
) RETURNS TABLE (
    session_id uuid,
    parent_session_id uuid,
    actor_id uuid,
    organization_id uuid,
    installation_id uuid,
    generation_id uuid,
    route text,
    expires_at timestamptz
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET row_security = off
STABLE
AS $$
WITH auth_instant AS MATERIALIZED (
    SELECT statement_timestamp() AS instant
), candidate AS (
    SELECT session.id AS session_id,
           session.parent_session_id,
           parent.user_id AS actor_id,
           session.installation_id,
           session.generation_id,
           session.organization_id,
           session.route,
           session.issued_at AS session_issued_at,
           session.expires_at,
           parent.issued_at AS parent_issued_at,
           parent.expires_at AS parent_expires_at,
           parent.revoked_at AS parent_revoked_at,
           account.status AS account_status,
           installation.scope AS installation_scope,
           installation.lifecycle AS installation_lifecycle,
           installation.current_generation_id,
           generation.release_id,
           generation.ui_key,
           generation.ui_scope,
           descriptor.scope AS descriptor_scope,
           descriptor.route_base,
           descriptor.entrypoint,
           descriptor.content_kind,
           CASE
               WHEN installation.scope = 'global'
                   THEN installation.organization_id
               ELSE target_project.organization_id
           END AS target_organization_id,
           COALESCE(target_repository.project_id, installation.project_id)
               AS target_project_id,
           installation.repository_id AS target_repository_id,
           source_project.id AS source_project_id,
           source_repository.id AS source_repository_id,
           source_project.organization_id AS source_organization_id,
           release_record.state AS release_state
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
    LEFT JOIN public.repositories AS target_repository
      ON target_repository.id = installation.repository_id
    LEFT JOIN public.projects AS target_project
      ON target_project.id = COALESCE(
          target_repository.project_id, installation.project_id
      )
    WHERE octet_length(p_session_digest) = 32
      AND session.session_digest = p_session_digest
      AND session.generation_id = p_expected_generation_id
), eligible_bindings AS (
    SELECT binding.binding_kind,
           binding.binding_key,
           binding.release_id,
           binding.ui_key,
           binding.release_agent_id,
           binding.gateway_id,
           binding.gateway_revision_id,
           binding.gateway_name,
           binding.method,
           binding.route,
           binding.exposure,
           revision.handler_contract
    FROM candidate AS c
    JOIN public.ui_installation_bindings AS binding
      ON binding.installation_id = c.installation_id
     AND binding.generation_id = c.generation_id
     AND binding.release_id = c.release_id
     AND binding.ui_key = c.ui_key
    JOIN public.gateways AS gateway
      ON gateway.id = binding.gateway_id
     AND gateway.project_id = c.source_project_id
     AND gateway.repository_id = c.source_repository_id
     AND gateway.lifecycle = 'enabled'
     AND gateway.active_revision_id = binding.gateway_revision_id
    JOIN public.gateway_revisions AS revision
      ON revision.id = binding.gateway_revision_id
     AND revision.gateway_id = binding.gateway_id
     AND revision.release_id = c.release_id
     AND revision.release_agent_id = binding.release_agent_id
     AND revision.project_id = c.source_project_id
     AND revision.repository_id = c.source_repository_id
     AND revision.exposure = binding.exposure
    JOIN public.gateway_routes AS gateway_route
      ON gateway_route.gateway_revision_id = revision.id
     AND gateway_route.gateway_id = gateway.id
     AND gateway_route.enabled
     AND (
         gateway_route.path = binding.route
         OR (
             length(binding.route) > length(gateway_route.path)
             AND left(binding.route, length(gateway_route.path) + 1)
                 = gateway_route.path || '/'
         )
     )
     AND binding.method = ANY(gateway_route.methods)
    WHERE gateway.name = binding.gateway_name
      AND binding.exposure = 'heph_authenticated'
      AND (
          (
              binding.binding_kind = 'managed_service'
              AND revision.handler_contract = 'http.service.v1'
          )
          OR (
              binding.binding_kind = 'api'
              AND revision.handler_contract IN ('http.v1', 'http.service.v1')
          )
      )
      AND public.check_permission(
          'user', c.actor_id::text, 'can_use',
          'release_agent', binding.release_agent_id::text
      ) = 1
), binding_set AS (
    SELECT c.session_id,
           (
               NOT EXISTS (
                   SELECT 1
                   FROM public.release_ui_managed_services AS managed
                   WHERE managed.release_id = c.release_id
                     AND managed.ui_key = c.ui_key
                     AND NOT EXISTS (
                         SELECT 1 FROM eligible_bindings AS eb
                         WHERE eb.binding_kind = 'managed_service'
                           AND eb.binding_key = 'service'
                           AND eb.release_id = managed.release_id
                           AND eb.ui_key = managed.ui_key
                           AND eb.gateway_name = managed.gateway_name
                           AND eb.route = managed.route
                           AND eb.release_agent_id = managed.release_agent_id
                     )
               )
               AND NOT EXISTS (
                   SELECT 1
                   FROM public.release_ui_api_bindings AS api
                   WHERE api.release_id = c.release_id
                     AND api.ui_key = c.ui_key
                     AND NOT EXISTS (
                         SELECT 1 FROM eligible_bindings AS eb
                         WHERE eb.binding_kind = 'api'
                           AND eb.binding_key = api.api_key
                           AND eb.release_id = api.release_id
                           AND eb.ui_key = api.ui_key
                           AND eb.gateway_name = api.gateway_name
                           AND eb.method = api.method
                           AND eb.route = api.route
                           AND eb.release_agent_id = api.release_agent_id
                     )
               )
               AND NOT EXISTS (
                   SELECT 1
                   FROM public.ui_installation_bindings AS binding
                   WHERE binding.installation_id = c.installation_id
                     AND binding.generation_id = c.generation_id
                     AND binding.release_id = c.release_id
                     AND binding.ui_key = c.ui_key
                     AND (
                         NOT EXISTS (
                             SELECT 1 FROM eligible_bindings AS eb
                             WHERE eb.binding_kind = binding.binding_kind
                               AND eb.binding_key = binding.binding_key
                               AND eb.release_id = binding.release_id
                               AND eb.ui_key = binding.ui_key
                               AND eb.gateway_id = binding.gateway_id
                               AND eb.gateway_revision_id = binding.gateway_revision_id
                               AND eb.release_agent_id = binding.release_agent_id
                               AND eb.gateway_name = binding.gateway_name
                               AND eb.method = binding.method
                               AND eb.route = binding.route
                         )
                         OR (
                             binding.binding_kind = 'managed_service'
                             AND NOT EXISTS (
                                 SELECT 1
                                 FROM public.release_ui_managed_services AS managed
                                 WHERE managed.release_id = binding.release_id
                                   AND managed.ui_key = binding.ui_key
                                   AND managed.gateway_name = binding.gateway_name
                                   AND managed.route = binding.route
                                   AND managed.release_agent_id = binding.release_agent_id
                             )
                         )
                         OR (
                             binding.binding_kind = 'api'
                             AND NOT EXISTS (
                                 SELECT 1
                                 FROM public.release_ui_api_bindings AS api
                                 WHERE api.release_id = binding.release_id
                                   AND api.ui_key = binding.ui_key
                                   AND api.api_key = binding.binding_key
                                   AND api.gateway_name = binding.gateway_name
                                   AND api.method = binding.method
                                   AND api.route = binding.route
                                   AND api.release_agent_id = binding.release_agent_id
                             )
                         )
                     )
               )
           ) AS complete
    FROM candidate AS c
) 
SELECT c.session_id,
       c.parent_session_id,
       c.actor_id,
       c.target_organization_id AS organization_id,
       c.installation_id,
       c.generation_id,
       c.route,
       c.expires_at
FROM candidate AS c
CROSS JOIN auth_instant
JOIN binding_set AS bindings
  ON bindings.session_id = c.session_id
 AND bindings.complete
WHERE c.current_generation_id = c.generation_id
  AND c.installation_lifecycle = 'enabled'
  AND c.account_status = 'active'
  AND c.parent_revoked_at IS NULL
  AND c.parent_issued_at <= auth_instant.instant
  AND c.parent_expires_at > auth_instant.instant
  AND c.session_issued_at <= auth_instant.instant
  AND c.expires_at > auth_instant.instant
  AND c.target_organization_id IS NOT NULL
  AND c.source_organization_id = c.target_organization_id
  AND c.organization_id = c.target_organization_id
  AND c.release_state = 'published'
  AND c.descriptor_scope = c.ui_scope
  AND c.route = c.route_base
  AND (
      (
          c.installation_scope = 'global'
          AND public.check_permission(
              'user', c.actor_id::text, 'can_read',
              'organization', c.target_organization_id::text
          ) = 1
      )
      OR (
          c.installation_scope = 'project'
          AND public.check_permission(
              'user', c.actor_id::text, 'can_read',
              'project', c.target_project_id::text
          ) = 1
      )
      OR (
          c.installation_scope = 'repository'
          AND public.check_permission(
              'user', c.actor_id::text, 'can_read',
              'project', c.target_project_id::text
          ) = 1
          AND public.check_permission(
              'user', c.actor_id::text, 'can_read',
              'repository', c.target_repository_id::text
          ) = 1
      )
  )
  AND public.check_permission(
      'user', c.actor_id::text, 'can_use',
      'release', c.release_id::text
  ) = 1
  AND (
      (
          NOT EXISTS (
              SELECT 1
              FROM public.release_ui_managed_services AS managed
              WHERE managed.release_id = c.release_id
                AND managed.ui_key = c.ui_key
          )
          AND NOT EXISTS (
              SELECT 1
              FROM public.release_ui_api_bindings AS api
              WHERE api.release_id = c.release_id
                AND api.ui_key = c.ui_key
          )
      )
      OR public.check_permission(
          'user', c.actor_id::text, 'can_read',
          'project', c.source_project_id::text
      ) = 1
  )
  AND (
      (
          c.content_kind = 'static'
          AND p_request_kind = 'static'
          AND p_request_method = 'GET'
          AND EXISTS (
              SELECT 1
              FROM public.release_ui_static_files AS file
              WHERE file.release_id = c.release_id
                AND file.ui_key = c.ui_key
                AND file.route = CASE
                    WHEN p_request_path = c.route_base THEN c.entrypoint
                    WHEN length(p_request_path) > length(c.route_base)
                         AND left(p_request_path, length(c.route_base) + 1)
                             = c.route_base || '/'
                    THEN substr(p_request_path, length(c.route_base) + 2)
                END
          )
      )
      OR (
          c.content_kind = 'managed_service'
          AND p_request_kind = 'managed_service'
          AND p_request_method = 'GET'
          AND EXISTS (
              SELECT 1
              FROM public.release_ui_managed_services AS managed
              JOIN eligible_bindings AS eb
                ON eb.binding_kind = 'managed_service'
               AND eb.binding_key = 'service'
               AND eb.release_id = managed.release_id
               AND eb.ui_key = managed.ui_key
               AND eb.gateway_name = managed.gateway_name
               AND eb.route = managed.route
               AND eb.release_agent_id = managed.release_agent_id
               AND eb.handler_contract = 'http.service.v1'
              WHERE managed.release_id = c.release_id
                AND managed.ui_key = c.ui_key
                AND (
                    p_request_path = c.route_base
                    OR (
                        length(p_request_path) > length(c.route_base)
                        AND left(p_request_path, length(c.route_base) + 1)
                            = c.route_base || '/'
                    )
                )
          )
      )
      OR (
          (c.content_kind = 'static' OR c.content_kind = 'managed_service')
          AND p_request_kind = 'api'
          AND EXISTS (
              SELECT 1
              FROM public.release_ui_api_bindings AS api
              JOIN eligible_bindings AS eb
                ON eb.binding_kind = 'api'
               AND eb.binding_key = api.api_key
               AND eb.release_id = api.release_id
               AND eb.ui_key = api.ui_key
               AND eb.gateway_name = api.gateway_name
               AND eb.method = api.method
               AND eb.route = api.route
               AND eb.release_agent_id = api.release_agent_id
              WHERE api.release_id = c.release_id
                AND api.ui_key = c.ui_key
                AND api.route = p_request_path
                AND api.method = p_request_method
          )
      )
  )
$$;

REVOKE ALL ON FUNCTION authenticate_ui_browser_session(
    bytea, uuid, text, text, text
) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION authenticate_ui_browser_session(
    bytea, uuid, text, text, text
) TO hephaestus_app;
