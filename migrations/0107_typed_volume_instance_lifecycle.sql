-- Typed imports retain named evidence without fabricating a legacy pointer.
-- Runtime support and terminal cleanup are deliberately separate live boundaries.
ALTER TABLE agent_instances ADD COLUMN volume_mode text NOT NULL DEFAULT 'legacy'
    CHECK (volume_mode IN ('legacy','named'));
ALTER TABLE agent_instances ADD CONSTRAINT named_instance_has_no_scalar_volume
    CHECK (volume_mode <> 'named' OR state_volume_id IS NULL);
ALTER TABLE release_command_inbox ADD COLUMN input_hash bytea
    CHECK (input_hash IS NULL OR octet_length(input_hash)=32);
ALTER TABLE release_command_inbox ADD CONSTRAINT typed_instance_command_has_input_hash
    CHECK(operation NOT IN ('import_agent_with_volumes','request_instance_removal') OR input_hash IS NOT NULL);
CREATE FUNCTION reject_typed_instance_command_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$ BEGIN
    IF OLD.operation IN ('import_agent_with_volumes','request_instance_removal') THEN
        RAISE EXCEPTION 'typed instance command evidence is immutable' USING ERRCODE='integrity_constraint_violation';
    END IF;
    RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END $$;
REVOKE ALL ON FUNCTION reject_typed_instance_command_mutation() FROM PUBLIC;
CREATE TRIGGER typed_instance_command_immutable BEFORE UPDATE OR DELETE ON release_command_inbox
    FOR EACH ROW EXECUTE FUNCTION reject_typed_instance_command_mutation();
ALTER TABLE agent_instance_revision_volume_bindings
    DROP CONSTRAINT agent_instance_revision_volume_bindings_provenance_check;
ALTER TABLE agent_instance_revision_volume_bindings
    ADD CONSTRAINT revision_volume_binding_provenance
    CHECK(provenance IN ('explicit','legacy_state','legacy_declaration'));

