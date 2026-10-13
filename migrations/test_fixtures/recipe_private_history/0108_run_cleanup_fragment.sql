-- PRIVATE: merge after 0108_run_volume_leases.sql exactly once. Do not publish
-- before legacy orchestration uses canonical cleanup. SQL records trusted
-- worker observations; it cannot prove physical VM/kernel destruction itself.
-- volume_acquisition_closed_at belongs to the shared metadata fragment.
ALTER TABLE runs
    ADD COLUMN vm_provider_namespace text,
    ADD COLUMN vm_host_id text,
    ADD COLUMN vm_binding_kind text CHECK(vm_binding_kind IN ('planned','persisted'));
CREATE FUNCTION run_cleanup_identifier_is_valid(value text) RETURNS boolean
LANGUAGE sql IMMUTABLE AS $$
    SELECT value IS NOT NULL AND value ~ '^[A-Za-z0-9_.:-]{1,128}$' AND value NOT IN ('.','..')
$$;
REVOKE ALL ON FUNCTION run_cleanup_identifier_is_valid(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION run_cleanup_identifier_is_valid(text) TO hephaestus_app,hephaestus_worker;
ALTER TABLE runs ADD CONSTRAINT run_vm_owner_scope_shape CHECK(
    num_nonnulls(vm_provider_namespace,vm_host_id,vm_binding_kind)=0
    OR (num_nonnulls(vm_provider_namespace,vm_host_id,vm_binding_kind)=3
        AND run_cleanup_identifier_is_valid(vm_provider_namespace)
        AND run_cleanup_identifier_is_valid(vm_host_id)
        AND run_cleanup_identifier_is_valid(vm_id)));

CREATE FUNCTION require_run_cleanup_worker() RETURNS void
LANGUAGE plpgsql AS $$ BEGIN
    IF current_user::text<>'hephaestus_worker' THEN
        RAISE EXCEPTION 'canonical cleanup requires trusted worker role' USING ERRCODE='42501';
    END IF;
END $$;
REVOKE ALL ON FUNCTION require_run_cleanup_worker() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION require_run_cleanup_worker() TO hephaestus_worker;

CREATE FUNCTION enforce_run_vm_owner_scope() RETURNS trigger
LANGUAGE plpgsql AS $$ BEGIN
    IF TG_OP='UPDATE' THEN
        IF OLD.vm_binding_kind IS NOT NULL AND
           ROW(NEW.id,NEW.instance_id,NEW.instance_revision_id,NEW.release_id,NEW.release_agent_id,NEW.attachment_id,NEW.run_kind,NEW.command_id,NEW.requires_state)
            IS DISTINCT FROM ROW(OLD.id,OLD.instance_id,OLD.instance_revision_id,OLD.release_id,OLD.release_agent_id,OLD.attachment_id,OLD.run_kind,OLD.command_id,OLD.requires_state) THEN
            RAISE EXCEPTION 'planned run inputs are immutable' USING ERRCODE='23000';
        END IF;
        IF ROW(NEW.vm_id,NEW.vm_provider_namespace,NEW.vm_host_id,NEW.vm_binding_kind)
            IS NOT DISTINCT FROM ROW(OLD.vm_id,OLD.vm_provider_namespace,OLD.vm_host_id,OLD.vm_binding_kind) THEN
            RETURN NEW;
        END IF;
        IF OLD.vm_id IS NOT NULL OR OLD.vm_binding_kind IS NOT NULL THEN
            RAISE EXCEPTION 'VM identity and owner are immutable or historical unresolved' USING ERRCODE='23000';
        END IF;
        IF NOT OLD.vm_plan_admission_required OR NOT EXISTS(SELECT 1 FROM run_vm_plan_admissions admission
            WHERE admission.run_id=NEW.id AND admission.command_id=NEW.command_id
              AND admission.instance_id=NEW.instance_id AND admission.instance_revision_id=NEW.instance_revision_id
              AND admission.consumed_at IS NOT NULL AND admission.vm_id=NEW.vm_id
              AND admission.vm_provider_namespace=NEW.vm_provider_namespace AND admission.vm_host_id=NEW.vm_host_id) THEN
            RAISE EXCEPTION 'historical NULL scope is not VM planning admission' USING ERRCODE='23000';
        END IF;
    ELSIF NEW.vm_id IS NULL AND NEW.vm_binding_kind IS NULL THEN
        RETURN NEW;
    END IF;
    IF NEW.vm_id IS DISTINCT FROM NEW.id::text THEN
        RAISE EXCEPTION 'fresh planned VM identity must equal exact run UUID' USING ERRCODE='23000';
    END IF;
    PERFORM require_run_cleanup_worker();
    IF NEW.vm_binding_kind IS DISTINCT FROM 'planned' OR NEW.vm_id IS NULL
       OR NEW.vm_provider_namespace IS NULL OR NEW.vm_host_id IS NULL
       OR ((NEW.volume_acquisition_closed_at IS NOT NULL OR NEW.state NOT IN ('queued','leasing_volume','provisioning'))
           AND NOT EXISTS(SELECT 1 FROM run_vm_plan_admissions WHERE run_id=NEW.id AND consumed_at IS NOT NULL AND consumption_kind='cleanup'
               AND vm_id=NEW.vm_id AND vm_provider_namespace=NEW.vm_provider_namespace AND vm_host_id=NEW.vm_host_id)) THEN
        RAISE EXCEPTION 'VM scope must be planned before provisioning IO' USING ERRCODE='23000';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_run_vm_owner_scope() FROM PUBLIC;
CREATE TRIGGER run_vm_owner_scope BEFORE INSERT OR UPDATE ON runs
    FOR EACH ROW EXECUTE FUNCTION enforce_run_vm_owner_scope();

CREATE FUNCTION validate_run_vm_plan_admission_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE admission run_vm_plan_admissions%ROWTYPE;
BEGIN
    SELECT * INTO admission FROM run_vm_plan_admissions WHERE run_id=NEW.run_id;
    IF NOT EXISTS(SELECT 1 FROM runs execution
        JOIN agent_updates update ON update.id=admission.update_id
        JOIN agent_instances consumer ON consumer.id=execution.instance_id
        JOIN agent_instance_revisions revision ON revision.id=execution.instance_revision_id AND revision.instance_id=consumer.id
        JOIN release_agents agent ON agent.id=revision.release_agent_id AND agent.id=execution.release_agent_id
        JOIN releases release ON release.id=agent.release_id AND release.id=execution.release_id
        JOIN release_command_inbox command ON command.command_key=admission.producer_command_key
          AND command.operation='begin_update_hook' AND command.aggregate_id=admission.update_id
          AND command.secondary_id=admission.run_id AND command.actor_id=admission.actor_id
          AND command.request_id=admission.request_id
        WHERE execution.id=admission.run_id AND execution.command_id=admission.command_id
          AND execution.instance_id=admission.instance_id AND execution.instance_revision_id=admission.instance_revision_id
          AND execution.run_kind='update' AND execution.vm_plan_admission_required AND execution.requires_state=agent.requires_state
          AND update.hook_run_id=execution.id
          AND update.instance_id=consumer.id AND update.candidate_revision_id=revision.id
          AND ((admission.consumed_at IS NULL AND execution.state='queued' AND execution.vm_id IS NULL
                AND execution.vm_binding_kind IS NULL AND execution.volume_acquisition_closed_at IS NULL
                AND release.state='published' AND update.state='hook_running' AND consumer.state='updating' AND NOT consumer.run_gate_open
                AND NOT EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=consumer.id))
            OR (admission.consumed_at IS NOT NULL
                AND ROW(execution.vm_id,execution.vm_provider_namespace,execution.vm_host_id,execution.vm_binding_kind)
                  IS NOT DISTINCT FROM ROW(admission.vm_id,admission.vm_provider_namespace,admission.vm_host_id,'planned'::text)
                AND ((admission.consumption_kind='provision' AND execution.state='queued' AND execution.volume_acquisition_closed_at IS NULL
                      AND release.state='published' AND update.state='hook_running' AND consumer.state='updating' AND NOT consumer.run_gate_open
                      AND NOT EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=consumer.id))
                  OR (admission.consumption_kind='cleanup' AND execution.volume_acquisition_closed_at IS NOT NULL
                      AND admission.vm_id=execution.id::text
                      AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id AND released_at IS NULL)))))) THEN
        RAISE EXCEPTION 'VM admission must commit its exact live hook and complete plan' USING ERRCODE='23000';
    END IF;
    RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_run_vm_plan_admission_commit() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER run_vm_plan_admission_commit AFTER INSERT OR UPDATE ON run_vm_plan_admissions
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_run_vm_plan_admission_commit();

