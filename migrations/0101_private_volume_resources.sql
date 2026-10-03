-- Generalize the canonical volume metadata in place. Existing resource IDs,
-- backing handles, filesystem UUIDs, lease generations, and mailbox evidence
-- remain authoritative. instance_id now records immutable legacy provenance.
ALTER TABLE agent_instance_state_volumes ADD COLUMN project_id uuid REFERENCES projects(id);
UPDATE agent_instance_state_volumes volume SET project_id = instance.project_id
FROM agent_instances instance WHERE instance.id = volume.instance_id;
ALTER TABLE agent_instance_state_volumes ALTER COLUMN project_id SET NOT NULL;
ALTER TABLE agent_instance_state_volumes ALTER COLUMN instance_id DROP NOT NULL;
ALTER TABLE agent_instance_state_volumes ADD CONSTRAINT private_volume_project_identity UNIQUE (id, project_id);
ALTER TABLE agent_instance_state_volumes ADD CONSTRAINT private_volume_origin_project
    FOREIGN KEY (instance_id, project_id) REFERENCES agent_instances(id, project_id);
ALTER TABLE agent_instance_state_volumes ADD CONSTRAINT private_volume_standalone_capacity
    CHECK (instance_id IS NOT NULL OR capacity_bytes <= 17592186044416);

CREATE FUNCTION enforce_private_volume_owner() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE origin_project uuid;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.project_id IS DISTINCT FROM OLD.project_id
           OR NEW.instance_id IS DISTINCT FROM OLD.instance_id THEN
            RAISE EXCEPTION 'volume owner and legacy provenance are immutable'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    ELSIF NEW.instance_id IS NOT NULL THEN
        SELECT project_id INTO origin_project FROM agent_instances WHERE id = NEW.instance_id;
        IF origin_project IS NULL THEN
            RAISE EXCEPTION 'legacy volume origin is unavailable' USING ERRCODE = 'foreign_key_violation';
        END IF;
        NEW.project_id := COALESCE(NEW.project_id, origin_project);
        IF NEW.project_id <> origin_project THEN
            RAISE EXCEPTION 'volume owner differs from legacy origin project'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    END IF;
    IF NEW.project_id IS NULL THEN
        RAISE EXCEPTION 'standalone volume requires an owning project' USING ERRCODE = 'not_null_violation';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_private_volume_owner() FROM PUBLIC;
CREATE TRIGGER private_volume_owner BEFORE INSERT OR UPDATE ON agent_instance_state_volumes
FOR EACH ROW EXECUTE FUNCTION enforce_private_volume_owner();

CREATE OR REPLACE FUNCTION capability_resource_project(p_kind text, p_id uuid) RETURNS uuid
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
    CASE p_kind
        WHEN 'repository' THEN RETURN (SELECT project_id FROM repositories WHERE id = p_id);
        WHEN 'project' THEN RETURN (SELECT id FROM projects WHERE id = p_id);
        WHEN 'agent_instance' THEN RETURN (SELECT project_id FROM agent_instances WHERE id = p_id);
        WHEN 'gateway' THEN RETURN (SELECT project_id FROM gateways WHERE id = p_id AND lifecycle <> 'removed');
        WHEN 'run' THEN RETURN (SELECT instance.project_id FROM runs run JOIN agent_instances instance ON instance.id = run.instance_id WHERE run.id = p_id);
        WHEN 'state_volume' THEN RETURN (SELECT project_id FROM agent_instance_state_volumes WHERE id = p_id);
    END CASE;
    RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION capability_resource_project(text, uuid) FROM PUBLIC;

-- Keep legacy instance invalidations and additionally invalidate the stable
-- owning project. Standalone resources never emit a NULL instance scope.
CREATE FUNCTION capture_private_volume_resource_event() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    volume agent_instance_state_volumes%ROWTYPE;
    change_kind text;
    occurrence uuid := COALESCE(NULLIF(current_setting('hephaestus.occurrence_id', true), '')::uuid, gen_random_uuid());
    organization uuid;