CREATE TABLE agent_instance_removal_requests (
    id uuid PRIMARY KEY CHECK(id<>'00000000-0000-0000-0000-000000000000'::uuid),
    instance_id uuid NOT NULL UNIQUE REFERENCES agent_instances(id),
    expected_version bigint NOT NULL CHECK(expected_version>=0),
    input_hash bytea NOT NULL CHECK(octet_length(input_hash)=32),
    created_by uuid NOT NULL REFERENCES users(id),
    request_id uuid NOT NULL,
    authorization_model_version text NOT NULL CHECK(length(authorization_model_version) BETWEEN 1 AND 128),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TRIGGER instance_removal_request_immutable BEFORE UPDATE OR DELETE ON agent_instance_removal_requests
    FOR EACH ROW EXECUTE FUNCTION reject_capability_record_mutation();
CREATE FUNCTION enforce_instance_removal_context() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE consumer agent_instances%ROWTYPE;
BEGIN
    SELECT * INTO consumer FROM agent_instances WHERE id=NEW.instance_id FOR UPDATE;
    IF NEW.created_by IS DISTINCT FROM NULLIF(current_setting('hephaestus.actor_id',true),'')::uuid
       OR NEW.request_id IS DISTINCT FROM NULLIF(current_setting('hephaestus.request_id',true),'')::uuid
       OR NEW.expected_version IS DISTINCT FROM consumer.version OR consumer.state='removed' THEN
        RAISE EXCEPTION 'removal admission lacks exact current actor/version context' USING ERRCODE='integrity_constraint_violation';
    END IF;
    UPDATE agent_instances SET run_gate_open=false WHERE id=NEW.instance_id;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_instance_removal_context() FROM PUBLIC;
CREATE TRIGGER instance_removal_request_context BEFORE INSERT ON agent_instance_removal_requests
    FOR EACH ROW EXECUTE FUNCTION enforce_instance_removal_context();
ALTER TABLE agent_instance_removal_requests ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_removal_requests FORCE ROW LEVEL SECURITY;
GRANT SELECT,INSERT ON agent_instance_removal_requests TO hephaestus_app;
GRANT SELECT ON agent_instance_removal_requests TO hephaestus_worker,hephaestus_authz_owner;
CREATE POLICY instance_removal_read ON agent_instance_removal_requests FOR SELECT TO hephaestus_app
    USING(check_permission('user',hephaestus_actor_id(),'can_read','agent_instance',instance_id::text)=1);
CREATE POLICY instance_removal_create ON agent_instance_removal_requests FOR INSERT TO hephaestus_app
    WITH CHECK(created_by=hephaestus_actor_id()::uuid AND check_permission('user',hephaestus_actor_id(),'can_manage','agent_instance',instance_id::text)=1);
CREATE POLICY instance_removal_worker_read ON agent_instance_removal_requests FOR SELECT TO hephaestus_worker USING(true);
CREATE TRIGGER instance_removal_application_event AFTER INSERT ON agent_instance_removal_requests
    FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event('agent_instance','instance_id','agent_instance','instance_id','agent_instance.changed','');

-- Historical inbox operations have no actor policies and retain their existing
-- worker path. New actor operations expose only their exact consumer evidence.
CREATE POLICY typed_instance_command_read ON release_command_inbox FOR SELECT TO hephaestus_app
    USING(operation IN ('import_agent_with_volumes','request_instance_removal')
      AND actor_id=hephaestus_actor_id()::uuid
      AND check_permission('user',hephaestus_actor_id(),'can_read','agent_instance',aggregate_id::text)=1);
CREATE POLICY typed_instance_command_create ON release_command_inbox FOR INSERT TO hephaestus_app
    WITH CHECK(actor_id=hephaestus_actor_id()::uuid
      AND request_id=NULLIF(current_setting('hephaestus.request_id',true),'')::uuid
      AND input_hash IS NOT NULL
      AND check_permission('user',hephaestus_actor_id(),'can_manage','agent_instance',aggregate_id::text)=1
      AND ((operation='import_agent_with_volumes' AND EXISTS(
            SELECT 1 FROM agent_instances instance JOIN agent_instance_revisions revision
              ON revision.id=secondary_id AND revision.instance_id=instance.id
            WHERE instance.id=aggregate_id AND instance.volume_mode='named'
              AND instance.active_revision_id=revision.id
              AND instance.created_by=actor_id AND revision.created_by=actor_id))
        OR (operation='request_instance_removal' AND EXISTS(
            SELECT 1 FROM agent_instance_removal_requests removal
            WHERE removal.id=secondary_id AND removal.instance_id=aggregate_id
              AND removal.created_by=actor_id AND removal.request_id=release_command_inbox.request_id
              AND removal.input_hash=release_command_inbox.input_hash))));

-- Only committed typed command evidence admits these two actor event forms.
-- Historical event producers retain their existing worker path and policies.
CREATE POLICY typed_instance_event_create ON agent_instance_events FOR INSERT TO hephaestus_app
    WITH CHECK(actor_id=hephaestus_actor_id()::uuid
      AND request_id=NULLIF(current_setting('hephaestus.request_id',true),'')::uuid
      AND update_id IS NULL
      AND check_permission('user',hephaestus_actor_id(),'can_manage','agent_instance',instance_id::text)=1
      AND EXISTS(SELECT 1 FROM release_command_inbox command
        WHERE command.aggregate_id=agent_instance_events.instance_id
          AND command.actor_id=agent_instance_events.actor_id
          AND command.request_id=agent_instance_events.request_id
          AND ((command.operation='import_agent_with_volumes'
              AND agent_instance_events.event_type='instance.created'
              AND command.secondary_id=agent_instance_events.revision_id
              AND agent_instance_events.payload='{"runnable":true,"volume_mode":"named","dispatch_supported":false}'::jsonb
              AND EXISTS(SELECT 1 FROM agent_instances instance
                WHERE instance.id=command.aggregate_id AND instance.volume_mode='named'
                  AND instance.active_revision_id=command.secondary_id))
            OR (command.operation='request_instance_removal'
              AND agent_instance_events.event_type='instance.removal_requested'
              AND agent_instance_events.revision_id IS NULL
              AND agent_instance_events.payload=jsonb_build_object('removal_id',command.secondary_id,'cleanup_complete',false)
              AND EXISTS(SELECT 1 FROM agent_instance_removal_requests removal
                WHERE removal.id=command.secondary_id AND removal.instance_id=command.aggregate_id
                  AND removal.created_by=command.actor_id AND removal.request_id=command.request_id
                  AND removal.input_hash=command.input_hash)))));

-- 0108 may replace this predicate only with the actual plural runtime gate.
-- It must not mutate the immutable completeness of published revisions.
CREATE FUNCTION named_volume_dispatch_supported(p_instance uuid) RETURNS boolean
LANGUAGE sql IMMUTABLE AS $$ SELECT false $$;
REVOKE ALL ON FUNCTION named_volume_dispatch_supported(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION named_volume_dispatch_supported(uuid) TO hephaestus_app,hephaestus_worker;

CREATE FUNCTION enforce_instance_volume_lifecycle() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
    IF TG_OP='UPDATE' AND NEW.volume_mode IS DISTINCT FROM OLD.volume_mode THEN
        RAISE EXCEPTION 'instance volume mode is immutable' USING ERRCODE='integrity_constraint_violation';
    END IF;
    IF NEW.volume_mode='named' AND (NEW.state_volume_id IS NOT NULL OR (NEW.run_gate_open AND NOT named_volume_dispatch_supported(NEW.id))) THEN
        RAISE EXCEPTION 'named runtime dispatch is unsupported' USING ERRCODE='integrity_constraint_violation';
    END IF;
    IF TG_OP='UPDATE' AND NEW.volume_mode='named' AND OLD.active_revision_id IS NOT NULL AND NEW.active_revision_id IS DISTINCT FROM OLD.active_revision_id THEN
        RAISE EXCEPTION 'typed revision updates are unsupported' USING ERRCODE='integrity_constraint_violation';
    END IF;
    IF EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=NEW.id)
       AND (NEW.run_gate_open OR (TG_OP='UPDATE' AND NEW.active_revision_id IS DISTINCT FROM OLD.active_revision_id) OR NEW.state='removed') THEN
        RAISE EXCEPTION 'permanent closure forbids reopening or unproven tombstone' USING ERRCODE='integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_instance_volume_lifecycle() FROM PUBLIC;
CREATE TRIGGER instance_volume_lifecycle BEFORE INSERT OR UPDATE ON agent_instances
    FOR EACH ROW EXECUTE FUNCTION enforce_instance_volume_lifecycle();

-- All admissions serialize against the same instance lock as permanent close.
-- Existing active lease heartbeats and cleanup updates remain possible.
-- UPDATE callers may already own run/delivery locks. NOWAIT prevents a cycle
-- with removal's instance-first order; contention fails with 55P03 and no work
-- is admitted. Durable worker delivery may retry, but not every caller has an
-- automatic retry policy; operators receive a real transient storage error.
CREATE FUNCTION assert_instance_accepts_work(p_instance uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE consumer agent_instances%ROWTYPE;
BEGIN
    SELECT * INTO consumer FROM agent_instances WHERE id=p_instance FOR UPDATE NOWAIT;
    IF consumer.id IS NULL OR consumer.state='removed' OR EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=p_instance)
       OR (consumer.volume_mode='named' AND NOT named_volume_dispatch_supported(p_instance)) THEN
        RAISE EXCEPTION 'instance is closed to new work' USING ERRCODE='integrity_constraint_violation';
    END IF;
END $$;
REVOKE ALL ON FUNCTION assert_instance_accepts_work(uuid) FROM PUBLIC;
CREATE FUNCTION enforce_instance_work_admission() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
    PERFORM assert_instance_accepts_work(NEW.instance_id);
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_instance_work_admission() FROM PUBLIC;
CREATE TRIGGER instance_run_admission BEFORE INSERT ON runs FOR EACH ROW EXECUTE FUNCTION enforce_instance_work_admission();
CREATE TRIGGER instance_run_request_admission BEFORE INSERT ON run_requests FOR EACH ROW EXECUTE FUNCTION enforce_instance_work_admission();
CREATE TRIGGER instance_lease_admission BEFORE INSERT ON agent_instance_volume_leases FOR EACH ROW EXECUTE FUNCTION enforce_instance_work_admission();
CREATE TRIGGER instance_mailbox_admission BEFORE INSERT ON mailbox_events FOR EACH ROW EXECUTE FUNCTION enforce_instance_work_admission();
CREATE TRIGGER instance_deferred_admission BEFORE INSERT ON deferred_agent_triggers FOR EACH ROW EXECUTE FUNCTION enforce_instance_work_admission();
CREATE TRIGGER instance_update_admission BEFORE INSERT ON agent_updates FOR EACH ROW EXECUTE FUNCTION enforce_instance_work_admission();
CREATE FUNCTION enforce_instance_revision_admission() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE consumer agent_instances%ROWTYPE;
BEGIN
    SELECT * INTO consumer FROM agent_instances WHERE id=NEW.instance_id FOR UPDATE;
    IF EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=NEW.instance_id)
       OR (consumer.volume_mode='named' AND consumer.active_revision_id IS NOT NULL) THEN
        RAISE EXCEPTION 'closed instance or unsupported typed revision update' USING ERRCODE='integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_instance_revision_admission() FROM PUBLIC;
