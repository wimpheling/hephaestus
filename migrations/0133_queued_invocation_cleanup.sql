-- PRIVATE119 queued-before117 cleanup metadata successor. SOURCE ONLY.
-- Requires frozen113/114/115/117/119A/B1/B2 and118 closure/wake/resume readers.
-- New owning composition supplies actual config bytes; stored config used by
-- trigger-only dispatch is immutable comparison data, never provider authority.
-- No state, target, receipt, outcome, Start consumption or credential writes.

CREATE FUNCTION invocation119_queued_cleanup_original(p_run uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
 removal agent_instance_removal_requests%ROWTYPE; command release_command_inbox%ROWTYPE;
 candidates uuid[]; drain recipe_effect_attempts%ROWTYPE; original jsonb;
BEGIN
 PERFORM require_run_cleanup_wake118();
 SELECT * INTO execution FROM runs WHERE id=p_run;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=execution.instance_id;
 IF execution.id IS NULL OR child.instance_id IS NULL
  OR (SELECT count(*) FROM agent_instance_removal_requests WHERE instance_id=execution.instance_id)<>1 THEN
  RAISE EXCEPTION 'one original queued cleanup closure required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO removal FROM agent_instance_removal_requests WHERE instance_id=execution.instance_id;
 IF (SELECT count(*) FROM release_command_inbox WHERE operation='request_instance_removal' AND secondary_id=removal.id)<>1 THEN
  RAISE EXCEPTION 'one original removal command required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO command FROM release_command_inbox WHERE operation='request_instance_removal' AND secondary_id=removal.id;
 SELECT array_agg(id ORDER BY id) INTO candidates FROM recipe_effect_attempts
  WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name AND action='drain'
   AND original_drain_removal118(id,execution.instance_id)=removal.id;
 IF COALESCE(cardinality(candidates),0)<>1 THEN
  RAISE EXCEPTION 'one immutable original Drain required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=candidates[1];
 original:=jsonb_build_object('drain_attempt',drain.id,'drain_command',drain.command_id,'removal_id',removal.id,
  'command_key',recipe_effect_hash_json(command.command_key),'input_hash',recipe_effect_hash_json(removal.input_hash),
  'expected_version',removal.expected_version,'creator',removal.created_by,'request',removal.request_id);
 -- This is exact original DATA discovery for trigger comparison, not an actor.
 PERFORM run_cleanup_wake_closure118(p_run,execution.instance_id,original);
 RETURN original;
END $$;
REVOKE ALL ON FUNCTION invocation119_queued_cleanup_original(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_queued_cleanup_zero_history(execution runs) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 RETURN num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)=0
  AND execution.volume_selection_snapshot IS NULL
  -- Global for this Run: no host/state/expiry/release filtering or old receipt adoption.
  AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_vm_plan_admissions WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_legacy_vm_placements WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_requests WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM mailbox_delivery_attempts WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM agent_updates WHERE hook_run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_instance_provenance WHERE run_id=execution.id)
  -- Canonical119 authority IDs equal this Run UUID. Foreign owners may still
  -- collide with those primary/FK IDs; all statuses and linked history count.
  AND NOT EXISTS(SELECT 1 FROM run_authorization_snapshots WHERE run_id=execution.id OR id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_authorization_snapshot_bindings binding JOIN run_authorization_snapshots snapshot
    ON snapshot.id=binding.snapshot_id WHERE snapshot.run_id=execution.id OR binding.snapshot_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM runtime_authority_sessions WHERE run_id=execution.id OR id=execution.id OR snapshot_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM capability_audit_events WHERE run_id=execution.id
    OR runtime_session_id=execution.id OR snapshot_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_git_authority_snapshots git JOIN run_authorization_snapshots snapshot
    ON snapshot.id=git.snapshot_id WHERE snapshot.run_id=execution.id OR git.snapshot_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM runtime_git_credentials credential
    LEFT JOIN runtime_authority_sessions session ON session.id=credential.runtime_session_id
    LEFT JOIN run_authorization_snapshots snapshot ON snapshot.id=credential.authorization_snapshot_id
    WHERE session.run_id=execution.id OR snapshot.run_id=execution.id
      OR session.snapshot_id=execution.id OR credential.runtime_session_id=execution.id
      OR credential.authorization_snapshot_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM git_receives receive JOIN runtime_authority_sessions session
    ON session.id=receive.runtime_session_id WHERE session.run_id=execution.id
      OR receive.runtime_session_id=execution.id OR session.snapshot_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM secret_runtime_sessions WHERE run_id=execution.id OR id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM secret_runtime_mounts WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_secret_provenance WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM secret_leases WHERE run_id=execution.id OR session_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM secret_audit_events audit LEFT JOIN secret_leases lease ON lease.id=audit.lease_id
    WHERE audit.runtime_run_id=execution.id OR lease.run_id=execution.id OR lease.session_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM brokered_secret_lease_snapshots WHERE run_id=execution.id OR runtime_session_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM brokered_secret_audit_events WHERE run_id=execution.id OR runtime_session_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_workspaces WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_results WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_grants grant_row
    WHERE grant_row.instance_id=execution.instance_id AND NOT EXISTS(
      SELECT 1 FROM agent_instance_volume_mount_revocations WHERE grant_id=grant_row.id))
  AND NOT EXISTS(SELECT 1 FROM runs other WHERE other.instance_id=execution.instance_id
    AND other.id<>execution.id AND other.state<>'cleaned_up');
