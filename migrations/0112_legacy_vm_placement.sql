-- PRIVATE additive Legacy placement admission, AFTER profile108, scalar and
-- lifecycle fragments. No history adoption, app enablement or physical proof.
ALTER TABLE runs ADD COLUMN legacy_vm_placement_required boolean NOT NULL DEFAULT false;

CREATE TABLE run_legacy_vm_placements (
    run_id uuid PRIMARY KEY REFERENCES runs(id),
    command_id uuid NOT NULL UNIQUE,
    instance_id uuid NOT NULL,
    instance_revision_id uuid NOT NULL,
    release_id uuid NOT NULL,
    release_agent_id uuid NOT NULL,
    project_id uuid NOT NULL REFERENCES projects(id),
    attachment_id uuid,
    run_kind text NOT NULL CHECK(run_kind IN ('normal','update')),
    requires_state boolean NOT NULL,
    release_contract_hash bytea NOT NULL CHECK(octet_length(release_contract_hash)=32),
    provider_namespace uuid NOT NULL CHECK(provider_namespace<>'00000000-0000-0000-0000-000000000000'::uuid),
    host_id text NOT NULL CHECK(run_cleanup_identifier_is_valid(host_id)),
    vm_id text NOT NULL CHECK(vm_id=run_id::text),
    producer_kind text NOT NULL CHECK(producer_kind IN ('normal_request','mailbox','update_hook')),
    producer_id uuid NOT NULL,
    event_id uuid,
    producer_command_key bytea,
    actor_id uuid REFERENCES users(id),
    request_id uuid,
    born_at timestamptz NOT NULL DEFAULT now(),
    consumed_at timestamptz,
    consumption_kind text CHECK(consumption_kind IN ('provision','cleanup_only')),
    FOREIGN KEY(run_id,instance_id,instance_revision_id,release_id,release_agent_id)
        REFERENCES runs(id,instance_id,instance_revision_id,release_id,release_agent_id),
    UNIQUE(provider_namespace,host_id,vm_id),
    CHECK((consumed_at IS NULL)=(consumption_kind IS NULL)),
    CHECK((producer_kind='normal_request' AND run_kind='normal' AND event_id IS NULL AND producer_command_key IS NULL)
       OR (producer_kind='mailbox' AND run_kind='normal' AND event_id IS NOT NULL AND producer_command_key IS NULL AND actor_id IS NULL AND request_id IS NULL)
       OR (producer_kind='update_hook' AND run_kind='update' AND event_id IS NULL AND octet_length(producer_command_key)=32 AND actor_id IS NOT NULL AND request_id IS NOT NULL))
);
ALTER TABLE run_legacy_vm_placements ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_legacy_vm_placements FORCE ROW LEVEL SECURITY;
GRANT SELECT,INSERT,UPDATE ON run_legacy_vm_placements TO hephaestus_worker;
CREATE POLICY legacy_vm_placement_worker ON run_legacy_vm_placements TO hephaestus_worker
    USING(true) WITH CHECK(true);

