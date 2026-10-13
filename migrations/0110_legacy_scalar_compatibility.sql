-- PRIVATE qualification fragment, applied AFTER both profile108 fragments.
-- Positive legacy_scalar is old scalar execution semantics, NOT a physical
-- absence receipt, live Source grant, or owned-recipe Retain/Delete evidence.
-- No schema discovery, history backfill, selection snapshot, or new table.

CREATE FUNCTION legacy_scalar_run_matches(execution runs) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT execution.execution_profile='legacy_scalar'
      AND NOT execution.vm_plan_admission_required
      AND num_nonnulls(execution.vm_provider_namespace,execution.vm_host_id,execution.vm_binding_kind)=0
      AND execution.volume_selection_snapshot IS NULL
      AND NOT EXISTS(SELECT 1 FROM run_cleanup_targets WHERE run_id=execution.id)
      AND NOT EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=execution.id)
      AND EXISTS(SELECT 1 FROM agent_instances consumer
        JOIN agent_instance_revisions revision ON revision.id=execution.instance_revision_id
            AND revision.instance_id=consumer.id AND revision.release_agent_id=execution.release_agent_id
        JOIN release_agents agent ON agent.id=revision.release_agent_id AND agent.release_id=execution.release_id
        WHERE consumer.id=execution.instance_id AND consumer.volume_mode='legacy'
          AND agent.requires_state=execution.requires_state AND octet_length(agent.runtime_contract_hash)=32
          AND jsonb_typeof(COALESCE(agent.runtime_contract->'volume_slots','[]'::jsonb))='array'
          AND jsonb_array_length(COALESCE(agent.runtime_contract->'volume_slots','[]'::jsonb))=0)
$$;

CREATE FUNCTION legacy_scalar_lease_matches(lease agent_instance_volume_leases) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT lease.selection_origin='legacy_origin' AND lease.update_id IS NULL
      AND EXISTS(SELECT 1 FROM runs execution
        JOIN agent_instances consumer ON consumer.id=execution.instance_id
        JOIN release_agents agent ON agent.id=execution.release_agent_id
        JOIN agent_instance_state_volumes volume ON volume.id=lease.volume_id
        WHERE execution.id=lease.run_id AND execution.requires_state AND legacy_scalar_run_matches(execution)
          AND ROW(lease.instance_id,volume.instance_id,volume.project_id,consumer.state_volume_id)
            IS NOT DISTINCT FROM ROW(consumer.id,consumer.id,consumer.project_id,volume.id)
          AND ROW(lease.selected_revision_id,lease.selected_release_id,lease.selected_release_agent_id,
                lease.selected_project_id,lease.slot_key,lease.access_mode,lease.guest_path,
                lease.slot_required,lease.minimum_capacity_bytes,lease.release_contract_hash)
            IS NOT DISTINCT FROM ROW(execution.instance_revision_id,execution.release_id,agent.id,
                consumer.project_id,'state'::text,'read_write'::text,'/var/lib/hephaestus'::text,
                true,1::bigint,agent.runtime_contract_hash))
$$;

-- Helper operations block in consumer -> Run -> volume -> lease order. Raw
-- row-trigger entry already owns the child row; its parent locks use NOWAIT so
-- it fails instead of introducing a child -> parent deadlock cycle.
CREATE FUNCTION lock_legacy_scalar_context(p_run uuid,p_volume uuid,p_host text,p_nowait boolean DEFAULT false)
RETURNS runs LANGUAGE plpgsql AS $$
DECLARE consumer_id uuid; consumer agent_instances%ROWTYPE; execution runs%ROWTYPE;
        volume agent_instance_state_volumes%ROWTYPE;
BEGIN
    PERFORM require_run_cleanup_worker();
    SELECT instance_id INTO consumer_id FROM runs WHERE id=p_run;
    IF p_nowait THEN
        SELECT * INTO consumer FROM agent_instances WHERE id=consumer_id FOR UPDATE NOWAIT;
        SELECT * INTO execution FROM runs WHERE id=p_run FOR UPDATE NOWAIT;
        SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=p_volume FOR UPDATE NOWAIT;
    ELSE
        SELECT * INTO consumer FROM agent_instances WHERE id=consumer_id FOR UPDATE;
        SELECT * INTO execution FROM runs WHERE id=p_run FOR UPDATE;
        SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=p_volume FOR UPDATE;
    END IF;
    IF execution.id IS NULL OR consumer.id IS NULL OR volume.id IS NULL
       OR legacy_scalar_run_matches(execution) IS NOT TRUE OR NOT execution.requires_state
       OR ROW(execution.instance_id,volume.instance_id,volume.project_id,consumer.state_volume_id)
           IS DISTINCT FROM ROW(consumer.id,consumer.id,consumer.project_id,volume.id)
       OR p_host IS NULL OR p_host='' THEN
        RAISE EXCEPTION 'scalar context is not an exact positively admitted legacy origin' USING ERRCODE='23000';
    END IF;
    RETURN execution;
