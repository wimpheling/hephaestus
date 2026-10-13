-- PRIVATE119 queued Cancel delivery, AFTER frozen119/118/14e83 queued proofs.
-- SOURCE ONLY: no original fragment, trigger or ACL is replaced.
-- Exact original actor/request IDs are comparison data, never current identity.
-- No117, Start processing, execution/session/lease/target/state authority.

CREATE FUNCTION invocation119_cancel_closure(p_run uuid,p_instance uuid,p_original jsonb,p_cancel jsonb) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
 drain recipe_effect_attempts%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 removal agent_instance_removal_requests%ROWTYPE; inbox release_command_inbox%ROWTYPE;
 expected jsonb; cancel jsonb; admission recipe_remove_resume_admissions%ROWTYPE; recorded jsonb;
BEGIN
 PERFORM require_run_cleanup_wake118();
 -- Global Run selection precedes ownership checks; no filtered absence inference.
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.id IS NULL OR execution.instance_id IS DISTINCT FROM p_instance OR execution.cancel_requested_at IS NULL THEN
  RAISE EXCEPTION 'exact original cancelled Run required' USING ERRCODE='P1180'; END IF;
 PERFORM run_cleanup_wake_original118(p_instance);
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=(p_original->>'drain_attempt')::uuid;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=drain.command_id;
 SELECT * INTO removal FROM agent_instance_removal_requests WHERE id=(p_original->>'removal_id')::uuid;
 IF (SELECT count(*) FROM agent_instance_removal_requests WHERE instance_id=p_instance)<>1
  OR (SELECT count(*) FROM release_command_inbox WHERE secondary_id=removal.id AND operation='request_instance_removal')<>1 THEN
  RAISE EXCEPTION 'unique original permanent closure required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO inbox FROM release_command_inbox WHERE secondary_id=removal.id AND operation='request_instance_removal';
 expected:=jsonb_build_object('drain_attempt',drain.id,'drain_command',drain.command_id,'removal_id',removal.id,
  'command_key',recipe_effect_hash_json(inbox.command_key),'input_hash',recipe_effect_hash_json(removal.input_hash),
  'expected_version',removal.expected_version,'creator',removal.created_by,'request',removal.request_id);
 IF p_original IS DISTINCT FROM expected OR drain.id IS NULL OR removal.id IS NULL
  OR drain.action IS DISTINCT FROM 'drain' OR command.operation IS DISTINCT FROM 'remove'
  OR command.deployment_id IS DISTINCT FROM child.deployment_id OR command.project_id IS DISTINCT FROM child.project_id
  OR command.actor_id IS DISTINCT FROM drain.actor_id OR drain.actor_id IS DISTINCT FROM removal.created_by
  OR drain.deployment_id IS DISTINCT FROM child.deployment_id OR drain.resource_name IS DISTINCT FROM child.resource_name
  OR drain.project_id IS DISTINCT FROM child.project_id
  OR drain.input_hash IS DISTINCT FROM (SELECT input_hash FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name)
  OR drain.claim_json->'identity' IS DISTINCT FROM (SELECT plan_json->'identity' FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name)
  OR removal.instance_id IS DISTINCT FROM p_instance OR removal.expected_version<1
  OR original_drain_removal118(drain.id,p_instance) IS DISTINCT FROM removal.id
  OR original_drain_command_key118(drain.id,p_instance) IS DISTINCT FROM inbox.command_key
  OR inbox.aggregate_id IS DISTINCT FROM p_instance OR inbox.actor_id IS DISTINCT FROM removal.created_by
  OR inbox.request_id IS DISTINCT FROM removal.request_id OR inbox.input_hash IS DISTINCT FROM removal.input_hash
  OR removal.input_hash IS DISTINCT FROM sha256(convert_to(recipe_import107_json_bytes(jsonb_build_object('version',1,'operation','request_instance_removal',
    'actor',drain.actor_id,'instance',p_instance,'removal',removal.id,'expected_version',removal.expected_version),0),'UTF8'))
  OR prepared_instance_removal118(p_instance) IS DISTINCT FROM removal.id
  OR NOT EXISTS(SELECT 1 FROM recipe_execution_transitions claimed JOIN application_events event ON event.id=claimed.event_id
   JOIN product_event_outbox product ON product.event_id=event.id
   WHERE claimed.attempt_id=drain.id AND claimed.kind='claimed' AND claimed.command_id=drain.command_id
    AND claimed.project_id=drain.project_id AND claimed.resource_name=drain.resource_name
    AND claimed.operation_id=drain.id AND claimed.fingerprint=drain.request_fingerprint AND claimed.before_state=drain.before_state
    AND claimed.after_state->>'active_attempt'=drain.id::text AND claimed.after_state->>'generation'=drain.generation::text
    AND event.actor_id=drain.actor_id AND event.request_id=drain.request_id
    AND event.cursor=claimed.event_cursor AND event.aggregate_version=claimed.event_aggregate_version) THEN
  RAISE EXCEPTION 'immutable original Drain and closure correlation required' USING ERRCODE='P1180'; END IF;
 -- A fresh B continuation may preserve A's closure but cannot replace its pins.
 FOR admission IN SELECT * FROM recipe_remove_resume_admissions WHERE deployment_id=child.deployment_id LOOP
  IF (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=child.resource_name)>1 THEN
   RAISE EXCEPTION 'ambiguous admitted original closure' USING ERRCODE='P1180'; END IF;
  SELECT item INTO recorded FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=child.resource_name;
  IF recorded IS NOT NULL AND recorded IS DISTINCT FROM jsonb_build_object('resource',child.resource_name,'drain_claim',drain.claim_json,
    'actor',command.actor_id,'idempotency',command.idempotency_id,'removal',to_jsonb(removal),'inbox',to_jsonb(inbox)) THEN
   RAISE EXCEPTION 'admitted original closure changed' USING ERRCODE='P1180'; END IF;
 END LOOP;
 cancel:=jsonb_build_object('command_id',run_cleanup_wake_cancel118(removal.id,p_run),'run_id',p_run,'reason','instance removal requested');
 -- Exact typed DTO and committed transport correlation; publisher ACK is not authority.
 IF p_cancel IS DISTINCT FROM cancel THEN
  RAISE EXCEPTION 'exact original typed Cancel required' USING ERRCODE='P1180'; END IF;
 IF (SELECT count(*) FROM outbox WHERE aggregate_type='release' AND aggregate_id=p_run
    AND subject='heph.run.command.cancel.v1' AND event_type='run.cancel_requested.v1'
    AND payload=cancel||jsonb_build_object('schema_version',1,'message_id',id,'idempotency_key',id,'request_id',NULL,'trace_id',NULL))<>1 THEN
  RAISE EXCEPTION 'one exact committed original Cancel outbox required' USING ERRCODE='P1180'; END IF;
 -- First delivery is absent; only an exact already-processed row is a duplicate.
 -- An unprocessed conflict cannot be completed by the frozen ON CONFLICT tail.
 IF EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(cancel->>'command_id')::uuid)
  AND NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(cancel->>'command_id')::uuid
    AND command_type='cancel_run' AND payload=cancel AND processed_at IS NOT NULL) THEN
  RAISE EXCEPTION 'original Cancel inbox conflict or pending delivery is held' USING ERRCODE='P1180'; END IF;
