-- The invoker lease guard reuses the existing read/lock/raise admission guard.
GRANT EXECUTE ON FUNCTION assert_instance_accepts_work(uuid) TO hephaestus_worker;

-- PRIVATE initial metadata proposal. DO NOT publish before coherent runtime,
-- canonical run cleanup receipt/release guards and the legacy upgrade gate.
-- This does not enable named_volume_dispatch_supported from migration 0107.

ALTER TABLE runs
    ADD COLUMN volume_acquisition_closed_at timestamptz,
    ADD COLUMN volume_selection_snapshot jsonb,
    ADD COLUMN vm_plan_admission_required boolean NOT NULL DEFAULT false,
    ADD COLUMN execution_profile text NOT NULL DEFAULT 'historical_unproven'
        CHECK(execution_profile IN ('historical_unproven','legacy_scalar','owned_canonical'));

-- Only fresh update-hook admission creates this positive no-IO evidence.
-- Historical queued rows are deliberately not backfilled or adopted.
CREATE TABLE run_vm_plan_admissions (
    run_id uuid PRIMARY KEY,
    command_id uuid NOT NULL UNIQUE,
    instance_id uuid NOT NULL,
    instance_revision_id uuid NOT NULL,
    update_id uuid NOT NULL UNIQUE REFERENCES agent_updates(id),
    producer_command_key bytea NOT NULL CHECK(octet_length(producer_command_key)=32),
    actor_id uuid NOT NULL REFERENCES users(id),
    request_id uuid NOT NULL,
    admitted_at timestamptz NOT NULL DEFAULT now(),
    consumed_at timestamptz,
    consumption_kind text CHECK(consumption_kind IN ('provision','cleanup')),
    vm_id text,
    vm_provider_namespace text,
    vm_host_id text,
    FOREIGN KEY(run_id,instance_id,instance_revision_id)
        REFERENCES runs(id,instance_id,instance_revision_id),
    CHECK((consumed_at IS NULL AND consumption_kind IS NULL AND num_nonnulls(vm_id,vm_provider_namespace,vm_host_id)=0)
        OR (consumed_at IS NOT NULL AND consumption_kind IS NOT NULL AND num_nonnulls(vm_id,vm_provider_namespace,vm_host_id)=3))
);
ALTER TABLE run_vm_plan_admissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_vm_plan_admissions FORCE ROW LEVEL SECURITY;
GRANT SELECT,INSERT ON run_vm_plan_admissions TO hephaestus_app;
GRANT SELECT,INSERT,UPDATE ON run_vm_plan_admissions TO hephaestus_worker;
CREATE POLICY run_vm_admission_actor_read ON run_vm_plan_admissions FOR SELECT TO hephaestus_app
    USING(actor_id=hephaestus_actor_id()::uuid AND
        check_permission('user',hephaestus_actor_id(),'can_update','agent_instance',instance_id::text)=1);
CREATE POLICY run_vm_admission_actor_insert ON run_vm_plan_admissions FOR INSERT TO hephaestus_app
    WITH CHECK(actor_id=hephaestus_actor_id()::uuid
        AND request_id=NULLIF(current_setting('hephaestus.request_id',true),'')::uuid
        AND consumed_at IS NULL
        AND check_permission('user',hephaestus_actor_id(),'can_update','agent_instance',instance_id::text)=1
        AND EXISTS(SELECT 1 FROM agent_instance_revisions revision
            WHERE revision.id=instance_revision_id AND revision.instance_id=run_vm_plan_admissions.instance_id
              AND check_permission('user',hephaestus_actor_id(),'can_use','release_agent',revision.release_agent_id::text)=1));
