-- PRIVATE119 terminal success, AFTER exact frozen private28 load order.
-- SOURCE ONLY. Worker/Core instance.wait supplies exit; no durable attestation.
-- Historical result qualification is never execution admission or provider IO.

-- SQL transaction identity only; this marker does not attest a guest exit.
CREATE TABLE invocation119_terminal_transactions (
 run_id uuid PRIMARY KEY REFERENCES runs(id),
 success_version bigint NOT NULL CHECK(success_version>0),
 success_xid xid8 NOT NULL,
 identity jsonb NOT NULL CHECK(jsonb_typeof(identity)='object'),
 CHECK(identity->>'id' IS NOT DISTINCT FROM run_id::text),
 CHECK(identity->>'run_kind' IS NOT DISTINCT FROM 'invocation'),
 CHECK(identity->>'outcome' IS NOT DISTINCT FROM 'succeeded'),
 CHECK(identity->'exit_code' IS NOT DISTINCT FROM '0'::jsonb),
 CHECK(identity->'exit_signal' IS NOT DISTINCT FROM 'null'::jsonb),
 CHECK(identity->'failure' IS NOT DISTINCT FROM 'null'::jsonb)
);
ALTER TABLE invocation119_terminal_transactions ENABLE ROW LEVEL SECURITY;
ALTER TABLE invocation119_terminal_transactions FORCE ROW LEVEL SECURITY;
REVOKE ALL ON invocation119_terminal_transactions FROM PUBLIC,hephaestus_app,hephaestus_worker;
-- Bound at creation to the legitimate migration owner; no app/worker policy.
CREATE POLICY invocation119_terminal_owner ON invocation119_terminal_transactions
 TO CURRENT_USER USING(true) WITH CHECK(true);

CREATE FUNCTION require_invocation119_integrity_mutator() RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE actual_role text;
BEGIN
 actual_role:=CASE WHEN current_setting('role',true)='none' THEN session_user::text
  ELSE current_setting('role',true) END;
 -- Integrity-only trigger dispatch, never a read or mutation authorization API.
 IF actual_role IS NULL OR actual_role NOT IN ('hephaestus_app','hephaestus_worker',current_user::text) THEN
  RAISE EXCEPTION 'Invocation integrity requires an existing mutation principal' USING ERRCODE='42501'; END IF;
END $$;
REVOKE ALL ON FUNCTION require_invocation119_integrity_mutator() FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_terminal_marker(p_row runs) RETURNS xid8
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE marker invocation119_terminal_transactions%ROWTYPE; step integer;
BEGIN
 PERFORM require_invocation119_integrity_mutator();
 SELECT * INTO marker FROM invocation119_terminal_transactions WHERE run_id=p_row.id;
 step:=CASE p_row.state WHEN 'succeeded' THEN 0 WHEN 'cleaning_up' THEN 1 WHEN 'cleaned_up' THEN 2 ELSE -1 END;
 -- Only state/version/time/Cancel stamp vary through legitimate later cleanup.
 IF marker.run_id IS NULL OR step<0 OR p_row.state_version<marker.success_version
  OR p_row.state_version-marker.success_version IS DISTINCT FROM step::bigint
  OR marker.identity IS DISTINCT FROM (to_jsonb(p_row)-ARRAY['state','state_version','updated_at','cancel_requested_at']) THEN
  RAISE EXCEPTION 'Invocation success lacks its exact immutable transaction marker' USING ERRCODE='23000'; END IF;
 RETURN marker.success_xid;
END $$;
REVOKE ALL ON FUNCTION invocation119_terminal_marker(runs) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION guard_invocation119_terminal_marker() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
 PERFORM require_invocation117_worker();
 IF TG_OP<>'INSERT' THEN
  RAISE EXCEPTION 'Invocation success transaction marker is immutable' USING ERRCODE='23000'; END IF;
 SELECT * INTO execution FROM runs WHERE id=NEW.run_id;
 IF execution.state IS DISTINCT FROM 'succeeded' OR execution.run_kind IS DISTINCT FROM 'invocation'
  OR NEW.success_version IS DISTINCT FROM execution.state_version
  OR NEW.success_xid IS DISTINCT FROM pg_current_xact_id()
  OR NEW.identity IS DISTINCT FROM (to_jsonb(execution)-ARRAY['state','state_version','updated_at','cancel_requested_at']) THEN
  RAISE EXCEPTION 'Invocation transaction marker must match the actual new success row' USING ERRCODE='23000'; END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_invocation119_terminal_marker() FROM PUBLIC,hephaestus_app,hephaestus_worker;
CREATE TRIGGER invocation119_terminal_marker_immutable BEFORE INSERT OR UPDATE OR DELETE
 ON invocation119_terminal_transactions FOR EACH ROW EXECUTE FUNCTION guard_invocation119_terminal_marker();
CREATE TRIGGER invocation119_terminal_marker_no_truncate BEFORE TRUNCATE
 ON invocation119_terminal_transactions FOR EACH STATEMENT EXECUTE FUNCTION guard_invocation119_terminal_marker();