END $$;
REVOKE ALL ON FUNCTION invocation119_cancel_closure(uuid,uuid,jsonb,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_cancel_original(p_run uuid,p_cancel jsonb) RETURNS jsonb
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
 PERFORM invocation119_cancel_closure(p_run,execution.instance_id,original,p_cancel);
 RETURN original;
END $$;
REVOKE ALL ON FUNCTION invocation119_cancel_original(uuid,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_cancel_first(p_run uuid,p_instance uuid,p_original jsonb,p_cancel jsonb,p_namespace text,p_host text,p_config bytea) RETURNS runs
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE;
BEGIN
 PERFORM require_run_cleanup_wake118();
 PERFORM invocation119_cancel_closure(p_run,p_instance,p_original,p_cancel);
 SELECT * INTO execution FROM runs WHERE id=p_run;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 IF execution.id IS NULL OR receipt.id IS NULL OR execution.instance_id IS DISTINCT FROM p_instance
  OR execution.state IS DISTINCT FROM 'queued'
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
  OR (SELECT count(*) FROM command_inbox WHERE payload->>'run_id'=p_run::text)<>1
  OR execution.volume_acquisition_closed_at IS NOT NULL
  OR invocation119_queued_cleanup_events(execution,false) IS DISTINCT FROM true
  OR EXISTS(SELECT 1 FROM run_cleanup_targets WHERE run_id=p_run)
  OR EXISTS(SELECT 1 FROM run_cleanup_target_leases WHERE run_id=p_run)
  OR EXISTS(SELECT 1 FROM run_cleanup_receipts WHERE run_id=p_run)
  OR EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=p_run) THEN
  RAISE EXCEPTION 'positive genuine115 closed-original zero-history cleanup required' USING ERRCODE='P1180'; END IF;
 -- The existing stored planned VM is compared by immutable115; none is minted here.
 RETURN execution;
END $$;
REVOKE ALL ON FUNCTION invocation119_cancel_first(uuid,uuid,jsonb,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_cancel_qualify(p_run uuid,p_cancel jsonb,p_namespace text,p_host text,p_config bytea) RETURNS runs
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; original jsonb;
BEGIN
 PERFORM require_run_cleanup_wake118();
 original:=invocation119_cancel_original(p_run,p_cancel);
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(p_cancel->>'command_id')::uuid) THEN
  -- The independent closure check already required exact processed DTO evidence.
  RETURN qualify_invocation119_queued_cleanup(p_run,execution.instance_id,original,p_namespace,p_host,p_config,true,false);
 END IF;
 RETURN invocation119_cancel_first(p_run,execution.instance_id,original,p_cancel,p_namespace,p_host,p_config);
END $$;

REVOKE ALL ON FUNCTION invocation119_cancel_qualify(uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION queued_cancel_reservations119(p_run uuid,p_cancel jsonb,p_namespace text,p_host text,p_config bytea)
RETURNS TABLE(lock_class integer,command_mutex bigint,first_mutex integer,second_mutex integer)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; execution runs%ROWTYPE; extra bytea; p_original jsonb; p_instance uuid;
BEGIN
 PERFORM require_run_cleanup_wake118();
 IF current_setting('transaction_isolation') IS DISTINCT FROM 'read committed'
  OR current_setting('transaction_read_only') IS DISTINCT FROM 'off' THEN
  RAISE EXCEPTION 'queued Cancel requires writable read-committed transaction' USING ERRCODE='P1180'; END IF;
 execution:=invocation119_cancel_qualify(p_run,p_cancel,p_namespace,p_host,p_config);
 p_instance:=execution.instance_id;
 p_original:=invocation119_cancel_original(p_run,p_cancel);
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
REVOKE ALL ON FUNCTION queued_cancel_reservations119(uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION queued_cancel_reservations119(uuid,jsonb,text,text,bytea) TO hephaestus_worker;

CREATE FUNCTION lock_queued_cancel_delivery119(p_run uuid,p_cancel jsonb,p_namespace text,p_host text,p_config bytea) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE early record; child recipe_prepared_instance_imports%ROWTYPE; extra bytea; execution runs%ROWTYPE; p_original jsonb; p_instance uuid;
BEGIN
 PERFORM require_run_cleanup_wake118();
 IF current_setting('transaction_isolation') IS DISTINCT FROM 'read committed'
  OR current_setting('transaction_read_only') IS DISTINCT FROM 'off' THEN
  RAISE EXCEPTION 'queued Cancel requires writable read-committed transaction' USING ERRCODE='P1180'; END IF;
 -- Bound uniqueness waits in the unchanged inbox INSERT after parent locks.
 -- Preserve any existing shorter positive operator timeout; zero is disabled.
 IF current_setting('lock_timeout')::interval=interval '0'
  OR current_setting('lock_timeout')::interval>interval '2 seconds' THEN
  PERFORM set_config('lock_timeout','2s',true);
 END IF;
 execution:=invocation119_cancel_qualify(p_run,p_cancel,p_namespace,p_host,p_config);
 p_instance:=execution.instance_id;
 p_original:=invocation119_cancel_original(p_run,p_cancel);
 FOR early IN SELECT * FROM queued_cancel_reservations119(p_run,p_cancel,p_namespace,p_host,p_config)
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
 PERFORM invocation119_cancel_qualify(p_run,p_cancel,p_namespace,p_host,p_config);
 -- The only mutation remains the unchanged owning Cancel inbox/timestamp tail.
END $$;
REVOKE ALL ON FUNCTION lock_queued_cancel_delivery119(uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION lock_queued_cancel_delivery119(uuid,jsonb,text,text,bytea) TO hephaestus_worker;

CREATE FUNCTION verify_queued_cancel_delivery119(p_run uuid,p_cancel jsonb,p_namespace text,p_host text,p_config bytea) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 IF current_setting('transaction_isolation') IS DISTINCT FROM 'read committed'
  OR current_setting('transaction_read_only') IS DISTINCT FROM 'off' THEN
  RAISE EXCEPTION 'queued Cancel requires writable read-committed transaction' USING ERRCODE='P1180'; END IF;
 -- Fresh same-transaction proof after either INSERT or ON CONFLICT. No locks
 -- or early waits are repeated while parent rows are already held.
 PERFORM invocation119_cancel_qualify(p_run,p_cancel,p_namespace,p_host,p_config);
 IF NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(p_cancel->>'command_id')::uuid
   AND command_type='cancel_run' AND payload=p_cancel AND processed_at IS NOT NULL) THEN
  RAISE EXCEPTION 'post-tail exact processed original Cancel required' USING ERRCODE='P1180'; END IF;
END $$;
REVOKE ALL ON FUNCTION verify_queued_cancel_delivery119(uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION verify_queued_cancel_delivery119(uuid,jsonb,text,text,bytea) TO hephaestus_worker;