CREATE POLICY run_vm_admission_worker_read ON run_vm_plan_admissions FOR SELECT TO hephaestus_worker USING(true);
CREATE POLICY run_vm_admission_worker_producer ON run_vm_plan_admissions FOR INSERT TO hephaestus_worker
    WITH CHECK(actor_id=hephaestus_actor_id()::uuid
        AND request_id=NULLIF(current_setting('hephaestus.request_id',true),'')::uuid
        AND consumed_at IS NULL
        AND check_permission('user',hephaestus_actor_id(),'can_update','agent_instance',instance_id::text)=1
        AND EXISTS(SELECT 1 FROM agent_instance_revisions revision
            WHERE revision.id=instance_revision_id AND revision.instance_id=run_vm_plan_admissions.instance_id
              AND check_permission('user',hephaestus_actor_id(),'can_use','release_agent',revision.release_agent_id::text)=1));
CREATE POLICY run_vm_admission_worker_consume ON run_vm_plan_admissions FOR UPDATE TO hephaestus_worker USING(true) WITH CHECK(true);

CREATE FUNCTION enforce_run_vm_plan_admission() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
    IF TG_OP='DELETE' THEN
        RAISE EXCEPTION 'VM planning admission history cannot be deleted' USING ERRCODE='23000';
    END IF;
    IF TG_OP='INSERT' THEN
        SELECT * INTO execution FROM runs WHERE id=NEW.run_id;
        IF NEW.actor_id IS DISTINCT FROM NULLIF(current_setting('hephaestus.actor_id',true),'')::uuid
           OR NEW.request_id IS DISTINCT FROM NULLIF(current_setting('hephaestus.request_id',true),'')::uuid
           OR execution.id IS NULL OR NEW.consumed_at IS NOT NULL OR execution.vm_id IS NOT NULL
           OR execution.execution_profile<>'owned_canonical'
           OR NOT execution.vm_plan_admission_required
           OR execution.state<>'queued' OR execution.run_kind<>'update'
           OR execution.command_id<>NEW.command_id THEN
            RAISE EXCEPTION 'VM planning needs fresh exact update admission' USING ERRCODE='23000';
        END IF;
    ELSE
        IF (CASE WHEN current_setting('role',true)='none' THEN session_user::text ELSE current_setting('role',true) END)<>'hephaestus_worker'
           OR OLD.consumed_at IS NOT NULL OR NEW.consumed_at IS NULL OR NEW.consumption_kind IS NULL
           OR NEW.vm_id IS DISTINCT FROM NEW.run_id::text
           OR ROW(NEW.run_id,NEW.command_id,NEW.instance_id,NEW.instance_revision_id,NEW.update_id,NEW.producer_command_key,NEW.actor_id,NEW.request_id,NEW.admitted_at)
              IS DISTINCT FROM ROW(OLD.run_id,OLD.command_id,OLD.instance_id,OLD.instance_revision_id,OLD.update_id,OLD.producer_command_key,OLD.actor_id,OLD.request_id,OLD.admitted_at) THEN
            RAISE EXCEPTION 'VM planning admission allows only one worker consumption' USING ERRCODE='23000';
        END IF;
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_run_vm_plan_admission() FROM PUBLIC;
CREATE TRIGGER run_vm_plan_admission_guard BEFORE INSERT OR UPDATE OR DELETE ON run_vm_plan_admissions
    FOR EACH ROW EXECUTE FUNCTION enforce_run_vm_plan_admission();