-- Producer correlation is immutable evidence, not a live phase or permission
-- predicate. Mutable update actor and delivery/request disposition are ignored.
CREATE FUNCTION legacy_vm_placement_producer_matches(plan run_legacy_vm_placements) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT CASE plan.producer_kind
      WHEN 'normal_request' THEN EXISTS(SELECT 1 FROM run_requests request
        WHERE request.id=plan.producer_id AND request.request_kind='instance_normal'
          AND ROW(request.run_id,request.command_id,request.instance_id,request.instance_revision_id,
                  request.release_id,request.release_agent_id,request.attachment_id,request.requires_state,
                  request.actor_id,request.request_id)
            IS NOT DISTINCT FROM ROW(plan.run_id,plan.command_id,plan.instance_id,plan.instance_revision_id,
                  plan.release_id,plan.release_agent_id,plan.attachment_id,plan.requires_state,plan.actor_id,plan.request_id)
          AND EXISTS(SELECT 1 FROM command_inbox inbox WHERE inbox.command_id=plan.command_id
            AND inbox.command_type='start_run' AND inbox.payload=jsonb_build_object(
              'command_id',plan.command_id,'run_id',plan.run_id,'instance_id',plan.instance_id,
              'instance_revision_id',plan.instance_revision_id,'release_id',plan.release_id,
              'release_agent_id',plan.release_agent_id,'attachment_id',plan.attachment_id,
              'kind',plan.run_kind,'requires_state',plan.requires_state)))
      WHEN 'mailbox' THEN EXISTS(SELECT 1 FROM mailbox_delivery_attempts attempt
        JOIN mailbox_events event ON event.id=attempt.event_id AND event.mailbox_id=attempt.mailbox_id
        WHERE ROW(attempt.id,attempt.event_id,attempt.run_id,attempt.command_id,
                  attempt.instance_id,attempt.instance_revision_id,event.instance_id,event.project_id)
            IS NOT DISTINCT FROM ROW(plan.producer_id,plan.event_id,plan.run_id,plan.command_id,
                  plan.instance_id,plan.instance_revision_id,plan.instance_id,plan.project_id)
          AND attempt.target_ref IS NOT NULL AND attempt.target_commit IS NOT NULL)
      WHEN 'update_hook' THEN EXISTS(SELECT 1 FROM agent_updates update
        JOIN release_command_inbox command ON command.operation='begin_update_hook'
          AND command.aggregate_id=update.id AND command.secondary_id=plan.run_id
        WHERE update.id=plan.producer_id
          AND update.instance_id=plan.instance_id AND update.candidate_revision_id=plan.instance_revision_id
          AND ROW(command.command_key,command.actor_id,command.request_id)
            IS NOT DISTINCT FROM ROW(plan.producer_command_key,plan.actor_id,plan.request_id))
      ELSE false END
$$;

CREATE FUNCTION legacy_vm_placement_matches(execution runs,plan run_legacy_vm_placements) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT execution.legacy_vm_placement_required AND execution.execution_profile='legacy_scalar'
      AND ROW(execution.id,execution.command_id,execution.instance_id,execution.instance_revision_id,
              execution.release_id,execution.release_agent_id,execution.attachment_id,execution.run_kind,execution.requires_state)
        IS NOT DISTINCT FROM ROW(plan.run_id,plan.command_id,plan.instance_id,plan.instance_revision_id,
              plan.release_id,plan.release_agent_id,plan.attachment_id,plan.run_kind,plan.requires_state)
      AND (execution.vm_id IS NULL OR execution.vm_id=plan.vm_id)
      AND EXISTS(SELECT 1 FROM agent_instances consumer
        JOIN agent_instance_revisions revision ON revision.instance_id=consumer.id AND revision.id=plan.instance_revision_id
        JOIN release_agents agent ON agent.id=revision.release_agent_id AND agent.id=plan.release_agent_id AND agent.release_id=plan.release_id
        WHERE consumer.id=plan.instance_id AND consumer.project_id=plan.project_id AND consumer.volume_mode='legacy'
          AND agent.requires_state=plan.requires_state AND agent.runtime_contract_hash=plan.release_contract_hash
          AND jsonb_typeof(COALESCE(agent.runtime_contract->'volume_slots','[]'::jsonb))='array'
          AND jsonb_array_length(COALESCE(agent.runtime_contract->'volume_slots','[]'::jsonb))=0)
      AND legacy_vm_placement_producer_matches(plan)
$$;

CREATE FUNCTION legacy_vm_placement_cleanup_eligible(execution runs) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT execution.volume_acquisition_closed_at IS NOT NULL AND execution.vm_id IS NULL
      AND execution.state IN ('queued','leasing_volume','failed','cancelled','cleaning_up')
      AND num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)=0
      AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id)
      AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease
        JOIN agent_updates update ON update.id=lease.update_id WHERE update.hook_run_id=execution.id)
      AND NOT EXISTS(SELECT 1 FROM run_events WHERE run_id=execution.id
        AND event_type NOT IN ('run.queued','run.leasing_volume','run.failed','run.cancelled','run.cleaning_up','run.legacy_vm_placement','run.legacy_vm_consumed'))
      AND NOT EXISTS(SELECT 1 FROM run_cleanup_targets WHERE run_id=execution.id)
      AND NOT EXISTS(SELECT 1 FROM runtime_authority_sessions WHERE run_id=execution.id)
      AND NOT EXISTS(SELECT 1 FROM run_authorization_snapshots WHERE run_id=execution.id)
