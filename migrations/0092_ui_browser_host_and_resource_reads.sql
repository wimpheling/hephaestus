-- Application-role host and resource reads use security-definer functions;
-- no browser-session, release-resource, or host metadata table grants cross
-- the application boundary.

CREATE FUNCTION resolve_active_ui_generation_host(p_generation_id uuid)
RETURNS TABLE (generation_id uuid)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET row_security = off
STABLE
AS $$
    SELECT generation.id
    FROM public.ui_installation_generations AS generation
    JOIN public.ui_installations AS installation
      ON installation.id = generation.installation_id
     AND installation.current_generation_id = generation.id
     AND installation.lifecycle = 'enabled'
    JOIN public.releases AS release_record
      ON release_record.id = generation.release_id
     AND release_record.state = 'published'
    WHERE generation.id = p_generation_id
$$;

REVOKE ALL ON FUNCTION resolve_active_ui_generation_host(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION resolve_active_ui_generation_host(uuid) TO hephaestus_app;

CREATE FUNCTION resolve_ui_browser_resource(
    p_session_digest bytea,
    p_expected_generation_id uuid,
    p_request_path text,
    p_request_method text
) RETURNS TABLE (
    session_id uuid,
    parent_session_id uuid,
    actor_id uuid,
    organization_id uuid,
    installation_id uuid,
    generation_id uuid,
    session_route text,
    expires_at timestamptz,
    canonical_path text,
    matched_kind text,
    artifact_id uuid,
    storage_key uuid,
    content_hash bytea,
    size_bytes bigint,
    media_type text,
    cache_policy text
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET row_security = off
STABLE
AS $$

-- This security-definer function calls authenticate_ui_browser_session and
-- performs the fresh verifier plus exact resource projection in one SQL
-- statement/transaction. Rust sets LOCAL actor/request context only from the
-- returned safe context after this function succeeds. This closes the READ
-- COMMITTED lifecycle and generation race. It must never accept actor,
-- installation, release, route, or storage-key authority from the HTTP caller,
-- and Rust must not duplicate the verifier's authorization SQL or use direct
-- application table grants.
WITH request_candidates(request_kind, request_path, request_method) AS (
    SELECT 'static'::text, ltrim($3, '/'), 'GET'::text
    WHERE $4 IN ('GET', 'HEAD')
    UNION ALL
    SELECT 'managed_service'::text, ltrim($3, '/'), 'GET'::text
    WHERE $4 = 'GET'
    UNION ALL
    SELECT 'api'::text, $3, $4
    WHERE $4 IN ('GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS')
), verified AS (
    SELECT candidate.request_kind,
           candidate.request_path,
           candidate.request_method,
           authenticated.*
    FROM request_candidates AS candidate
    CROSS JOIN LATERAL public.authenticate_ui_browser_session(
        $1, $2, candidate.request_kind, candidate.request_path,
        candidate.request_method
    ) AS authenticated
), matches AS (
    SELECT verified.session_id,
           verified.parent_session_id,
           verified.actor_id,
           verified.organization_id,
           verified.installation_id,
           verified.generation_id,
           verified.route AS session_route,
           verified.expires_at,
           CASE
               WHEN verified.request_path = descriptor.route_base
               THEN '/' || descriptor.route_base || '/' || descriptor.entrypoint
               ELSE '/' || verified.request_path
           END AS canonical_path,
           'static'::text AS matched_kind,
           file.artifact_id,
           artifact.storage_key,
           artifact.content_hash,
           artifact.size_bytes,
           file.artifact_media_type AS media_type,
           descriptor.cache AS cache_policy
    FROM verified
    JOIN public.ui_installation_generations AS generation
      ON generation.id = verified.generation_id
    JOIN public.release_ui_descriptors AS descriptor
      ON descriptor.release_id = generation.release_id
     AND descriptor.ui_key = generation.ui_key
     AND descriptor.scope = generation.ui_scope
     AND descriptor.content_kind = 'static'
    JOIN public.release_ui_static_files AS file
      ON file.release_id = descriptor.release_id
     AND file.ui_key = descriptor.ui_key
     AND file.route = CASE
         WHEN verified.request_path = descriptor.route_base THEN descriptor.entrypoint
         WHEN length(verified.request_path) > length(descriptor.route_base)
              AND left(verified.request_path, length(descriptor.route_base) + 1)
                  = descriptor.route_base || '/'
         THEN substr(verified.request_path, length(descriptor.route_base) + 2)
     END
    JOIN public.release_artifacts AS artifact
      ON artifact.id = file.artifact_id
     AND artifact.release_id = file.release_id
     AND artifact.kind = 'file'
     AND artifact.media_type = file.artifact_media_type
    WHERE verified.request_kind = 'static'
    UNION ALL
    SELECT verified.session_id,
           verified.parent_session_id,
           verified.actor_id,
           verified.organization_id,
           verified.installation_id,
           verified.generation_id,
           verified.route,
           verified.expires_at,
           CASE
               WHEN verified.request_path = descriptor.route_base
               THEN '/' || descriptor.route_base || '/' || descriptor.entrypoint
               ELSE '/' || verified.request_path
           END AS canonical_path,
           'managed_service'::text,
           NULL::uuid,
           NULL::uuid,
           NULL::bytea,
           NULL::bigint,
           NULL::text,
           NULL::text
    FROM verified
    JOIN public.ui_installation_generations AS generation
      ON generation.id = verified.generation_id
    JOIN public.release_ui_descriptors AS descriptor
      ON descriptor.release_id = generation.release_id
     AND descriptor.ui_key = generation.ui_key
     AND descriptor.scope = generation.ui_scope
     AND descriptor.content_kind = 'managed_service'
    JOIN public.release_ui_managed_services AS managed
      ON managed.release_id = descriptor.release_id
     AND managed.ui_key = descriptor.ui_key
    WHERE verified.request_kind = 'managed_service'
      AND (
          verified.request_path = descriptor.route_base
          OR (
              length(verified.request_path) > length(descriptor.route_base)
              AND left(verified.request_path, length(descriptor.route_base) + 1)
                  = descriptor.route_base || '/'
          )
      )
    UNION ALL
    SELECT verified.session_id,
           verified.parent_session_id,
           verified.actor_id,
           verified.organization_id,
           verified.installation_id,
           verified.generation_id,
           verified.route,
           verified.expires_at,
           verified.request_path AS canonical_path,
           'api'::text,
           NULL::uuid,
           NULL::uuid,
           NULL::bytea,
           NULL::bigint,
           NULL::text,
           NULL::text
    FROM verified
    JOIN public.ui_installation_generations AS generation
      ON generation.id = verified.generation_id
    JOIN public.release_ui_descriptors AS descriptor
      ON descriptor.release_id = generation.release_id
     AND descriptor.ui_key = generation.ui_key
     AND descriptor.scope = generation.ui_scope
    JOIN public.release_ui_api_bindings AS api
      ON api.release_id = descriptor.release_id
     AND api.ui_key = descriptor.ui_key
     AND api.route = verified.request_path
     AND api.method = verified.request_method
    WHERE verified.request_kind = 'api'
)
SELECT session_id,
       parent_session_id,
       actor_id,
       organization_id,
       installation_id,
       generation_id,
       session_route,
       expires_at,
       canonical_path,
       matched_kind,
       artifact_id,
       storage_key,
       content_hash,
       size_bytes,
       media_type,
       cache_policy
FROM matches
WHERE (SELECT count(*) FROM matches) = 1
$$;

REVOKE ALL ON FUNCTION resolve_ui_browser_resource(bytea, uuid, text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION resolve_ui_browser_resource(bytea, uuid, text, text) TO hephaestus_app;