CREATE FUNCTION validate_new_run_vm_admission_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
    IF NEW.vm_plan_admission_required AND NOT EXISTS(SELECT 1 FROM run_vm_plan_admissions WHERE run_id=NEW.id) THEN
        RAISE EXCEPTION 'fresh update prohibition requires committed producer admission' USING ERRCODE='23000';
    END IF;
    RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_new_run_vm_admission_commit() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER new_run_vm_admission_commit AFTER INSERT ON runs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_new_run_vm_admission_commit();

CREATE FUNCTION protect_planned_start_inbox() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
    IF EXISTS(SELECT 1 FROM runs WHERE command_id=OLD.command_id AND vm_binding_kind IS NOT NULL)
       OR EXISTS(SELECT 1 FROM run_vm_plan_admissions WHERE command_id=OLD.command_id) THEN
        IF TG_OP='DELETE' OR ROW(NEW.command_id,NEW.command_type,NEW.payload)
            IS DISTINCT FROM ROW(OLD.command_id,OLD.command_type,OLD.payload) THEN
            RAISE EXCEPTION 'planned start inbox inputs are immutable' USING ERRCODE='23000';
        END IF;
    END IF;
    RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END $$;
REVOKE ALL ON FUNCTION protect_planned_start_inbox() FROM PUBLIC;
CREATE TRIGGER planned_start_inbox_immutable BEFORE UPDATE OR DELETE ON command_inbox
    FOR EACH ROW EXECUTE FUNCTION protect_planned_start_inbox();