CREATE TRIGGER instance_revision_admission BEFORE INSERT ON agent_instance_revisions FOR EACH ROW EXECUTE FUNCTION enforce_instance_revision_admission();
CREATE FUNCTION enforce_instance_run_progress() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
    IF NEW.state IS DISTINCT FROM OLD.state AND NEW.state IN ('leasing_volume','provisioning','starting','running') THEN
        PERFORM assert_instance_accepts_work(NEW.instance_id);
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_instance_run_progress() FROM PUBLIC;
CREATE TRIGGER instance_run_progress BEFORE UPDATE ON runs FOR EACH ROW EXECUTE FUNCTION enforce_instance_run_progress();
CREATE FUNCTION enforce_instance_mailbox_attempt() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE consumer uuid;
BEGIN
    SELECT instance_id INTO consumer FROM mailboxes WHERE id=NEW.mailbox_id;
    IF TG_OP='INSERT' OR (NEW.state IS DISTINCT FROM OLD.state AND NEW.state='running') THEN
        PERFORM assert_instance_accepts_work(consumer);
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_instance_mailbox_attempt() FROM PUBLIC;
CREATE TRIGGER instance_mailbox_attempt BEFORE INSERT OR UPDATE ON mailbox_delivery_attempts FOR EACH ROW EXECUTE FUNCTION enforce_instance_mailbox_attempt();

