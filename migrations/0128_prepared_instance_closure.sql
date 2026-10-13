-- PRIVATE118 additive target loading and parent-first closure qualification.
-- Append after the immutable read-only observer fragment. No new tables.
CREATE FUNCTION prepared_original_attempt118(p_deployment uuid,p_resource text) RETURNS uuid
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE candidates uuid[];
BEGIN
 SELECT array_agg(attempt.id ORDER BY attempt.id) INTO candidates
 FROM recipe_deployment_resources resource
 JOIN recipe_deployment_execution_preparations header ON header.deployment_id=resource.deployment_id
 JOIN recipe_prepared_instance_imports child ON child.deployment_id=resource.deployment_id AND child.resource_name=resource.resource_name
 JOIN recipe_effect_attempts attempt ON attempt.deployment_id=resource.deployment_id
   AND attempt.resource_name=resource.resource_name AND attempt.command_id=header.command_id AND attempt.action='create'
 WHERE resource.deployment_id=p_deployment AND resource.resource_name=p_resource
   AND resource.resource_kind='instance' AND resource.ownership='owned'
   AND (resource.active_attempt_id=attempt.id OR prepared_instance_created118(child.instance_id,attempt.id) IS TRUE);
 IF COALESCE(cardinality(candidates),0)<>1 THEN
  RAISE EXCEPTION 'one exact active or completed original creation required' USING ERRCODE='P1180'; END IF;
 RETURN candidates[1];
END $$;
CREATE FUNCTION load_prepared_original_target118(p_command uuid,p_deployment uuid,p_resource text,p_project uuid,p_intent_hash bytea)
RETURNS TABLE(claim_json jsonb,original_actor uuid,original_idempotency uuid,canonical_input bytea,preparation_hash bytea)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE original uuid; child recipe_prepared_instance_imports%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app' THEN
  RAISE EXCEPTION 'current manager actor pool required' USING ERRCODE='42501'; END IF;
 IF NOT EXISTS(SELECT 1 FROM recipe_deployment_commands command JOIN users actor ON actor.id=command.actor_id
   WHERE command.id=p_command AND command.operation='remove' AND command.deployment_id=p_deployment
    AND command.project_id=p_project AND actor.status='active' AND actor.id::text=hephaestus_actor_id()
    AND command.idempotency_id::text=current_setting('hephaestus.occurrence_id',true)
    AND check_permission('user',actor.id::text,'can_manage','project',p_project::text)=1) THEN
  RAISE EXCEPTION 'fresh exact current manager required before original lookup' USING ERRCODE='42501'; END IF;
 original:=prepared_original_attempt118(p_deployment,p_resource);
 PERFORM prepared_instance_actor118(p_command,original);
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=p_deployment;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE deployment_id=p_deployment AND resource_name=p_resource;
 IF header.project_id IS DISTINCT FROM p_project OR header.intent_hash IS DISTINCT FROM p_intent_hash
  OR child.command_id IS DISTINCT FROM header.command_id
  OR NOT EXISTS(SELECT 1 FROM recipe_deployments WHERE id=p_deployment AND project_id=p_project AND input_hash=p_intent_hash) THEN
  RAISE EXCEPTION 'complete stored intent comparison required' USING ERRCODE='P1180'; END IF;
 PERFORM prepared_instance_target118(original,(SELECT attempt.claim_json FROM recipe_effect_attempts attempt WHERE id=original),convert_from(child.canonical_import,'UTF8')::jsonb);
 RETURN QUERY SELECT attempt.claim_json,command.actor_id,command.idempotency_id,header.canonical_input,header.input_hash
  FROM recipe_effect_attempts attempt JOIN recipe_deployment_commands command ON command.id=attempt.command_id
  WHERE attempt.id=original;
END $$;

-- Invoked by raw insertion and by the qualified owning adapter. Trusted fresh
-- adapter waits on these SAME advisory identities before any row lock; raw
-- callers use try-locks and all parent/late rows NOWAIT, never inverse waits.
CREATE FUNCTION lock_prepared_closure_parents118(p_instance uuid,p_extra_key bytea) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; mutex record; extra_mutex bigint;
BEGIN
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 IF child.instance_id IS NULL THEN RAISE EXCEPTION 'prepared original instance required' USING ERRCODE='P1180'; END IF;
 -- Raw generic removal has no extra command key. Evaluate the strict immutable
 -- helper only inside this branch; a SELECT filter can run after constant folding.
 IF p_extra_key IS NOT NULL THEN
  extra_mutex:=recipe_import107_command_mutex(p_extra_key);
 END IF;
 FOR mutex IN SELECT DISTINCT key FROM (
    SELECT recipe_import107_command_mutex(import_command_key) key FROM recipe_prepared_instance_imports WHERE deployment_id=child.deployment_id
    UNION SELECT extra_mutex WHERE extra_mutex IS NOT NULL) valueset ORDER BY key LOOP
  IF pg_try_advisory_xact_lock(mutex.key) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'original command reservation is held' USING ERRCODE='55P03'; END IF;
 END LOOP;
 FOR mutex IN SELECT DISTINCT keys.first_key,keys.second_key FROM recipe_prepared_instance_imports prepared
   CROSS JOIN LATERAL recipe_import113_instance_mutex(prepared.instance_id) keys
   WHERE prepared.deployment_id=child.deployment_id ORDER BY keys.first_key,keys.second_key LOOP
  IF pg_try_advisory_xact_lock(mutex.first_key,mutex.second_key) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'original instance reservation is held' USING ERRCODE='55P03'; END IF;
 END LOOP;
 PERFORM id FROM projects WHERE id=child.project_id FOR NO KEY UPDATE NOWAIT;
 PERFORM id FROM recipe_deployments WHERE id=child.deployment_id FOR UPDATE NOWAIT;
 PERFORM resource_name FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id ORDER BY resource_name FOR UPDATE NOWAIT;
