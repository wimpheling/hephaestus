-- PRIVATE DESIGN PROPOSAL ONLY. UNEXECUTED, UNWIRED, NOT A PUBLIC MIGRATION.
-- Mandatory missing integration: concrete opaque Local pinned-journal flock
-- custody held by owning service through worker receipt -> actor TX commit.
-- SQL, DTOs, timestamps and worker role do not prove custody. Do not publish
-- until that concrete native witness API and actual integration tests exist.
-- This file follows ordered 108/109/110/111. Frozen sources remain unchanged.

CREATE TABLE volume_owned_partial_receipts (
 id uuid PRIMARY KEY CHECK(id<>'00000000-0000-0000-0000-000000000000'),
 volume_id uuid NOT NULL,
 project_id uuid NOT NULL,
 seal_id uuid NOT NULL,
 operation_id uuid NOT NULL,
 generation bigint NOT NULL CHECK(generation>0),
 purpose_hash bytea NOT NULL CHECK(octet_length(purpose_hash)=32),
 previous_observation_id uuid,
 previous_observation_version bigint NOT NULL CHECK(previous_observation_version>=0),
 observation_version bigint NOT NULL CHECK(observation_version=previous_observation_version+1),
 phase text NOT NULL CHECK(phase IN ('claimed_empty','allocated_never_format','format_intent_incomplete')),
 declared_capacity_bytes bigint NOT NULL CHECK(declared_capacity_bytes>0),
 actual_length_bytes bigint NOT NULL CHECK(actual_length_bytes>=0),
 proof_json jsonb NOT NULL CHECK(jsonb_typeof(proof_json)='object' AND octet_length(proof_json::text)<=65536),
 evidence_hash bytea NOT NULL CHECK(evidence_hash=sha256(convert_to(proof_json::text,'UTF8'))),
 event_id uuid NOT NULL REFERENCES application_events(id),
 recorded_at timestamptz NOT NULL DEFAULT now(),
 transaction_id bigint NOT NULL DEFAULT txid_current(),
 UNIQUE(id,volume_id,observation_version,phase),
 FOREIGN KEY(seal_id,volume_id,project_id) REFERENCES volume_creation_seals(id,volume_id,project_id),
 FOREIGN KEY(operation_id,volume_id,generation) REFERENCES volume_owned_provisioning_operations(id,volume_id,generation),
 FOREIGN KEY(volume_id,purpose_hash) REFERENCES volume_owned_backing_purposes(volume_id,purpose_hash),
 CHECK((previous_observation_version=0)=(previous_observation_id IS NULL)),
 CHECK((phase='claimed_empty' AND actual_length_bytes=0)
    OR (phase IN ('allocated_never_format','format_intent_incomplete') AND actual_length_bytes=declared_capacity_bytes))
);

-- No historic backfill and no timestamp/UUID-selected 'latest' row. No head
-- means unknown current history, not absence/permission. Fresh custody-backed
-- positive receipt can bootstrap at version 1, preserving all original rows.
CREATE TABLE volume_owned_observation_heads (
 volume_id uuid PRIMARY KEY REFERENCES volume_owned_backing_purposes(volume_id),
 version bigint NOT NULL CHECK(version>0),
 backing_observation_id uuid,
 partial_receipt_id uuid,
 phase text NOT NULL CHECK(phase IN ('recorded','format_intent','failed_before_format','ready','uncertain',
    'claimed_empty','allocated_never_format','format_intent_incomplete')),
 FOREIGN KEY(backing_observation_id,volume_id) REFERENCES volume_owned_backing_observations(id,volume_id),
 FOREIGN KEY(partial_receipt_id,volume_id,version,phase) REFERENCES volume_owned_partial_receipts(id,volume_id,observation_version,phase),
 CHECK(num_nonnulls(backing_observation_id,partial_receipt_id)=1),
 CHECK((partial_receipt_id IS NOT NULL)=(phase IN ('claimed_empty','allocated_never_format','format_intent_incomplete')))
);

CREATE TABLE volume_owned_birth_retirements (
 volume_id uuid PRIMARY KEY REFERENCES agent_instance_state_volumes(id),
 receipt_id uuid NOT NULL UNIQUE REFERENCES volume_owned_partial_receipts(id),
 transition_id uuid NOT NULL UNIQUE REFERENCES recipe_execution_transitions(id) DEFERRABLE INITIALLY DEFERRED,
 command_id uuid NOT NULL REFERENCES recipe_deployment_commands(id),
 operation_id uuid NOT NULL UNIQUE CHECK(operation_id<>'00000000-0000-0000-0000-000000000000'),
 actor_id uuid NOT NULL REFERENCES users(id),
 request_id uuid NOT NULL,
 project_id uuid NOT NULL REFERENCES projects(id),
 fingerprint bytea NOT NULL CHECK(octet_length(fingerprint)=32),
 event_id uuid NOT NULL REFERENCES application_events(id),
 transaction_id bigint NOT NULL DEFAULT txid_current(),
 created_at timestamptz NOT NULL DEFAULT now()
);