-- Manager admission changes only scheduling/cancellation, never lease or VM
-- cleanup evidence. The privileged helper cannot be invoked without the exact
-- actor's admitted request and live consumer management authority.
CREATE FUNCTION close_instance_launches(p_removal uuid) RETURNS SETOF uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE admission agent_instance_removal_requests%ROWTYPE;
BEGIN
    SELECT * INTO admission FROM agent_instance_removal_requests WHERE id=p_removal;
    IF admission.id IS NULL OR admission.created_by IS DISTINCT FROM NULLIF(current_setting('hephaestus.actor_id',true),'')::uuid
       OR admission.request_id IS DISTINCT FROM NULLIF(current_setting('hephaestus.request_id',true),'')::uuid
       OR check_permission('user',hephaestus_actor_id(),'can_manage','agent_instance',admission.instance_id::text) IS DISTINCT FROM 1 THEN
        RAISE EXCEPTION 'closure needs current admitted consumer manager' USING ERRCODE='insufficient_privilege';
    END IF;
    PERFORM 1 FROM agent_instances WHERE id=admission.instance_id FOR UPDATE;
    UPDATE agent_instances SET run_gate_open=false,updated_at=now(),version=version+1 WHERE id=admission.instance_id;
    UPDATE mailboxes SET state='paused',updated_at=now() WHERE instance_id=admission.instance_id AND state='active';
    UPDATE mailbox_deliveries SET disposition='cancelled',terminal_at=now(),next_eligible_at=NULL,denial_code=NULL,updated_at=now()
        WHERE instance_id=admission.instance_id AND disposition IN ('pending','eligible','retryable');
    UPDATE run_requests SET dispatch_state='cancelled' WHERE instance_id=admission.instance_id AND dispatch_state='pending';
    UPDATE deferred_agent_triggers SET state='denied',resolved_at=now(),diagnostics='[{"code":"instance_removal_requested"}]'::jsonb
        WHERE instance_id=admission.instance_id AND state='deferred';
    RETURN QUERY UPDATE runs SET cancel_requested_at=COALESCE(cancel_requested_at,now()),updated_at=now()
        WHERE instance_id=admission.instance_id AND state<>'cleaned_up' RETURNING id;