CREATE FUNCTION record_invocation119_terminal_transaction() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_invocation117_worker();
 PERFORM require_invocation119_terminal_transaction();
 PERFORM assert_invocation119_terminal_row(OLD,NEW);
 INSERT INTO invocation119_terminal_transactions(run_id,success_version,success_xid,identity)
 VALUES(NEW.id,NEW.state_version,pg_current_xact_id(),
  to_jsonb(NEW)-ARRAY['state','state_version','updated_at','cancel_requested_at']);
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION record_invocation119_terminal_transaction() FROM PUBLIC,hephaestus_app,hephaestus_worker;
-- Ordinary AFTER, not a constraint trigger; caller cannot defer/drain its birth.
CREATE TRIGGER a_invocation119_terminal_transaction_birth AFTER UPDATE ON runs FOR EACH ROW
 WHEN (NEW.run_kind='invocation' AND OLD.state='running' AND NEW.state='succeeded')
 EXECUTE FUNCTION record_invocation119_terminal_transaction();

CREATE FUNCTION guard_invocation119_terminal_graph() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE run_ids uuid[]:=ARRAY[]::uuid[]; volume_ids uuid[]:=ARRAY[]::uuid[];
 execution runs%ROWTYPE; locked runs%ROWTYPE; config bytea;
BEGIN
 PERFORM require_invocation119_integrity_mutator();
 IF TG_TABLE_NAME='agent_instance_volume_leases' THEN
  IF TG_OP IN ('UPDATE','DELETE') THEN
   run_ids:=array_append(run_ids,OLD.run_id); volume_ids:=array_append(volume_ids,OLD.volume_id); END IF;
  IF TG_OP IN ('INSERT','UPDATE') THEN
   run_ids:=array_append(run_ids,NEW.run_id); volume_ids:=array_append(volume_ids,NEW.volume_id); END IF;
 ELSE
  IF TG_OP IN ('UPDATE','DELETE') THEN volume_ids:=array_append(volume_ids,OLD.id); END IF;
  IF TG_OP='UPDATE' THEN volume_ids:=array_append(volume_ids,NEW.id); END IF;
 END IF;
 -- Immutable original target membership catches foreign lease INSERT/DELETE,
 -- including after a row disappeared; current NEW.run_id alone is insufficient.
 FOR execution IN SELECT run.* FROM runs run
  WHERE run.run_kind='invocation' AND run.state='succeeded'
   AND (run.id=ANY(run_ids) OR EXISTS(SELECT 1 FROM run_cleanup_target_leases original
    WHERE original.run_id=run.id AND original.generation=1 AND original.volume_id=ANY(volume_ids)))
  ORDER BY run.id LOOP
  IF invocation119_terminal_marker(execution)=pg_current_xact_id() THEN
   RAISE EXCEPTION 'Invocation success graph cannot mutate before its transaction commits' USING ERRCODE='23000'; END IF;
  -- App/owner ordinary DML reached this dispatch unchanged. Affected Succeeded
  -- history stays Held for them: existing result/config helpers remain worker-only.
  PERFORM require_invocation117_worker();
  SELECT config_bytes INTO config FROM run_invocation_consumptions WHERE run_id=execution.id;
  -- Raw child-row DML may already hold a child: TRY/NOWAIT never waits backwards.
  locked:=lock_qualified_invocation119(execution.id,config);
  PERFORM invocation119_terminal_marker(locked);
  IF locked.state='succeeded' AND invocation119_terminal_context(locked.id) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'Invocation succeeded graph must retain the entire original held history' USING ERRCODE='23000'; END IF;
 END LOOP;
 RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END $$;
REVOKE ALL ON FUNCTION guard_invocation119_terminal_graph() FROM PUBLIC,hephaestus_app,hephaestus_worker;
CREATE TRIGGER a_invocation119_terminal_lease_graph AFTER INSERT OR UPDATE OR DELETE
 ON agent_instance_volume_leases FOR EACH ROW EXECUTE FUNCTION guard_invocation119_terminal_graph();
CREATE TRIGGER a_invocation119_terminal_volume_graph AFTER UPDATE OR DELETE
 ON agent_instance_state_volumes FOR EACH ROW EXECUTE FUNCTION guard_invocation119_terminal_graph();

CREATE FUNCTION guard_invocation119_terminal_cancel_inbox() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE p_run uuid; execution runs%ROWTYPE; consumed run_invocation_consumptions%ROWTYPE;
BEGIN
 PERFORM require_invocation119_integrity_mutator();
 -- Unclassifiable ordinary payloads keep their existing guards/behavior.
 IF jsonb_typeof(NEW.payload->'run_id') IS DISTINCT FROM 'string' THEN RETURN NEW; END IF;
 BEGIN p_run:=(NEW.payload->>'run_id')::uuid;
 EXCEPTION WHEN invalid_text_representation THEN RETURN NEW; END;
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.run_kind IS DISTINCT FROM 'invocation'
  OR NOT EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=p_run) THEN RETURN NEW; END IF;
 -- Consumed original Cancel delivery is a worker-only owning mutation, unlike
 -- ordinary dispatch and unchanged unconsumed queued closure delivery.
 PERFORM require_invocation117_worker();
 IF jsonb_typeof(NEW.payload->'reason') IS DISTINCT FROM 'string'
  OR NEW.payload IS DISTINCT FROM jsonb_build_object('command_id',NEW.command_id,'run_id',p_run,'reason',NEW.payload->>'reason') THEN
  RAISE EXCEPTION 'Invocation Cancel requires the exact original typed command' USING ERRCODE='23000'; END IF;
 SELECT * INTO consumed FROM run_invocation_consumptions WHERE run_id=p_run;
 -- Before INSERT uniqueness/inbox row effects: complete sorted TRY reservations,
 -- original configured parents/Run/volumes/grants/fences, all late rows NOWAIT.
 execution:=lock_qualified_invocation119(p_run,consumed.config_bytes);
 IF execution.vm_binding_kind IS DISTINCT FROM 'planned'
  OR ROW(execution.vm_provider_namespace,execution.vm_host_id,execution.vm_id)
   IS DISTINCT FROM ROW(consumed.vm_provider_namespace,consumed.vm_host_id,consumed.vm_id) THEN
  RAISE EXCEPTION 'Invocation Cancel differs from original configured provider scope' USING ERRCODE='23000'; END IF;
 IF execution.state='succeeded' OR execution.outcome='succeeded' THEN
  IF invocation119_terminal_marker(execution)=pg_current_xact_id() THEN
   RAISE EXCEPTION 'Invocation Cancel cannot enter its uncommitted success transaction' USING ERRCODE='23000'; END IF;
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_invocation119_terminal_cancel_inbox() FROM PUBLIC,hephaestus_app,hephaestus_worker;
CREATE TRIGGER a_invocation119_terminal_cancel_inbox BEFORE INSERT ON command_inbox FOR EACH ROW
 WHEN (NEW.command_type='cancel_run') EXECUTE FUNCTION guard_invocation119_terminal_cancel_inbox();

