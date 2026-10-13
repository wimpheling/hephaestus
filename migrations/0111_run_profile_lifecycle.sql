-- PRIVATE additive lifecycle fragment, AFTER corrected profile108 copies and
-- 0108_legacy_scalar_compatibility.sql. No history adoption or app activation.
-- Scalar completion preserves the trusted legacy provider callback boundary;
-- it is not an owned VM absence receipt or recipe retention/delete proof.

CREATE FUNCTION profiled_run_locked(p_run uuid) RETURNS runs
LANGUAGE plpgsql AS $$
DECLARE owner uuid; execution runs%ROWTYPE;
BEGIN
    PERFORM require_run_cleanup_worker();
    SELECT instance_id INTO owner FROM runs WHERE id=p_run;
    IF owner IS NULL THEN RAISE EXCEPTION 'Run not found' USING ERRCODE='23000'; END IF;
    PERFORM id FROM agent_instances WHERE id=owner FOR UPDATE;
    SELECT * INTO execution FROM runs WHERE id=p_run AND instance_id=owner FOR UPDATE;
    IF execution.execution_profile NOT IN ('legacy_scalar','owned_canonical') THEN
        RAISE EXCEPTION 'Run history has no positive execution profile' USING ERRCODE='23000';
    END IF;
    -- Same global resource order as acquisition and canonical cleanup. Include
    -- the released scalar tuple for read-only replay, never reset its resource.
    PERFORM id FROM agent_instance_state_volumes WHERE id=execution.volume_id OR id IN (
        SELECT volume_id FROM agent_instance_volume_leases WHERE run_id=p_run AND released_at IS NULL)
        ORDER BY id FOR UPDATE;
    PERFORM id FROM agent_instance_volume_leases WHERE run_id=p_run ORDER BY volume_id,id FOR UPDATE;
    RETURN execution;
END $$;

CREATE FUNCTION profiled_run_pins_match(execution runs,p_live boolean) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT execution.execution_profile IN ('legacy_scalar','owned_canonical') AND EXISTS(
        SELECT 1 FROM agent_instances consumer
        JOIN agent_instance_revisions revision ON revision.id=execution.instance_revision_id
          AND revision.instance_id=consumer.id AND revision.release_agent_id=execution.release_agent_id
        JOIN release_agents agent ON agent.id=revision.release_agent_id AND agent.release_id=execution.release_id
        JOIN releases release ON release.id=agent.release_id
        WHERE consumer.id=execution.instance_id AND agent.requires_state=execution.requires_state
          AND octet_length(agent.runtime_contract_hash)=32
          AND (execution.execution_profile<>'legacy_scalar' OR legacy_scalar_run_matches(execution))
          AND (NOT p_live OR (release.state='published' AND revision.runnable
            AND execution.cancel_requested_at IS NULL AND execution.volume_acquisition_closed_at IS NULL
            AND NOT EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=consumer.id)
            AND (consumer.volume_mode<>'named' OR named_volume_dispatch_supported(consumer.id))
            AND ((execution.run_kind='normal' AND consumer.active_revision_id=revision.id
                AND consumer.run_gate_open AND consumer.state IN ('active','update_rejected')
                AND EXISTS(SELECT 1 FROM agent_attachments attachment WHERE attachment.id=execution.attachment_id
                  AND attachment.instance_id=consumer.id AND attachment.enabled AND attachment.removed_at IS NULL))
              OR (execution.run_kind='update' AND consumer.state='updating' AND NOT consumer.run_gate_open
                AND EXISTS(SELECT 1 FROM agent_updates update WHERE update.hook_run_id=execution.id
                  AND update.instance_id=consumer.id AND update.candidate_revision_id=revision.id
                  AND update.state='hook_running'))))))
$$;