CREATE TABLE run_cleanup_targets (
    run_id uuid NOT NULL REFERENCES runs(id),
    instance_id uuid NOT NULL REFERENCES agent_instances(id),
    instance_revision_id uuid NOT NULL REFERENCES agent_instance_revisions(id),
    generation bigint NOT NULL CHECK(generation>0),
    vm_target_kind text NOT NULL CHECK(vm_target_kind IN ('planned','persisted','historical_unresolved')),
    vm_provider_namespace text,
    vm_host_id text,
    vm_id text,
    target_digest bytea NOT NULL CHECK(octet_length(target_digest)=32),
    lease_count integer NOT NULL CHECK(lease_count BETWEEN 0 AND 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(run_id,generation),
    UNIQUE(run_id),
    UNIQUE(run_id,generation,target_digest),
    CHECK((vm_target_kind='historical_unresolved' AND num_nonnulls(vm_provider_namespace,vm_host_id,vm_id)=0)
      OR (vm_target_kind<>'historical_unresolved' AND num_nonnulls(vm_provider_namespace,vm_host_id,vm_id)=3
        AND run_cleanup_identifier_is_valid(vm_provider_namespace)
        AND run_cleanup_identifier_is_valid(vm_host_id)
        AND run_cleanup_identifier_is_valid(vm_id)))
);
CREATE TABLE run_cleanup_target_leases (
    run_id uuid NOT NULL,
    generation bigint NOT NULL,
    lease_id uuid NOT NULL REFERENCES agent_instance_volume_leases(id),
    volume_id uuid NOT NULL REFERENCES agent_instance_state_volumes(id),
    host_id text NOT NULL CHECK(run_cleanup_identifier_is_valid(host_id)),
    fencing_token bigint NOT NULL CHECK(fencing_token>0),
    PRIMARY KEY(run_id,generation,volume_id),
    UNIQUE(run_id,generation,lease_id),
    FOREIGN KEY(run_id,generation) REFERENCES run_cleanup_targets(run_id,generation)
);
CREATE TABLE run_cleanup_receipts (
    run_id uuid NOT NULL,
    generation bigint NOT NULL,
    target_digest bytea NOT NULL,
    observation_kind text NOT NULL CHECK(observation_kind IN ('destroyed','authoritatively_absent')),
    vm_provider_namespace text NOT NULL CHECK(run_cleanup_identifier_is_valid(vm_provider_namespace)),
    vm_host_id text NOT NULL CHECK(run_cleanup_identifier_is_valid(vm_host_id)),
    vm_id text NOT NULL CHECK(run_cleanup_identifier_is_valid(vm_id)),
    observed_at timestamptz NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(run_id,generation),
    UNIQUE(run_id,generation,target_digest),
    FOREIGN KEY(run_id,generation,target_digest) REFERENCES run_cleanup_targets(run_id,generation,target_digest)
);
CREATE TABLE run_cleanup_completions (
    run_id uuid NOT NULL,
    generation bigint NOT NULL,
    target_digest bytea NOT NULL,
    completed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(run_id,generation),
    FOREIGN KEY(run_id,generation,target_digest) REFERENCES run_cleanup_receipts(run_id,generation,target_digest)
);
ALTER TABLE run_cleanup_targets ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_cleanup_targets FORCE ROW LEVEL SECURITY;
ALTER TABLE run_cleanup_target_leases ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_cleanup_target_leases FORCE ROW LEVEL SECURITY;
ALTER TABLE run_cleanup_receipts ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_cleanup_receipts FORCE ROW LEVEL SECURITY;
ALTER TABLE run_cleanup_completions ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_cleanup_completions FORCE ROW LEVEL SECURITY;
GRANT SELECT,INSERT ON run_cleanup_targets,run_cleanup_target_leases,run_cleanup_receipts,run_cleanup_completions TO hephaestus_worker;
CREATE POLICY run_cleanup_targets_worker ON run_cleanup_targets TO hephaestus_worker USING(true) WITH CHECK(true);
CREATE POLICY run_cleanup_target_leases_worker ON run_cleanup_target_leases TO hephaestus_worker USING(true) WITH CHECK(true);
CREATE POLICY run_cleanup_receipts_worker ON run_cleanup_receipts TO hephaestus_worker USING(true) WITH CHECK(true);
CREATE POLICY run_cleanup_completions_worker ON run_cleanup_completions TO hephaestus_worker USING(true) WITH CHECK(true);
CREATE TRIGGER run_cleanup_target_immutable BEFORE UPDATE OR DELETE ON run_cleanup_targets
    FOR EACH ROW EXECUTE FUNCTION reject_capability_record_mutation();
CREATE TRIGGER run_cleanup_target_lease_immutable BEFORE UPDATE OR DELETE ON run_cleanup_target_leases
    FOR EACH ROW EXECUTE FUNCTION reject_capability_record_mutation();
CREATE TRIGGER run_cleanup_receipt_immutable BEFORE UPDATE OR DELETE ON run_cleanup_receipts
    FOR EACH ROW EXECUTE FUNCTION reject_capability_record_mutation();
CREATE TRIGGER run_cleanup_completion_immutable BEFORE UPDATE OR DELETE ON run_cleanup_completions
    FOR EACH ROW EXECUTE FUNCTION reject_capability_record_mutation();

-- Fixed encoding must match run-domain's independent golden vector: UUIDs,
-- generation/fences big-endian, kind byte, u16 string lengths, u8 set count,
-- followed by every lease sorted by VolumeId. Empty scope/set is explicit.
CREATE FUNCTION run_cleanup_target_digest(p_run uuid,p_generation bigint) RETURNS bytea
LANGUAGE sql STABLE AS $$
    SELECT sha256(convert_to('heph.run.cleanup-target.v1','UTF8')||decode('00','hex')
      ||uuid_send(target.run_id)||uuid_send(target.instance_id)||uuid_send(target.instance_revision_id)
      ||int8send(target.generation)
      ||decode(CASE target.vm_target_kind WHEN 'persisted' THEN '01' WHEN 'planned' THEN '02' ELSE '03' END,'hex')
      ||int2send(octet_length(COALESCE(target.vm_provider_namespace,''))::smallint)||convert_to(COALESCE(target.vm_provider_namespace,''),'UTF8')
      ||int2send(octet_length(COALESCE(target.vm_host_id,''))::smallint)||convert_to(COALESCE(target.vm_host_id,''),'UTF8')
      ||int2send(octet_length(COALESCE(target.vm_id,''))::smallint)||convert_to(COALESCE(target.vm_id,''),'UTF8')
      ||decode(lpad(to_hex(target.lease_count),2,'0'),'hex')
      ||COALESCE((SELECT string_agg(uuid_send(lease.lease_id)||uuid_send(lease.volume_id)
          ||int2send(octet_length(lease.host_id)::smallint)||convert_to(lease.host_id,'UTF8')
          ||int8send(lease.fencing_token),''::bytea ORDER BY lease.volume_id)
        FROM run_cleanup_target_leases lease WHERE lease.run_id=target.run_id AND lease.generation=target.generation),''::bytea))
    FROM run_cleanup_targets target WHERE target.run_id=p_run AND target.generation=p_generation
$$;
REVOKE ALL ON FUNCTION run_cleanup_target_digest(uuid,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION run_cleanup_target_digest(uuid,bigint) TO hephaestus_worker;

CREATE FUNCTION run_cleanup_set_matches(p_run uuid,p_generation bigint,p_released boolean) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT EXISTS(SELECT 1 FROM run_cleanup_targets target
      WHERE target.run_id=p_run AND target.generation=p_generation
        AND target.lease_count=(SELECT count(*) FROM run_cleanup_target_leases item
          WHERE item.run_id=p_run AND item.generation=p_generation)
        AND target.target_digest=run_cleanup_target_digest(p_run,p_generation)
        AND NOT EXISTS(SELECT 1 FROM run_cleanup_target_leases item
          LEFT JOIN agent_instance_volume_leases lease ON lease.id=item.lease_id
          WHERE item.run_id=p_run AND item.generation=p_generation
            AND (lease.id IS NULL OR ROW(lease.run_id,lease.volume_id,lease.host_id,lease.fencing_token)
              IS DISTINCT FROM ROW(item.run_id,item.volume_id,item.host_id,item.fencing_token)
              OR (lease.released_at IS NOT NULL) IS DISTINCT FROM p_released
              OR (p_released AND lease.state<>'released')))
        AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease
          WHERE lease.run_id=p_run AND lease.released_at IS NULL
            AND (p_released OR NOT EXISTS(SELECT 1 FROM run_cleanup_target_leases item
              WHERE item.run_id=p_run AND item.generation=p_generation AND item.lease_id=lease.id))))
$$;
REVOKE ALL ON FUNCTION run_cleanup_set_matches(uuid,bigint,boolean) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION run_cleanup_set_matches(uuid,bigint,boolean) TO hephaestus_worker;

CREATE FUNCTION enforce_run_cleanup_target() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
    PERFORM require_run_cleanup_worker();
    -- All callers hold consumer -> run before globally sorted resource/lease
    -- locks. This lock serializes direct target insertion against acquisition.
    SELECT * INTO execution FROM runs WHERE id=NEW.run_id FOR UPDATE;
    IF execution.id IS NULL OR execution.volume_acquisition_closed_at IS NULL
       OR execution.state='cleaned_up'
       OR NEW.instance_id IS DISTINCT FROM execution.instance_id
       OR NEW.instance_revision_id IS DISTINCT FROM execution.instance_revision_id
       OR NEW.generation<>1
       OR (NEW.vm_target_kind='historical_unresolved' AND execution.vm_binding_kind IS NOT NULL)
       OR (NEW.vm_target_kind<>'historical_unresolved' AND
         ROW(NEW.vm_target_kind,NEW.vm_provider_namespace,NEW.vm_host_id,NEW.vm_id)
         IS DISTINCT FROM ROW(execution.vm_binding_kind,execution.vm_provider_namespace,execution.vm_host_id,execution.vm_id)) THEN
        RAISE EXCEPTION 'cleanup target differs from exact closed execution' USING ERRCODE='23000';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_run_cleanup_target() FROM PUBLIC;
CREATE TRIGGER run_cleanup_target_context BEFORE INSERT ON run_cleanup_targets
    FOR EACH ROW EXECUTE FUNCTION enforce_run_cleanup_target();
CREATE FUNCTION enforce_run_cleanup_target_lease() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE target run_cleanup_targets%ROWTYPE; held agent_instance_volume_leases%ROWTYPE;
BEGIN
    PERFORM require_run_cleanup_worker();
    SELECT * INTO target FROM run_cleanup_targets WHERE run_id=NEW.run_id AND generation=NEW.generation;
    SELECT * INTO held FROM agent_instance_volume_leases WHERE id=NEW.lease_id;
    IF target.run_id IS NULL OR held.id IS NULL OR held.released_at IS NOT NULL
       OR ROW(held.run_id,held.volume_id,held.host_id,held.fencing_token)
         IS DISTINCT FROM ROW(NEW.run_id,NEW.volume_id,NEW.host_id,NEW.fencing_token)
       OR (target.vm_target_kind<>'historical_unresolved' AND NEW.host_id IS DISTINCT FROM target.vm_host_id) THEN
        RAISE EXCEPTION 'cleanup lease differs from canonical held fence' USING ERRCODE='23000';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_run_cleanup_target_lease() FROM PUBLIC;
CREATE TRIGGER run_cleanup_target_lease_context BEFORE INSERT ON run_cleanup_target_leases
    FOR EACH ROW EXECUTE FUNCTION enforce_run_cleanup_target_lease();
CREATE FUNCTION validate_run_cleanup_snapshot() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE completed boolean;
BEGIN
    completed:=EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=NEW.run_id AND generation=NEW.generation);
    IF NOT run_cleanup_set_matches(NEW.run_id,NEW.generation,completed) THEN
        RAISE EXCEPTION 'cleanup snapshot must cover the exact complete set and digest' USING ERRCODE='23000';
    END IF;
    RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_run_cleanup_snapshot() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER run_cleanup_snapshot_complete AFTER INSERT ON run_cleanup_targets
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_run_cleanup_snapshot();
CREATE CONSTRAINT TRIGGER run_cleanup_snapshot_entry_complete AFTER INSERT ON run_cleanup_target_leases
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_run_cleanup_snapshot();