END $$;

CREATE FUNCTION legacy_scalar_acquire(p_volume uuid,p_run uuid,p_host text,p_now timestamptz,p_expires timestamptz)
RETURNS SETOF agent_instance_volume_leases LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE; volume agent_instance_state_volumes%ROWTYPE;
        held agent_instance_volume_leases%ROWTYPE; inserted agent_instance_volume_leases%ROWTYPE;
BEGIN
    execution:=lock_legacy_scalar_context(p_run,p_volume,p_host);
    SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=p_volume;
    IF execution.volume_acquisition_closed_at IS NOT NULL OR execution.cancel_requested_at IS NOT NULL
       OR execution.state NOT IN ('leasing_volume','provisioning')
       OR volume.host_id IS DISTINCT FROM p_host OR volume.provisioning_state<>'ready' THEN
        RAISE EXCEPTION 'scalar acquisition is closed or on another provider' USING ERRCODE='23000';
    END IF;
    SELECT * INTO held FROM agent_instance_volume_leases
        WHERE volume_id=p_volume AND released_at IS NULL FOR UPDATE;
    IF held.id IS NOT NULL THEN
        IF held.run_id IS DISTINCT FROM p_run OR legacy_scalar_lease_matches(held) IS NOT TRUE
           OR held.fencing_token<>volume.lease_generation OR held.host_id IS DISTINCT FROM p_host
           OR held.recovering_at IS NOT NULL OR held.state<>'active'
           OR held.expires_at<=clock_timestamp() OR volume.state NOT IN ('ready','attached')
           OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=p_run AND id<>held.id AND released_at IS NULL)
           OR ROW(execution.volume_id,execution.lease_id,execution.lease_fencing_token)
                IS DISTINCT FROM ROW(held.volume_id,held.id,held.fencing_token) THEN
            RAISE EXCEPTION 'scalar replay is not the exact live singleton fence' USING ERRCODE='23000';
        END IF;
        RETURN NEXT held; RETURN;
    END IF;
    IF EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=p_run)
       OR num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)>0
       OR volume.state<>'ready' OR p_now IS NULL OR p_expires IS NULL
       OR p_expires<=p_now OR p_expires<=clock_timestamp() THEN
        RAISE EXCEPTION 'scalar acquisition cannot adopt a prior lease or non-ready resource' USING ERRCODE='23000';
    END IF;
    -- The existing exact-run INSERT guard rechecks current admission/catalog
    -- and advances lease_generation exactly once. Never freeze a canonical set.
    INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,
        state,acquired_at,heartbeat_at,expires_at,selection_origin)
    VALUES(gen_random_uuid(),p_volume,execution.instance_id,p_run,p_host,volume.lease_generation+1,
        'active',p_now,p_now,p_expires,'legacy_origin') RETURNING * INTO inserted;
    -- Persist the exact scalar binding before the lease can become externally
    -- visible. vm_id remains NULL until later actual provider binding.
    UPDATE runs SET volume_id=inserted.volume_id,lease_id=inserted.id,
        lease_fencing_token=inserted.fencing_token,updated_at=now() WHERE id=p_run;
    RETURN NEXT inserted;
END $$;

CREATE FUNCTION legacy_scalar_locked_lease(p_lease uuid,p_volume uuid,p_run uuid,p_host text,p_fence bigint)
RETURNS agent_instance_volume_leases LANGUAGE plpgsql AS $$
DECLARE held agent_instance_volume_leases%ROWTYPE;
BEGIN
    PERFORM lock_legacy_scalar_context(p_run,p_volume,p_host);
    SELECT * INTO held FROM agent_instance_volume_leases WHERE id=p_lease FOR UPDATE;
    IF held.id IS NULL OR legacy_scalar_lease_matches(held) IS NOT TRUE
       OR ROW(held.volume_id,held.run_id,held.host_id,held.fencing_token)
          IS DISTINCT FROM ROW(p_volume,p_run,p_host,p_fence) THEN
        RAISE EXCEPTION 'scalar lease identity differs from immutable selected origin' USING ERRCODE='23000';
    END IF;
    IF NOT EXISTS(SELECT 1 FROM runs WHERE id=p_run
        AND ROW(volume_id,lease_id,lease_fencing_token) IS NOT DISTINCT FROM ROW(p_volume,p_lease,p_fence)) THEN
        RAISE EXCEPTION 'scalar lease differs from its acquisition-time Run binding' USING ERRCODE='23000';
    END IF;
    RETURN held;