-- Same stable predicate as 111: no missing112 table on genuine111 code paths.
CREATE OR REPLACE FUNCTION owned_birth_is_live(p_volume uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public AS $$
 SELECT NOT EXISTS(SELECT 1 FROM volume_owned_birth_retirements WHERE volume_id=p_volume)
$$;
REVOKE ALL ON FUNCTION owned_birth_is_live(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION owned_birth_is_live(uuid) TO hephaestus_app,hephaestus_worker;

CREATE FUNCTION guard_owned_observation_head() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
 IF TG_OP='DELETE' OR COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_worker'
    OR EXISTS(SELECT 1 FROM volume_owned_birth_retirements WHERE volume_id=NEW.volume_id)
    OR (TG_OP='INSERT' AND NEW.version<>1)
    OR (TG_OP='UPDATE' AND (NEW.volume_id<>OLD.volume_id OR NEW.version<>OLD.version+1)) THEN
   RAISE EXCEPTION 'observation head needs exact worker monotonic progress' USING ERRCODE='23000';
 END IF;
 IF NEW.backing_observation_id IS NOT NULL AND NOT EXISTS(
    SELECT 1 FROM volume_owned_backing_observations observation
    JOIN volume_owned_transition_proofs transition ON transition.id=observation.id
    WHERE observation.id=NEW.backing_observation_id AND observation.volume_id=NEW.volume_id
      AND observation.phase=NEW.phase AND transition.transaction_id=txid_current()) THEN
   RAISE EXCEPTION 'observation head does not name exact immutable observation' USING ERRCODE='23000';
 END IF;
 IF NEW.partial_receipt_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM volume_owned_partial_receipts receipt
    WHERE receipt.id=NEW.partial_receipt_id AND receipt.volume_id=NEW.volume_id
      AND receipt.observation_version=NEW.version AND receipt.phase=NEW.phase
      AND receipt.transaction_id=txid_current()) THEN
   RAISE EXCEPTION 'partial head requires same-transaction worker receipt' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER owned_observation_head_guard BEFORE INSERT OR UPDATE OR DELETE ON volume_owned_observation_heads
 FOR EACH ROW EXECUTE FUNCTION guard_owned_observation_head();

CREATE FUNCTION advance_owned_observation_head() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE current_version bigint;
BEGIN
 -- The canonical volume row serializes 111 progress, partial recording and
 -- retirement. Explicit INSERT/UPDATE avoids UPSERT's preliminary INSERT
 -- trigger evaluating a positive-version replacement as a new version1 head.
 PERFORM 1 FROM agent_instance_state_volumes WHERE id=NEW.volume_id FOR UPDATE;
 SELECT version INTO current_version FROM volume_owned_observation_heads WHERE volume_id=NEW.volume_id FOR UPDATE;
 IF TG_TABLE_NAME='volume_owned_backing_observations' THEN
   IF current_version IS NULL THEN
     INSERT INTO volume_owned_observation_heads(volume_id,version,backing_observation_id,phase)
     VALUES(NEW.volume_id,1,NEW.id,NEW.phase);
   ELSE
     UPDATE volume_owned_observation_heads SET version=current_version+1,
        backing_observation_id=NEW.id,partial_receipt_id=NULL,phase=NEW.phase
     WHERE volume_id=NEW.volume_id AND version=current_version;
   END IF;
 ELSE
   IF COALESCE(current_version,0)<>NEW.previous_observation_version THEN
     RAISE EXCEPTION 'partial receipt head changed' USING ERRCODE='40001';
   END IF;
   IF current_version IS NULL THEN
     INSERT INTO volume_owned_observation_heads(volume_id,version,partial_receipt_id,phase)
     VALUES(NEW.volume_id,NEW.observation_version,NEW.id,NEW.phase);
   ELSE
     UPDATE volume_owned_observation_heads SET version=NEW.observation_version,
        backing_observation_id=NULL,partial_receipt_id=NEW.id,phase=NEW.phase
     WHERE volume_id=NEW.volume_id AND version=current_version;
   END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER owned_observation_head_111 AFTER INSERT ON volume_owned_backing_observations
 FOR EACH ROW EXECUTE FUNCTION advance_owned_observation_head();
CREATE TRIGGER owned_observation_head_partial AFTER INSERT ON volume_owned_partial_receipts
 FOR EACH ROW EXECUTE FUNCTION advance_owned_observation_head();

CREATE FUNCTION guard_owned_partial_receipt() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE volume agent_instance_state_volumes%ROWTYPE; purpose volume_owned_backing_purposes%ROWTYPE;
 seal volume_creation_seals%ROWTYPE; head volume_owned_observation_heads%ROWTYPE; proof jsonb; original jsonb;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'partial evidence is immutable' USING ERRCODE='23000'; END IF;
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_worker' THEN
   RAISE EXCEPTION 'only trusted worker records physical partial evidence' USING ERRCODE='42501';
 END IF;
 SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=NEW.volume_id FOR UPDATE;
 SELECT * INTO purpose FROM volume_owned_backing_purposes WHERE volume_id=NEW.volume_id;
 SELECT * INTO seal FROM volume_creation_seals WHERE id=NEW.seal_id;
 SELECT * INTO head FROM volume_owned_observation_heads WHERE volume_id=NEW.volume_id;
 proof:=NEW.proof_json;
 original:=jsonb_build_object('scope_id',seal.scope_id,'operation_id',seal.operation_id,
    'command_id',seal.command_id,'effect_id',seal.effect_id,'attempt_id',seal.attempt_id,
    'actor_id',seal.actor_id,'request_id',seal.origin_request_id,
    'resource_version',seal.resource_version,'generation',seal.generation,
    'input_hash',recipe_effect_hash_json(seal.input_hash),'intent_hash',recipe_effect_hash_json(seal.intent_hash));
 IF NEW.transaction_id<>txid_current() OR volume.id IS NULL OR purpose.volume_id IS NULL OR seal.id IS NULL
    OR volume.creation_seal_id<>NEW.seal_id OR volume.project_id<>NEW.project_id
    OR volume.state<>'uninitialized' OR volume.provisioning_state='ready'
    OR volume.provisioning_generation<>NEW.generation OR purpose.purpose_hash<>NEW.purpose_hash
    OR EXISTS(SELECT 1 FROM volume_owned_birth_retirements WHERE volume_id=NEW.volume_id)
    OR COALESCE(head.version,0)<>NEW.previous_observation_version
    OR COALESCE(head.backing_observation_id,head.partial_receipt_id) IS DISTINCT FROM NEW.previous_observation_id
    OR EXISTS(SELECT 1 FROM volume_owned_backing_observations WHERE volume_id=NEW.volume_id AND phase='ready')
    OR NOT EXISTS(SELECT 1 FROM volume_owned_provisioning_operations WHERE id=NEW.operation_id
       AND volume_id=NEW.volume_id AND generation=NEW.generation AND purpose_hash=NEW.purpose_hash)
    OR proof->>'id' IS DISTINCT FROM NEW.id::text
    OR (proof->>'observation_version')::bigint IS DISTINCT FROM NEW.observation_version
    OR proof->>'volume_id' IS DISTINCT FROM NEW.volume_id::text
    OR proof->>'project_id' IS DISTINCT FROM NEW.project_id::text
    OR proof->>'seal_id' IS DISTINCT FROM NEW.seal_id::text
    OR proof->>'provisioning_operation_id' IS DISTINCT FROM NEW.operation_id::text
    OR proof->'original_creation' IS DISTINCT FROM original
    OR proof->'purpose_hash' IS DISTINCT FROM recipe_effect_hash_json(NEW.purpose_hash)
    OR proof->>'phase' IS DISTINCT FROM NEW.phase
    OR proof->>'owner_namespace' IS DISTINCT FROM purpose.owner_namespace::text
    OR (proof->>'generation')::bigint IS DISTINCT FROM NEW.generation
    OR (proof->>'declared_capacity_bytes')::bigint IS DISTINCT FROM seal.capacity_bytes
    OR NEW.declared_capacity_bytes<>seal.capacity_bytes
    OR (proof->>'actual_length_bytes')::bigint IS DISTINCT FROM NEW.actual_length_bytes
    OR NOT EXISTS(SELECT 1 FROM application_events event WHERE event.id=NEW.event_id
       AND event.occurrence_id=NEW.id AND event.scope_kind='project' AND event.scope_id=NEW.project_id
       AND event.aggregate_type='project' AND event.aggregate_id=NEW.project_id AND event.event_type='project.changed') THEN
   RAISE EXCEPTION 'partial receipt differs from exact birth/admission/current head' USING ERRCODE='23000';
 END IF;
 IF NOT(proof ?& ARRAY['root_device','root_inode','root_owner_uid','namespace_device','namespace_inode',
    'owner_uid','backing_device','backing_inode','birth_record_hash','inode_record_hash','journal_hash'])
    OR (SELECT count(*) FROM jsonb_object_keys(proof))<>24
    OR EXISTS(SELECT 1 FROM unnest(ARRAY['root_device','root_inode','namespace_device','namespace_inode','backing_device','backing_inode','owner_uid','root_owner_uid']) field
       WHERE jsonb_typeof(proof->field)<>'number' OR proof->>field !~ '^[0-9]+$' OR (proof->>field)::numeric>9223372036854775807)
    OR EXISTS(SELECT 1 FROM unnest(ARRAY['root_inode','namespace_inode','backing_inode']) field WHERE (proof->>field)::bigint<=0)
    OR (proof->>'owner_uid')::bigint>4294967295 OR (proof->>'root_owner_uid')::bigint>4294967295
    OR EXISTS(SELECT 1 FROM unnest(ARRAY['birth_record_hash','inode_record_hash','journal_hash']) field
       WHERE jsonb_typeof(proof->field)<>'array' OR jsonb_array_length(proof->field)<>32)
    OR EXISTS(SELECT 1 FROM jsonb_array_elements((proof->'birth_record_hash')||(proof->'inode_record_hash')||(proof->'journal_hash')) byte
       WHERE jsonb_typeof(byte)<>'number' OR byte::text !~ '^[0-9]+$' OR byte::text::integer NOT BETWEEN 0 AND 255)
    OR EXISTS(SELECT 1 FROM volume_owned_backing_observations observation WHERE observation.volume_id=NEW.volume_id
       AND owned_backing_identity(observation.proof_json) IS DISTINCT FROM owned_backing_identity(proof))
    OR EXISTS(SELECT 1 FROM volume_owned_partial_receipts previous WHERE previous.volume_id=NEW.volume_id
       AND (owned_backing_identity(previous.proof_json) IS DISTINCT FROM owned_backing_identity(proof)
         OR previous.proof_json->'root_owner_uid' IS DISTINCT FROM proof->'root_owner_uid'))
    OR (NEW.phase IN ('claimed_empty','allocated_never_format') AND EXISTS(
       SELECT 1 FROM volume_owned_backing_observations WHERE volume_id=NEW.volume_id AND phase IN ('format_intent','ready'))) THEN
   RAISE EXCEPTION 'partial observation requires complete contradiction-free positive birth' USING ERRCODE='23000';
 END IF;
 -- These are comparison checks. The actual worker still must acquire/retain the
 -- opaque native witness; no JSON field may claim a lock or filesystem Ready.
 RETURN NEW;
END $$;
CREATE TRIGGER owned_partial_receipt_guard BEFORE INSERT OR UPDATE OR DELETE ON volume_owned_partial_receipts
 FOR EACH ROW EXECUTE FUNCTION guard_owned_partial_receipt();

-- No expiry establishes lease release or physical drain. Every global lease,
-- including unrelated/historical_unproven/recovery leases, must be released.
-- Every consumer of any historic binding/scalar pointer must be permanently
-- closed AND each of its runs must have exact canonical cleanup completion.
CREATE FUNCTION require_partial_birth_quiescence(p_volume uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE consumer uuid;
BEGIN
 FOR consumer IN SELECT instance.id FROM agent_instances instance WHERE instance.state_volume_id=p_volume
    OR EXISTS(SELECT 1 FROM agent_instance_revision_volume_bindings binding WHERE binding.volume_id=p_volume AND binding.instance_id=instance.id)
    OR EXISTS(SELECT 1 FROM agent_instance_volume_leases lease WHERE lease.volume_id=p_volume AND lease.instance_id=instance.id)
    ORDER BY instance.id LOOP
   PERFORM 1 FROM agent_instances WHERE id=consumer FOR UPDATE;
   PERFORM 1 FROM runs WHERE instance_id=consumer ORDER BY id FOR UPDATE;
 END LOOP;
 PERFORM 1 FROM agent_instance_state_volumes WHERE id=p_volume FOR UPDATE;
 IF EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE volume_id=p_volume AND released_at IS NULL)
    OR EXISTS(SELECT 1 FROM agent_instances instance WHERE
      (instance.state_volume_id=p_volume
        OR EXISTS(SELECT 1 FROM agent_instance_revision_volume_bindings binding WHERE binding.volume_id=p_volume AND binding.instance_id=instance.id)
        OR EXISTS(SELECT 1 FROM agent_instance_volume_leases lease WHERE lease.volume_id=p_volume AND lease.instance_id=instance.id))
      AND (instance.run_gate_open OR NOT EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=instance.id)
        OR EXISTS(SELECT 1 FROM mailboxes WHERE instance_id=instance.id AND state='active')
        OR EXISTS(SELECT 1 FROM mailbox_deliveries WHERE instance_id=instance.id AND terminal_at IS NULL)
        OR EXISTS(SELECT 1 FROM run_requests WHERE instance_id=instance.id AND dispatch_state='pending')
        OR EXISTS(SELECT 1 FROM deferred_agent_triggers WHERE instance_id=instance.id AND state='deferred')
        OR EXISTS(SELECT 1 FROM agent_updates WHERE instance_id=instance.id AND state NOT IN ('activated','rejected'))
        OR EXISTS(SELECT 1 FROM runs execution WHERE execution.instance_id=instance.id
          AND (execution.state<>'cleaned_up' OR execution.volume_acquisition_closed_at IS NULL
            OR NOT EXISTS(SELECT 1 FROM run_cleanup_completions completion WHERE completion.run_id=execution.id))))) THEN
   RAISE EXCEPTION 'partial retain requires all global leases released and all consumers closed/drained' USING ERRCODE='23000';
 END IF;