CREATE FUNCTION enforce_run_cleanup_receipt() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE target run_cleanup_targets%ROWTYPE;
BEGIN
    PERFORM require_run_cleanup_worker();
    SELECT * INTO target FROM run_cleanup_targets WHERE run_id=NEW.run_id AND generation=NEW.generation;
    IF target.run_id IS NULL OR target.vm_target_kind='historical_unresolved'
       OR ROW(NEW.target_digest,NEW.vm_provider_namespace,NEW.vm_host_id,NEW.vm_id)
         IS DISTINCT FROM ROW(target.target_digest,target.vm_provider_namespace,target.vm_host_id,target.vm_id)
       OR NOT EXISTS(SELECT 1 FROM runs WHERE id=NEW.run_id AND volume_acquisition_closed_at IS NOT NULL) THEN
        RAISE EXCEPTION 'worker observation must match exact known cleanup target' USING ERRCODE='23000';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_run_cleanup_receipt() FROM PUBLIC;
CREATE TRIGGER run_cleanup_receipt_context BEFORE INSERT ON run_cleanup_receipts
    FOR EACH ROW EXECUTE FUNCTION enforce_run_cleanup_receipt();

CREATE FUNCTION enforce_run_cleanup_completion() RETURNS trigger
LANGUAGE plpgsql AS $$ BEGIN
    PERFORM require_run_cleanup_worker();
    IF NOT EXISTS(SELECT 1 FROM run_cleanup_receipts receipt
        WHERE receipt.run_id=NEW.run_id AND receipt.generation=NEW.generation AND receipt.target_digest=NEW.target_digest)
       OR NOT EXISTS(SELECT 1 FROM runs WHERE id=NEW.run_id AND state='cleaning_up' AND volume_acquisition_closed_at IS NOT NULL)
       OR NOT run_cleanup_set_matches(NEW.run_id,NEW.generation,true)
       OR EXISTS(SELECT 1 FROM run_cleanup_target_leases item
          JOIN agent_instance_state_volumes volume ON volume.id=item.volume_id
          WHERE item.run_id=NEW.run_id AND item.generation=NEW.generation
            AND (volume.lease_generation<>item.fencing_token OR volume.state<>'ready'
              OR volume.host_id IS DISTINCT FROM item.host_id
              OR volume.provisioning_state<>'ready'
              OR EXISTS(SELECT 1 FROM agent_instance_volume_leases held WHERE held.volume_id=volume.id AND held.released_at IS NULL))) THEN
        RAISE EXCEPTION 'completion needs exact whole-set release and current resource fences' USING ERRCODE='23000';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_run_cleanup_completion() FROM PUBLIC;
