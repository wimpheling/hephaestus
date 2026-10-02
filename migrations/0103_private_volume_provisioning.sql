-- Provisioning intent stays on the canonical volume resource. Progress fencing
-- is separate from run attachment leases; neither changes legacy lease fences.
ALTER TABLE agent_instance_state_volumes
    ADD COLUMN provisioning_state text NOT NULL DEFAULT 'reserved'
        CHECK (provisioning_state IN ('reserved','creating','formatting','ready','uncertain')),
    ADD COLUMN provisioning_generation bigint NOT NULL DEFAULT 0
        CHECK (provisioning_generation >= 0);
UPDATE agent_instance_state_volumes SET provisioning_state = CASE
    WHEN state <> 'uninitialized' THEN 'ready'
    WHEN host_id IS NOT NULL THEN 'uncertain'
    ELSE 'reserved' END;
-- The original unnamed check couples UUID assignment to host handles. New
-- standalone resources predict their UUID before a worker reserves a host.
ALTER TABLE agent_instance_state_volumes DROP CONSTRAINT agent_instance_state_volumes_check;
-- Existing <=102 standalone registrations could omit a filesystem identity.
-- Assign it once only when stable ownership is present and no host/backing
-- intent exists. Local recovery still rejects any unknown preexisting bytes.
UPDATE agent_instance_state_volumes SET filesystem_uuid = gen_random_uuid()
WHERE instance_id IS NULL AND filesystem_uuid IS NULL
  AND host_id IS NULL AND host_path IS NULL AND project_id IS NOT NULL;
ALTER TABLE agent_instance_state_volumes ADD CONSTRAINT private_volume_provider_handles
    CHECK ((host_id IS NULL) = (host_path IS NULL)
       AND (host_id IS NULL OR filesystem_uuid IS NOT NULL)
       AND (state = 'uninitialized' OR
            (host_id IS NOT NULL AND host_path IS NOT NULL AND filesystem_uuid IS NOT NULL)));
ALTER TABLE agent_instance_state_volumes ADD CONSTRAINT private_volume_provisioned_ready
    CHECK (provisioning_state <> 'ready' OR
           (state <> 'uninitialized' AND host_id IS NOT NULL AND filesystem_uuid IS NOT NULL));
CREATE FUNCTION enforce_private_volume_intent() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF NEW.instance_id IS NULL AND
           (NEW.filesystem_uuid IS NULL OR NEW.filesystem_uuid = '00000000-0000-0000-0000-000000000000'::uuid
            OR NEW.capacity_bytes < 16777216 OR NEW.capacity_bytes % 4096 <> 0) THEN
            RAISE EXCEPTION 'standalone volume needs a valid filesystem UUID and aligned provider capacity'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
        IF current_user = 'hephaestus_app' AND
           (NEW.host_id IS NOT NULL OR NEW.provisioning_state <> 'reserved'
            OR NEW.provisioning_generation <> 0 OR NEW.state <> 'uninitialized') THEN
            RAISE EXCEPTION 'actor registration cannot assign provider handles or provisioning progress'
                USING ERRCODE = 'insufficient_privilege';
        END IF;
        RETURN NEW;
    END IF;
    IF (OLD.instance_id IS NULL OR OLD.host_id IS NOT NULL) AND
       (NEW.capacity_bytes IS DISTINCT FROM OLD.capacity_bytes
        OR NEW.filesystem_uuid IS DISTINCT FROM OLD.filesystem_uuid) THEN
        RAISE EXCEPTION 'reserved volume capacity and filesystem UUID are immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.filesystem_uuid IS NOT NULL AND NEW.filesystem_uuid IS DISTINCT FROM OLD.filesystem_uuid THEN
        RAISE EXCEPTION 'filesystem UUID cannot change after reservation'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.host_id IS NOT NULL AND
       (NEW.host_id IS DISTINCT FROM OLD.host_id OR NEW.host_path IS DISTINCT FROM OLD.host_path) THEN
        RAISE EXCEPTION 'reserved provider handles are immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF current_user = 'hephaestus_app' AND
       (NEW.host_id IS DISTINCT FROM OLD.host_id OR NEW.host_path IS DISTINCT FROM OLD.host_path
        OR NEW.provisioning_state IS DISTINCT FROM OLD.provisioning_state
        OR NEW.provisioning_generation IS DISTINCT FROM OLD.provisioning_generation
        OR NEW.capacity_bytes IS DISTINCT FROM OLD.capacity_bytes
        OR NEW.filesystem_uuid IS DISTINCT FROM OLD.filesystem_uuid) THEN
        RAISE EXCEPTION 'provider reservation and progress require a trusted worker'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    IF NEW.provisioning_generation <> OLD.provisioning_generation
       AND (NEW.provisioning_generation <> OLD.provisioning_generation + 1
            OR NEW.provisioning_state <> OLD.provisioning_state
            OR OLD.provisioning_state = 'ready') THEN
        RAISE EXCEPTION 'invalid provisioning claim generation'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NEW.provisioning_state IS DISTINCT FROM OLD.provisioning_state AND
       (NEW.provisioning_generation <> OLD.provisioning_generation
        OR NOT ((OLD.provisioning_state IN ('reserved','uncertain') AND NEW.provisioning_state IN ('creating','ready','uncertain'))
             OR (OLD.provisioning_state = 'creating' AND NEW.provisioning_state IN ('formatting','ready','uncertain'))
             OR (OLD.provisioning_state = 'formatting' AND NEW.provisioning_state IN ('creating','ready','uncertain')))) THEN
        RAISE EXCEPTION 'invalid provisioning progress transition'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_private_volume_intent() FROM PUBLIC;
CREATE TRIGGER private_volume_intent BEFORE INSERT OR UPDATE ON agent_instance_state_volumes
FOR EACH ROW EXECUTE FUNCTION enforce_private_volume_intent();