BEGIN
    IF TG_OP = 'DELETE' THEN volume := OLD; ELSE volume := NEW; END IF;
    change_kind := CASE TG_OP WHEN 'INSERT' THEN 'created' WHEN 'DELETE' THEN 'removed' ELSE 'updated' END;
    IF TG_OP = 'UPDATE' AND NEW.state IS DISTINCT FROM OLD.state THEN change_kind := 'state_changed'; END IF;
    IF volume.instance_id IS NOT NULL THEN
        PERFORM append_application_event(occurrence, 'agent_instance', volume.instance_id,
            'agent_instance', volume.instance_id, 'agent_instance.changed', change_kind,
            volume.state, volume.project_id, NULL);
    END IF;
    SELECT organization_id INTO organization FROM projects WHERE id = volume.project_id;
    PERFORM append_application_event(occurrence, 'project', volume.project_id,
        'project', volume.project_id, 'project.changed', change_kind, NULL, organization, NULL);
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END $$;
REVOKE ALL ON FUNCTION capture_private_volume_resource_event() FROM PUBLIC;
DROP TRIGGER agent_instance_state_volumes_application_event ON agent_instance_state_volumes;
CREATE TRIGGER agent_instance_state_volumes_application_event
AFTER INSERT OR UPDATE OR DELETE ON agent_instance_state_volumes
FOR EACH ROW EXECUTE FUNCTION capture_private_volume_resource_event();

DROP POLICY agent_instance_state_volumes_select ON agent_instance_state_volumes;
DROP POLICY agent_instance_state_volumes_write ON agent_instance_state_volumes;
CREATE POLICY private_volume_select ON agent_instance_state_volumes FOR SELECT TO hephaestus_app
USING (check_permission('user', hephaestus_actor_id(), 'can_read', 'state_volume', id::text) = 1
    -- INSERT RETURNING runs before the new object's tuple is visible to the
    -- stable evaluator. Match the model's exact owning-project manager grant.
    OR check_permission('user', hephaestus_actor_id(), 'can_manage', 'project', project_id::text) = 1);
CREATE POLICY private_volume_insert ON agent_instance_state_volumes FOR INSERT TO hephaestus_app
WITH CHECK (
    (instance_id IS NULL AND check_permission('user', hephaestus_actor_id(), 'can_manage', 'project', project_id::text) = 1)
    OR (instance_id IS NOT NULL AND check_permission('user', hephaestus_actor_id(), 'can_update', 'agent_instance', instance_id::text) = 1)
);
CREATE POLICY private_volume_update ON agent_instance_state_volumes FOR UPDATE TO hephaestus_app
USING (check_permission('user', hephaestus_actor_id(), 'can_manage', 'state_volume', id::text) = 1)
WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_manage', 'state_volume', id::text) = 1);
CREATE POLICY private_volume_delete ON agent_instance_state_volumes FOR DELETE TO hephaestus_app
USING (check_permission('user', hephaestus_actor_id(), 'can_manage', 'state_volume', id::text) = 1);

CREATE FUNCTION private_volume_guest_path_is_valid(path text) RETURNS boolean
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT COALESCE(octet_length(path) BETWEEN 2 AND 256 AND left(path, 1) = '/'
        AND right(path, 1) <> '/' AND position('//' IN path) = 0
        AND position(chr(92) IN path) = 0 AND path !~ '[[:cntrl:]]'
        AND path !~ '(^|/)\.{1,2}(/|$)'
        AND path !~ '^/(proc|sys|dev|run|release|workspace)(/|$)', false)