CREATE FUNCTION profiled_run_snapshot_matches(execution runs) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE expected_identity jsonb; expected_selections jsonb;
BEGIN
    SELECT jsonb_build_object('run',execution.id,'instance',execution.instance_id,
        'revision',execution.instance_revision_id,'release',execution.release_id,
        'release_agent',execution.release_agent_id,'project',consumer.project_id)
      INTO expected_identity FROM agent_instances consumer WHERE consumer.id=execution.instance_id;
    SELECT COALESCE(jsonb_agg(jsonb_build_object('identity',expected_identity,
        'scope',jsonb_build_object('instance_id',lease.instance_id,'revision_id',lease.selected_revision_id,
          'release_agent_id',lease.selected_release_agent_id,'slot',lease.slot_key,'volume_id',lease.volume_id,
          'access_mode',lease.access_mode,'release_contract_hash',(SELECT jsonb_agg(get_byte(lease.release_contract_hash,i) ORDER BY i) FROM generate_series(0,31) i)),
        'declaration',jsonb_build_object('slot',lease.slot_key,'guest_path',lease.guest_path,
          'access_mode',lease.access_mode,'required',lease.slot_required,'minimum_capacity_bytes',lease.minimum_capacity_bytes),
        'origin',lease.selection_origin) ORDER BY lease.slot_key),'[]'::jsonb)
      INTO expected_selections FROM agent_instance_volume_leases lease
      WHERE lease.run_id=execution.id AND lease.released_at IS NULL;
    RETURN execution.volume_selection_snapshot IS NOT DISTINCT FROM
        jsonb_build_object('identity',expected_identity,'selections',expected_selections)
      AND NOT EXISTS(SELECT 1 FROM agent_instance_revision_volume_bindings binding
        WHERE binding.instance_revision_id=execution.instance_revision_id AND binding.slot_required
          AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease WHERE lease.run_id=execution.id
            AND lease.released_at IS NULL AND lease.slot_key=binding.slot_key AND lease.volume_id=binding.volume_id))
      AND NOT EXISTS(SELECT 1 FROM agent_instances consumer JOIN release_agents agent ON agent.id=execution.release_agent_id
        WHERE consumer.id=execution.instance_id AND consumer.volume_mode='legacy' AND agent.requires_state
          AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease WHERE lease.run_id=execution.id
            AND lease.released_at IS NULL AND lease.selection_origin='legacy_origin'));
END $$;

CREATE FUNCTION profiled_run_binding_matches(execution runs) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE held agent_instance_volume_leases%ROWTYPE; count_held bigint;
BEGIN
    IF execution.vm_id IS NULL OR NOT run_cleanup_identifier_is_valid(execution.vm_id)
       OR num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token) NOT IN (0,3) THEN RETURN false; END IF;
    SELECT count(*) INTO count_held FROM agent_instance_volume_leases WHERE run_id=execution.id AND released_at IS NULL;
    IF execution.execution_profile='legacy_scalar' THEN
        IF NOT execution.requires_state THEN
            RETURN execution.volume_id IS NULL AND NOT EXISTS(
                SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id);
        END IF;
        IF count_held<>1 THEN RETURN false; END IF;
        SELECT * INTO held FROM agent_instance_volume_leases WHERE id=execution.lease_id AND run_id=execution.id;
        IF held.id IS NULL OR legacy_scalar_lease_matches(held) IS NOT TRUE
           OR ROW(held.volume_id,held.fencing_token) IS DISTINCT FROM ROW(execution.volume_id,execution.lease_fencing_token) THEN RETURN false; END IF;
    ELSE
        IF execution.vm_binding_kind IS DISTINCT FROM 'planned' OR execution.vm_id<>execution.id::text
           OR execution.vm_provider_namespace IS NULL OR execution.vm_host_id IS NULL
           OR execution.volume_selection_snapshot IS NULL
           OR jsonb_array_length(execution.volume_selection_snapshot->'selections')<>count_held
           OR profiled_run_snapshot_matches(execution) IS NOT TRUE
           OR EXISTS(SELECT 1 FROM run_vm_plan_admissions WHERE run_id=execution.id
               AND (consumed_at IS NULL OR consumption_kind IS DISTINCT FROM 'provision')) THEN RETURN false; END IF;
        -- The existing checked complete-set acquisition freezes the snapshot.
        -- Only its singleton proven LegacyOrigin can have a scalar projection.
        IF execution.volume_id IS NOT NULL THEN
            SELECT * INTO held FROM agent_instance_volume_leases WHERE id=execution.lease_id AND run_id=execution.id;
            IF count_held<>1 OR held.selection_origin IS DISTINCT FROM 'legacy_origin'
              OR ROW(held.volume_id,held.fencing_token) IS DISTINCT FROM ROW(execution.volume_id,execution.lease_fencing_token) THEN RETURN false; END IF;
        ELSIF EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id
            AND released_at IS NULL AND selection_origin='legacy_origin') THEN RETURN false; END IF;
    END IF;
    RETURN NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease
        JOIN agent_instance_state_volumes volume ON volume.id=lease.volume_id
        WHERE lease.run_id=execution.id AND lease.released_at IS NULL
          AND (lease.state<>'active' OR lease.recovering_at IS NOT NULL OR lease.expires_at<=clock_timestamp()
            OR volume.provisioning_state<>'ready' OR volume.state NOT IN ('ready','attached')
            OR volume.host_id IS DISTINCT FROM lease.host_id OR volume.lease_generation<>lease.fencing_token
            OR lease.selected_revision_id IS DISTINCT FROM execution.instance_revision_id
            OR lease.selected_release_id IS DISTINCT FROM execution.release_id
            OR lease.selected_release_agent_id IS DISTINCT FROM execution.release_agent_id
            OR lease.selected_project_id IS DISTINCT FROM volume.project_id
            OR lease.release_contract_hash IS DISTINCT FROM (SELECT runtime_contract_hash FROM release_agents WHERE id=execution.release_agent_id)
            OR lease.instance_id IS DISTINCT FROM execution.instance_id
            OR (execution.execution_profile='owned_canonical' AND lease.host_id IS DISTINCT FROM execution.vm_host_id)));