END $$;
REVOKE ALL ON FUNCTION invocation119_queued_cleanup_zero_history(runs) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_queued_cleanup_base(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea) RETURNS runs
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE;
BEGIN
 PERFORM require_run_cleanup_wake118();
 PERFORM run_cleanup_wake_closure118(p_run,p_instance,p_original);
 SELECT * INTO execution FROM runs WHERE id=p_run;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 IF execution.id IS NULL OR receipt.id IS NULL OR execution.instance_id IS DISTINCT FROM p_instance
  OR execution.state NOT IN ('queued','cancelled','cleaning_up','cleaned_up')
  OR execution.execution_profile IS DISTINCT FROM 'owned_canonical' OR execution.run_kind IS DISTINCT FROM 'invocation'
  OR execution.vm_binding_kind IS DISTINCT FROM 'planned' OR execution.vm_plan_admission_required IS DISTINCT FROM false
  OR ROW(execution.vm_provider_namespace,execution.vm_host_id) IS DISTINCT FROM ROW(p_namespace,p_host)
  OR NOT run_cleanup_identifier_is_valid(execution.vm_id)
  OR NOT run_cleanup_identifier_is_valid(p_namespace) OR NOT run_cleanup_identifier_is_valid(p_host)
  OR p_config IS NULL OR receipt.config_bytes IS DISTINCT FROM p_config
  OR receipt.config_hash IS DISTINCT FROM sha256(receipt.config_bytes)
  OR receipt.config_json IS DISTINCT FROM convert_from(receipt.config_bytes,'UTF8')::jsonb
  OR receipt.config_json->>'version' IS DISTINCT FROM '1' OR receipt.config_json->>'profile' IS DISTINCT FROM 'runtime_named_v1'
  OR receipt.config_json->'vm' IS DISTINCT FROM jsonb_build_object('namespace',p_namespace,'host_id',p_host)
  OR receipt.config_json->'volume'->>'host_id' IS DISTINCT FROM p_host
  OR qualified_invocation_graph_matches(receipt) IS DISTINCT FROM true
  OR invocation119_queued_cleanup_zero_history(execution) IS DISTINCT FROM true
  OR num_nonnulls(execution.exit_code,execution.exit_signal,execution.failure)<>0
  OR (execution.state='queued' AND execution.outcome IS NOT NULL)
  OR (execution.state<>'queued' AND execution.outcome IS DISTINCT FROM 'cancelled')
  OR NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=receipt.start_command_id
    AND command_type='start_run' AND payload=qualified_invocation_start_json(receipt) AND processed_at IS NULL)
  OR (SELECT count(*) FROM outbox WHERE aggregate_id=p_run)<>2
  OR (SELECT count(*) FROM command_inbox WHERE payload->>'run_id'=p_run::text)<>2 THEN
  RAISE EXCEPTION 'positive genuine115 closed-original zero-history cleanup required' USING ERRCODE='P1180'; END IF;
 -- The existing stored planned VM is compared by immutable115; none is minted here.
 RETURN execution;