END $$;
CREATE FUNCTION lock_prepared_closure_participants118(p_instance uuid,p_project uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM id FROM agent_instances WHERE id=p_instance FOR UPDATE NOWAIT;
 PERFORM id FROM runs WHERE instance_id=p_instance ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM agent_instance_state_volumes WHERE id IN (
  SELECT state_volume_id FROM agent_instances WHERE id=p_instance AND state_volume_id IS NOT NULL
  UNION SELECT volume_id FROM agent_instance_revision_volume_bindings WHERE instance_id=p_instance
  UNION SELECT volume_id FROM agent_instance_volume_leases WHERE instance_id=p_instance OR run_id IN (SELECT id FROM runs WHERE instance_id=p_instance)
 ) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM agent_instance_volume_leases WHERE instance_id=p_instance OR run_id IN (SELECT id FROM runs WHERE instance_id=p_instance) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM slot_key FROM agent_instance_revision_volume_bindings WHERE instance_id=p_instance ORDER BY instance_revision_id,slot_key FOR UPDATE NOWAIT;
 PERFORM id FROM agent_instance_volume_mount_grants WHERE instance_id=p_instance ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM agent_instance_volume_mount_revocations WHERE grant_id IN (SELECT id FROM agent_instance_volume_mount_grants WHERE instance_id=p_instance) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM mailboxes WHERE instance_id=p_instance ORDER BY id FOR UPDATE NOWAIT;
 PERFORM event_id FROM mailbox_deliveries WHERE instance_id=p_instance ORDER BY event_id FOR UPDATE NOWAIT;
 PERFORM id FROM mailbox_delivery_attempts WHERE mailbox_id IN (SELECT id FROM mailboxes WHERE instance_id=p_instance) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM run_requests WHERE instance_id=p_instance ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM deferred_agent_triggers WHERE instance_id=p_instance ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM agent_updates WHERE instance_id=p_instance ORDER BY id FOR UPDATE NOWAIT;
 PERFORM scope_id FROM application_event_scopes WHERE (scope_kind='project' AND scope_id=p_project)
  OR (scope_kind='agent_instance' AND scope_id=p_instance) ORDER BY scope_kind,scope_id FOR UPDATE NOWAIT;
END $$;
CREATE FUNCTION prepare_prepared_instance_closure118(p_original uuid,p_original_claim jsonb,p_prepared jsonb,p_whole bytea,p_drain jsonb,p_key bytea,p_removal uuid,p_expected bigint,p_hash bytea) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE consumer uuid; original recipe_effect_attempts%ROWTYPE; drain recipe_effect_attempts%ROWTYPE;
 resource recipe_deployment_resources%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 instance agent_instances%ROWTYPE; prior release_command_inbox%ROWTYPE; removal agent_instance_removal_requests%ROWTYPE;
 expected_hash bytea;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app' THEN
  RAISE EXCEPTION 'current manager actor pool required' USING ERRCODE='42501'; END IF;
 consumer:=prepared_instance_target118(p_original,p_original_claim,p_prepared);
 SELECT * INTO original FROM recipe_effect_attempts WHERE id=p_original;
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=(p_drain->>'attempt_id')::uuid;
 PERFORM prepared_instance_actor118(drain.command_id,p_original);
 IF drain.action IS DISTINCT FROM 'drain' OR drain.claim_json IS DISTINCT FROM p_drain
  OR drain.deployment_id IS DISTINCT FROM original.deployment_id OR drain.resource_name IS DISTINCT FROM original.resource_name
  OR drain.input_hash IS DISTINCT FROM original.input_hash OR drain.claim_json->'identity' IS DISTINCT FROM original.claim_json->'identity'
  OR p_key IS NULL OR octet_length(p_key)<>32 OR p_removal IS NULL OR p_removal='00000000-0000-0000-0000-000000000000'
  OR p_expected IS NULL OR p_expected<1 THEN
  RAISE EXCEPTION 'exact current Remove Drain and closure input required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=original.deployment_id;
 IF header.canonical_input IS DISTINCT FROM p_whole THEN
  RAISE EXCEPTION 'complete original reservation set required' USING ERRCODE='P1180'; END IF;
 PERFORM lock_prepared_closure_parents118(consumer,p_key);
 PERFORM lock_prepared_closure_participants118(consumer,original.project_id);
 -- Fresh current permissions and all immutable correlations repeat under locks.
 PERFORM prepared_instance_actor118(drain.command_id,p_original);
 IF prepared_instance_target118(p_original,p_original_claim,p_prepared) IS DISTINCT FROM consumer
  OR prepared_original_attempt118(original.deployment_id,original.resource_name) IS DISTINCT FROM p_original
  OR prepared_instance_created118(consumer,p_original) IS NOT TRUE THEN
  RAISE EXCEPTION 'original committed creation and completion required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO instance FROM agent_instances WHERE id=consumer;
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=drain.deployment_id AND resource_name=drain.resource_name;
 expected_hash:=sha256(convert_to(recipe_import107_json_bytes(jsonb_build_object('version',1,'operation','request_instance_removal',
  'actor',drain.actor_id,'instance',consumer,'removal',p_removal,'expected_version',p_expected),0),'UTF8'));
 IF p_hash IS DISTINCT FROM expected_hash THEN
  RAISE EXCEPTION 'exact closure fingerprint required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO prior FROM release_command_inbox WHERE command_key=p_key;
 IF prior.command_key IS NOT NULL THEN
  SELECT * INTO removal FROM agent_instance_removal_requests WHERE id=p_removal;
  IF prior.operation IS DISTINCT FROM 'request_instance_removal' OR prior.aggregate_id IS DISTINCT FROM consumer
   OR prior.secondary_id IS DISTINCT FROM p_removal OR prior.actor_id IS DISTINCT FROM drain.actor_id OR prior.input_hash IS DISTINCT FROM p_hash
   OR removal.instance_id IS DISTINCT FROM consumer OR removal.expected_version IS DISTINCT FROM p_expected
   OR removal.created_by IS DISTINCT FROM prior.actor_id OR removal.request_id IS DISTINCT FROM prior.request_id
   OR removal.input_hash IS DISTINCT FROM p_hash OR prepared_instance_removal118(consumer) IS DISTINCT FROM p_removal THEN
   RAISE EXCEPTION 'closure replay must preserve exact committed history' USING ERRCODE='P1180'; END IF;
  RETURN true;
 END IF;
 IF instance.id IS NULL OR instance.state='removed' OR instance.version IS DISTINCT FROM p_expected
  OR EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=consumer)
  OR resource.active_attempt_id IS DISTINCT FROM drain.id OR resource.execution_generation IS DISTINCT FROM drain.generation
  OR resource.version::text IS DISTINCT FROM p_drain->>'resource_version'
  OR resource.install_progress IS DISTINCT FROM 'ready' OR resource.removal_progress IS DISTINCT FROM 'draining'
  OR NOT EXISTS(SELECT 1 FROM recipe_deployments WHERE id=drain.deployment_id AND lifecycle IN ('removing','recovery_required')) THEN
  RAISE EXCEPTION 'current Drain CAS or instance version changed' USING ERRCODE='P1180'; END IF;
 RETURN false;