END $$;
REVOKE ALL ON FUNCTION require_partial_birth_quiescence(uuid) FROM PUBLIC;

-- Called ONLY inside the recipe actor transaction which already owns current
-- Remove and exact immutable old Create. Deferred proof forbids standalone seal.
CREATE FUNCTION retire_owned_partial_birth(p_receipt uuid, t recipe_execution_transitions) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE receipt volume_owned_partial_receipts%ROWTYPE; head volume_owned_observation_heads%ROWTYPE;
 volume agent_instance_state_volumes%ROWTYPE; original volume_creation_seals%ROWTYPE;
 existing volume_owned_birth_retirements%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
BEGIN
 SELECT * INTO receipt FROM volume_owned_partial_receipts WHERE id=p_receipt;
 IF receipt.id IS NULL OR COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app'
    OR t.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
    OR t.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
    OR check_permission('user',hephaestus_actor_id(),'can_manage','project',receipt.project_id::text) IS DISTINCT FROM 1
    OR check_permission('user',hephaestus_actor_id(),'can_manage','state_volume',receipt.volume_id::text) IS DISTINCT FROM 1 THEN
   RAISE EXCEPTION 'current project and exact resource management required before replay' USING ERRCODE='42501';
 END IF;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=t.command_id;
 IF command.actor_id IS DISTINCT FROM t.actor_id OR command.operation<>'remove'
    OR command.project_id IS DISTINCT FROM receipt.project_id OR t.project_id IS DISTINCT FROM receipt.project_id THEN
   RAISE EXCEPTION 'current admitted removal required; historical command cannot be borrowed' USING ERRCODE='23000';
 END IF;
 SELECT * INTO existing FROM volume_owned_birth_retirements WHERE volume_id=receipt.volume_id;
 IF existing.volume_id IS NOT NULL THEN
   IF (existing.receipt_id,existing.transition_id,existing.command_id,existing.operation_id,existing.actor_id,existing.fingerprint)
      IS DISTINCT FROM (p_receipt,t.id,t.command_id,t.operation_id,t.actor_id,t.fingerprint) THEN
     RAISE EXCEPTION 'birth retirement input conflicts' USING ERRCODE='23000';
   END IF;
   RETURN;
 END IF;
 PERFORM require_partial_birth_quiescence(receipt.volume_id);
 SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=receipt.volume_id;
 SELECT * INTO head FROM volume_owned_observation_heads WHERE volume_id=receipt.volume_id;
 SELECT * INTO original FROM volume_creation_seals WHERE id=receipt.seal_id;
 IF check_permission('user',hephaestus_actor_id(),'can_manage','project',receipt.project_id::text) IS DISTINCT FROM 1
    OR check_permission('user',hephaestus_actor_id(),'can_manage','state_volume',receipt.volume_id::text) IS DISTINCT FROM 1 THEN
   RAISE EXCEPTION 'manager authority changed during quiescence wait' USING ERRCODE='42501';
 END IF;
 IF volume.creation_seal_id IS DISTINCT FROM receipt.seal_id OR volume.project_id IS DISTINCT FROM receipt.project_id
    OR volume.state<>'uninitialized' OR volume.provisioning_state='ready'
    OR volume.provisioning_generation<>receipt.generation
    OR (head.partial_receipt_id,head.version,head.phase) IS DISTINCT FROM (receipt.id,receipt.observation_version,receipt.phase)
    OR original.attempt_id IS DISTINCT FROM t.attempt_id THEN
   RAISE EXCEPTION 'partial receipt is stale or names different current sealed birth' USING ERRCODE='40001';
 END IF;
 INSERT INTO volume_owned_birth_retirements(volume_id,receipt_id,transition_id,command_id,operation_id,
    actor_id,request_id,project_id,fingerprint,event_id)
 VALUES(receipt.volume_id,receipt.id,t.id,t.command_id,t.operation_id,t.actor_id,t.request_id,t.project_id,t.fingerprint,t.event_id);