$$;

CREATE FUNCTION enforce_legacy_vm_placement() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
    PERFORM require_run_cleanup_worker();
    IF TG_OP='DELETE' THEN RAISE EXCEPTION 'Legacy placement history is immutable' USING ERRCODE='23000'; END IF;
    -- Raw child entry already holds its row; NOWAIT prevents inverse lock order.
    PERFORM id FROM agent_instances WHERE id=NEW.instance_id FOR UPDATE NOWAIT;
    SELECT * INTO execution FROM runs WHERE id=NEW.run_id FOR UPDATE NOWAIT;
    IF execution.id IS NULL OR NOT execution.legacy_vm_placement_required
       OR execution.execution_profile<>'legacy_scalar' THEN
        RAISE EXCEPTION 'Legacy placement needs exact positive birth marker' USING ERRCODE='23000';
    END IF;
    IF TG_OP='INSERT' THEN
        IF execution.state<>'queued' OR execution.vm_id IS NOT NULL
           OR execution.volume_acquisition_closed_at IS NOT NULL
           OR num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)>0
           OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id)
           OR (NEW.producer_kind='normal_request' AND NEW.consumption_kind IS DISTINCT FROM 'provision')
           OR (NEW.producer_kind<>'normal_request' AND NEW.consumed_at IS NOT NULL) THEN
            RAISE EXCEPTION 'Legacy placement can only be stamped at fresh producer birth' USING ERRCODE='23000';
        END IF;
    ELSE
        IF OLD.consumed_at IS NOT NULL OR NEW.consumed_at IS NULL OR NEW.consumption_kind IS NULL
           OR (to_jsonb(NEW)-'consumed_at'-'consumption_kind') IS DISTINCT FROM
              (to_jsonb(OLD)-'consumed_at'-'consumption_kind')
           OR legacy_vm_placement_matches(execution,NEW) IS NOT TRUE THEN
            RAISE EXCEPTION 'Legacy placement permits one exact worker consumption' USING ERRCODE='23000';
        END IF;
        IF NEW.consumption_kind='cleanup_only' THEN
            IF legacy_vm_placement_cleanup_eligible(execution) IS NOT TRUE THEN
                RAISE EXCEPTION 'Legacy cleanup consumption lacks positive never-IO evidence' USING ERRCODE='23000';
            END IF;
        ELSIF execution.state NOT IN ('queued','leasing_volume') OR execution.vm_id IS NOT NULL
           OR execution.cancel_requested_at IS NOT NULL OR execution.volume_acquisition_closed_at IS NOT NULL
           OR profiled_run_pins_match(execution,true) IS NOT TRUE
           OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id)
           OR NOT run_profile_producer_matches(execution) THEN
            RAISE EXCEPTION 'Legacy provision consumption requires an open exact fresh producer' USING ERRCODE='23000';
        END IF;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER legacy_vm_placement_guard BEFORE INSERT OR UPDATE OR DELETE ON run_legacy_vm_placements
    FOR EACH ROW EXECUTE FUNCTION enforce_legacy_vm_placement();