CREATE FUNCTION require_invocation119_terminal_transaction() RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_invocation117_worker();
 IF current_setting('transaction_isolation') IS DISTINCT FROM 'read committed'
  OR current_setting('transaction_read_only') IS DISTINCT FROM 'off' THEN
  RAISE EXCEPTION 'Invocation terminal success requires owning READ COMMITTED writable transaction' USING ERRCODE='23000'; END IF;
END $$;
REVOKE ALL ON FUNCTION require_invocation119_terminal_transaction() FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_terminal_cancelled(p_run uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_invocation117_worker();
 -- Do not filter on processed_at: committed original Cancel delivery wins.
 IF EXISTS(SELECT 1 FROM command_inbox WHERE command_type='cancel_run' AND payload->>'run_id'=p_run::text
   AND (jsonb_typeof(payload->'reason') IS DISTINCT FROM 'string'
    OR payload IS DISTINCT FROM jsonb_build_object('command_id',command_id,'run_id',p_run,'reason',payload->>'reason'))) THEN
  RAISE EXCEPTION 'Invocation Cancel inbox differs from exact original typed command' USING ERRCODE='23000'; END IF;
 RETURN EXISTS(SELECT 1 FROM runs WHERE id=p_run AND cancel_requested_at IS NOT NULL)
  OR EXISTS(SELECT 1 FROM command_inbox WHERE command_type='cancel_run'
    AND payload->>'run_id'=p_run::text);
END $$;
REVOKE ALL ON FUNCTION invocation119_terminal_cancelled(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_terminal_context(p_run uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE;
 session runtime_authority_sessions%ROWTYPE; selected_count integer;
BEGIN
 PERFORM require_invocation117_worker();
 SELECT * INTO execution FROM runs WHERE id=p_run;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 SELECT * INTO session FROM runtime_authority_sessions WHERE run_id=p_run;
 selected_count:=jsonb_array_length(invocation119_selections(p_run)->'selections');
 RETURN execution.id IS NOT NULL AND receipt.id IS NOT NULL
  AND invocation119_immutable(p_run) IS NOT DISTINCT FROM true
  AND execution.vm_binding_kind IS NOT DISTINCT FROM 'planned'
  AND execution.vm_plan_admission_required IS DISTINCT FROM true
  AND execution.volume_acquisition_closed_at IS NOT NULL
  AND ROW(execution.vm_provider_namespace,execution.vm_host_id,execution.vm_id)
   IS NOT DISTINCT FROM ROW(receipt.config_json->'vm'->>'namespace',receipt.config_json->'vm'->>'host_id',p_run::text)
  AND num_nonnulls(execution.vm_provider_namespace,execution.vm_host_id,execution.vm_id)=3
  AND receipt.config_hash IS NOT DISTINCT FROM sha256(receipt.config_bytes)
  AND receipt.config_json IS NOT DISTINCT FROM convert_from(receipt.config_bytes,'UTF8')::jsonb
  -- Full original IDs/ceiling/interval identity, without current rights/status.
  AND invocation119_session_graph(p_run,false) IS NOT DISTINCT FROM true
  AND session.status IN ('active','revoked','expired')
  AND session.acknowledged_at IS NOT NULL AND session.acknowledged_at>=session.issued_at
  AND session.acknowledged_at<session.expires_at
  AND NOT EXISTS(SELECT 1 FROM runtime_authority_sessions other
   WHERE (other.run_id=p_run OR other.id=p_run) AND (other.run_id<>p_run OR other.id<>p_run))
  AND NOT EXISTS(SELECT 1 FROM run_authorization_snapshots other
   WHERE (other.run_id=p_run OR other.id=p_run) AND (other.run_id<>p_run OR other.id<>p_run))
  AND selected_count BETWEEN 0 AND 32
  AND execution.volume_selection_snapshot IS NOT DISTINCT FROM invocation119_selections(p_run)
  AND profiled_run_snapshot_matches(execution) IS NOT DISTINCT FROM true
  -- Cardinality is only an extra bound; both original and held identities match.
  AND (SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=p_run)=selected_count
  AND NOT EXISTS(SELECT 1 FROM agent_instance_revision_volume_bindings binding
   WHERE binding.instance_revision_id=execution.instance_revision_id
    AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease
     WHERE lease.run_id=p_run AND lease.slot_key=binding.slot_key AND lease.volume_id=binding.volume_id))
  AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease
   LEFT JOIN agent_instance_state_volumes volume ON volume.id=lease.volume_id
   LEFT JOIN agent_instance_revision_volume_bindings binding
    ON binding.instance_revision_id=execution.instance_revision_id AND binding.slot_key=lease.slot_key
   WHERE lease.run_id=p_run AND (volume.id IS NULL OR binding.volume_id IS DISTINCT FROM lease.volume_id
    OR binding.project_id IS DISTINCT FROM receipt.project_id OR volume.project_id IS DISTINCT FROM receipt.project_id
    OR ROW(lease.instance_id,lease.selected_revision_id,lease.selected_release_id,lease.selected_release_agent_id,
      lease.selected_project_id,lease.selection_origin,lease.access_mode,lease.guest_path,lease.slot_required,lease.minimum_capacity_bytes)
     IS DISTINCT FROM ROW(execution.instance_id,execution.instance_revision_id,execution.release_id,execution.release_agent_id,
      binding.project_id,binding.provenance,binding.access_mode,binding.guest_path,binding.slot_required,binding.minimum_capacity_bytes)
    OR lease.release_contract_hash IS DISTINCT FROM (SELECT runtime_contract_hash FROM release_agents WHERE id=execution.release_agent_id)
    OR lease.update_id IS NOT NULL OR lease.state IS DISTINCT FROM 'active'
    OR lease.released_at IS NOT NULL OR lease.recovering_at IS NOT NULL OR lease.attached_at IS NULL
    OR lease.expires_at<=lease.acquired_at OR lease.attached_at<lease.acquired_at OR lease.attached_at>=lease.expires_at
    OR session.expires_at>lease.expires_at OR volume.lease_generation IS DISTINCT FROM lease.fencing_token
    OR lease.host_id IS DISTINCT FROM execution.vm_host_id OR volume.host_id IS DISTINCT FROM lease.host_id
    OR volume.state IS DISTINCT FROM 'attached' OR volume.provisioning_state IS DISTINCT FROM 'ready'
    OR EXISTS(SELECT 1 FROM agent_instance_volume_leases other
     WHERE other.volume_id=lease.volume_id AND other.id<>lease.id AND other.released_at IS NULL)))
  AND EXISTS(SELECT 1 FROM run_cleanup_targets target JOIN run_cleanup_receipts observed
    ON observed.run_id=target.run_id AND observed.generation=target.generation AND observed.target_digest=target.target_digest
   WHERE target.run_id=p_run AND target.generation=1 AND target.vm_target_kind='planned'
    AND target.instance_id=execution.instance_id AND target.instance_revision_id=execution.instance_revision_id
    AND target.lease_count=selected_count
    AND ROW(target.vm_provider_namespace,target.vm_host_id,target.vm_id)
     IS NOT DISTINCT FROM ROW(execution.vm_provider_namespace,execution.vm_host_id,execution.vm_id)
    AND ROW(observed.vm_provider_namespace,observed.vm_host_id,observed.vm_id)
     IS NOT DISTINCT FROM ROW(target.vm_provider_namespace,target.vm_host_id,target.vm_id)
    AND observed.observation_kind IN ('destroyed','authoritatively_absent')
    AND run_cleanup_set_matches(p_run,1,false) IS NOT DISTINCT FROM true)
  AND NOT EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=p_run);