CREATE TRIGGER run_cleanup_completion_context BEFORE INSERT ON run_cleanup_completions
    FOR EACH ROW EXECUTE FUNCTION enforce_run_cleanup_completion();

CREATE FUNCTION validate_run_cleanup_completion_commit() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS(SELECT 1 FROM runs WHERE id=NEW.run_id AND state='cleaned_up')
       OR NOT run_cleanup_set_matches(NEW.run_id,NEW.generation,true) THEN
        RAISE EXCEPTION 'completion must commit with CleanedUp and the exact released set' USING ERRCODE='23000';
    END IF;
    RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_run_cleanup_completion_commit() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER run_cleanup_completion_commit AFTER INSERT ON run_cleanup_completions
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_run_cleanup_completion_commit();

CREATE FUNCTION enforce_canonical_run_cleanup() RETURNS trigger
LANGUAGE plpgsql AS $$ BEGIN
    IF NEW.state='cleaned_up' AND OLD.state IS DISTINCT FROM 'cleaned_up' THEN
        PERFORM require_run_cleanup_worker();
        IF OLD.state<>'cleaning_up' OR NEW.volume_acquisition_closed_at IS NULL
           OR NOT EXISTS(SELECT 1 FROM run_cleanup_completions completion
              WHERE completion.run_id=NEW.id AND run_cleanup_set_matches(NEW.id,completion.generation,true)) THEN
            RAISE EXCEPTION 'CleanedUp needs persisted canonical whole-set completion' USING ERRCODE='23000';
        END IF;
    ELSIF OLD.state='cleaned_up' AND NEW.state<>'cleaned_up' THEN
        RAISE EXCEPTION 'cleaned history cannot reopen' USING ERRCODE='23000';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_canonical_run_cleanup() FROM PUBLIC;
