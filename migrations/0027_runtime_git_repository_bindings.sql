-- Runtime Git authority is selected by an explicit release-owned capability
-- slot and resolved to one exact immutable revision binding. Trigger
-- attachment provenance remains deliberately unrelated.

ALTER TABLE release_agents
    ADD COLUMN publication_repository_slot text CHECK (
        publication_repository_slot IS NULL
        OR publication_repository_slot ~ '^[a-z][a-z0-9_-]{0,63}$'
    ),
    ADD CHECK (
        (publication_mode = 'proposal' AND publication_repository_slot IS NULL)
        OR
        (publication_mode = 'runtime_git' AND publication_repository_slot IS NOT NULL)
    );

CREATE FUNCTION enforce_release_publication_repository_slot() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
BEGIN
    IF NEW.publication_repository_slot IS NOT NULL
       AND NOT EXISTS (
           SELECT 1
           FROM release_capability_requirements AS requirement
           WHERE requirement.release_agent_id = NEW.id
             AND requirement.slot_key = NEW.publication_repository_slot
             AND requirement.resource_kind = 'repository'
             AND requirement.slot_required
       )
    THEN
        RAISE EXCEPTION 'runtime Git publication slot must name a required repository capability'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION enforce_release_publication_repository_slot() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER release_agents_publication_repository_slot_valid
AFTER INSERT OR UPDATE OF publication_repository_slot ON release_agents
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_release_publication_repository_slot();

CREATE FUNCTION reject_release_publication_repository_slot_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'release publication repository slot is immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
CREATE TRIGGER release_agents_publication_repository_slot_immutable
BEFORE UPDATE OF publication_repository_slot ON release_agents
FOR EACH ROW
WHEN (OLD.publication_repository_slot IS DISTINCT FROM NEW.publication_repository_slot)
EXECUTE FUNCTION reject_release_publication_repository_slot_mutation();

ALTER TABLE agent_instance_revisions
    ADD COLUMN publication_repository_binding_id uuid,
    ADD CHECK (
        (publication_mode = 'proposal'
            AND publication_repository_binding_id IS NULL)
        OR
        (publication_mode = 'runtime_git'
            AND (NOT runnable OR publication_repository_binding_id IS NOT NULL))
    ),
    ADD CONSTRAINT revision_publication_repository_binding
    FOREIGN KEY (publication_repository_binding_id, id)
    REFERENCES agent_capability_bindings(id, instance_revision_id)
    DEFERRABLE INITIALLY DEFERRED;

CREATE FUNCTION enforce_revision_publication_repository_binding() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    publication_slot text;
BEGIN
    SELECT publication_repository_slot INTO publication_slot
    FROM release_agents
    WHERE id = NEW.release_agent_id;

    IF NEW.publication_repository_binding_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1
           FROM agent_capability_bindings AS binding
           WHERE binding.id = NEW.publication_repository_binding_id
             AND binding.instance_revision_id = NEW.id
             AND binding.slot_key = publication_slot
             AND binding.resource_kind = 'repository'
       )
    THEN
        RAISE EXCEPTION 'revision publication binding must resolve the release repository slot'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION enforce_revision_publication_repository_binding() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER revision_publication_repository_binding_valid
AFTER INSERT ON agent_instance_revisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_revision_publication_repository_binding();

ALTER TABLE run_authorization_snapshots
    ADD COLUMN publication_repository_binding_id uuid,
    ADD CONSTRAINT snapshot_publication_repository_binding
    FOREIGN KEY (id, publication_repository_binding_id)
    REFERENCES run_authorization_snapshot_bindings(snapshot_id, binding_id)
    DEFERRABLE INITIALLY DEFERRED;

CREATE FUNCTION copy_snapshot_publication_repository_binding() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    revision_binding_id uuid;
BEGIN
    SELECT publication_repository_binding_id INTO revision_binding_id
    FROM agent_instance_revisions
    WHERE id = NEW.instance_revision_id;

    IF NEW.publication_repository_binding_id IS NOT NULL
       AND NEW.publication_repository_binding_id IS DISTINCT FROM revision_binding_id
    THEN
        RAISE EXCEPTION 'authorization snapshot publication binding does not match revision'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    NEW.publication_repository_binding_id := revision_binding_id;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION copy_snapshot_publication_repository_binding() FROM PUBLIC;
CREATE TRIGGER run_authorization_snapshot_publication_binding
BEFORE INSERT ON run_authorization_snapshots
FOR EACH ROW EXECUTE FUNCTION copy_snapshot_publication_repository_binding();

COMMENT ON COLUMN release_agents.publication_repository_slot IS
    'Release-owned capability slot for runtime Git; never inferred from a trigger attachment.';
COMMENT ON COLUMN agent_instance_revisions.publication_repository_binding_id IS
    'Exact immutable repository capability binding selected for runtime Git publication.';
COMMENT ON COLUMN run_authorization_snapshots.publication_repository_binding_id IS
    'Dispatch-time copy of the exact revision publication binding.';