END $$;
REVOKE ALL ON FUNCTION invocation119_terminal_context(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION assert_invocation119_terminal_row(p_old runs,p_new runs) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_invocation117_worker();
 IF p_old.run_kind IS DISTINCT FROM 'invocation' OR p_new.run_kind IS DISTINCT FROM 'invocation'
  OR p_old.state IS DISTINCT FROM 'running' OR num_nonnulls(p_old.outcome,p_old.exit_code,p_old.exit_signal,p_old.failure)<>0
  OR p_new.state IS DISTINCT FROM 'succeeded' OR p_new.outcome IS DISTINCT FROM 'succeeded'
  OR p_new.exit_code IS DISTINCT FROM 0 OR p_new.exit_signal IS NOT NULL OR p_new.failure IS NOT NULL
  OR p_new.state_version IS DISTINCT FROM p_old.state_version+1
  OR (to_jsonb(p_new)-ARRAY['state','outcome','exit_code','exit_signal','failure','state_version','updated_at'])
   IS DISTINCT FROM (to_jsonb(p_old)-ARRAY['state','outcome','exit_code','exit_signal','failure','state_version','updated_at'])
  OR invocation119_terminal_cancelled(p_old.id) IS DISTINCT FROM false
  OR invocation119_terminal_context(p_old.id) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'Invocation success needs exact Running/new result/closed acknowledged full set' USING ERRCODE='23000';
 END IF;
END $$;
REVOKE ALL ON FUNCTION assert_invocation119_terminal_row(runs,runs) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION qualify_invocation119_terminal_result(p_run uuid,p_config bytea,p_namespace text,p_host text,
 p_exit_present boolean,p_code integer,p_signal integer,p_failure text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
 PERFORM require_invocation117_worker();
 PERFORM require_invocation119_terminal_transaction();
 -- Caller values, before any COALESCE tail; stored exit cannot substitute.
 IF p_exit_present IS DISTINCT FROM true OR p_code IS DISTINCT FROM 0 OR p_signal IS NOT NULL OR p_failure IS NOT NULL THEN
  RAISE EXCEPTION 'Invocation success requires actual caller VmExit0 without signal/failure' USING ERRCODE='23000'; END IF;
 -- Rust already acquired complete sorted early reservations in this owning TX.
 -- Raw callers use existing TRY reservations and NOWAIT ordered parent/child locks.
 execution:=lock_qualified_invocation119(p_run,p_config);
 IF execution.state IS DISTINCT FROM 'running'
  OR num_nonnulls(execution.outcome,execution.exit_code,execution.exit_signal,execution.failure)<>0
  OR execution.execution_profile IS DISTINCT FROM 'owned_canonical'
  OR execution.vm_binding_kind IS DISTINCT FROM 'planned'
  OR num_nonnulls(p_namespace,p_host)<>2
  OR ROW(execution.vm_provider_namespace,execution.vm_host_id,execution.vm_id)
   IS DISTINCT FROM ROW(p_namespace,p_host,p_run::text) THEN
  RAISE EXCEPTION 'Invocation result differs from locked Running/configured provider owner' USING ERRCODE='23000'; END IF;
 IF invocation119_terminal_cancelled(p_run) IS NOT DISTINCT FROM true THEN RETURN 'cancelled'; END IF;
 IF invocation119_terminal_context(p_run) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'Invocation result needs historical ACK/full original fences/scoped receipt' USING ERRCODE='23000'; END IF;
 RETURN 'succeeded';
END $$;
REVOKE ALL ON FUNCTION qualify_invocation119_terminal_result(uuid,bytea,text,text,boolean,integer,integer,text) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION qualify_invocation119_terminal_result(uuid,bytea,text,text,boolean,integer,integer,text) TO hephaestus_worker;

CREATE FUNCTION guard_invocation119_terminal_result() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE config bytea; execution runs%ROWTYPE; actual_role text;
BEGIN
 PERFORM require_invocation119_integrity_mutator();
 actual_role:=CASE WHEN current_setting('role',true)='none' THEN session_user::text
  ELSE current_setting('role',true) END;
 -- Original authenticated closure only stamps; it never advances cleanup/result.
 IF actual_role='hephaestus_app' AND (OLD.state='succeeded' OR OLD.outcome='succeeded') THEN
  IF invocation119_terminal_marker(OLD)=pg_current_xact_id() THEN
   RAISE EXCEPTION 'Invocation success must commit before a later row mutation' USING ERRCODE='23000'; END IF;
  PERFORM invocation119_terminal_marker(NEW);
  IF OLD.state NOT IN ('succeeded','cleaning_up') OR OLD.outcome IS DISTINCT FROM 'succeeded'
   OR NEW.cancel_requested_at IS NULL
   OR (OLD.cancel_requested_at IS NOT NULL AND NEW.cancel_requested_at IS DISTINCT FROM OLD.cancel_requested_at)
   OR (to_jsonb(NEW)-ARRAY['updated_at','cancel_requested_at'])
    IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['updated_at','cancel_requested_at']) THEN
   RAISE EXCEPTION 'App Invocation closure permits only an immutable monotonic cancellation stamp' USING ERRCODE='23000'; END IF;
  RETURN NEW;
 END IF;
 PERFORM require_invocation117_worker();
 IF OLD.state='succeeded' OR OLD.outcome='succeeded' THEN
  -- Immediate system transaction boundary, independent of constraint timing.
  IF invocation119_terminal_marker(OLD)=pg_current_xact_id() THEN
   RAISE EXCEPTION 'Invocation success must commit before a later row mutation' USING ERRCODE='23000'; END IF;
  PERFORM invocation119_terminal_marker(NEW);
  IF ROW(NEW.outcome,NEW.exit_code,NEW.exit_signal,NEW.failure)
    IS DISTINCT FROM ROW(OLD.outcome,OLD.exit_code,OLD.exit_signal,OLD.failure) THEN
   RAISE EXCEPTION 'Invocation terminal result is immutable through later cleanup' USING ERRCODE='23000'; END IF;
  IF (OLD.state='succeeded' AND NEW.state NOT IN ('succeeded','cleaning_up'))
   OR (OLD.state='cleaning_up' AND NEW.state NOT IN ('cleaning_up','cleaned_up'))
   OR (OLD.state='cleaned_up' AND NEW.state<>'cleaned_up') THEN
   RAISE EXCEPTION 'Invocation success only advances through owning cleanup' USING ERRCODE='23000'; END IF;
  -- Subsequent owning TX cleanup/cancellation keeps its existing lifecycle.
  IF NEW.state='succeeded' AND (to_jsonb(NEW)-ARRAY['updated_at','cancel_requested_at'])
    IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['updated_at','cancel_requested_at']) THEN
   RAISE EXCEPTION 'Invocation succeeded row cannot be replaced in place' USING ERRCODE='23000'; END IF;
  RETURN NEW;
 END IF;
 PERFORM require_invocation119_terminal_transaction();
 SELECT config_bytes INTO config FROM run_invocation_consumptions WHERE run_id=OLD.id;
 execution:=lock_qualified_invocation119(OLD.id,config);
 IF to_jsonb(execution) IS DISTINCT FROM to_jsonb(OLD) THEN
  RAISE EXCEPTION 'Invocation terminal candidate differs from locked OLD row' USING ERRCODE='23000'; END IF;
 PERFORM assert_invocation119_terminal_row(OLD,NEW);
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_invocation119_terminal_result() FROM PUBLIC,hephaestus_app,hephaestus_worker;
-- Proposal: alphabetical BEFORE ordering ahead of instance/profile progress.
-- C must verify actual pg_trigger order before any runtime acceptance.
CREATE TRIGGER a_invocation119_terminal_result BEFORE UPDATE ON runs FOR EACH ROW
 WHEN ((OLD.run_kind='invocation' OR NEW.run_kind='invocation')
  AND (NEW.state='succeeded' OR NEW.outcome='succeeded' OR OLD.state='succeeded' OR OLD.outcome='succeeded'))
 EXECUTE FUNCTION guard_invocation119_terminal_result();