CREATE FUNCTION enforce_run_vm_io_admission() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
    IF NEW.vm_plan_admission_required AND NEW.execution_profile<>'owned_canonical' THEN
        RAISE EXCEPTION 'only an explicit canonical profile can require owned planning' USING ERRCODE='23000';
    END IF;
    IF TG_OP='INSERT' AND NEW.vm_plan_admission_required AND
       ((CASE WHEN current_setting('role',true)='none' THEN session_user::text ELSE current_setting('role',true) END)<>'hephaestus_worker'
        OR NULLIF(current_setting('hephaestus.actor_id',true),'') IS NULL
        OR NULLIF(current_setting('hephaestus.request_id',true),'') IS NULL
        OR NEW.run_kind<>'update' OR NEW.state<>'queued'
        OR check_permission('user',hephaestus_actor_id(),'can_update','agent_instance',NEW.instance_id::text) IS DISTINCT FROM 1
        OR check_permission('user',hephaestus_actor_id(),'can_use','release_agent',NEW.release_agent_id::text) IS DISTINCT FROM 1) THEN
        RAISE EXCEPTION 'new IO prohibition requires authorized worker hook producer context' USING ERRCODE='42501';
    END IF;
    IF TG_OP='UPDATE' AND NEW.vm_plan_admission_required IS DISTINCT FROM OLD.vm_plan_admission_required THEN
        RAISE EXCEPTION 'VM admission prohibition is immutable' USING ERRCODE='23000';
    END IF;
    IF NEW.vm_plan_admission_required AND NEW.state IN ('leasing_volume','provisioning','starting','running')
       AND NOT EXISTS(SELECT 1 FROM run_vm_plan_admissions WHERE run_id=NEW.id AND consumed_at IS NOT NULL AND consumption_kind='provision') THEN
        RAISE EXCEPTION 'update VM IO is prohibited before admission consumption' USING ERRCODE='23000';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_run_vm_io_admission() FROM PUBLIC;
CREATE TRIGGER run_vm_io_admission BEFORE INSERT OR UPDATE ON runs
    FOR EACH ROW EXECUTE FUNCTION enforce_run_vm_io_admission();
ALTER TABLE runs ADD CONSTRAINT run_volume_selection_snapshot_shape CHECK (
    volume_selection_snapshot IS NULL OR
    (jsonb_typeof(volume_selection_snapshot)='object'
     AND jsonb_typeof(volume_selection_snapshot->'identity')='object'
     AND jsonb_typeof(volume_selection_snapshot->'selections')='array'
     AND jsonb_array_length(volume_selection_snapshot->'selections')<=32));
CREATE FUNCTION enforce_run_volume_acquisition_closure() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.volume_acquisition_closed_at IS NOT NULL AND
        NEW.volume_acquisition_closed_at IS DISTINCT FROM OLD.volume_acquisition_closed_at THEN
        RAISE EXCEPTION 'volume acquisition closure is permanent' USING ERRCODE='23000';
    END IF;
    IF OLD.volume_selection_snapshot IS NOT NULL AND
        NEW.volume_selection_snapshot IS DISTINCT FROM OLD.volume_selection_snapshot THEN
        RAISE EXCEPTION 'run volume selection snapshot is immutable' USING ERRCODE='23000';
    END IF;
    IF NEW.state IN ('cleaning_up','cleaned_up') THEN
        NEW.volume_acquisition_closed_at:=COALESCE(NEW.volume_acquisition_closed_at,now());
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_run_volume_acquisition_closure() FROM PUBLIC;
CREATE TRIGGER run_volume_acquisition_closure BEFORE UPDATE ON runs
    FOR EACH ROW EXECUTE FUNCTION enforce_run_volume_acquisition_closure();
ALTER TABLE runs ADD CONSTRAINT exact_run_volume_consumer_identity
    UNIQUE(id,instance_id,instance_revision_id,release_id,release_agent_id);

ALTER TABLE agent_instance_volume_leases
    ADD COLUMN selection_origin text NOT NULL DEFAULT 'historical_unproven'
        CHECK(selection_origin IN ('historical_unproven','legacy_origin','explicit','legacy_declaration')),
    ADD COLUMN selected_revision_id uuid,
    ADD COLUMN selected_release_id uuid,
    ADD COLUMN selected_release_agent_id uuid,
    ADD COLUMN selected_project_id uuid,
    ADD COLUMN slot_key text,
    ADD COLUMN access_mode text,
    ADD COLUMN guest_path text,
    ADD COLUMN slot_required boolean,
    ADD COLUMN minimum_capacity_bytes bigint,
    ADD COLUMN release_contract_hash bytea;