CREATE FUNCTION guard_legacy_vm_placement_run() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE plan run_legacy_vm_placements%ROWTYPE;
BEGIN
    IF TG_OP='UPDATE' AND NEW.legacy_vm_placement_required IS DISTINCT FROM OLD.legacy_vm_placement_required THEN
        RAISE EXCEPTION 'Legacy placement birth marker is immutable' USING ERRCODE='23000';
    END IF;
    IF NOT NEW.legacy_vm_placement_required THEN RETURN NEW; END IF;
    PERFORM require_run_cleanup_worker();
    IF NEW.execution_profile<>'legacy_scalar' THEN
        RAISE EXCEPTION 'Legacy placement belongs only to explicit Legacy profile' USING ERRCODE='23000';
    END IF;
    IF TG_OP='INSERT' THEN
        IF NEW.state<>'queued' OR NEW.vm_id IS NOT NULL OR NEW.volume_acquisition_closed_at IS NOT NULL THEN
            RAISE EXCEPTION 'Legacy birth marker needs fresh no-IO Run' USING ERRCODE='23000';
        END IF;
    ELSE
        SELECT * INTO plan FROM run_legacy_vm_placements WHERE run_id=NEW.id;
        IF plan.run_id IS NULL OR legacy_vm_placement_matches(NEW,plan) IS NOT TRUE THEN
            RAISE EXCEPTION 'Legacy placement pins or recorded scope contradict Run' USING ERRCODE='23000';
        END IF;
        IF (NEW.vm_id IS DISTINCT FROM OLD.vm_id
            OR (NEW.state IS DISTINCT FROM OLD.state AND NEW.state IN ('leasing_volume','provisioning','starting','running'))
            OR ROW(NEW.volume_id,NEW.lease_id,NEW.lease_fencing_token)
                IS DISTINCT FROM ROW(OLD.volume_id,OLD.lease_id,OLD.lease_fencing_token))
           AND (plan.consumption_kind IS DISTINCT FROM 'provision' OR plan.consumed_at IS NULL
                OR NEW.volume_acquisition_closed_at IS NOT NULL) THEN
            RAISE EXCEPTION 'Legacy IO requires exact provision-consumed placement' USING ERRCODE='23000';
        END IF;
        IF NEW.state='cleaned_up' AND plan.consumed_at IS NULL THEN
            RAISE EXCEPTION 'Unconsumed Legacy eligibility cannot certify terminal cleanup' USING ERRCODE='23000';
        END IF;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER legacy_vm_placement_run_guard BEFORE INSERT OR UPDATE ON runs
    FOR EACH ROW EXECUTE FUNCTION guard_legacy_vm_placement_run();

CREATE FUNCTION validate_legacy_vm_placement_birth() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE; plan run_legacy_vm_placements%ROWTYPE;
BEGIN
    SELECT * INTO execution FROM runs WHERE id=NEW.id;
    IF NOT execution.legacy_vm_placement_required THEN RETURN NULL; END IF;
    SELECT * INTO plan FROM run_legacy_vm_placements WHERE run_id=execution.id;
    IF plan.run_id IS NULL OR legacy_vm_placement_matches(execution,plan) IS NOT TRUE
       OR profiled_run_pins_match(execution,true) IS NOT TRUE
       OR NOT run_profile_producer_matches(execution) THEN
        RAISE EXCEPTION 'Legacy birth must commit exact placement, published pins and producer atomically' USING ERRCODE='23000';
    END IF;
    RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER legacy_vm_placement_birth_commit AFTER INSERT ON runs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_legacy_vm_placement_birth();