END $$;
REVOKE ALL ON FUNCTION retire_owned_partial_birth(uuid,recipe_execution_transitions) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION retire_owned_partial_birth(uuid,recipe_execution_transitions) TO hephaestus_app;

CREATE FUNCTION verify_partial_birth_retirement_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
 IF NEW.transaction_id<>txid_current() OR NOT EXISTS(
    SELECT 1 FROM recipe_execution_transitions t JOIN recipe_deployment_resources r
      ON r.deployment_id=t.deployment_id AND r.resource_name=t.resource_name
    JOIN recipe_deployments d ON d.id=t.deployment_id
    WHERE t.id=NEW.transition_id AND t.kind='creation_retained_partial'
      AND t.command_id=NEW.command_id AND t.operation_id=NEW.operation_id
      AND t.actor_id=NEW.actor_id AND t.request_id=NEW.request_id AND t.fingerprint=NEW.fingerprint
      AND t.event_id=NEW.event_id AND t.project_id=NEW.project_id
      AND r.resource_id=NEW.volume_id AND r.active_attempt_id IS NULL AND r.removal_progress='retained'
      AND recipe_effect_resource_state(r)=t.after_state AND d.version=t.after_deployment_version)
   OR NOT EXISTS(SELECT 1 FROM authorization_audit_events audit WHERE audit.actor_id=NEW.actor_id
      AND audit.request_id=NEW.request_id AND audit.object_type='project' AND audit.object_id=NEW.project_id
      AND audit.permission='can_manage' AND audit.decision='allow')
   OR NOT EXISTS(SELECT 1 FROM authorization_audit_events audit WHERE audit.actor_id=NEW.actor_id
      AND audit.request_id=NEW.request_id AND audit.object_type='state_volume' AND audit.object_id=NEW.volume_id
      AND audit.permission='can_manage' AND audit.decision='allow') THEN
   RAISE EXCEPTION 'birth fence must commit with exact removal transition, state, audit and event' USING ERRCODE='23000';
 END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER owned_partial_retirement_commit AFTER INSERT ON volume_owned_birth_retirements
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_partial_birth_retirement_commit();