CREATE TRIGGER run_canonical_cleanup BEFORE UPDATE ON runs
    FOR EACH ROW EXECUTE FUNCTION enforce_canonical_run_cleanup();

CREATE FUNCTION enforce_canonical_lease_release() RETURNS trigger
LANGUAGE plpgsql AS $$ BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'lease history cannot be deleted' USING ERRCODE='23000';
    END IF;
    IF OLD.released_at IS NOT NULL AND
       ROW(NEW.released_at,NEW.state) IS DISTINCT FROM ROW(OLD.released_at,OLD.state) THEN
        RAISE EXCEPTION 'released fence evidence is immutable' USING ERRCODE='23000';
    END IF;
    IF OLD.released_at IS NULL AND (NEW.released_at IS NOT NULL OR NEW.state='released') THEN
        PERFORM require_run_cleanup_worker();
        IF NEW.run_id IS NULL OR NEW.released_at IS NULL OR NEW.state<>'released'
           OR NOT EXISTS(SELECT 1 FROM run_cleanup_target_leases item
              JOIN run_cleanup_receipts receipt ON receipt.run_id=item.run_id AND receipt.generation=item.generation
              WHERE item.run_id=NEW.run_id AND item.lease_id=NEW.id AND item.volume_id=NEW.volume_id
                AND item.host_id=NEW.host_id AND item.fencing_token=NEW.fencing_token) THEN
            RAISE EXCEPTION 'lease release needs canonical trusted receipt; run-null history remains held' USING ERRCODE='23000';
        END IF;
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_canonical_lease_release() FROM PUBLIC;
CREATE TRIGGER canonical_lease_release BEFORE UPDATE OR DELETE ON agent_instance_volume_leases
    FOR EACH ROW EXECUTE FUNCTION enforce_canonical_lease_release();
