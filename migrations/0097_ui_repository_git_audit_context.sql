-- Add the complete verified child-session projection needed by the existing
-- UI request audit boundary. Authorization remains delegated to 0096's live
-- verifier; this wrapper returns only its authorized row plus the same
-- generation-bound context, never a secret or caller-selected identity.
CREATE FUNCTION resolve_ui_browser_repository_git_context(
    p_session_digest bytea,
    p_expected_generation_id uuid,
    p_repository_id uuid,
    p_operation text
) RETURNS TABLE (
    session_id uuid,
    parent_session_id uuid,
    actor_id uuid,
    organization_id uuid,
    installation_id uuid,
    generation_id uuid,
    repository_id uuid,
    access text,
    session_route text,
    expires_at timestamptz
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET row_security = off
STABLE
AS $$
WITH authorized AS MATERIALIZED (
    SELECT *
    FROM public.resolve_ui_browser_repository_git_access(
        p_session_digest,
        p_expected_generation_id,
        p_repository_id,
        p_operation
    )
), session_context AS MATERIALIZED (
    SELECT session.id AS session_id,
           session.parent_session_id,
           parent.user_id AS actor_id,
           session.organization_id,
           session.installation_id,
           session.generation_id,
           session.route AS session_route,
           session.expires_at
    FROM public.ui_browser_sessions AS session
    JOIN public.human_browser_sessions AS parent
      ON parent.id = session.parent_session_id
    WHERE octet_length(p_session_digest) = 32
      AND session.session_digest = p_session_digest
      AND session.generation_id = p_expected_generation_id
)
SELECT context.session_id,
       context.parent_session_id,
       authorized.actor_id,
       context.organization_id,
       context.installation_id,
       context.generation_id,
       authorized.repository_id,
       authorized.access,
       context.session_route,
       context.expires_at
FROM authorized
JOIN session_context AS context
  ON context.actor_id = authorized.actor_id;
$$;

REVOKE ALL ON FUNCTION resolve_ui_browser_repository_git_context(bytea, uuid, uuid, text)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION resolve_ui_browser_repository_git_context(bytea, uuid, uuid, text)
    TO hephaestus_app;