END $$;

CREATE FUNCTION assert_legacy_scalar_current_fence(held agent_instance_volume_leases) RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
    IF held.released_at IS NOT NULL OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes volume
        WHERE volume.id=held.volume_id AND volume.host_id=held.host_id
          AND volume.lease_generation=held.fencing_token AND volume.provisioning_state='ready')
       OR EXISTS(SELECT 1 FROM agent_instance_volume_leases
          WHERE released_at IS NULL AND id<>held.id AND (run_id=held.run_id OR volume_id=held.volume_id)) THEN
        RAISE EXCEPTION 'scalar lease is not the sole global held current fence' USING ERRCODE='23000';
    END IF;
END $$;

CREATE FUNCTION legacy_scalar_refresh(p_lease uuid,p_volume uuid,p_run uuid,p_host text,p_fence bigint,
    p_now timestamptz,p_expires timestamptz,p_attached boolean)
RETURNS SETOF agent_instance_volume_leases LANGUAGE plpgsql AS $$
DECLARE held agent_instance_volume_leases%ROWTYPE; execution runs%ROWTYPE;
BEGIN
    held:=legacy_scalar_locked_lease(p_lease,p_volume,p_run,p_host,p_fence);
    PERFORM assert_legacy_scalar_current_fence(held);
    SELECT * INTO execution FROM runs WHERE id=p_run;
    IF execution.volume_acquisition_closed_at IS NOT NULL OR execution.cancel_requested_at IS NOT NULL
       OR execution.state IN ('cleaning_up','cleaned_up') OR held.state<>'active'
       OR held.recovering_at IS NOT NULL OR held.expires_at<=clock_timestamp()
       OR p_now IS NULL OR p_now<held.heartbeat_at OR held.expires_at<=p_now
       OR p_expires IS NULL OR p_expires<=p_now OR p_expires<=clock_timestamp()
       OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes WHERE id=p_volume AND state IN ('ready','attached')) THEN
        RAISE EXCEPTION 'closed, expired or recovering scalar leases cannot renew or attach' USING ERRCODE='23000';
    END IF;
    UPDATE agent_instance_volume_leases SET heartbeat_at=p_now,expires_at=p_expires,
        attached_at=CASE WHEN p_attached THEN COALESCE(attached_at,p_now) ELSE attached_at END
        WHERE id=p_lease RETURNING * INTO held;
    IF p_attached THEN
        UPDATE agent_instance_state_volumes SET state='attached',updated_at=now()
            WHERE id=p_volume AND lease_generation=p_fence AND host_id=p_host AND state IN ('ready','attached');
        IF NOT FOUND THEN RAISE EXCEPTION 'scalar attach resource CAS failed' USING ERRCODE='23000'; END IF;
    END IF;
    RETURN NEXT held;
END $$;

CREATE FUNCTION legacy_scalar_active(p_run uuid,p_host text) RETURNS SETOF agent_instance_volume_leases
LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE; held agent_instance_volume_leases%ROWTYPE; held_count bigint;
BEGIN
    PERFORM require_run_cleanup_worker();
    SELECT * INTO execution FROM runs WHERE id=p_run;
    IF execution.id IS NULL OR legacy_scalar_run_matches(execution) IS NOT TRUE THEN
        RAISE EXCEPTION 'scalar read cannot project canonical or unresolved history' USING ERRCODE='23000';
    END IF;
    SELECT count(*) INTO held_count FROM agent_instance_volume_leases WHERE run_id=p_run AND released_at IS NULL;
    IF held_count=0 THEN RETURN; END IF;
    IF held_count<>1 THEN RAISE EXCEPTION 'scalar read is not a singleton' USING ERRCODE='23000'; END IF;
    SELECT * INTO held FROM agent_instance_volume_leases WHERE run_id=p_run AND released_at IS NULL;
    IF legacy_scalar_lease_matches(held) IS NOT TRUE OR held.host_id IS DISTINCT FROM p_host
       OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes WHERE id=held.volume_id
          AND lease_generation=held.fencing_token AND host_id=p_host) THEN
        RAISE EXCEPTION 'scalar read cannot hide another host or selected origin' USING ERRCODE='23000';
    END IF;
    RETURN NEXT held;
