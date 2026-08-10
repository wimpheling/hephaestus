-- Explicit immutable release publication modes. Existing releases and
-- revisions retain the historical controlled proposal/result-ref behavior.

ALTER TABLE release_agents
    ADD COLUMN publication_mode text NOT NULL DEFAULT 'proposal'
        CHECK (publication_mode IN ('proposal', 'runtime_git'));

ALTER TABLE agent_instance_revisions
    ADD COLUMN publication_mode text NOT NULL DEFAULT 'proposal'
        CHECK (publication_mode IN ('proposal', 'runtime_git'));

CREATE FUNCTION reject_release_agent_publication_mode_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'release agent publication mode is immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
CREATE TRIGGER release_agents_publication_mode_immutable
BEFORE UPDATE OF publication_mode ON release_agents
FOR EACH ROW
WHEN (OLD.publication_mode IS DISTINCT FROM NEW.publication_mode)
EXECUTE FUNCTION reject_release_agent_publication_mode_mutation();

CREATE FUNCTION enforce_revision_publication_mode() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM release_agents AS release_agent
        WHERE release_agent.id = NEW.release_agent_id
          AND release_agent.publication_mode = NEW.publication_mode
    ) THEN
        RAISE EXCEPTION 'instance revision publication mode must match its release agent'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_revision_publication_mode() FROM PUBLIC;
CREATE TRIGGER agent_instance_revision_publication_mode_matches_release
BEFORE INSERT ON agent_instance_revisions
FOR EACH ROW EXECUTE FUNCTION enforce_revision_publication_mode();