CREATE FUNCTION record_legacy_vm_placement(p_run uuid,p_namespace uuid,p_host text,p_producer text) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE; producer uuid; event uuid; key bytea; actor uuid; request uuid;
BEGIN
    PERFORM require_run_cleanup_worker();
    PERFORM id FROM agent_instances WHERE id=(SELECT instance_id FROM runs WHERE id=p_run) FOR UPDATE;
    SELECT * INTO execution FROM runs WHERE id=p_run FOR UPDATE;
    IF p_producer='normal_request' THEN
        SELECT id,actor_id,request_id INTO producer,actor,request FROM run_requests
            WHERE run_id=p_run AND command_id=execution.command_id;
    ELSIF p_producer='mailbox' THEN
        SELECT id,event_id INTO producer,event FROM mailbox_delivery_attempts
            WHERE run_id=p_run AND command_id=execution.command_id;
    ELSIF p_producer='update_hook' THEN
        SELECT update.id,command.command_key,command.actor_id,command.request_id INTO producer,key,actor,request
          FROM agent_updates update JOIN release_command_inbox command ON command.aggregate_id=update.id
            AND command.secondary_id=p_run AND command.operation='begin_update_hook'
          WHERE update.hook_run_id=p_run;
    ELSE RAISE EXCEPTION 'Unknown Legacy placement producer' USING ERRCODE='23000'; END IF;
    INSERT INTO run_legacy_vm_placements(run_id,command_id,instance_id,instance_revision_id,release_id,
        release_agent_id,project_id,attachment_id,run_kind,requires_state,release_contract_hash,
        provider_namespace,host_id,vm_id,producer_kind,producer_id,event_id,producer_command_key,
        actor_id,request_id,consumed_at,consumption_kind)
      SELECT execution.id,execution.command_id,execution.instance_id,execution.instance_revision_id,
        execution.release_id,execution.release_agent_id,consumer.project_id,execution.attachment_id,
        execution.run_kind,execution.requires_state,agent.runtime_contract_hash,p_namespace,p_host,p_run::text,
        p_producer,producer,event,key,actor,request,
        CASE WHEN p_producer='normal_request' THEN now() END,
        CASE WHEN p_producer='normal_request' THEN 'provision' END
      FROM agent_instances consumer JOIN release_agents agent ON agent.id=execution.release_agent_id
      WHERE consumer.id=execution.instance_id;
    IF NOT FOUND THEN RAISE EXCEPTION 'Legacy placement source unavailable' USING ERRCODE='23000'; END IF;
END $$;

CREATE FUNCTION consume_legacy_vm_placement(p_run uuid,p_command uuid,p_namespace uuid,p_host text,p_kind text)
RETURNS run_legacy_vm_placements LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE; plan run_legacy_vm_placements%ROWTYPE;
BEGIN
    PERFORM require_run_cleanup_worker();
    PERFORM id FROM agent_instances WHERE id=(SELECT instance_id FROM runs WHERE id=p_run) FOR UPDATE;
    SELECT * INTO execution FROM runs WHERE id=p_run FOR UPDATE;
    SELECT * INTO plan FROM run_legacy_vm_placements WHERE run_id=p_run FOR UPDATE;
    PERFORM id FROM agent_instance_state_volumes WHERE id=execution.volume_id OR id IN (
        SELECT volume_id FROM agent_instance_volume_leases WHERE run_id=p_run) ORDER BY id FOR UPDATE;
    PERFORM id FROM agent_instance_volume_leases WHERE run_id=p_run ORDER BY volume_id,id FOR UPDATE;
    IF plan.run_id IS NULL OR plan.command_id<>p_command OR plan.provider_namespace<>p_namespace OR plan.host_id<>p_host
       OR p_kind NOT IN ('provision','cleanup_only') OR legacy_vm_placement_matches(execution,plan) IS NOT TRUE THEN
        RAISE EXCEPTION 'Legacy placement is held: absent or mismatched scope/producer' USING ERRCODE='23000';
    END IF;
    IF plan.consumed_at IS NOT NULL THEN
        IF plan.consumption_kind<>p_kind OR (p_kind='provision' AND execution.volume_acquisition_closed_at IS NOT NULL) THEN
            RAISE EXCEPTION 'Legacy consumed placement cannot promote or reopen IO' USING ERRCODE='23000';
        END IF;
        RETURN plan;
    END IF;
    UPDATE run_legacy_vm_placements SET consumed_at=now(),consumption_kind=p_kind WHERE run_id=p_run RETURNING * INTO plan;
    RETURN plan;
END $$;

CREATE FUNCTION close_legacy_vm_acquisition(p_run uuid,p_namespace uuid,p_host text)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE; plan run_legacy_vm_placements%ROWTYPE;
BEGIN
    PERFORM require_run_cleanup_worker();
    PERFORM id FROM agent_instances WHERE id=(SELECT instance_id FROM runs WHERE id=p_run) FOR UPDATE;
    SELECT * INTO execution FROM runs WHERE id=p_run FOR UPDATE;
    SELECT * INTO plan FROM run_legacy_vm_placements WHERE run_id=p_run FOR UPDATE;
    IF plan.run_id IS NULL OR plan.provider_namespace<>p_namespace OR plan.host_id<>p_host
       OR legacy_vm_placement_matches(execution,plan) IS NOT TRUE THEN
        RAISE EXCEPTION 'Legacy closure is held: exact birth scope unavailable' USING ERRCODE='23000';
    END IF;
    UPDATE runs SET volume_acquisition_closed_at=COALESCE(volume_acquisition_closed_at,now()) WHERE id=p_run;