-- Immutable birth retirement is enforced independently of the recipe service.
-- Actor begin/replay must additionally SELECT this fence before returning a
-- historical 111 operation (SQL INSERT guards cannot intercept a read-only replay).
CREATE FUNCTION reject_retired_owned_birth_progress() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE target uuid;
BEGIN
 IF TG_TABLE_NAME='agent_instance_state_volumes' THEN
   target:=OLD.id;
 ELSE
   target:=NEW.volume_id;
 END IF;
 PERFORM 1 FROM agent_instance_state_volumes WHERE id=target FOR UPDATE;
 IF EXISTS(SELECT 1 FROM volume_owned_birth_retirements WHERE volume_id=target) THEN
   RAISE EXCEPTION 'partial owned birth is permanently retired and unusable' USING ERRCODE='23000';
 END IF;
 RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END $$;
CREATE TRIGGER retired_owned_operation BEFORE INSERT ON volume_owned_provisioning_operations FOR EACH ROW EXECUTE FUNCTION reject_retired_owned_birth_progress();
CREATE TRIGGER retired_owned_transition BEFORE INSERT ON volume_owned_transition_proofs FOR EACH ROW EXECUTE FUNCTION reject_retired_owned_birth_progress();
CREATE TRIGGER retired_owned_observation BEFORE INSERT ON volume_owned_backing_observations FOR EACH ROW EXECUTE FUNCTION reject_retired_owned_birth_progress();
CREATE TRIGGER retired_owned_partial_observation BEFORE INSERT ON volume_owned_partial_receipts FOR EACH ROW EXECUTE FUNCTION reject_retired_owned_birth_progress();
CREATE TRIGGER retired_owned_resource BEFORE UPDATE OR DELETE ON agent_instance_state_volumes FOR EACH ROW EXECUTE FUNCTION reject_retired_owned_birth_progress();
CREATE TRIGGER retired_owned_lease BEFORE INSERT OR UPDATE ON agent_instance_volume_leases FOR EACH ROW EXECUTE FUNCTION reject_retired_owned_birth_progress();
CREATE TRIGGER retired_owned_binding BEFORE INSERT OR UPDATE ON agent_instance_revision_volume_bindings FOR EACH ROW EXECUTE FUNCTION reject_retired_owned_birth_progress();

ALTER TABLE volume_owned_partial_receipts ENABLE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_partial_receipts FORCE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_observation_heads ENABLE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_observation_heads FORCE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_birth_retirements ENABLE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_birth_retirements FORCE ROW LEVEL SECURITY;
GRANT SELECT,INSERT ON volume_owned_partial_receipts TO hephaestus_worker;
GRANT SELECT,INSERT,UPDATE ON volume_owned_observation_heads TO hephaestus_worker;
GRANT SELECT ON volume_owned_partial_receipts,volume_owned_observation_heads,volume_owned_birth_retirements TO hephaestus_app;
GRANT SELECT ON volume_owned_birth_retirements TO hephaestus_worker;
CREATE POLICY partial_receipt_worker ON volume_owned_partial_receipts TO hephaestus_worker USING(true) WITH CHECK(true);
CREATE POLICY physical_head_worker ON volume_owned_observation_heads TO hephaestus_worker USING(true) WITH CHECK(true);
CREATE POLICY partial_retirement_worker ON volume_owned_birth_retirements FOR SELECT TO hephaestus_worker USING(true);
CREATE POLICY partial_receipt_actor ON volume_owned_partial_receipts FOR SELECT TO hephaestus_app
 USING(check_permission('user',hephaestus_actor_id(),'can_manage','project',project_id::text)=1
   AND check_permission('user',hephaestus_actor_id(),'can_manage','state_volume',volume_id::text)=1);