END $$;

CREATE FUNCTION legacy_scalar_stale(p_now timestamptz) RETURNS SETOF agent_instance_volume_leases
LANGUAGE plpgsql AS $$ BEGIN
    PERFORM require_run_cleanup_worker();
    -- Actionable candidates only. Every omitted row stays globally held.
    RETURN QUERY SELECT lease.* FROM agent_instance_volume_leases lease
      JOIN agent_instance_state_volumes volume ON volume.id=lease.volume_id
      WHERE lease.released_at IS NULL AND lease.expires_at<=LEAST(p_now,clock_timestamp())
        AND legacy_scalar_lease_matches(lease) AND volume.provisioning_state='ready'
        AND volume.lease_generation=lease.fencing_token AND volume.host_id=lease.host_id
      ORDER BY lease.expires_at,lease.id;
END $$;

CREATE FUNCTION legacy_scalar_begin_recovery(p_lease uuid,p_volume uuid,p_run uuid,p_host text,p_fence bigint)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE held agent_instance_volume_leases%ROWTYPE;
BEGIN
    held:=legacy_scalar_locked_lease(p_lease,p_volume,p_run,p_host,p_fence);
    PERFORM assert_legacy_scalar_current_fence(held);
    IF held.expires_at>clock_timestamp()
       OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes WHERE id=p_volume AND state IN ('ready','attached','recovering')) THEN
        RAISE EXCEPTION 'recovery requires an expired exact fence, never presumed detach' USING ERRCODE='23000';
    END IF;
    UPDATE runs SET volume_acquisition_closed_at=COALESCE(volume_acquisition_closed_at,now()),updated_at=now()
        WHERE id=p_run;
    UPDATE agent_instance_volume_leases SET recovering_at=COALESCE(recovering_at,now()),state='recovery_required'
        WHERE id=p_lease;
    UPDATE agent_instance_state_volumes SET state='recovering',updated_at=now()
        WHERE id=p_volume AND lease_generation=p_fence AND host_id=p_host;
    IF NOT FOUND THEN RAISE EXCEPTION 'scalar recovery resource CAS failed' USING ERRCODE='23000'; END IF;
END $$;

CREATE FUNCTION release_legacy_scalar_lease_after_detach(p_lease uuid,p_volume uuid,p_run uuid,p_host text,
    p_fence bigint,p_recovering boolean) RETURNS void LANGUAGE plpgsql AS $$
DECLARE held agent_instance_volume_leases%ROWTYPE; execution runs%ROWTYPE;
BEGIN
    held:=legacy_scalar_locked_lease(p_lease,p_volume,p_run,p_host,p_fence);
    -- Idempotent lifecycle replay only, even after resource reuse. No UPDATE and
    -- no comparison against the newer resource generation, host, or state.
    IF held.released_at IS NOT NULL AND held.state='released' THEN RETURN; END IF;
    PERFORM assert_legacy_scalar_current_fence(held);
    SELECT * INTO execution FROM runs WHERE id=p_run;
    IF execution.volume_acquisition_closed_at IS NULL OR p_recovering IS NULL
       OR (p_recovering AND (held.recovering_at IS NULL OR held.state<>'recovery_required'
            OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes WHERE id=p_volume AND state='recovering')))
       OR (NOT p_recovering AND (execution.state<>'cleaning_up' OR held.recovering_at IS NOT NULL
            OR held.state<>'active' OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes
               WHERE id=p_volume AND state IN ('ready','attached')))) THEN
        RAISE EXCEPTION 'scalar release requires closed cleanup or exact closed recovery' USING ERRCODE='23000';
    END IF;
    UPDATE agent_instance_volume_leases SET released_at=now(),state='released' WHERE id=p_lease;
    UPDATE agent_instance_state_volumes SET state='ready',updated_at=now()
        WHERE id=p_volume AND lease_generation=p_fence AND host_id=p_host
          AND provisioning_state='ready' AND state IN ('ready','attached','recovering');
    IF NOT FOUND THEN RAISE EXCEPTION 'scalar release resource CAS failed' USING ERRCODE='23000'; END IF;
END $$;