CREATE FUNCTION validate_invocation119_terminal_candidate_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
 PERFORM require_invocation117_worker();
 PERFORM require_invocation119_terminal_transaction();
 -- Deferred NEW is the saved success candidate, not a later final cleanup row.
 SELECT * INTO execution FROM runs WHERE id=NEW.id;
 IF to_jsonb(execution) IS DISTINCT FROM to_jsonb(NEW) THEN
  RAISE EXCEPTION 'Invocation success must commit before later owning cleanup transaction' USING ERRCODE='23000'; END IF;
 PERFORM assert_invocation119_terminal_row(OLD,NEW);
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_invocation119_terminal_candidate_commit() FROM PUBLIC,hephaestus_app,hephaestus_worker;
CREATE CONSTRAINT TRIGGER invocation119_terminal_candidate_commit AFTER UPDATE ON runs
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW
 WHEN (NEW.run_kind='invocation' AND NEW.state='succeeded' AND OLD.state IS DISTINCT FROM NEW.state)
 EXECUTE FUNCTION validate_invocation119_terminal_candidate_commit();


-- Static successor: replace ONLY marked success branch to restore order25.
CREATE OR REPLACE FUNCTION assert_invocation119_transition(p_run uuid,p_next text) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
 PERFORM require_invocation117_worker();
 SELECT * INTO execution FROM runs WHERE id=p_run;
 -- BEGIN queued119 cleanup-only dispatch.
 IF execution.volume_acquisition_closed_at IS NOT NULL
  AND NOT EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=p_run) THEN
  PERFORM assert_invocation119_queued_cleanup_transition(p_run,p_next);
  RETURN;
 END IF;
 -- END queued119 cleanup-only dispatch.
 IF invocation119_immutable(p_run) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'Invocation transition needs immutable consumed producer' USING ERRCODE='23000';
 END IF;
 IF p_next IN ('starting','running') THEN
  IF (p_next='starting' AND execution.state<>'provisioning')
   OR (p_next='running' AND execution.state<>'starting')
   OR invocation119_live(p_run) IS DISTINCT FROM true
   OR invocation119_acknowledged_set(p_run) IS DISTINCT FROM true
   OR invocation119_capabilities_live(p_run) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'Invocation progress needs current acknowledged exact attached ceiling' USING ERRCODE='23000'; END IF;
 ELSIF p_next='succeeded' THEN
  -- BEGIN terminal119 historical success.
  execution:=lock_qualified_invocation119(p_run,(SELECT config_bytes FROM run_invocation_consumptions WHERE run_id=p_run));
  IF execution.state IS DISTINCT FROM 'running'
   OR num_nonnulls(execution.outcome,execution.exit_code,execution.exit_signal,execution.failure)<>0
   OR invocation119_terminal_cancelled(p_run) IS DISTINCT FROM false
   OR invocation119_terminal_context(p_run) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'Invocation success needs locked Running/closed historical result context' USING ERRCODE='23000'; END IF;
  -- END terminal119 historical success.
 ELSIF p_next='leasing_volume' THEN
  IF execution.state<>'queued' OR execution.state_version<>0 OR invocation119_open_set(p_run) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'first Invocation progress needs current queued admission' USING ERRCODE='23000';
  END IF;
 ELSIF p_next='provisioning' THEN
  IF execution.state<>'leasing_volume' OR invocation119_live(p_run) IS DISTINCT FROM true OR invocation119_frozen_set(p_run) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'Invocation Provisioning needs the complete frozen live set' USING ERRCODE='23000';
  END IF;
 ELSIF p_next='cleaned_up' THEN
  IF execution.state<>'cleaning_up' OR execution.volume_acquisition_closed_at IS NULL
    OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=p_run AND released_at IS NULL)
    OR NOT EXISTS(SELECT 1 FROM run_cleanup_targets target JOIN run_cleanup_completions done
      ON done.run_id=target.run_id AND done.generation=target.generation AND done.target_digest=target.target_digest
      WHERE target.run_id=p_run AND ROW(target.vm_provider_namespace,target.vm_host_id,target.vm_id)
       IS NOT DISTINCT FROM ROW(execution.vm_provider_namespace,execution.vm_host_id,execution.vm_id)
       AND run_cleanup_set_matches(p_run,target.generation,true)) THEN
   RAISE EXCEPTION 'Invocation cleanup lacks protected whole-set completion' USING ERRCODE='23000';
  END IF;
 END IF;