END $$;
REVOKE ALL ON FUNCTION invocation119_queued_cleanup_base(uuid,uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- Existing115 birth is version0. Actual Cancel writes metadata only, preparation
-- appends sequence1 without changing state/version, and each actual state
-- transition increments version once. Count/compare those exact emissions.
CREATE FUNCTION invocation119_queued_cleanup_events(execution runs,p_prepared boolean) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE phases integer; expected jsonb;
BEGIN
 PERFORM require_run_cleanup_wake118();
 phases:=CASE execution.state WHEN 'queued' THEN 0 WHEN 'cancelled' THEN 1
   WHEN 'cleaning_up' THEN 2 WHEN 'cleaned_up' THEN 3 ELSE -1 END;
 IF phases<0 OR execution.state_version IS DISTINCT FROM phases::bigint THEN RETURN false; END IF;
 IF NOT p_prepared THEN
  RETURN phases=0 AND NOT EXISTS(SELECT 1 FROM run_events WHERE run_id=execution.id);
 END IF;
 expected:=jsonb_build_array(jsonb_build_object('type','run.cleanup_started','payload',
    jsonb_build_object('run_id',execution.id,'generation',1,'lease_count',0)));
 IF phases>=1 THEN expected:=expected||jsonb_build_array(jsonb_build_object('type','run.cancelled','payload',
    jsonb_build_object('run_id',execution.id,'state','cancelled','outcome','cancelled','exit',NULL,'failure',NULL))); END IF;
 IF phases>=2 THEN expected:=expected||jsonb_build_array(jsonb_build_object('type','run.cleaning_up','payload',
    jsonb_build_object('run_id',execution.id,'state','cleaning_up','outcome',NULL,'exit',NULL,'failure',NULL))); END IF;
 IF phases>=3 THEN expected:=expected||jsonb_build_array(jsonb_build_object('type','run.cleaned_up','payload',
    jsonb_build_object('run_id',execution.id,'state','cleaned_up','cleanup_generation',1))); END IF;
 RETURN (SELECT count(*) FROM run_events WHERE run_id=execution.id)=jsonb_array_length(expected)
  AND NOT EXISTS(SELECT 1 FROM run_events event WHERE event.run_id=execution.id
    AND (event.sequence NOT BETWEEN 1 AND jsonb_array_length(expected)
      OR event.event_type IS DISTINCT FROM (expected->(CASE WHEN event.sequence BETWEEN 1 AND 4 THEN event.sequence::integer-1 ELSE -1 END))->>'type'
      OR event.payload IS DISTINCT FROM (expected->(CASE WHEN event.sequence BETWEEN 1 AND 4 THEN event.sequence::integer-1 ELSE -1 END))->'payload'));
END $$;
REVOKE ALL ON FUNCTION invocation119_queued_cleanup_events(runs,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_queued_cleanup_candidate(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea) RETURNS runs
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
 PERFORM require_run_cleanup_wake118();
 execution:=invocation119_queued_cleanup_base(p_run,p_instance,p_original,p_namespace,p_host,p_config);
 IF execution.state IS DISTINCT FROM 'queued' OR execution.volume_acquisition_closed_at IS NOT NULL
  OR invocation119_queued_cleanup_events(execution,false) IS DISTINCT FROM true
  OR EXISTS(SELECT 1 FROM run_cleanup_targets WHERE run_id=p_run)
  OR EXISTS(SELECT 1 FROM run_cleanup_target_leases WHERE run_id=p_run)
  OR EXISTS(SELECT 1 FROM run_cleanup_receipts WHERE run_id=p_run)
  OR EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=p_run) THEN
  RAISE EXCEPTION 'open genuine queued cleanup candidate required' USING ERRCODE='P1180'; END IF;
 RETURN execution;
END $$;
REVOKE ALL ON FUNCTION invocation119_queued_cleanup_candidate(uuid,uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_cleanup_only_matches(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea,p_completing boolean) RETURNS runs
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; target run_cleanup_targets%ROWTYPE; receipt run_cleanup_receipts%ROWTYPE;
 done run_cleanup_completions%ROWTYPE;
BEGIN
 PERFORM require_run_cleanup_wake118();
 execution:=invocation119_queued_cleanup_base(p_run,p_instance,p_original,p_namespace,p_host,p_config);
 SELECT * INTO target FROM run_cleanup_targets WHERE run_id=p_run;
 SELECT * INTO receipt FROM run_cleanup_receipts WHERE run_id=p_run AND generation=1;
 SELECT * INTO done FROM run_cleanup_completions WHERE run_id=p_run AND generation=1;
 IF p_completing IS NULL OR execution.volume_acquisition_closed_at IS NULL OR target.run_id IS NULL
  OR target.generation IS DISTINCT FROM 1::bigint OR target.lease_count IS DISTINCT FROM 0
  OR target.target_digest IS DISTINCT FROM run_cleanup_target_digest(p_run,target.generation)
  OR ROW(target.instance_id,target.instance_revision_id,target.vm_target_kind,target.vm_provider_namespace,target.vm_host_id,target.vm_id)
    IS DISTINCT FROM ROW(execution.instance_id,execution.instance_revision_id,execution.vm_binding_kind,
      execution.vm_provider_namespace,execution.vm_host_id,execution.vm_id)
  OR EXISTS(SELECT 1 FROM run_cleanup_target_leases WHERE run_id=p_run)
  OR invocation119_queued_cleanup_events(execution,true) IS DISTINCT FROM true
  OR (SELECT count(*) FROM run_cleanup_receipts WHERE run_id=p_run)<>(CASE WHEN receipt.run_id IS NULL THEN 0 ELSE 1 END)
  OR (SELECT count(*) FROM run_cleanup_completions WHERE run_id=p_run)<>(CASE WHEN done.run_id IS NULL THEN 0 ELSE 1 END)
  OR (receipt.run_id IS NOT NULL AND (receipt.target_digest IS DISTINCT FROM target.target_digest
    OR ROW(receipt.vm_provider_namespace,receipt.vm_host_id,receipt.vm_id)
      IS DISTINCT FROM ROW(target.vm_provider_namespace,target.vm_host_id,target.vm_id)
    OR receipt.observation_kind NOT IN ('destroyed','authoritatively_absent')))
  OR (execution.state='queued' AND receipt.run_id IS NOT NULL)
  OR (execution.state IN ('cleaning_up','cleaned_up') AND receipt.run_id IS NULL)
  OR (p_completing AND execution.state IS DISTINCT FROM 'cleaning_up') THEN
  RAISE EXCEPTION 'exact closed queued zero-target cleanup proof required' USING ERRCODE='P1180'; END IF;
 IF execution.state='cleaned_up' OR p_completing THEN
  IF receipt.run_id IS NULL OR done.run_id IS NULL OR done.target_digest IS DISTINCT FROM target.target_digest
    OR run_cleanup_set_matches(p_run,target.generation,true) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'original scoped receipt zero-set completion required' USING ERRCODE='P1180'; END IF;
 ELSE
  IF done.run_id IS NOT NULL OR run_cleanup_set_matches(p_run,target.generation,false) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'unfinished closed zero-target cleanup proof required' USING ERRCODE='P1180'; END IF;
 END IF;
 RETURN execution;
END $$;
REVOKE ALL ON FUNCTION invocation119_cleanup_only_matches(uuid,uuid,jsonb,text,text,bytea,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION qualify_invocation119_queued_cleanup(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea,p_preparing boolean,p_completing boolean) RETURNS runs
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 IF p_preparing IS NULL OR p_completing IS NULL OR (p_preparing AND p_completing) THEN
  RAISE EXCEPTION 'explicit queued cleanup boundary required' USING ERRCODE='P1180'; END IF;
 IF p_preparing AND NOT EXISTS(SELECT 1 FROM run_cleanup_targets WHERE run_id=p_run) THEN
  RETURN invocation119_queued_cleanup_candidate(p_run,p_instance,p_original,p_namespace,p_host,p_config);
 END IF;
 RETURN invocation119_cleanup_only_matches(p_run,p_instance,p_original,p_namespace,p_host,p_config,p_completing);
END $$;
REVOKE ALL ON FUNCTION qualify_invocation119_queued_cleanup(uuid,uuid,jsonb,text,text,bytea,boolean,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION load_queued_run_cleanup_candidate119(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea) RETURNS SETOF runs
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 RETURN NEXT invocation119_queued_cleanup_candidate(p_run,p_instance,p_original,p_namespace,p_host,p_config);
END $$;
REVOKE ALL ON FUNCTION load_queued_run_cleanup_candidate119(uuid,uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION load_queued_run_cleanup_candidate119(uuid,uuid,jsonb,text,text,bytea) TO hephaestus_worker;

CREATE FUNCTION load_invocation119_cleanup_only(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea) RETURNS SETOF runs
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 RETURN NEXT invocation119_cleanup_only_matches(p_run,p_instance,p_original,p_namespace,p_host,p_config,false);
END $$;
REVOKE ALL ON FUNCTION load_invocation119_cleanup_only(uuid,uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION load_invocation119_cleanup_only(uuid,uuid,jsonb,text,text,bytea) TO hephaestus_worker;

CREATE FUNCTION invocation119_queued_cleanup_reservations(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea,p_preparing boolean,p_completing boolean)
RETURNS TABLE(lock_class integer,command_mutex bigint,first_mutex integer,second_mutex integer)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; execution runs%ROWTYPE; extra bytea;
BEGIN
 PERFORM require_run_cleanup_wake118();
 execution:=qualify_invocation119_queued_cleanup(p_run,p_instance,p_original,p_namespace,p_host,p_config,p_preparing,p_completing);
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 SELECT command_key INTO extra FROM release_command_inbox WHERE secondary_id=(p_original->>'removal_id')::uuid AND operation='request_instance_removal';
 RETURN QUERY SELECT 0,keys.key,NULL::integer,NULL::integer FROM (
   SELECT recipe_import107_command_mutex(prepared.import_command_key) key FROM recipe_prepared_instance_imports prepared WHERE prepared.deployment_id=child.deployment_id
   UNION SELECT recipe_import107_command_mutex(extra)
   UNION SELECT recipe_import107_command_mutex(receipt.command_key) FROM agent_instance_execution_invocations receipt WHERE receipt.run_id=p_run) keys ORDER BY keys.key;
 RETURN QUERY SELECT DISTINCT 1,NULL::bigint,mutex.first_key,mutex.second_key
  FROM recipe_prepared_instance_imports prepared CROSS JOIN LATERAL recipe_import113_instance_mutex(prepared.instance_id) mutex
  WHERE prepared.deployment_id=child.deployment_id ORDER BY mutex.first_key,mutex.second_key;
END $$;
REVOKE ALL ON FUNCTION invocation119_queued_cleanup_reservations(uuid,uuid,jsonb,text,text,bytea,boolean,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- Owning adapters may wait on this complete sorted set ONLY before parent rows.
CREATE FUNCTION queued_run_cleanup_reservations119(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea)
RETURNS TABLE(lock_class integer,command_mutex bigint,first_mutex integer,second_mutex integer)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 RETURN QUERY SELECT * FROM invocation119_queued_cleanup_reservations(p_run,p_instance,p_original,p_namespace,p_host,p_config,true,false)
  ORDER BY lock_class,command_mutex,first_mutex,second_mutex;
END $$;
REVOKE ALL ON FUNCTION queued_run_cleanup_reservations119(uuid,uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION queued_run_cleanup_reservations119(uuid,uuid,jsonb,text,text,bytea) TO hephaestus_worker;

CREATE FUNCTION lock_invocation119_queued_cleanup(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea,p_preparing boolean,p_completing boolean) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE early record; child recipe_prepared_instance_imports%ROWTYPE; extra bytea;
BEGIN
 PERFORM require_run_cleanup_wake118();
 FOR early IN SELECT * FROM invocation119_queued_cleanup_reservations(p_run,p_instance,p_original,p_namespace,p_host,p_config,p_preparing,p_completing)
   ORDER BY lock_class,command_mutex,first_mutex,second_mutex LOOP
  IF early.lock_class=0 THEN
   IF pg_try_advisory_xact_lock(early.command_mutex) IS DISTINCT FROM true THEN
    RAISE EXCEPTION 'queued original command reservation held' USING ERRCODE='55P03'; END IF;
  ELSE
   IF pg_try_advisory_xact_lock(early.first_mutex,early.second_mutex) IS DISTINCT FROM true THEN
    RAISE EXCEPTION 'queued original instance reservation held' USING ERRCODE='55P03'; END IF;
  END IF;
 END LOOP;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 SELECT command_key INTO extra FROM release_command_inbox WHERE secondary_id=(p_original->>'removal_id')::uuid AND operation='request_instance_removal';
 -- Frozen helpers repeat TRY reservations then parent Project/Deployment/ALL
 -- Resources and Instance/ALL Runs/sorted Volumes/grants/remaining late NOWAIT.
 PERFORM lock_prepared_closure_parents118(p_instance,extra);
 PERFORM lock_prepared_closure_participants118(p_instance,child.project_id);
 PERFORM run_id FROM run_cleanup_targets WHERE run_id=p_run FOR UPDATE NOWAIT;
 PERFORM lease_id FROM run_cleanup_target_leases WHERE run_id=p_run ORDER BY volume_id,lease_id FOR UPDATE NOWAIT;
 PERFORM run_id FROM run_cleanup_receipts WHERE run_id=p_run ORDER BY generation FOR UPDATE NOWAIT;
 PERFORM run_id FROM run_cleanup_completions WHERE run_id=p_run ORDER BY generation FOR UPDATE NOWAIT;
 PERFORM command_id FROM command_inbox WHERE payload->>'run_id'=p_run::text ORDER BY command_id FOR UPDATE NOWAIT;
 PERFORM id FROM outbox WHERE aggregate_id=p_run ORDER BY id FOR UPDATE NOWAIT;
 PERFORM qualify_invocation119_queued_cleanup(p_run,p_instance,p_original,p_namespace,p_host,p_config,p_preparing,p_completing);
 -- Closure and target freezing stay in the existing owning transaction tail.
END $$;
REVOKE ALL ON FUNCTION lock_invocation119_queued_cleanup(uuid,uuid,jsonb,text,text,bytea,boolean,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION lock_prepare_queued_run_cleanup_wake119(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 PERFORM lock_invocation119_queued_cleanup(p_run,p_instance,p_original,p_namespace,p_host,p_config,true,false);
END $$;
REVOKE ALL ON FUNCTION lock_prepare_queued_run_cleanup_wake119(uuid,uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION lock_prepare_queued_run_cleanup_wake119(uuid,uuid,jsonb,text,text,bytea) TO hephaestus_worker;

CREATE FUNCTION lock_invocation119_cleanup_only(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 PERFORM lock_invocation119_queued_cleanup(p_run,p_instance,p_original,p_namespace,p_host,p_config,false,false);
END $$;
REVOKE ALL ON FUNCTION lock_invocation119_cleanup_only(uuid,uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION lock_invocation119_cleanup_only(uuid,uuid,jsonb,text,text,bytea) TO hephaestus_worker;

-- Trigger-only comparisons deliberately supply saved115 bytes as DATA. Actual
-- worker/provider composition is independently mandatory through the six-arg
-- facades before hydration/physical IO; this helper confers no such authority.
CREATE FUNCTION assert_invocation119_queued_cleanup_transition(p_run uuid,p_next text) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE; original jsonb; completing boolean;
BEGIN
 PERFORM require_run_cleanup_wake118();
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.id IS NULL OR execution.state NOT IN ('queued','cancelled','cleaning_up','cleaned_up')
  OR p_next IS NULL OR NOT (p_next=execution.state OR (execution.state='queued' AND p_next='cancelled')
    OR (execution.state='cancelled' AND p_next='cleaning_up') OR (execution.state='cleaning_up' AND p_next='cleaned_up')) THEN
  RAISE EXCEPTION 'only closed queued cancellation and cleanup progress permitted' USING ERRCODE='23000'; END IF;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 original:=invocation119_queued_cleanup_original(p_run);
 completing:=execution.state='cleaning_up' AND p_next='cleaned_up';
 PERFORM lock_invocation119_queued_cleanup(p_run,execution.instance_id,original,execution.vm_provider_namespace,
   execution.vm_host_id,receipt.config_bytes,false,completing);
 IF p_next='cleaning_up' AND NOT EXISTS(SELECT 1 FROM run_cleanup_receipts WHERE run_id=p_run AND generation=1) THEN
  RAISE EXCEPTION 'cleanup progress requires actual scoped observation' USING ERRCODE='23000'; END IF;
END $$;
REVOKE ALL ON FUNCTION assert_invocation119_queued_cleanup_transition(uuid,text) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- Deferred Run-only validation precedes the original unchanged-state shortcut.
-- Lease INSERT/refresh paths never dispatch here and retain frozen119 invariants.
CREATE FUNCTION validate_invocation119_queued_cleanup(p_old runs,p_new runs) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE; original jsonb;
BEGIN
 PERFORM require_run_cleanup_wake118();
 SELECT * INTO execution FROM runs WHERE id=p_new.id;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_new.id;
 original:=invocation119_queued_cleanup_original(p_new.id);
 PERFORM lock_invocation119_queued_cleanup(p_new.id,execution.instance_id,original,execution.vm_provider_namespace,
   execution.vm_host_id,receipt.config_bytes,false,false);
 IF p_old.run_kind IS DISTINCT FROM 'invocation' OR p_new.run_kind IS DISTINCT FROM 'invocation'
  OR (to_jsonb(p_new)-ARRAY['state','outcome','state_version','updated_at','volume_acquisition_closed_at'])
    IS DISTINCT FROM (to_jsonb(p_old)-ARRAY['state','outcome','state_version','updated_at','volume_acquisition_closed_at'])
  OR p_new.state NOT IN ('queued','cancelled','cleaning_up','cleaned_up')
  OR (p_new.state='queued' AND p_new.outcome IS NOT NULL)
  OR (p_new.state<>'queued' AND p_new.outcome IS DISTINCT FROM 'cancelled') THEN
  RAISE EXCEPTION 'queued cleanup metadata cannot change producer or outcome authority' USING ERRCODE='23000'; END IF;
 IF p_new.state IS DISTINCT FROM p_old.state THEN
  IF NOT ((p_old.state='queued' AND p_new.state='cancelled') OR (p_old.state='cancelled' AND p_new.state='cleaning_up')
    OR (p_old.state='cleaning_up' AND p_new.state='cleaned_up'))
   OR p_new.state_version IS DISTINCT FROM p_old.state_version+1
   OR p_new.volume_acquisition_closed_at IS DISTINCT FROM p_old.volume_acquisition_closed_at THEN
   RAISE EXCEPTION 'queued cleanup requires exact normal transition/version and permanent closure' USING ERRCODE='23000'; END IF;
 ELSE
  IF p_new.state_version IS DISTINCT FROM p_old.state_version OR p_new.outcome IS DISTINCT FROM p_old.outcome
    OR (p_new.volume_acquisition_closed_at IS DISTINCT FROM p_old.volume_acquisition_closed_at
      AND NOT (p_old.state='queued' AND p_old.volume_acquisition_closed_at IS NULL
        AND p_new.volume_acquisition_closed_at IS NOT NULL)) THEN
   RAISE EXCEPTION 'queued preparation cannot stamp state and replay cannot change history' USING ERRCODE='23000'; END IF;
 END IF;
END $$;
REVOKE ALL ON FUNCTION validate_invocation119_queued_cleanup(runs,runs) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- Static successor; remove ONLY marked dispatch to recover exact frozen body.
CREATE OR REPLACE FUNCTION enforce_instance_run_progress() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE config bytea;
BEGIN
    IF OLD.run_kind='invocation' AND NEW.state IS DISTINCT FROM OLD.state THEN
        PERFORM require_invocation117_worker();
        -- BEGIN queued119 cleanup-only dispatch.
        IF OLD.volume_acquisition_closed_at IS NOT NULL
          AND NOT EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=OLD.id) THEN
            PERFORM assert_invocation119_queued_cleanup_transition(OLD.id,NEW.state);
            RETURN NEW;
        END IF;
        -- END queued119 cleanup-only dispatch.
        SELECT config_bytes INTO config FROM run_invocation_consumptions WHERE run_id=OLD.id;
        PERFORM lock_qualified_invocation119(OLD.id,config);
        PERFORM assert_invocation119_transition(OLD.id,NEW.state);
        RETURN NEW;
    END IF;
    IF NEW.state IS DISTINCT FROM OLD.state AND NEW.state IN ('leasing_volume','provisioning','starting','running') THEN
        PERFORM assert_instance_accepts_work(NEW.instance_id);
    END IF;
    RETURN NEW;
END $$;

-- Static successor; remove ONLY marked dispatch to recover exact frozen body.
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
  RAISE EXCEPTION 'Invocation actual exit/result integration is not provided by119-B1' USING ERRCODE='0A000';
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

-- Static successor; remove ONLY marked dispatch to recover exact frozen body.
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