CREATE FUNCTION legacy_scalar_lease_was_released(p_lease uuid,p_run uuid,p_volume uuid,p_host text,p_fence bigint)
RETURNS boolean LANGUAGE plpgsql AS $$ BEGIN
    PERFORM require_run_cleanup_worker();
    RETURN EXISTS(SELECT 1 FROM agent_instance_volume_leases lease JOIN runs execution ON execution.id=lease.run_id
      WHERE lease.id=p_lease AND ROW(lease.run_id,lease.volume_id,lease.host_id,lease.fencing_token)
        IS NOT DISTINCT FROM ROW(p_run,p_volume,p_host,p_fence)
        AND lease.released_at IS NOT NULL AND lease.state='released'
        AND execution.state='cleaned_up' AND execution.volume_acquisition_closed_at IS NOT NULL
        AND legacy_scalar_lease_matches(lease));
END $$;

-- All functions are invoker-rights. Neither app nor actor can gain worker
-- privileges through these helpers; no actor receipt token bypass exists.
REVOKE ALL ON FUNCTION legacy_scalar_run_matches(runs),
 legacy_scalar_lease_matches(agent_instance_volume_leases),
 lock_legacy_scalar_context(uuid,uuid,text,boolean),
 legacy_scalar_acquire(uuid,uuid,text,timestamptz,timestamptz),
 legacy_scalar_locked_lease(uuid,uuid,uuid,text,bigint),
 assert_legacy_scalar_current_fence(agent_instance_volume_leases),
 legacy_scalar_refresh(uuid,uuid,uuid,text,bigint,timestamptz,timestamptz,boolean),
 legacy_scalar_active(uuid,text),legacy_scalar_stale(timestamptz),
 legacy_scalar_begin_recovery(uuid,uuid,uuid,text,bigint),
 release_legacy_scalar_lease_after_detach(uuid,uuid,uuid,text,bigint,boolean),
 legacy_scalar_lease_was_released(uuid,uuid,uuid,text,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION legacy_scalar_run_matches(runs),
 legacy_scalar_lease_matches(agent_instance_volume_leases),
 lock_legacy_scalar_context(uuid,uuid,text,boolean),
 legacy_scalar_acquire(uuid,uuid,text,timestamptz,timestamptz),
 legacy_scalar_locked_lease(uuid,uuid,uuid,text,bigint),
 assert_legacy_scalar_current_fence(agent_instance_volume_leases),
 legacy_scalar_refresh(uuid,uuid,uuid,text,bigint,timestamptz,timestamptz,boolean),
 legacy_scalar_active(uuid,text),legacy_scalar_stale(timestamptz),
 legacy_scalar_begin_recovery(uuid,uuid,uuid,text,bigint),
 release_legacy_scalar_lease_after_detach(uuid,uuid,uuid,text,bigint,boolean),
 legacy_scalar_lease_was_released(uuid,uuid,uuid,text,bigint) TO hephaestus_worker;

-- An invoker worker is the trusted scalar provider boundary. Raw SQL still
-- cannot release just a lease, renew an expired fence, reset a held resource,
-- or use this qualified path for owned/historical execution.
CREATE FUNCTION guard_legacy_scalar_lease_progress() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
    -- Check the real invoker before any Run lookup: RLS-hidden classification
    -- must never turn an actor lifecycle mutation into an unchecked branch.
    IF ROW(NEW.state,NEW.acquired_at,NEW.heartbeat_at,NEW.expires_at,NEW.attached_at,NEW.recovering_at,NEW.released_at)
        IS DISTINCT FROM ROW(OLD.state,OLD.acquired_at,OLD.heartbeat_at,OLD.expires_at,OLD.attached_at,OLD.recovering_at,OLD.released_at) THEN
        PERFORM require_run_cleanup_worker();
    END IF;
    SELECT * INTO execution FROM runs WHERE id=OLD.run_id;
    IF execution.execution_profile IS DISTINCT FROM 'legacy_scalar' THEN RETURN NEW; END IF;
    IF OLD.released_at IS NOT NULL THEN
        IF ROW(NEW.acquired_at,NEW.heartbeat_at,NEW.expires_at,NEW.attached_at,NEW.recovering_at)
            IS DISTINCT FROM ROW(OLD.acquired_at,OLD.heartbeat_at,OLD.expires_at,OLD.attached_at,OLD.recovering_at) THEN
            RAISE EXCEPTION 'released scalar lifecycle evidence is immutable' USING ERRCODE='23000';
        END IF;
        RETURN NEW;
    END IF;
    PERFORM lock_legacy_scalar_context(OLD.run_id,OLD.volume_id,OLD.host_id,true);
    IF legacy_scalar_lease_matches(OLD) IS NOT TRUE THEN
        RAISE EXCEPTION 'scalar progress cannot adopt non-origin history' USING ERRCODE='23000';
    END IF;
    PERFORM assert_legacy_scalar_current_fence(OLD);
    IF NEW.acquired_at IS DISTINCT FROM OLD.acquired_at THEN
        RAISE EXCEPTION 'scalar birth time is immutable' USING ERRCODE='23000';
    END IF;
    IF NEW.state IS DISTINCT FROM OLD.state AND NEW.state NOT IN ('released','recovery_required') THEN
        RAISE EXCEPTION 'scalar recovery cannot reopen an active lease' USING ERRCODE='23000';
    END IF;
    IF NEW.heartbeat_at IS DISTINCT FROM OLD.heartbeat_at
       OR NEW.expires_at IS DISTINCT FROM OLD.expires_at
       OR NEW.attached_at IS DISTINCT FROM OLD.attached_at THEN
        IF execution.volume_acquisition_closed_at IS NOT NULL OR execution.cancel_requested_at IS NOT NULL
           OR execution.state IN ('cleaning_up','cleaned_up') OR OLD.state<>'active'
           OR OLD.recovering_at IS NOT NULL OR OLD.expires_at<=clock_timestamp()
           OR NEW.heartbeat_at<OLD.heartbeat_at OR NEW.heartbeat_at>=OLD.expires_at
           OR NEW.expires_at<=NEW.heartbeat_at OR NEW.expires_at<=clock_timestamp()
           OR (OLD.attached_at IS NOT NULL AND NEW.attached_at IS DISTINCT FROM OLD.attached_at)
           OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes
                WHERE id=OLD.volume_id AND state IN ('ready','attached')) THEN
            RAISE EXCEPTION 'expired or closed scalar progress is held' USING ERRCODE='23000';
        END IF;
    END IF;
    IF NEW.recovering_at IS DISTINCT FROM OLD.recovering_at OR NEW.state='recovery_required' THEN
        IF execution.volume_acquisition_closed_at IS NULL OR OLD.expires_at>clock_timestamp()
           OR NEW.state<>'recovery_required' OR NEW.recovering_at IS NULL
           OR (OLD.recovering_at IS NOT NULL AND NEW.recovering_at IS DISTINCT FROM OLD.recovering_at) THEN
            RAISE EXCEPTION 'scalar recovery needs permanent closure and exact expired fence' USING ERRCODE='23000';
        END IF;
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_legacy_scalar_lease_progress() FROM PUBLIC;
CREATE TRIGGER legacy_scalar_lease_progress BEFORE UPDATE ON agent_instance_volume_leases
    FOR EACH ROW EXECUTE FUNCTION guard_legacy_scalar_lease_progress();