CREATE POLICY physical_head_actor ON volume_owned_observation_heads FOR SELECT TO hephaestus_app
 USING(check_permission('user',hephaestus_actor_id(),'can_manage','state_volume',volume_id::text)=1);
CREATE POLICY partial_retirement_actor ON volume_owned_birth_retirements FOR SELECT TO hephaestus_app
 USING(check_permission('user',hephaestus_actor_id(),'can_manage','project',project_id::text)=1
   AND check_permission('user',hephaestus_actor_id(),'can_manage','state_volume',volume_id::text)=1);
CREATE TRIGGER partial_retirement_immutable BEFORE UPDATE OR DELETE ON volume_owned_birth_retirements
 FOR EACH ROW EXECUTE FUNCTION recipe_ledger_immutable();
-- No app INSERT grant on retirement table; only exact checked helper may write.
-- Following fragment adds the narrow truthful recipe transition, keeping all
-- 109 branches verbatim. New actor port/service is intentionally not provided.

ALTER TABLE recipe_execution_transitions ADD COLUMN partial_retention_receipt_id uuid
 REFERENCES volume_owned_partial_receipts(id);
ALTER TABLE recipe_execution_transitions DROP CONSTRAINT recipe_execution_transitions_kind_check;
ALTER TABLE recipe_execution_transitions ADD CONSTRAINT recipe_execution_transitions_kind_check
 CHECK(kind IN ('claimed','completed','ambiguous','reconciled','finished','observation_retired','creation_retained_partial'));
CREATE OR REPLACE FUNCTION recipe_execution_transition_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
DECLARE command recipe_deployment_commands%ROWTYPE; deployment recipe_deployments%ROWTYPE; resource recipe_deployment_resources%ROWTYPE;
    attempt recipe_effect_attempts%ROWTYPE; proof recipe_effect_verifications%ROWTYPE; expected jsonb; outcome text; diagnostic text;
    expected_lifecycle text; uncertain boolean; partial volume_owned_partial_receipts%ROWTYPE; seal volume_creation_seals%ROWTYPE;