-- Only frozen exact legacy evidence is classified. Old update-only/unknown
-- identities remain immutable HistoricalUnproven and can only be recovered.
UPDATE agent_instance_volume_leases lease SET
    selection_origin='legacy_origin',selected_revision_id=run.instance_revision_id,
    selected_release_id=run.release_id,selected_release_agent_id=run.release_agent_id,
    selected_project_id=volume.project_id,slot_key='state',access_mode='read_write',
    guest_path='/var/lib/hephaestus',slot_required=true,minimum_capacity_bytes=1,
    release_contract_hash=agent.runtime_contract_hash
FROM runs run
JOIN agent_instances instance ON instance.id=run.instance_id
JOIN agent_instance_revisions revision ON revision.id=run.instance_revision_id
    AND revision.instance_id=instance.id
    AND revision.release_agent_id=run.release_agent_id
JOIN release_agents agent ON agent.id=run.release_agent_id AND agent.release_id=run.release_id
JOIN agent_instance_state_volumes volume ON volume.id=instance.state_volume_id
    AND volume.instance_id=instance.id AND volume.project_id=instance.project_id
WHERE lease.run_id=run.id AND lease.instance_id=instance.id AND lease.volume_id=volume.id
    AND instance.volume_mode='legacy' AND agent.requires_state
    AND octet_length(agent.runtime_contract_hash)=32
    AND jsonb_array_length(COALESCE(agent.runtime_contract->'volume_slots','[]'::jsonb))=0;

ALTER TABLE agent_instance_volume_leases ADD CONSTRAINT selected_lease_shape CHECK (
    (selection_origin='historical_unproven' AND num_nonnulls(selected_revision_id,
        selected_release_id,selected_release_agent_id,selected_project_id,slot_key,
        access_mode,guest_path,slot_required,minimum_capacity_bytes,release_contract_hash)=0)
    OR (selection_origin<>'historical_unproven' AND run_id IS NOT NULL
        AND num_nonnulls(selected_revision_id,selected_release_id,selected_release_agent_id,
            selected_project_id,slot_key,access_mode,guest_path,slot_required,
            minimum_capacity_bytes,release_contract_hash)=10
        AND access_mode IN ('read_only','read_write')
        AND slot_key ~ '^[a-z][a-z0-9_-]{0,63}$'
        AND private_volume_guest_path_is_valid(guest_path)
        AND minimum_capacity_bytes BETWEEN 1 AND 17592186044416
        AND octet_length(release_contract_hash)=32));
ALTER TABLE agent_instance_volume_leases ADD CONSTRAINT selected_lease_exact_run
    FOREIGN KEY(run_id,instance_id,selected_revision_id,selected_release_id,selected_release_agent_id)
    REFERENCES runs(id,instance_id,instance_revision_id,release_id,release_agent_id);
ALTER TABLE agent_instance_volume_leases ADD CONSTRAINT selected_lease_owner
    FOREIGN KEY(volume_id,selected_project_id)
    REFERENCES agent_instance_state_volumes(id,project_id);

ALTER TABLE agent_instance_revision_volume_bindings ADD CONSTRAINT exact_lease_named_binding_identity
    UNIQUE(instance_revision_id,slot_key,volume_id,access_mode,instance_id,release_agent_id,project_id);
ALTER TABLE agent_instance_volume_leases ADD COLUMN named_binding_revision_id uuid
    GENERATED ALWAYS AS (CASE WHEN selection_origin IN ('explicit','legacy_declaration')
        THEN selected_revision_id ELSE NULL END) STORED;
ALTER TABLE agent_instance_volume_leases ADD CONSTRAINT selected_lease_exact_named_binding
    FOREIGN KEY(named_binding_revision_id,slot_key,volume_id,access_mode,instance_id,
        selected_release_agent_id,selected_project_id)
    REFERENCES agent_instance_revision_volume_bindings(instance_revision_id,slot_key,
        volume_id,access_mode,instance_id,release_agent_id,project_id);

-- Exact selected consumers replace the historical origin FK. Old origin and
-- mailbox triple constraints/IDs remain unchanged; history is not retargeted.
ALTER TABLE agent_instance_volume_leases DROP CONSTRAINT
    agent_instance_volume_leases_volume_id_instance_id_fkey;

