-- A project creator must have an authorization tuple before the first
-- request-facing project read or repository write can succeed. This trigger
-- keeps project creation atomic and also covers every future project creator.
CREATE FUNCTION ensure_project_maintainer(p_project_id uuid, p_user_id uuid)
RETURNS void
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET row_security = off
AS $$
    INSERT INTO project_maintainers (project_id, user_id)
    VALUES (p_project_id, p_user_id)
    ON CONFLICT (project_id, user_id) DO NOTHING
$$;

ALTER FUNCTION ensure_project_maintainer(uuid, uuid) OWNER TO hephaestus_authz_owner;

CREATE FUNCTION assign_initial_project_maintainer() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET row_security = off
AS $$
DECLARE
    actor uuid;
BEGIN
    actor := NULLIF(current_setting('hephaestus.actor_id', true), '')::uuid;
    IF actor IS NOT NULL THEN
        PERFORM ensure_project_maintainer(NEW.id, actor);
    END IF;
    RETURN NEW;
END;
$$;

ALTER FUNCTION assign_initial_project_maintainer() OWNER TO hephaestus_authz_owner;
GRANT INSERT, SELECT ON project_maintainers TO hephaestus_authz_owner;
GRANT EXECUTE ON FUNCTION ensure_project_maintainer(uuid, uuid) TO hephaestus_app;

CREATE TRIGGER assign_project_creator_as_maintainer
    AFTER INSERT ON projects
    FOR EACH ROW
    EXECUTE FUNCTION assign_initial_project_maintainer();