CREATE OR REPLACE FUNCTION enforce_canonical_lease_release() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'lease history cannot be deleted' USING ERRCODE='23000';
    END IF;
    IF OLD.released_at IS NOT NULL AND
       ROW(NEW.released_at,NEW.state) IS DISTINCT FROM ROW(OLD.released_at,OLD.state) THEN
        RAISE EXCEPTION 'released fence evidence is immutable' USING ERRCODE='23000';
    END IF;
    IF OLD.released_at IS NULL AND (NEW.released_at IS NOT NULL OR NEW.state='released') THEN
        PERFORM require_run_cleanup_worker();
        SELECT * INTO execution FROM runs WHERE id=OLD.run_id;
        IF execution.execution_profile='legacy_scalar' THEN
            PERFORM lock_legacy_scalar_context(OLD.run_id,OLD.volume_id,OLD.host_id,true);
            PERFORM assert_legacy_scalar_current_fence(OLD);
            IF NEW.released_at IS NULL OR NEW.state<>'released' OR legacy_scalar_lease_matches(OLD) IS NOT TRUE
               OR execution.volume_acquisition_closed_at IS NULL
               OR NOT ((execution.state='cleaning_up' AND OLD.state='active' AND OLD.recovering_at IS NULL
                          AND EXISTS(SELECT 1 FROM agent_instance_state_volumes WHERE id=OLD.volume_id AND state IN ('ready','attached')))
                    OR (OLD.state='recovery_required' AND OLD.recovering_at IS NOT NULL
                          AND EXISTS(SELECT 1 FROM agent_instance_state_volumes WHERE id=OLD.volume_id AND state='recovering')))
               OR ROW(NEW.acquired_at,NEW.heartbeat_at,NEW.expires_at,NEW.attached_at,NEW.recovering_at)
                    IS DISTINCT FROM ROW(OLD.acquired_at,OLD.heartbeat_at,OLD.expires_at,OLD.attached_at,OLD.recovering_at) THEN
                RAISE EXCEPTION 'legacy release requires exact closed trusted scalar cleanup' USING ERRCODE='23000';
            END IF;
            RETURN NEW;
        END IF;
        -- Original canonical branch is intentionally unchanged: complete set
        -- and committed trusted receipt remain mandatory, including zero slots.
        IF NEW.run_id IS NULL OR NEW.released_at IS NULL OR NEW.state<>'released'
           OR NOT EXISTS(SELECT 1 FROM runs WHERE id=NEW.run_id AND execution_profile='owned_canonical')
           OR NOT EXISTS(SELECT 1 FROM run_cleanup_target_leases item
              JOIN run_cleanup_receipts receipt ON receipt.run_id=item.run_id AND receipt.generation=item.generation
              WHERE item.run_id=NEW.run_id AND item.lease_id=NEW.id AND item.volume_id=NEW.volume_id
                AND item.host_id=NEW.host_id AND item.fencing_token=NEW.fencing_token) THEN
            RAISE EXCEPTION 'lease release needs canonical trusted receipt; run-null history remains held' USING ERRCODE='23000';
        END IF;
    END IF;
    RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION validate_canonical_lease_release() RETURNS trigger