CREATE FUNCTION enforce_exact_run_volume_lease() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    execution runs%ROWTYPE;
    consumer agent_instances%ROWTYPE;
    agent release_agents%ROWTYPE;
    volume agent_instance_state_volumes%ROWTYPE;
    binding agent_instance_revision_volume_bindings%ROWTYPE;
BEGIN
    IF TG_OP='UPDATE' THEN
        IF ROW(NEW.id,NEW.volume_id,NEW.instance_id,NEW.run_id,NEW.update_id,
            NEW.host_id,NEW.fencing_token,NEW.selection_origin,NEW.selected_revision_id,
            NEW.selected_release_id,NEW.selected_release_agent_id,NEW.selected_project_id,
            NEW.slot_key,NEW.access_mode,NEW.guest_path,NEW.slot_required,
            NEW.minimum_capacity_bytes,NEW.release_contract_hash)
            IS DISTINCT FROM ROW(OLD.id,OLD.volume_id,OLD.instance_id,OLD.run_id,OLD.update_id,
            OLD.host_id,OLD.fencing_token,OLD.selection_origin,OLD.selected_revision_id,
            OLD.selected_release_id,OLD.selected_release_agent_id,OLD.selected_project_id,
            OLD.slot_key,OLD.access_mode,OLD.guest_path,OLD.slot_required,
            OLD.minimum_capacity_bytes,OLD.release_contract_hash) THEN
            RAISE EXCEPTION 'lease selection identity is immutable' USING ERRCODE='23000';
        END IF;
        IF (OLD.selection_origin='historical_unproven'
            OR NOT EXISTS(SELECT 1 FROM runs WHERE id=OLD.run_id AND execution_profile IN ('legacy_scalar','owned_canonical'))) AND
            (NEW.heartbeat_at IS DISTINCT FROM OLD.heartbeat_at OR
             NEW.expires_at IS DISTINCT FROM OLD.expires_at OR
             NEW.attached_at IS DISTINCT FROM OLD.attached_at) THEN
            RAISE EXCEPTION 'historical unproven lease is recovery only' USING ERRCODE='23000';
        END IF;
        RETURN NEW;
    END IF;
    IF NEW.selection_origin='historical_unproven' OR NEW.run_id IS NULL THEN
        RAISE EXCEPTION 'cannot manufacture historical run lease evidence' USING ERRCODE='23000';
    END IF;
    -- Same global order as permanent closure: consumer -> run -> resource.
    SELECT * INTO consumer FROM agent_instances WHERE id=NEW.instance_id FOR UPDATE;
    SELECT * INTO execution FROM runs WHERE id=NEW.run_id FOR UPDATE;
    IF execution.execution_profile NOT IN ('legacy_scalar','owned_canonical')
       OR (NEW.selection_origin IN ('explicit','legacy_declaration') AND execution.execution_profile<>'owned_canonical')
       OR execution.instance_id IS DISTINCT FROM consumer.id
       OR execution.volume_acquisition_closed_at IS NOT NULL
       OR (execution.vm_plan_admission_required AND NOT EXISTS(SELECT 1 FROM run_vm_plan_admissions WHERE run_id=execution.id AND consumed_at IS NOT NULL AND consumption_kind='provision'))
       OR execution.state NOT IN ('leasing_volume','provisioning')
       OR execution.cancel_requested_at IS NOT NULL THEN
        RAISE EXCEPTION 'run is closed to new volume acquisitions' USING ERRCODE='23000';
    END IF;
    PERFORM assert_instance_accepts_work(consumer.id);
    SELECT * INTO agent FROM release_agents WHERE id=execution.release_agent_id;
    SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=NEW.volume_id FOR UPDATE;
    IF agent.id IS NULL OR agent.release_id IS DISTINCT FROM execution.release_id
       OR NOT EXISTS(SELECT 1 FROM agent_instance_revisions revision
            WHERE revision.id=execution.instance_revision_id AND revision.instance_id=consumer.id
            AND revision.release_agent_id=agent.id)
       OR volume.project_id IS DISTINCT FROM consumer.project_id THEN
        RAISE EXCEPTION 'lease differs from exact run consumer' USING ERRCODE='23000';
    END IF;
    IF NOT EXISTS(SELECT 1 FROM releases WHERE id=agent.release_id AND state='published') THEN
        RAISE EXCEPTION 'selected release is not published' USING ERRCODE='23000';
    END IF;
    IF NEW.selection_origin='legacy_origin' AND execution.run_kind='update' THEN
        IF consumer.state<>'updating' OR consumer.run_gate_open OR
           NOT EXISTS(SELECT 1 FROM agent_updates candidate
                WHERE candidate.instance_id=consumer.id
                AND candidate.candidate_revision_id=execution.instance_revision_id
                AND candidate.hook_run_id=execution.id AND candidate.state='hook_running') THEN
            RAISE EXCEPTION 'legacy update hook is not the admitted candidate' USING ERRCODE='23000';
        END IF;
    ELSE
        IF execution.run_kind<>'normal' OR consumer.state NOT IN ('active','update_rejected')
           OR NOT consumer.run_gate_open
           OR consumer.active_revision_id IS DISTINCT FROM execution.instance_revision_id
           OR NOT EXISTS(SELECT 1 FROM agent_instance_revisions revision
                WHERE revision.id=execution.instance_revision_id AND revision.runnable) THEN
            RAISE EXCEPTION 'exact normal run revision is not active and admitted' USING ERRCODE='23000';
        END IF;
    END IF;
    IF NEW.selection_origin='legacy_origin' THEN
        IF consumer.volume_mode<>'legacy' OR NOT agent.requires_state
           OR consumer.state_volume_id IS DISTINCT FROM volume.id
           OR volume.instance_id IS DISTINCT FROM consumer.id
           OR jsonb_array_length(COALESCE(agent.runtime_contract->'volume_slots','[]'::jsonb))<>0 THEN
            RAISE EXCEPTION 'legacy origin is not proven' USING ERRCODE='23000';
        END IF;
        IF num_nonnulls(NEW.selected_revision_id,NEW.selected_release_id,
            NEW.selected_release_agent_id,NEW.selected_project_id,NEW.slot_key,
            NEW.access_mode,NEW.guest_path,NEW.slot_required,NEW.minimum_capacity_bytes,
            NEW.release_contract_hash)>0 AND
            ROW(NEW.selected_revision_id,NEW.selected_release_id,NEW.selected_release_agent_id,
                NEW.selected_project_id,NEW.slot_key,NEW.access_mode,NEW.guest_path,
                NEW.slot_required,NEW.minimum_capacity_bytes,NEW.release_contract_hash)
            IS DISTINCT FROM ROW(execution.instance_revision_id,execution.release_id,
                execution.release_agent_id,consumer.project_id,'state'::text,'read_write'::text,
                '/var/lib/hephaestus'::text,true,1::bigint,agent.runtime_contract_hash) THEN
            RAISE EXCEPTION 'supplied legacy selection is not exact' USING ERRCODE='23000';
        END IF;
        -- Also covers old scalar callers without a fabricated named binding.
        NEW.selected_revision_id:=execution.instance_revision_id;
        NEW.selected_release_id:=execution.release_id;
        NEW.selected_release_agent_id:=execution.release_agent_id;
        NEW.selected_project_id:=consumer.project_id;
        NEW.slot_key:='state';NEW.access_mode:='read_write';
        NEW.guest_path:='/var/lib/hephaestus';NEW.slot_required:=true;
        NEW.minimum_capacity_bytes:=1;NEW.release_contract_hash:=agent.runtime_contract_hash;
    ELSE
        SELECT * INTO binding FROM agent_instance_revision_volume_bindings
            WHERE instance_revision_id=execution.instance_revision_id AND slot_key=NEW.slot_key;
        IF consumer.volume_mode<>'named' OR execution.run_kind<>'normal'
           OR NOT named_volume_dispatch_supported(consumer.id)
           OR binding.volume_id IS DISTINCT FROM volume.id
           OR binding.provenance IS DISTINCT FROM NEW.selection_origin
           OR ROW(NEW.selected_revision_id,NEW.selected_release_id,NEW.selected_release_agent_id,
                  NEW.selected_project_id,NEW.access_mode,NEW.guest_path,NEW.slot_required,
                  NEW.minimum_capacity_bytes,NEW.release_contract_hash)
              IS DISTINCT FROM ROW(execution.instance_revision_id,execution.release_id,agent.id,
                  consumer.project_id,binding.access_mode,binding.guest_path,binding.slot_required,
                  binding.minimum_capacity_bytes,agent.runtime_contract_hash)
           OR NOT private_volume_mount_binding_is_declared(binding.instance_revision_id,binding.slot_key)
           OR NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_grants grant_record
                WHERE grant_record.instance_id=consumer.id
                AND grant_record.instance_revision_id=binding.instance_revision_id
                AND grant_record.release_agent_id=agent.id AND grant_record.slot_key=binding.slot_key
                AND grant_record.volume_id=volume.id AND grant_record.access_mode=binding.access_mode
                AND grant_record.release_contract_hash=agent.runtime_contract_hash
                AND private_volume_mount_source_is_live(grant_record.created_by,volume.id)
                AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_revocations revoked
                    WHERE revoked.grant_id=grant_record.id))
           OR check_permission('agent_instance',consumer.id::text,'agent_attach',
                'state_volume',volume.id::text)<>1 THEN
            RAISE EXCEPTION 'exact named mount authority is unavailable' USING ERRCODE='42501';
        END IF;
    END IF;
    IF volume.state<>'ready' OR volume.provisioning_state<>'ready'
       OR volume.host_id IS NULL OR NEW.host_id IS DISTINCT FROM volume.host_id
       OR NEW.fencing_token IS DISTINCT FROM volume.lease_generation+1 THEN
        RAISE EXCEPTION 'volume is not ready on the exact host and next fence' USING ERRCODE='23000';
    END IF;
    IF volume.capacity_bytes<NEW.minimum_capacity_bytes THEN
        RAISE EXCEPTION 'volume capacity is below the exact declaration' USING ERRCODE='23000';
    END IF;
    UPDATE agent_instance_state_volumes SET lease_generation=NEW.fencing_token,updated_at=now()
        WHERE id=NEW.volume_id;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_exact_run_volume_lease() FROM PUBLIC;