END $$;

CREATE FUNCTION assert_profiled_run_bind(p_run uuid,p_volume uuid,p_lease uuid,p_fence bigint,p_vm text)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
    PERFORM require_run_cleanup_worker();
    SELECT * INTO execution FROM runs WHERE id=p_run;
    IF execution.state NOT IN ('leasing_volume','provisioning')
       OR profiled_run_pins_match(execution,true) IS NOT TRUE
       OR (ROW(execution.volume_id,execution.lease_id,execution.lease_fencing_token)
          IS DISTINCT FROM ROW(p_volume,p_lease,p_fence)
          AND NOT (execution.execution_profile='owned_canonical'
            AND num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)=0
            AND num_nonnulls(p_volume,p_lease,p_fence)=3))
       OR (execution.vm_id IS NOT NULL AND execution.vm_id IS DISTINCT FROM p_vm) THEN
        RAISE EXCEPTION 'profile binding differs from exact open persisted Run tuple' USING ERRCODE='23000';
    END IF;
    execution.vm_id:=p_vm;
    execution.volume_id:=p_volume;
    execution.lease_id:=p_lease;
    execution.lease_fencing_token:=p_fence;
    IF profiled_run_binding_matches(execution) IS NOT TRUE THEN
        RAISE EXCEPTION 'profile binding lacks exact complete live resources or plan' USING ERRCODE='23000';
    END IF;
END $$;

CREATE FUNCTION legacy_scalar_terminal_matches(execution runs) RETURNS boolean
LANGUAGE plpgsql AS $$
BEGIN
    IF legacy_scalar_run_matches(execution) IS NOT TRUE OR execution.volume_acquisition_closed_at IS NULL
       OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id AND released_at IS NULL)
       OR EXISTS(SELECT 1 FROM run_cleanup_targets WHERE run_id=execution.id)
       OR EXISTS(SELECT 1 FROM run_cleanup_receipts WHERE run_id=execution.id)
       OR EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=execution.id) THEN RETURN false; END IF;
    IF num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)=3 THEN
        RETURN execution.requires_state AND EXISTS(SELECT 1 FROM agent_instance_volume_leases lease
            WHERE lease.id=execution.lease_id AND lease.run_id=execution.id
              AND lease.volume_id=execution.volume_id AND lease.fencing_token=execution.lease_fencing_token
              AND lease.released_at IS NOT NULL AND lease.state='released' AND legacy_scalar_lease_matches(lease));
    END IF;
    IF num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)<>0
       OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id) THEN RETURN false; END IF;
    IF execution.vm_id IS NOT NULL THEN
        -- Positively stateless scalar callback after trusted provider cleanup.
        -- No physical ownership/absence certificate can be inferred from this.
        RETURN NOT execution.requires_state AND run_cleanup_identifier_is_valid(execution.vm_id);
    END IF;
    -- Positively admitted pre-IO failure, never a historical absence inference.
    RETURN NOT EXISTS(SELECT 1 FROM run_events WHERE run_id=execution.id
        AND event_type IN ('run.provisioning','run.starting','run.running'));
END $$;