$$;
REVOKE ALL ON FUNCTION private_volume_guest_path_is_valid(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION private_volume_guest_path_is_valid(text) TO hephaestus_app, hephaestus_worker;

-- Exact release ceilings and selected IDs are evidence, not grants. These
-- rows contribute no agent_attach tuples. Runtime materialization is deferred.
CREATE TABLE agent_instance_revision_volume_bindings (
    instance_revision_id uuid NOT NULL,
    instance_id uuid NOT NULL REFERENCES agent_instances(id),
    project_id uuid NOT NULL REFERENCES projects(id),
    release_agent_id uuid NOT NULL REFERENCES release_agents(id),
    slot_key text NOT NULL CHECK (slot_key ~ '^[a-z][a-z0-9_-]{0,63}$'),
    volume_id uuid NOT NULL,
    access_mode text NOT NULL CHECK (access_mode IN ('read_only', 'read_write')),
    guest_path text NOT NULL CHECK (private_volume_guest_path_is_valid(guest_path)),
    slot_required boolean NOT NULL,
    minimum_capacity_bytes bigint NOT NULL CHECK (minimum_capacity_bytes BETWEEN 1 AND 17592186044416),
    provenance text NOT NULL CHECK (provenance IN ('explicit', 'legacy_state')),
    created_by uuid REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (instance_revision_id, slot_key),
    UNIQUE (instance_revision_id, volume_id),
    FOREIGN KEY (instance_id, instance_revision_id) REFERENCES agent_instance_revisions(instance_id, id),
    FOREIGN KEY (instance_id, project_id) REFERENCES agent_instances(id, project_id),
    FOREIGN KEY (volume_id, project_id) REFERENCES agent_instance_state_volumes(id, project_id)
);

-- Backfill only the old pointer's proven same-instance, same-project relation.
-- Preserve incompatible historical capability catalogs without inventing slots
-- or consumer grants; their old runtime and mailbox records remain untouched.
INSERT INTO agent_instance_revision_volume_bindings
    (instance_revision_id, instance_id, project_id, release_agent_id, slot_key,
     volume_id, access_mode, guest_path, slot_required, minimum_capacity_bytes, provenance)
SELECT revision.id, instance.id, instance.project_id, revision.release_agent_id, 'state',
    volume.id, 'read_write', '/var/lib/hephaestus', true, 1, 'legacy_state'
FROM agent_instance_revisions revision
JOIN agent_instances instance ON instance.id = revision.instance_id
JOIN agent_instance_state_volumes volume ON volume.id = instance.state_volume_id
    AND volume.instance_id = instance.id AND volume.project_id = instance.project_id
JOIN release_agents agent ON agent.id = revision.release_agent_id AND agent.requires_state
WHERE NOT EXISTS (SELECT 1 FROM release_capability_requirements requirement
    WHERE requirement.release_agent_id = agent.id AND requirement.slot_key = 'state')
AND NOT EXISTS (SELECT 1 FROM jsonb_array_elements(COALESCE(agent.runtime_contract->'volume_slots', '[]'::jsonb)) declaration
    WHERE declaration->>'slot' = 'state');

CREATE FUNCTION enforce_revision_volume_binding() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    revision agent_instance_revisions%ROWTYPE;
    agent release_agents%ROWTYPE;
    volume agent_instance_state_volumes%ROWTYPE;
    declaration jsonb;
    legacy_volume uuid;
    matching_count bigint;
BEGIN
    -- Serialize overlap/count checks for distinct slots in the same revision.
    SELECT * INTO revision FROM agent_instance_revisions WHERE id = NEW.instance_revision_id FOR UPDATE;
    SELECT * INTO agent FROM release_agents WHERE id = revision.release_agent_id;
    SELECT * INTO volume FROM agent_instance_state_volumes WHERE id = NEW.volume_id;
    IF revision.id IS NULL OR revision.instance_id <> NEW.instance_id
       OR revision.release_agent_id <> NEW.release_agent_id OR volume.id IS NULL
       OR volume.project_id <> NEW.project_id THEN
        RAISE EXCEPTION 'volume binding does not match exact revision and owner' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF EXISTS (SELECT 1 FROM release_capability_requirements
        WHERE release_agent_id = agent.id AND slot_key = NEW.slot_key) THEN
        RAISE EXCEPTION 'volume slot collides with a generic capability catalog' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    SELECT count(*), min(value::text)::jsonb INTO matching_count, declaration
    FROM jsonb_array_elements(COALESCE(agent.runtime_contract->'volume_slots', '[]'::jsonb))
    WHERE value->>'slot' = NEW.slot_key;
    IF NEW.provenance = 'legacy_state' THEN
        SELECT state_volume_id INTO legacy_volume FROM agent_instances WHERE id = NEW.instance_id;
        IF NOT agent.requires_state OR NEW.slot_key <> 'state' OR matching_count <> 0
           OR legacy_volume IS DISTINCT FROM NEW.volume_id OR volume.instance_id IS DISTINCT FROM NEW.instance_id THEN
            RAISE EXCEPTION 'legacy volume binding lacks proven origin pointer' USING ERRCODE = 'integrity_constraint_violation';
        END IF;
        declaration := jsonb_build_object('slot', 'state', 'guest_path', '/var/lib/hephaestus',
            'access_mode', 'read_write', 'required', true, 'minimum_capacity_bytes', 1);
    ELSIF matching_count <> 1 THEN
        RAISE EXCEPTION 'volume slot is not declared exactly once' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF declaration->>'guest_path' IS DISTINCT FROM NEW.guest_path
       OR declaration->>'access_mode' IS DISTINCT FROM NEW.access_mode
       OR (declaration->>'required')::boolean IS DISTINCT FROM NEW.slot_required
       OR (declaration->>'minimum_capacity_bytes')::bigint IS DISTINCT FROM NEW.minimum_capacity_bytes
       OR volume.capacity_bytes < NEW.minimum_capacity_bytes THEN
        RAISE EXCEPTION 'volume binding exceeds or mismatches release ceiling' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF (SELECT count(*) FROM agent_instance_revision_volume_bindings WHERE instance_revision_id = NEW.instance_revision_id) >= 32
       OR EXISTS (SELECT 1 FROM agent_instance_revision_volume_bindings previous
        WHERE previous.instance_revision_id = NEW.instance_revision_id AND (
            previous.guest_path = NEW.guest_path
            OR position(previous.guest_path || '/' IN NEW.guest_path) = 1
            OR position(NEW.guest_path || '/' IN previous.guest_path) = 1)) THEN
        RAISE EXCEPTION 'volume binding count or mount overlap exceeds contract' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_revision_volume_binding() FROM PUBLIC;
CREATE TRIGGER revision_volume_binding_integrity BEFORE INSERT ON agent_instance_revision_volume_bindings
FOR EACH ROW EXECUTE FUNCTION enforce_revision_volume_binding();
CREATE TRIGGER revision_volume_binding_immutable BEFORE UPDATE OR DELETE ON agent_instance_revision_volume_bindings
FOR EACH ROW EXECUTE FUNCTION reject_capability_record_mutation();

ALTER TABLE agent_instance_revision_volume_bindings ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_revision_volume_bindings FORCE ROW LEVEL SECURITY;
GRANT SELECT, INSERT ON agent_instance_revision_volume_bindings TO hephaestus_app, hephaestus_worker;
CREATE POLICY revision_volume_binding_select ON agent_instance_revision_volume_bindings FOR SELECT TO hephaestus_app
USING (check_permission('user', hephaestus_actor_id(), 'can_read', 'agent_instance', instance_id::text) = 1);
CREATE POLICY revision_volume_binding_insert ON agent_instance_revision_volume_bindings FOR INSERT TO hephaestus_app
WITH CHECK (created_by = hephaestus_actor_id()::uuid
    AND check_permission('user', hephaestus_actor_id(), 'can_manage', 'agent_instance', instance_id::text) = 1
    AND check_permission('user', hephaestus_actor_id(), 'can_grant_agent_capability', 'state_volume', volume_id::text) = 1
    AND check_permission('user', hephaestus_actor_id(), 'can_attach', 'state_volume', volume_id::text) = 1);
CREATE POLICY revision_volume_binding_worker ON agent_instance_revision_volume_bindings TO hephaestus_worker
USING (true) WITH CHECK (true);
CREATE TRIGGER revision_volume_binding_application_event AFTER INSERT ON agent_instance_revision_volume_bindings
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'agent_instance', 'instance_id', 'agent_instance', 'instance_id', 'agent_instance.changed', '');

-- Deliberately preserve old lease composite FKs, both active-lease uniqueness
-- indexes, scalar run/mailbox evidence, and source-instance RLS for leases.
-- Standalone attachment needs explicit plural runtime/evidence integration.