CREATE TRIGGER exact_run_volume_lease BEFORE INSERT OR UPDATE ON agent_instance_volume_leases
    FOR EACH ROW EXECUTE FUNCTION enforce_exact_run_volume_lease();
ALTER TABLE agent_instance_volume_leases ALTER COLUMN selection_origin SET DEFAULT 'legacy_origin';

-- Preserve the exclusive per-resource index for both RO and RW. Only after
-- complete immutable selected identity is enforced is the old per-run cap removed.
CREATE UNIQUE INDEX one_active_run_volume_slot ON agent_instance_volume_leases(run_id,slot_key)
    WHERE released_at IS NULL AND selection_origin<>'historical_unproven';
DROP INDEX one_active_instance_run_lease;

-- REQUIRED before publication: canonical run cleanup generation/VM target,
-- immutable complete fence-set receipt and atomic release/CleanedUp guards.
-- Existing legacy acquisition/RunLaunchAuthorizer paths must persist checked
-- origin and retain caller SourceCanUse/attachment/update checks. This storage
-- proof does not authorize a caller. Named dispatch must still remain disabled.

-- PROFILE FOUNDATION TODO (not a compatibility activation): explicit producer
-- writes, scalar qualification/release and source-authority adapters remain
-- separate bounded tasks. No schema-presence probe selects owned execution.
-- Every previous row and every unspecified INSERT stays historical_unproven.
-- Frozen lease-selection classification above does NOT classify the Run profile.