CREATE FUNCTION assert_profiled_run_transition(p_run uuid,p_next text) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
    PERFORM require_run_cleanup_worker();
    SELECT * INTO execution FROM runs WHERE id=p_run;
    IF profiled_run_pins_match(execution,false) IS NOT TRUE THEN
        RAISE EXCEPTION 'transition requires exact positive released Run pins' USING ERRCODE='23000';
    END IF;
    IF p_next IN ('leasing_volume','provisioning','starting','running') THEN
        IF profiled_run_pins_match(execution,true) IS NOT TRUE
           OR (p_next<>'leasing_volume' AND profiled_run_binding_matches(execution) IS NOT TRUE) THEN
            RAISE EXCEPTION 'new IO needs open exact profile, pins and binding' USING ERRCODE='23000';
        END IF;
    ELSIF p_next='cleaned_up' THEN
        IF execution.state<>'cleaning_up' OR execution.volume_acquisition_closed_at IS NULL
           OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id AND released_at IS NULL)
           OR (execution.execution_profile='legacy_scalar' AND legacy_scalar_terminal_matches(execution) IS NOT TRUE)
           OR (execution.execution_profile='owned_canonical' AND NOT EXISTS(
               SELECT 1 FROM run_cleanup_completions completion WHERE completion.run_id=execution.id
                 AND run_cleanup_set_matches(execution.id,completion.generation,true))) THEN
            RAISE EXCEPTION 'terminal profile transition lacks exact closed completion evidence' USING ERRCODE='23000';
        END IF;
    END IF;
END $$;

CREATE FUNCTION guard_profiled_run_lifecycle() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.execution_profile='historical_unproven' THEN RETURN NEW; END IF;
    IF NEW.vm_id IS DISTINCT FROM OLD.vm_id OR NEW.state IS DISTINCT FROM OLD.state THEN
        PERFORM require_run_cleanup_worker();
        -- A raw caller already owns Run; parent NOWAIT avoids inversion with
        -- the adapter's blocking consumer-first lock order.
        PERFORM id FROM agent_instances WHERE id=OLD.instance_id FOR UPDATE NOWAIT;
    END IF;
    -- Owned first planning/cleanup-only admission has its existing exact
    -- deferred producer guards, before acquisition and even at Queued. This
    -- scalar guard must not reinterpret it as a post-acquisition bind.
    IF NEW.vm_id IS DISTINCT FROM OLD.vm_id AND OLD.execution_profile='legacy_scalar' THEN
        PERFORM assert_profiled_run_bind(OLD.id,NEW.volume_id,NEW.lease_id,NEW.lease_fencing_token,NEW.vm_id);
    END IF;
    IF OLD.execution_profile='owned_canonical' AND
        ROW(NEW.volume_id,NEW.lease_id,NEW.lease_fencing_token)
          IS DISTINCT FROM ROW(OLD.volume_id,OLD.lease_id,OLD.lease_fencing_token) THEN
        PERFORM require_run_cleanup_worker();
        PERFORM id FROM agent_instances WHERE id=OLD.instance_id FOR UPDATE NOWAIT;
        PERFORM assert_profiled_run_bind(OLD.id,NEW.volume_id,NEW.lease_id,NEW.lease_fencing_token,NEW.vm_id);
    END IF;
    IF NEW.state IS DISTINCT FROM OLD.state THEN
        PERFORM assert_profiled_run_transition(OLD.id,NEW.state);
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER z_profiled_run_lifecycle BEFORE UPDATE ON runs
    FOR EACH ROW EXECUTE FUNCTION guard_profiled_run_lifecycle();

REVOKE ALL ON FUNCTION profiled_run_locked(uuid),profiled_run_pins_match(runs,boolean),
 profiled_run_snapshot_matches(runs),profiled_run_binding_matches(runs),assert_profiled_run_bind(uuid,uuid,uuid,bigint,text),
 legacy_scalar_terminal_matches(runs),assert_profiled_run_transition(uuid,text),
 guard_profiled_run_lifecycle() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION profiled_run_locked(uuid),profiled_run_pins_match(runs,boolean),
 profiled_run_snapshot_matches(runs),profiled_run_binding_matches(runs),assert_profiled_run_bind(uuid,uuid,uuid,bigint,text),
 legacy_scalar_terminal_matches(runs),assert_profiled_run_transition(uuid,text) TO hephaestus_worker;