CREATE FUNCTION validate_canonical_lease_release() RETURNS trigger
LANGUAGE plpgsql AS $$ BEGIN
    IF OLD.released_at IS NULL AND NEW.released_at IS NOT NULL AND
       NOT EXISTS(SELECT 1 FROM runs execution JOIN run_cleanup_completions completion ON completion.run_id=execution.id
          WHERE execution.id=NEW.run_id AND execution.state='cleaned_up'
            AND run_cleanup_set_matches(execution.id,completion.generation,true)) THEN
        RAISE EXCEPTION 'released lease must commit with whole-set run completion' USING ERRCODE='23000';
    END IF;
    RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_canonical_lease_release() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER canonical_lease_release_complete AFTER UPDATE ON agent_instance_volume_leases
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_canonical_lease_release();

CREATE TRIGGER run_cleanup_target_application_event AFTER INSERT ON run_cleanup_targets
    FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event('run','run_id','run','run_id','run.changed','');
CREATE TRIGGER run_cleanup_receipt_application_event AFTER INSERT ON run_cleanup_receipts
    FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event('run','run_id','run','run_id','run.changed','');
CREATE TRIGGER run_cleanup_completion_application_event AFTER INSERT ON run_cleanup_completions
    FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event('run','run_id','run','run_id','run.changed','');

-- No historical CleanedUp/released row is changed or given invented proof.
-- Unresolved VM scope and run-null leases remain held. Future proven resolution
-- requires a new append-only generation, not edits to this immutable snapshot.
-- Named dispatch remains false. Transient cleanup must succeed before finish.