BEGIN
 IF (NEW.kind='creation_retained_partial') IS DISTINCT FROM (NEW.partial_retention_receipt_id IS NOT NULL) THEN
   RAISE EXCEPTION 'partial receipt belongs only to truthful partial retirement' USING ERRCODE='23000';
 END IF;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=NEW.command_id;
 SELECT * INTO deployment FROM recipe_deployments WHERE id=NEW.deployment_id;
 IF NOT recipe_effect_authorized(command)
    OR NEW.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
    OR NEW.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
    OR NEW.before_deployment_version<>deployment.version OR NEW.before_lifecycle<>deployment.lifecycle
    OR NOT EXISTS(SELECT 1 FROM application_events event WHERE event.id=NEW.event_id
       AND event.occurrence_id=NEW.id AND event.scope_kind='project' AND event.scope_id=NEW.project_id
       AND event.aggregate_type='project' AND event.aggregate_id=NEW.project_id AND event.event_type='project.changed'
       AND event.actor_id=NEW.actor_id AND event.request_id=NEW.request_id
       AND event.cursor=NEW.event_cursor AND event.aggregate_version=NEW.event_aggregate_version)
    OR NOT EXISTS(SELECT 1 FROM authorization_audit_events audit WHERE audit.actor_id=NEW.actor_id
       AND audit.request_id=NEW.request_id AND audit.object_type='project' AND audit.object_id=NEW.project_id
       AND audit.permission='can_manage' AND audit.decision='allow') THEN
    RAISE EXCEPTION 'execution transition requires exact authority, audit, CAS, and committed event' USING ERRCODE='integrity_constraint_violation';
 END IF;
 IF NEW.kind='finished' THEN
    expected_lifecycle:=CASE WHEN command.operation='install' THEN 'installed' ELSE 'removed' END;
    IF NEW.operation_id<>command.id OR NEW.verification_id IS NOT NULL
       OR (command.operation='install' AND recipe_cleanup_started(NEW.deployment_id))
       OR (command.operation='remove' AND NOT recipe_cleanup_started(NEW.deployment_id))
       OR EXISTS(SELECT 1 FROM recipe_deployment_resources WHERE deployment_id=NEW.deployment_id
           AND (active_attempt_id IS NOT NULL OR (command.operation='install' AND install_progress<>'ready')
               OR (command.operation='remove' AND removal_progress NOT IN ('retained','deleted'))))
       OR NEW.after_lifecycle<>expected_lifecycle THEN
       RAISE EXCEPTION 'terminal command resources are incomplete' USING ERRCODE='integrity_constraint_violation';
    END IF;
    RETURN NEW;
 END IF;
 SELECT * INTO attempt FROM recipe_effect_attempts WHERE id=NEW.attempt_id;
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=NEW.deployment_id AND resource_name=NEW.resource_name;
 IF NEW.kind='creation_retained_partial' THEN
    -- Lock/recheck ledger CAS before calling generic volume boundary. No source
    -- CanUse/original creator role is needed for current manager cleanup.
    SELECT * INTO deployment FROM recipe_deployments WHERE id=NEW.deployment_id FOR UPDATE;
    SELECT * INTO resource FROM recipe_deployment_resources
       WHERE deployment_id=NEW.deployment_id AND resource_name=NEW.resource_name FOR UPDATE;
    SELECT * INTO partial FROM volume_owned_partial_receipts WHERE id=NEW.partial_retention_receipt_id;
    SELECT * INTO seal FROM volume_creation_seals WHERE id=partial.seal_id;
    IF command.operation<>'remove' OR NOT recipe_cleanup_started(NEW.deployment_id)
       OR deployment.lifecycle NOT IN ('removing','recovery_required')
       OR NEW.before_deployment_version<>deployment.version OR NEW.before_lifecycle<>deployment.lifecycle
       OR resource.ownership<>'owned' OR resource.resource_kind<>'volume' OR resource.removal_policy<>'retain'
       OR resource.install_progress='ready' OR resource.removal_progress IN ('retained','deleted')
       OR attempt.action<>'create' OR NOT EXISTS(SELECT 1 FROM recipe_deployment_commands original
          WHERE original.id=attempt.command_id AND original.operation='install')
       OR attempt.deployment_id IS DISTINCT FROM NEW.deployment_id
       OR attempt.resource_name IS DISTINCT FROM NEW.resource_name
       OR NEW.before_state IS DISTINCT FROM recipe_effect_resource_state(resource)
       OR resource.active_attempt_id IS DISTINCT FROM attempt.id OR resource.execution_generation<>attempt.generation
       OR resource.input_hash<>attempt.input_hash OR resource.plan_json->'identity' IS DISTINCT FROM attempt.claim_json->'identity'
       OR partial.id IS NULL OR partial.volume_id IS DISTINCT FROM resource.resource_id
       OR partial.project_id IS DISTINCT FROM resource.project_id
       OR seal.scope_id IS DISTINCT FROM deployment.id OR seal.command_id IS DISTINCT FROM attempt.command_id
       OR seal.attempt_id IS DISTINCT FROM attempt.id OR seal.actor_id IS DISTINCT FROM attempt.actor_id
       OR seal.origin_request_id IS DISTINCT FROM attempt.request_id OR seal.generation<>attempt.generation
       OR seal.resource_version<>(attempt.claim_json->>'resource_version')::bigint
       OR seal.input_hash<>attempt.input_hash OR seal.intent_hash<>deployment.input_hash
       OR seal.filesystem_uuid IS DISTINCT FROM resource.filesystem_uuid
       OR NEW.verification_id IS NOT NULL OR NEW.operation_id=attempt.id
       OR EXISTS(SELECT 1 FROM recipe_effect_attempts WHERE id=NEW.operation_id)
       OR EXISTS(SELECT 1 FROM recipe_effect_verifications WHERE verification_attempt_id=NEW.operation_id)
       OR EXISTS(SELECT 1 FROM recipe_execution_transitions WHERE operation_id=NEW.operation_id)
       OR EXISTS(SELECT 1 FROM recipe_deployment_resources dependent
          WHERE dependent.deployment_id=NEW.deployment_id
            AND dependent.plan_json->'intent'->'dependencies' ? NEW.resource_name
            AND (dependent.active_attempt_id IS NOT NULL OR dependent.removal_progress NOT IN ('retained','deleted'))) THEN
       RAISE EXCEPTION 'partial retain needs exact original owned Create and reverse-terminal cleanup' USING ERRCODE='23000';
    END IF;
    -- Preserve original install, diagnostic, creation generation, last action.
    -- Retained here means permanently unusable retained birth, never Detach or Ready.
    expected:=NEW.before_state||jsonb_build_object('active_attempt',NULL,'removal','retained','version',resource.version+1);
    IF NEW.after_state IS DISTINCT FROM expected THEN
       RAISE EXCEPTION 'partial retention cannot fabricate installation/absence/verification facts' USING ERRCODE='23000';
    END IF;
    PERFORM retire_owned_partial_birth(partial.id,NEW);
 ELSIF NEW.kind='observation_retired' THEN
    IF command.operation<>'remove' OR NOT recipe_cleanup_started(NEW.deployment_id)
       OR deployment.lifecycle NOT IN ('removing','recovery_required')
       OR resource.ownership<>'external' OR resource.resource_kind<>'volume'
       OR resource.removal_progress<>'retained' OR attempt.action<>'verify_external'
       OR NOT EXISTS(SELECT 1 FROM recipe_deployment_commands original
          WHERE original.id=attempt.command_id AND original.operation='install')
       OR attempt.deployment_id IS DISTINCT FROM NEW.deployment_id
       OR attempt.resource_name IS DISTINCT FROM NEW.resource_name
       OR NEW.before_state IS DISTINCT FROM recipe_effect_resource_state(resource)
       OR resource.active_attempt_id IS DISTINCT FROM attempt.id
       OR resource.execution_generation<>attempt.generation
       OR resource.input_hash<>attempt.input_hash
       OR resource.plan_json->'identity' IS DISTINCT FROM attempt.claim_json->'identity'
       OR NEW.operation_id=attempt.id OR NEW.verification_id IS NOT NULL
       OR EXISTS(SELECT 1 FROM recipe_effect_attempts WHERE id=NEW.operation_id)
       OR EXISTS(SELECT 1 FROM recipe_effect_verifications WHERE verification_attempt_id=NEW.operation_id)
       OR EXISTS(SELECT 1 FROM recipe_execution_transitions WHERE operation_id=NEW.operation_id) THEN
       RAISE EXCEPTION 'retirement requires exact external read-only observation and current cleanup' USING ERRCODE='integrity_constraint_violation';
    END IF;
    expected:=NEW.before_state||jsonb_build_object('active_attempt',NULL,'version',resource.version+1);
    IF NEW.after_state IS DISTINCT FROM expected THEN
       RAISE EXCEPTION 'retirement only clears the active observation and advances CAS' USING ERRCODE='integrity_constraint_violation';
    END IF;
 ELSE
 IF (NEW.kind<>'reconciled' AND attempt.command_id IS DISTINCT FROM NEW.command_id)
    OR (NEW.kind='reconciled' AND attempt.command_id IS DISTINCT FROM NEW.command_id AND NOT (
       command.operation='remove' AND recipe_cleanup_started(NEW.deployment_id)
       AND deployment.lifecycle IN ('removing','recovery_required') AND resource.ownership='owned'
       AND EXISTS(SELECT 1 FROM recipe_deployment_commands original WHERE original.id=attempt.command_id AND original.operation='install')))
    OR attempt.deployment_id IS DISTINCT FROM NEW.deployment_id
    OR attempt.resource_name IS DISTINCT FROM NEW.resource_name
    OR NEW.before_state IS DISTINCT FROM recipe_effect_resource_state(resource)
    OR (NEW.kind<>'claimed' AND (resource.active_attempt_id IS DISTINCT FROM attempt.id OR resource.execution_generation<>attempt.generation))
    OR (NEW.kind IN ('completed','ambiguous') AND resource.version<>(attempt.claim_json->>'resource_version')::bigint)
    OR (NEW.kind='claimed' AND (NEW.operation_id<>attempt.id OR NEW.fingerprint<>attempt.request_fingerprint OR resource.active_attempt_id IS NOT NULL))
    OR (NEW.kind IN ('completed','ambiguous') AND NEW.operation_id<>attempt.id)
    OR (NEW.kind='reconciled' AND (NEW.operation_id=attempt.id OR EXISTS(SELECT 1 FROM recipe_effect_attempts WHERE id=NEW.operation_id))) THEN
    RAISE EXCEPTION 'execution differs from active immutable attempt' USING ERRCODE='integrity_constraint_violation';
 END IF;
 IF NEW.verification_id IS NOT NULL THEN
    SELECT * INTO proof FROM recipe_effect_verifications WHERE id=NEW.verification_id;
    IF proof.attempt_id IS DISTINCT FROM attempt.id OR proof.verification_attempt_id<>NEW.operation_id
       OR proof.generation<>attempt.generation OR proof.action<>attempt.action
       OR proof.input_hash<>attempt.input_hash OR proof.identity_json IS DISTINCT FROM attempt.claim_json->'identity'
       OR (NEW.kind='completed' AND proof.outcome NOT IN ('applied','failed'))
       OR (NEW.kind='reconciled' AND proof.outcome NOT IN ('applied','not_applied'))
       OR NEW.kind NOT IN ('completed','reconciled') THEN
       RAISE EXCEPTION 'execution proof does not match its exact operation' USING ERRCODE='integrity_constraint_violation';
    END IF;
    outcome:=proof.outcome; diagnostic:=proof.diagnostic;
 ELSIF NEW.kind='completed' THEN
    RAISE EXCEPTION 'caller evidence is not trusted verification' USING ERRCODE='integrity_constraint_violation';
 ELSE
    outcome:='unproven'; diagnostic:=NEW.after_state->>'diagnostic';
    IF NEW.kind<>'claimed' AND diagnostic IS NULL THEN
       RAISE EXCEPTION 'unproven outcome needs a safe diagnostic' USING ERRCODE='integrity_constraint_violation';
    END IF;
 END IF;
 expected:=recipe_effect_expected_state(NEW.before_state,attempt.before_state,attempt.action,NEW.kind,outcome,diagnostic,attempt.id);
 IF NEW.after_state IS DISTINCT FROM expected THEN
    RAISE EXCEPTION 'illegal closed effect state transition' USING ERRCODE='integrity_constraint_violation';
 END IF;
 END IF;
 uncertain:=(expected->>'active_attempt' IS NOT NULL AND
    ((expected->>'install') IN ('failed','recovery_required') OR expected->>'removal'='recovery_required'))
    OR EXISTS(SELECT 1 FROM recipe_deployment_resources WHERE deployment_id=NEW.deployment_id AND resource_name<>NEW.resource_name
       AND active_attempt_id IS NOT NULL AND (install_progress IN ('failed','recovery_required') OR removal_progress='recovery_required'));
 expected_lifecycle:=CASE WHEN uncertain THEN 'recovery_required' WHEN deployment.lifecycle='recovery_required' THEN
    (CASE WHEN recipe_cleanup_started(NEW.deployment_id) THEN 'removing' ELSE 'installing' END) ELSE deployment.lifecycle END;
 IF NEW.after_lifecycle IS DISTINCT FROM expected_lifecycle THEN
    RAISE EXCEPTION 'execution lifecycle differs from resource evidence' USING ERRCODE='integrity_constraint_violation';
 END IF;
 RETURN NEW;