END $$;

-- Static successor: remove ONLY marked success branch to restore order25.
CREATE OR REPLACE FUNCTION validate_invocation119_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE p_run uuid; execution runs%ROWTYPE;
BEGIN
 IF TG_TABLE_NAME='runs' THEN
  IF NEW.run_kind<>'invocation' THEN RETURN NULL; END IF;
  p_run:=NEW.id;
  -- BEGIN queued119 cleanup-only dispatch.
  IF NEW.volume_acquisition_closed_at IS NOT NULL
   AND NOT EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=p_run) THEN
   PERFORM validate_invocation119_queued_cleanup(OLD,NEW);
   RETURN NULL;
  END IF;
  -- END queued119 cleanup-only dispatch.
  IF NEW.state IS NOT DISTINCT FROM OLD.state AND NEW.volume_selection_snapshot IS NOT DISTINCT FROM OLD.volume_selection_snapshot THEN RETURN NULL; END IF;
 ELSE
  p_run:=NEW.run_id;
  IF TG_OP='UPDATE' THEN
   IF ROW(NEW.heartbeat_at,NEW.expires_at,NEW.attached_at) IS NOT DISTINCT FROM ROW(OLD.heartbeat_at,OLD.expires_at,OLD.attached_at) THEN RETURN NULL; END IF;
  END IF;
 END IF;
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.run_kind IS DISTINCT FROM 'invocation' THEN RETURN NULL; END IF;
 PERFORM require_invocation117_worker();
 -- Cleanup has independent existing receipt/full-set proof. It must remain
 -- possible after Execute/Source/gate loss and cannot be mistaken for admission.
 -- Failed/Cancelled outcomes do not clear a fence or prove physical quiescence;
 -- canonical receipts remain mandatory before any reuse or CleanedUp.
 -- A lease birth must finish the complete frozen set even if the same TX
 -- later records a failure/cleanup outcome. Those outcomes cannot adopt a
 -- partial acquisition or bypass the birth fence invariant.
 IF TG_TABLE_NAME='agent_instance_volume_leases' AND TG_OP='INSERT' THEN
  IF invocation119_frozen_set(p_run) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'Invocation lease birth lacks complete frozen acquisition' USING ERRCODE='23000'; END IF;
 END IF;
 -- BEGIN terminal119 historical success.
 IF execution.state='succeeded' THEN
  IF execution.outcome IS DISTINCT FROM 'succeeded' OR execution.exit_code IS DISTINCT FROM 0
   OR execution.exit_signal IS NOT NULL OR execution.failure IS NOT NULL
   OR invocation119_terminal_cancelled(p_run) IS DISTINCT FROM false
   OR invocation119_terminal_context(p_run) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'Invocation committed success lacks exact historical result graph' USING ERRCODE='23000'; END IF;
  RETURN NULL;
 END IF;
 -- END terminal119 historical success.
 IF execution.state IN ('failed','cancelled','cleaning_up','cleaned_up') THEN RETURN NULL; END IF;
 IF (execution.state IN ('starting','running') AND (invocation119_acknowledged_set(p_run) IS DISTINCT FROM true
      OR invocation119_capabilities_live(p_run) IS DISTINCT FROM true))
   OR invocation119_live(p_run) IS DISTINCT FROM true
   OR (execution.volume_selection_snapshot IS NOT NULL AND invocation119_frozen_set(p_run) IS DISTINCT FROM true)
   OR (execution.state='provisioning' AND invocation119_frozen_set(p_run) IS DISTINCT FROM true) THEN
  RAISE EXCEPTION 'Invocation transaction lacks complete current119 acquisition/progress evidence' USING ERRCODE='23000';
 END IF;
 RETURN NULL;