END $$;

CREATE FUNCTION lock_legacy_vm_placement_scope(p_run uuid,p_namespace uuid,p_host text)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE; plan run_legacy_vm_placements%ROWTYPE;
BEGIN
    PERFORM require_run_cleanup_worker();
    PERFORM id FROM agent_instances WHERE id=(SELECT instance_id FROM runs WHERE id=p_run) FOR UPDATE;
    SELECT * INTO execution FROM runs WHERE id=p_run FOR UPDATE;
    SELECT * INTO plan FROM run_legacy_vm_placements WHERE run_id=p_run FOR UPDATE;
    IF plan.run_id IS NULL OR plan.provider_namespace<>p_namespace OR plan.host_id<>p_host
       OR legacy_vm_placement_matches(execution,plan) IS NOT TRUE THEN
        RAISE EXCEPTION 'Legacy lifecycle is held: exact configured birth scope unavailable' USING ERRCODE='23000';
    END IF;
END $$;

-- Placement-aware held leases cannot escape the recorded physical host. This
-- additive raw guard conveys no volume authority and does not change old tuples.
CREATE FUNCTION guard_legacy_placement_lease() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE; plan run_legacy_vm_placements%ROWTYPE;
BEGIN
    SELECT * INTO execution FROM runs WHERE id=NEW.run_id;
    IF execution.legacy_vm_placement_required AND (TG_OP='INSERT' OR NEW.released_at IS NULL) THEN
        PERFORM require_run_cleanup_worker();
        SELECT * INTO plan FROM run_legacy_vm_placements WHERE run_id=NEW.run_id;
        IF plan.run_id IS NULL OR plan.consumption_kind IS DISTINCT FROM 'provision'
           OR plan.consumed_at IS NULL OR plan.host_id<>NEW.host_id
           OR legacy_vm_placement_matches(execution,plan) IS NOT TRUE THEN
            RAISE EXCEPTION 'Legacy lease needs exact consumed VM placement host' USING ERRCODE='23000';
        END IF;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER legacy_placement_lease_guard BEFORE INSERT OR UPDATE ON agent_instance_volume_leases
    FOR EACH ROW EXECUTE FUNCTION guard_legacy_placement_lease();

REVOKE ALL ON FUNCTION legacy_vm_placement_producer_matches(run_legacy_vm_placements),
    legacy_vm_placement_matches(runs,run_legacy_vm_placements),legacy_vm_placement_cleanup_eligible(runs),
    enforce_legacy_vm_placement(),guard_legacy_vm_placement_run(),validate_legacy_vm_placement_birth(),
    record_legacy_vm_placement(uuid,uuid,text,text),consume_legacy_vm_placement(uuid,uuid,uuid,text,text),
    close_legacy_vm_acquisition(uuid,uuid,text),
    lock_legacy_vm_placement_scope(uuid,uuid,text),
    guard_legacy_placement_lease() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION legacy_vm_placement_producer_matches(run_legacy_vm_placements),
    legacy_vm_placement_matches(runs,run_legacy_vm_placements),legacy_vm_placement_cleanup_eligible(runs),
    enforce_legacy_vm_placement(),guard_legacy_vm_placement_run(),validate_legacy_vm_placement_birth(),
    record_legacy_vm_placement(uuid,uuid,text,text),consume_legacy_vm_placement(uuid,uuid,uuid,text,text),
    close_legacy_vm_acquisition(uuid,uuid,text),
    lock_legacy_vm_placement_scope(uuid,uuid,text),
    guard_legacy_placement_lease() TO hephaestus_worker;

-- The new worker-invoker birth/consumption guards reuse this frozen read-only
-- producer predicate. PUBLIC/app retain no EXECUTE privilege.
GRANT EXECUTE ON FUNCTION run_profile_producer_matches(runs) TO hephaestus_worker;