END $$;

-- Future recipe owning-service requirements (not satisfied by this proposal):
-- * exact freshly admitted current Remove identity + fresh middleware request;
-- * exact immutable old 106 Create and all its fields matched to 110/111 purpose;
-- * extra110 effect_id/registration-operation labels matched against an explicit
--   persisted canonical provider-operation mapping before original invocation.
--   106 has no separate effect_id field: never invent effect_id=attempt_id.
-- * actual opaque pinned Local guard remains held throughout worker record and
--   actor transaction; no public Remove input accepts worker receipt IDs;
-- * before returning any historical111 begin replay, check retirement fence;
-- * worker receipt lookup is backend-only, not a client capability/preview;
-- * exact committed result lookup remains106 recipe_command_results: successful
--   partial transition is intermediate progress, not its own terminal Remove.
--
-- Acceptance gates still REQUIRED before wiring/publication:
-- pure bounds/correlation tests; genuine111->112 preservation; worker-only/rawSQL
-- rejection; formatter-progress stale head; two concurrent retirees one winner;
-- original source/actor loss -> new manager retained partial; sameactor oldcmd
-- borrowing denied; live permission loss before replay; all unrelated held lease
-- and open/undrained consumer rejection; reverse dependency pending rejection;
-- Ready/unknown/nohostinode denied; atomic rollback if ledger/audit/event missing;
-- old creator restored cannot begin/replay/format/Ready/attach; native actual
-- witness lock exclusion and custody surviving worker-record->actor-commit gap.