END $$;

-- Raw generic107 closure retains its authority semantics. A prepared consumer
-- cannot invert the original birth lock protocol or block on late participants.
CREATE FUNCTION guard_prepared_instance_removal_order118() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE;
BEGIN
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=NEW.instance_id;
 IF child.instance_id IS NULL THEN RETURN NEW; END IF;
 PERFORM lock_prepared_closure_parents118(NEW.instance_id,NULL);
 PERFORM lock_prepared_closure_participants118(NEW.instance_id,child.project_id);
 RETURN NEW;
END $$;
CREATE TRIGGER aab_prepared_instance_removal_order118 BEFORE INSERT ON agent_instance_removal_requests
 FOR EACH ROW EXECUTE FUNCTION guard_prepared_instance_removal_order118();
REVOKE ALL ON FUNCTION prepared_original_attempt118(uuid,text),
 load_prepared_original_target118(uuid,uuid,text,uuid,bytea),lock_prepared_closure_parents118(uuid,bytea),
 lock_prepared_closure_participants118(uuid,uuid),prepare_prepared_instance_closure118(uuid,jsonb,jsonb,bytea,jsonb,bytea,uuid,bigint,bytea),
 guard_prepared_instance_removal_order118() FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION load_prepared_original_target118(uuid,uuid,text,uuid,bytea),
 prepare_prepared_instance_closure118(uuid,jsonb,jsonb,bytea,jsonb,bytea,uuid,bigint,bytea) TO hephaestus_app;