END $$;

-- PRIVATE119 saved-empty cleanup extension. Historical scope only, never admission.
-- Global absence includes foreign runtime-ID collisions and every history status.
CREATE FUNCTION invocation119_cleanup_secret_history_empty(p_run uuid,p_instance uuid,p_revision uuid)
RETURNS boolean LANGUAGE plpgsql STABLE SECURITY DEFINER
SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE empty_history boolean;
BEGIN
 PERFORM public.require_invocation117_worker();
 IF current_setting('transaction_isolation') IS DISTINCT FROM 'repeatable read'
  OR current_setting('transaction_read_only') IS DISTINCT FROM 'on' THEN
  RAISE EXCEPTION 'Invocation empty-secret cleanup requires owning REPEATABLE READ read-only transaction'
   USING ERRCODE='23000'; END IF;
 IF p_run IS NULL OR p_instance IS NULL OR p_revision IS NULL
  OR p_run='00000000-0000-0000-0000-000000000000'::uuid
  OR p_instance='00000000-0000-0000-0000-000000000000'::uuid
  OR p_revision='00000000-0000-0000-0000-000000000000'::uuid THEN RETURN false; END IF;
 WITH bindings AS (
  SELECT id FROM public.agent_secret_bindings WHERE instance_revision_id=p_revision OR id=p_run),
 sessions AS (
  SELECT id FROM public.secret_runtime_sessions WHERE run_id=p_run OR id=p_run
   OR (instance_id=p_instance AND instance_revision_id=p_revision)),
 rules AS (
  SELECT id FROM public.brokered_secret_rules WHERE instance_revision_id=p_revision OR id=p_run
   OR binding_id=p_run OR binding_id IN (SELECT id FROM bindings)),
 leases AS (
  SELECT id FROM public.secret_leases WHERE run_id=p_run OR id=p_run OR session_id=p_run OR binding_id=p_run
   OR session_id IN (SELECT id FROM sessions) OR binding_id IN (SELECT id FROM bindings)),
 snapshots AS (
  SELECT id FROM public.brokered_secret_lease_snapshots
   WHERE run_id=p_run OR id=p_run OR runtime_session_id=p_run OR lease_id=p_run OR binding_id=p_run OR rule_id=p_run
    OR runtime_session_id IN (SELECT id FROM sessions) OR lease_id IN (SELECT id FROM leases)
    OR binding_id IN (SELECT id FROM bindings) OR rule_id IN (SELECT id FROM rules))
 SELECT NOT EXISTS(SELECT 1 FROM bindings)
  AND NOT EXISTS(SELECT 1 FROM public.run_instance_provenance WHERE run_id=p_run
   OR (instance_id=p_instance AND instance_revision_id=p_revision))
  AND NOT EXISTS(SELECT 1 FROM public.run_secret_provenance WHERE run_id=p_run OR binding_id=p_run
   OR binding_id IN (SELECT id FROM bindings))
  AND NOT EXISTS(SELECT 1 FROM sessions)
  AND NOT EXISTS(SELECT 1 FROM leases)
  AND NOT EXISTS(SELECT 1 FROM public.secret_runtime_mounts WHERE run_id=p_run OR opaque_directory=p_run)
  AND NOT EXISTS(SELECT 1 FROM rules)
  AND NOT EXISTS(SELECT 1 FROM snapshots)
  -- Canonical command audit targets are project/repository, not runtime aliases.
  -- Runtime writers correlate through runtime_run_id, binding_id and lease_id.
  AND NOT EXISTS(SELECT 1 FROM public.secret_audit_events WHERE id=p_run OR runtime_run_id=p_run
   OR binding_id=p_run OR lease_id=p_run OR binding_id IN (SELECT id FROM bindings)
   OR lease_id IN (SELECT id FROM leases))
  AND NOT EXISTS(SELECT 1 FROM public.brokered_secret_audit_events
   WHERE id=p_run OR run_id=p_run OR runtime_session_id=p_run OR lease_snapshot_id=p_run OR rule_id=p_run
    OR runtime_session_id IN (SELECT id FROM sessions) OR lease_snapshot_id IN (SELECT id FROM snapshots)
    OR rule_id IN (SELECT id FROM rules)) INTO empty_history;
 RETURN empty_history;