LANGUAGE plpgsql AS $$ BEGIN
    IF OLD.released_at IS NULL AND NEW.released_at IS NOT NULL THEN
        IF EXISTS(SELECT 1 FROM runs WHERE id=NEW.run_id AND execution_profile='legacy_scalar') THEN
            IF legacy_scalar_lease_matches(NEW) IS NOT TRUE
               OR NOT EXISTS(SELECT 1 FROM runs WHERE id=NEW.run_id AND volume_acquisition_closed_at IS NOT NULL)
               OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes volume
                   WHERE volume.id=NEW.volume_id AND volume.lease_generation=NEW.fencing_token
                     AND volume.host_id=NEW.host_id AND volume.state='ready' AND volume.provisioning_state='ready')
               OR EXISTS(SELECT 1 FROM agent_instance_volume_leases
                   WHERE released_at IS NULL AND (run_id=NEW.run_id OR volume_id=NEW.volume_id)) THEN
                RAISE EXCEPTION 'legacy release must atomically commit same-fence Ready with no global holder' USING ERRCODE='23000';
            END IF;
            RETURN NULL;
        END IF;
        IF NOT EXISTS(SELECT 1 FROM runs execution JOIN run_cleanup_completions completion ON completion.run_id=execution.id
          WHERE execution.id=NEW.run_id AND execution.execution_profile='owned_canonical' AND execution.state='cleaned_up'
            AND run_cleanup_set_matches(execution.id,completion.generation,true)) THEN
            RAISE EXCEPTION 'released lease must commit with whole-set run completion' USING ERRCODE='23000';
        END IF;
    END IF;
    RETURN NULL;
END $$;

CREATE FUNCTION guard_scalar_or_canonical_ready_reset() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE released agent_instance_volume_leases%ROWTYPE;
BEGIN
    IF OLD.state NOT IN ('attached','recovering') OR NEW.state<>'ready' THEN RETURN NEW; END IF;
    PERFORM require_run_cleanup_worker();
    IF NEW.lease_generation IS DISTINCT FROM OLD.lease_generation OR NEW.host_id IS DISTINCT FROM OLD.host_id
       OR NEW.provisioning_state<>'ready' OR NEW.provisioning_generation IS DISTINCT FROM OLD.provisioning_generation
       OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE volume_id=OLD.id AND released_at IS NULL) THEN
        RAISE EXCEPTION 'Ready reset cannot clear any global holder or change the fence' USING ERRCODE='23000';
    END IF;
    SELECT * INTO released FROM agent_instance_volume_leases
        WHERE volume_id=OLD.id AND fencing_token=OLD.lease_generation AND host_id=OLD.host_id
          AND state='released' AND released_at IS NOT NULL;
    IF released.id IS NULL THEN
        RAISE EXCEPTION 'Ready reset needs exact released current-fence evidence' USING ERRCODE='23000';
    END IF;
    IF legacy_scalar_lease_matches(released) THEN
        PERFORM lock_legacy_scalar_context(released.run_id,OLD.id,OLD.host_id,true);
        IF NOT EXISTS(SELECT 1 FROM runs WHERE id=released.run_id AND volume_acquisition_closed_at IS NOT NULL) THEN
            RAISE EXCEPTION 'legacy Ready reset requires permanent closure' USING ERRCODE='23000';
        END IF;
    ELSIF NOT EXISTS(SELECT 1 FROM runs execution
        JOIN run_cleanup_target_leases item ON item.run_id=execution.id
        JOIN run_cleanup_receipts receipt ON receipt.run_id=item.run_id AND receipt.generation=item.generation
        WHERE execution.id=released.run_id AND execution.execution_profile='owned_canonical'
          AND item.lease_id=released.id AND item.volume_id=OLD.id AND item.fencing_token=OLD.lease_generation
          AND item.host_id=OLD.host_id) THEN
        RAISE EXCEPTION 'Ready reset cannot adopt unresolved or unrelated release history' USING ERRCODE='23000';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_scalar_or_canonical_ready_reset() FROM PUBLIC;