END $$;
REVOKE ALL ON FUNCTION close_instance_launches(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION close_instance_launches(uuid) TO hephaestus_app;

CREATE OR REPLACE FUNCTION enforce_revision_volume_binding() RETURNS trigger
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
    ELSIF NEW.provenance = 'legacy_declaration' THEN
        IF NOT agent.requires_state OR NEW.slot_key <> 'state' OR matching_count <> 0
           OR NOT EXISTS(SELECT 1 FROM agent_instances WHERE id=NEW.instance_id AND volume_mode='named' AND state_volume_id IS NULL) THEN
            RAISE EXCEPTION 'legacy declaration requires named consumer and frozen legacy ceiling' USING ERRCODE='integrity_constraint_violation';
        END IF;
        declaration := jsonb_build_object('slot','state','guest_path','/var/lib/hephaestus',
            'access_mode','read_write','required',true,'minimum_capacity_bytes',1);
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

CREATE OR REPLACE FUNCTION private_volume_mount_binding_is_declared(p_revision uuid,p_slot text) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public AS $$
    SELECT EXISTS(SELECT 1 FROM agent_instance_revision_volume_bindings binding
        JOIN agent_instance_revisions revision ON revision.id=binding.instance_revision_id
        JOIN release_agents agent ON agent.id=revision.release_agent_id
        JOIN agent_instances instance ON instance.id=binding.instance_id
        JOIN agent_instance_state_volumes volume ON volume.id=binding.volume_id
        WHERE binding.instance_revision_id=p_revision AND binding.slot_key=p_slot
          AND binding.release_agent_id=agent.id AND binding.instance_id=revision.instance_id
          AND binding.project_id=instance.project_id AND binding.project_id=volume.project_id
          AND volume.capacity_bytes>=binding.minimum_capacity_bytes
          AND NOT EXISTS(SELECT 1 FROM release_capability_requirements requirement
              WHERE requirement.release_agent_id=agent.id AND requirement.slot_key=binding.slot_key)
          AND (
            (binding.provenance='explicit' AND
              (SELECT count(*) FROM jsonb_array_elements(CASE WHEN jsonb_typeof(agent.runtime_contract->'volume_slots')='array'
                  THEN agent.runtime_contract->'volume_slots' ELSE '[]'::jsonb END) declaration
               WHERE declaration->>'slot'=binding.slot_key)=1 AND
              EXISTS(SELECT 1 FROM jsonb_array_elements(CASE WHEN jsonb_typeof(agent.runtime_contract->'volume_slots')='array'
                  THEN agent.runtime_contract->'volume_slots' ELSE '[]'::jsonb END) declaration
               WHERE declaration->>'slot'=binding.slot_key
                 AND declaration->>'guest_path'=binding.guest_path
                 AND declaration->>'access_mode'=binding.access_mode
                 AND declaration->'required'=to_jsonb(binding.slot_required)
                 AND declaration->'minimum_capacity_bytes'=to_jsonb(binding.minimum_capacity_bytes)))
            OR (binding.provenance IN ('legacy_state','legacy_declaration') AND agent.requires_state
                AND binding.slot_key='state' AND binding.guest_path='/var/lib/hephaestus'
                AND binding.access_mode='read_write' AND binding.slot_required
                AND binding.minimum_capacity_bytes=1
                AND ((binding.provenance='legacy_state' AND volume.instance_id=instance.id AND instance.state_volume_id=volume.id)
                  OR (binding.provenance='legacy_declaration' AND instance.volume_mode='named' AND instance.state_volume_id IS NULL))
                AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(CASE WHEN jsonb_typeof(agent.runtime_contract->'volume_slots')='array'
                    THEN agent.runtime_contract->'volume_slots' ELSE '[]'::jsonb END) declaration WHERE declaration->>'slot'='state'))))