END $$;
REVOKE ALL ON FUNCTION invocation119_cleanup_secret_history_empty(uuid,uuid,uuid)
 FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION verify_invocation119_empty_secret_cleanup(p_run uuid,p_config bytea,p_namespace text,p_host text)
RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER
SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution public.runs%ROWTYPE; receipt public.agent_instance_execution_invocations%ROWTYPE;
 revision public.agent_instance_revisions%ROWTYPE;
BEGIN
 PERFORM public.require_invocation117_worker();
 IF current_setting('transaction_isolation') IS DISTINCT FROM 'repeatable read'
  OR current_setting('transaction_read_only') IS DISTINCT FROM 'on' THEN
  RAISE EXCEPTION 'Invocation empty-secret cleanup requires owning REPEATABLE READ read-only transaction'
   USING ERRCODE='23000'; END IF;
 IF p_run IS NULL OR p_run='00000000-0000-0000-0000-000000000000'::uuid
  OR p_config IS NULL OR p_namespace IS NULL OR p_host IS NULL THEN
  RAISE EXCEPTION 'qualified Invocation empty-secret cleanup context is unavailable' USING ERRCODE='23000'; END IF;
 SELECT * INTO execution FROM public.runs WHERE id=p_run;
 SELECT * INTO receipt FROM public.agent_instance_execution_invocations WHERE run_id=p_run;
 SELECT * INTO revision FROM public.agent_instance_revisions WHERE id=execution.instance_revision_id;
 IF execution.id IS NULL OR receipt.id IS NULL OR revision.id IS NULL
  OR execution.run_kind IS DISTINCT FROM 'invocation'
  OR execution.execution_profile IS DISTINCT FROM 'owned_canonical'
  OR execution.state NOT IN ('running','succeeded','failed','cancelled','cleaning_up')
  OR revision.instance_id IS DISTINCT FROM execution.instance_id
  OR jsonb_typeof(revision.secret_bindings) IS DISTINCT FROM 'array'
  OR revision.secret_bindings IS DISTINCT FROM '[]'::jsonb
  OR p_config IS DISTINCT FROM receipt.config_bytes
  OR ROW(p_namespace,p_host) IS DISTINCT FROM ROW(execution.vm_provider_namespace,execution.vm_host_id)
  OR ROW(p_namespace,p_host) IS DISTINCT FROM ROW(receipt.config_json->'vm'->>'namespace',receipt.config_json->'vm'->>'host_id') THEN
  RAISE EXCEPTION 'qualified Invocation empty-secret cleanup context is unavailable' USING ERRCODE='23000'; END IF;
 -- Unchanged terminal context verifies immutable115/117, original ACK/session(false),
 -- complete still-held attached fences, closed target/digest/scoped receipt, no completion.
 -- It proves neither live admission nor a guest exit/result/marker.
 IF public.invocation119_terminal_context(p_run) IS DISTINCT FROM true
  OR public.invocation119_cleanup_secret_history_empty(p_run,execution.instance_id,execution.instance_revision_id)
   IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'qualified Invocation empty-secret cleanup context is unavailable' USING ERRCODE='23000'; END IF;
 RETURN jsonb_build_object('decision','empty','run_id',p_run);
END $$;
REVOKE ALL ON FUNCTION verify_invocation119_empty_secret_cleanup(uuid,bytea,text,text)
 FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION verify_invocation119_empty_secret_cleanup(uuid,bytea,text,text) TO hephaestus_worker;