CREATE TRIGGER scalar_or_canonical_ready_reset BEFORE UPDATE ON agent_instance_state_volumes
    FOR EACH ROW EXECUTE FUNCTION guard_scalar_or_canonical_ready_reset();

CREATE FUNCTION guard_legacy_scalar_run_binding() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE held agent_instance_volume_leases%ROWTYPE;
BEGIN
    IF NEW.execution_profile<>'legacy_scalar' THEN RETURN NEW; END IF;
    IF ROW(NEW.volume_id,NEW.lease_id,NEW.lease_fencing_token)
        IS NOT DISTINCT FROM ROW(OLD.volume_id,OLD.lease_id,OLD.lease_fencing_token) THEN RETURN NEW; END IF;
    PERFORM require_run_cleanup_worker();
    IF num_nonnulls(OLD.volume_id,OLD.lease_id,OLD.lease_fencing_token)>0
       OR num_nonnulls(NEW.volume_id,NEW.lease_id,NEW.lease_fencing_token)<>3
       OR NEW.volume_acquisition_closed_at IS NOT NULL OR NEW.cancel_requested_at IS NOT NULL
       OR OLD.vm_id IS NOT NULL OR NEW.vm_id IS NOT NULL
       OR NEW.state NOT IN ('leasing_volume','provisioning') THEN
        RAISE EXCEPTION 'legacy scalar binding is fresh pre-VM and permanently immutable' USING ERRCODE='23000';
    END IF;
    SELECT * INTO held FROM agent_instance_volume_leases WHERE id=NEW.lease_id;
    IF held.id IS NULL OR legacy_scalar_lease_matches(held) IS NOT TRUE OR held.released_at IS NOT NULL
       OR held.state<>'active' OR held.recovering_at IS NOT NULL
       OR ROW(held.volume_id,held.run_id,held.fencing_token)
            IS DISTINCT FROM ROW(NEW.volume_id,NEW.id,NEW.lease_fencing_token) THEN
        RAISE EXCEPTION 'legacy scalar first binding needs exact checked held lease' USING ERRCODE='23000';
    END IF;
    PERFORM lock_legacy_scalar_context(NEW.id,NEW.volume_id,held.host_id,true);
    PERFORM assert_legacy_scalar_current_fence(held);
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_legacy_scalar_run_binding() FROM PUBLIC;
CREATE TRIGGER legacy_scalar_run_binding BEFORE UPDATE ON runs
    FOR EACH ROW EXECUTE FUNCTION guard_legacy_scalar_run_binding();

CREATE FUNCTION validate_legacy_scalar_acquisition_binding() RETURNS trigger
LANGUAGE plpgsql AS $$ BEGIN
    IF EXISTS(SELECT 1 FROM runs WHERE id=NEW.run_id AND execution_profile='legacy_scalar')
       AND (legacy_scalar_lease_matches(NEW) IS NOT TRUE
         OR NOT EXISTS(SELECT 1 FROM runs WHERE id=NEW.run_id
            AND ROW(volume_id,lease_id,lease_fencing_token)
                IS NOT DISTINCT FROM ROW(NEW.volume_id,NEW.id,NEW.fencing_token))) THEN
        RAISE EXCEPTION 'legacy acquisition must commit with its exact pre-VM Run binding' USING ERRCODE='23000';
    END IF;
    RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_legacy_scalar_acquisition_binding() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER legacy_scalar_acquisition_binding_complete AFTER INSERT ON agent_instance_volume_leases
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_legacy_scalar_acquisition_binding();

-- Remaining separately bounded integration tasks:
-- * RunPG scalar bind locks/revalidation/transition/restart inventory;
-- * ReleasePG callback replay accepts exact already-released legacy tuple;
-- * reviewed app composition selects positive producers and schema mode.
-- This fragment does not activate those producers or prove provider cleanup.