$$;
ALTER FUNCTION private_volume_mount_binding_is_declared(uuid,text) OWNER TO hephaestus_authz_owner;
REVOKE ALL ON FUNCTION private_volume_mount_binding_is_declared(uuid,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION private_volume_mount_binding_is_declared(uuid,text) TO hephaestus_app,hephaestus_worker;
GRANT SELECT ON agent_instance_revision_volume_bindings,agent_instance_revisions,release_agents,
    agent_instances,release_capability_requirements TO hephaestus_authz_owner;


-- Closed consumers cannot acquire fresh authority after withdrawal.
CREATE OR REPLACE FUNCTION enforce_volume_mount_grant() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE binding agent_instance_revision_volume_bindings%ROWTYPE;
BEGIN
    IF NEW.created_by IS DISTINCT FROM NULLIF(current_setting('hephaestus.actor_id',true),'')::uuid
       OR NEW.request_id IS DISTINCT FROM NULLIF(current_setting('hephaestus.request_id',true),'')::uuid THEN
        RAISE EXCEPTION 'explicit mount grant requires actor request context' USING ERRCODE='insufficient_privilege';
    END IF;
    -- Serialize deliberate grants with permanent close before looking at
    -- closure or inserting authority. Removal lists only after the same lock.
    PERFORM 1 FROM agent_instances WHERE id=NEW.instance_id FOR UPDATE;
    SELECT * INTO binding FROM agent_instance_revision_volume_bindings
        WHERE instance_revision_id=NEW.instance_revision_id AND slot_key=NEW.slot_key;
    IF binding.instance_id IS DISTINCT FROM NEW.instance_id
       OR binding.release_agent_id IS DISTINCT FROM NEW.release_agent_id
       OR binding.volume_id IS DISTINCT FROM NEW.volume_id OR binding.access_mode IS DISTINCT FROM NEW.access_mode
       OR NOT private_volume_mount_binding_is_declared(NEW.instance_revision_id,NEW.slot_key)
       OR NOT EXISTS(SELECT 1 FROM release_agents agent JOIN releases release ON release.id=agent.release_id
            WHERE agent.id=NEW.release_agent_id AND agent.runtime_contract_hash=NEW.release_contract_hash AND release.state='published')
       OR EXISTS(SELECT 1 FROM agent_updates WHERE candidate_revision_id=NEW.instance_revision_id)
       OR EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=NEW.instance_id) THEN
        RAISE EXCEPTION 'mount grant differs from exact frozen declaration or uses unsupported update candidate'
            USING ERRCODE='integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_volume_mount_grant() FROM PUBLIC;

CREATE FUNCTION enforce_instance_lease_progress() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
    IF NEW.state='active' AND OLD.state<>'active' THEN
        PERFORM assert_instance_accepts_work(NEW.instance_id);
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_instance_lease_progress() FROM PUBLIC;
CREATE TRIGGER instance_lease_progress BEFORE UPDATE ON agent_instance_volume_leases
    FOR EACH ROW EXECUTE FUNCTION enforce_instance_lease_progress();
