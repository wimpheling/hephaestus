-- Generic installed-UI target discovery. The target is derived from the
-- immutable repository-scoped installation and returned only after the same
-- live child-session verifier used by the UI content boundary succeeds.
CREATE FUNCTION resolve_ui_browser_repository_target_context(
    p_session_digest bytea,
    p_expected_generation_id uuid
) RETURNS TABLE (
    session_id uuid,
    parent_session_id uuid,
    actor_id uuid,
    organization_id uuid,
    installation_id uuid,
    generation_id uuid,
    repository_id uuid,
    session_route text,
    expires_at timestamptz
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET row_security = off
STABLE
AS $$
WITH candidate AS MATERIALIZED (
    SELECT session.id AS session_id,
           session.parent_session_id,
           parent.user_id AS actor_id,
           session.organization_id,
           session.installation_id,
           session.generation_id,
           session.route AS session_route,
           session.expires_at,
           installation.repository_id,
           generation.ui_key,
           generation.ui_scope,
           generation.repository_git_access,
           descriptor.content_kind,
           descriptor.route_base,
           descriptor.repository_git_access AS descriptor_git_access
    FROM public.ui_browser_sessions AS session
    JOIN public.human_browser_sessions AS parent
      ON parent.id = session.parent_session_id
    JOIN public.ui_installations AS installation
      ON installation.id = session.installation_id
     AND installation.scope = 'repository'
    JOIN public.ui_installation_generations AS generation
      ON generation.id = session.generation_id
     AND generation.installation_id = installation.id
    JOIN public.release_ui_descriptors AS descriptor
      ON descriptor.release_id = generation.release_id
     AND descriptor.ui_key = generation.ui_key
     AND descriptor.scope = generation.ui_scope
    WHERE octet_length(p_session_digest) = 32
      AND session.session_digest = p_session_digest
      AND session.generation_id = p_expected_generation_id
), verified AS (
    SELECT candidate.*,
           live.session_id AS live_session_id,
           live.parent_session_id AS live_parent_session_id,
           live.actor_id AS live_actor_id,
           live.organization_id AS live_organization_id,
           live.installation_id AS live_installation_id,
           live.generation_id AS live_generation_id
    FROM candidate
    CROSS JOIN LATERAL public.authenticate_ui_browser_session(
        p_session_digest,
        p_expected_generation_id,
        CASE
            WHEN candidate.content_kind = 'static' THEN 'static'
            ELSE 'managed_service'
        END,
        candidate.route_base,
        'GET'
    ) AS live
    WHERE live.session_id = candidate.session_id
      AND live.parent_session_id = candidate.parent_session_id
      AND live.actor_id = candidate.actor_id
      AND live.organization_id = candidate.organization_id
      AND live.installation_id = candidate.installation_id
      AND live.generation_id = candidate.generation_id
      AND candidate.repository_git_access = candidate.descriptor_git_access
      AND candidate.repository_git_access IN ('read', 'read_write')
)
SELECT live_session_id,
       live_parent_session_id,
       live_actor_id,
       live_organization_id,
       live_installation_id,
       live_generation_id,
       repository_id,
       session_route,
       expires_at
FROM verified
WHERE (SELECT count(*) FROM verified) = 1;
$$;

REVOKE ALL ON FUNCTION resolve_ui_browser_repository_target_context(bytea, uuid)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION resolve_ui_browser_repository_target_context(bytea, uuid)
    TO hephaestus_app;
