-- PRIVATE additive readonly original closure under a fresh manager continuation.
-- Append after immutable observer/closure/generic Remove-resume prerequisites.
-- No tables, lock admission, credentials, writes, receipts or physical IO authority.
CREATE FUNCTION load_existing_instance_closure118(p_command uuid,p_deployment uuid,p_project uuid,p_intent_hash bytea,
 p_attempt uuid,p_claim jsonb,p_prepared jsonb,p_namespace text,p_host text,p_platform jsonb) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE admission recipe_remove_resume_admissions%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 command recipe_deployment_commands%ROWTYPE; consumer uuid; fresh jsonb; selected jsonb; recorded jsonb;
 original recipe_effect_attempts%ROWTYPE; original_command recipe_deployment_commands%ROWTYPE; count integer;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app' THEN
  RAISE EXCEPTION 'current manager actor pool required' USING ERRCODE='42501'; END IF;
 PERFORM remove_resume_actor118(p_deployment);
 PERFORM prepared_instance_actor118(p_command,p_attempt);
 consumer:=prepared_instance_target118(p_attempt,p_claim,p_prepared);
 SELECT * INTO admission FROM recipe_remove_resume_admissions WHERE command_id=p_command;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=p_deployment;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=p_command;
 IF admission.command_id IS NULL OR admission.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
  OR NULLIF(current_setting('hephaestus.request_id',true),'') IS NULL
  OR command.operation IS DISTINCT FROM 'remove' OR command.actor_id IS DISTINCT FROM admission.actor_id
  OR command.first_request_id IS DISTINCT FROM admission.request_id OR command.event_id IS DISTINCT FROM admission.event_id
  OR command.idempotency_id::text IS DISTINCT FROM current_setting('hephaestus.occurrence_id',true)
  OR command.receipt_version IS DISTINCT FROM admission.expected_version OR command.receipt_lifecycle IS DISTINCT FROM 'removing'
  OR command.input_hash IS DISTINCT FROM sha256(convert_to(format('["hephaestus-recipe-remove-v1","%s",%s]',p_deployment,admission.expected_version),'UTF8'))
  OR admission.deployment_id IS DISTINCT FROM p_deployment OR admission.project_id IS DISTINCT FROM p_project
  OR admission.intent_hash IS DISTINCT FROM p_intent_hash OR header.intent_hash IS DISTINCT FROM p_intent_hash
  OR header.project_id IS DISTINCT FROM p_project OR header.profile IS DISTINCT FROM 'runtime_named_v1'
  OR admission.preparation_hash IS DISTINCT FROM header.input_hash OR header.platform_json IS DISTINCT FROM p_platform
  OR admission.vm_provider_namespace IS DISTINCT FROM p_namespace OR admission.vm_host_id IS DISTINCT FROM p_host
  OR admission.platform_json IS DISTINCT FROM p_platform
  OR NOT EXISTS(SELECT 1 FROM recipe_prepared_instance_imports child WHERE child.deployment_id=p_deployment
    AND child.instance_id=consumer AND child.resource_name=p_claim->>'resource') THEN
  RAISE EXCEPTION 'exact current continuation and original preparation required' USING ERRCODE='P1180'; END IF;
 -- Current request is fresh provenance; the immutable first admission and original
 -- closure request remain history and are deliberately not substituted for it.
 IF EXISTS(SELECT 1 FROM runs execution JOIN recipe_prepared_instance_imports child ON child.instance_id=execution.instance_id
    WHERE child.deployment_id=p_deployment AND (
      (execution.execution_profile='owned_canonical' AND execution.vm_provider_namespace=admission.vm_provider_namespace
       AND execution.vm_host_id=admission.vm_host_id AND EXISTS(SELECT 1 FROM run_vm_plan_admissions plan
         WHERE plan.run_id=execution.id AND plan.consumed_at IS NOT NULL AND plan.vm_id=execution.vm_id
           AND plan.vm_provider_namespace=execution.vm_provider_namespace AND plan.vm_host_id=execution.vm_host_id))
      OR (execution.execution_profile='legacy_scalar' AND EXISTS(SELECT 1 FROM run_legacy_vm_placements plan
         WHERE plan.run_id=execution.id AND plan.consumed_at IS NOT NULL
           AND plan.provider_namespace::text=admission.vm_provider_namespace AND plan.host_id=admission.vm_host_id
           AND legacy_vm_placement_matches(execution,plan)))) IS DISTINCT FROM true) THEN
  RAISE EXCEPTION 'unknown or foreign current runtime history held' USING ERRCODE='P1180'; END IF;
 fresh:=remove_resume_context118(p_deployment);
 SELECT count(*) INTO count FROM jsonb_array_elements(fresh->'closures') item WHERE item->>'resource'=p_claim->>'resource';
 IF count<>1 THEN RAISE EXCEPTION 'one exact original closure required' USING ERRCODE='P1180'; END IF;
 SELECT item INTO selected FROM jsonb_array_elements(fresh->'closures') item WHERE item->>'resource'=p_claim->>'resource';
 SELECT count(*) INTO count FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=p_claim->>'resource';
 IF count<>1 THEN RAISE EXCEPTION 'one admitted original closure required' USING ERRCODE='P1180'; END IF;
 SELECT item INTO recorded FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=p_claim->>'resource';
 IF selected IS DISTINCT FROM recorded THEN RAISE EXCEPTION 'original closure changed after admission' USING ERRCODE='P1180'; END IF;
 SELECT * INTO original FROM recipe_effect_attempts WHERE id=(selected->'drain_claim'->>'attempt_id')::uuid;
 SELECT * INTO original_command FROM recipe_deployment_commands WHERE id=original.command_id;
 IF original.id IS NULL OR original.action IS DISTINCT FROM 'drain' OR original.claim_json IS DISTINCT FROM selected->'drain_claim'
  OR original.deployment_id IS DISTINCT FROM p_deployment OR original.project_id IS DISTINCT FROM p_project
  OR original.resource_name IS DISTINCT FROM p_claim->>'resource' OR original.input_hash IS DISTINCT FROM
    (SELECT input_hash FROM recipe_deployment_resources WHERE deployment_id=p_deployment AND resource_name=p_claim->>'resource')
  OR original.claim_json->'identity' IS DISTINCT FROM p_claim->'identity'
  OR original_command.operation IS DISTINCT FROM 'remove' OR original_command.actor_id IS DISTINCT FROM original.actor_id
  OR original_command.actor_id IS DISTINCT FROM (selected->>'actor')::uuid
  OR original_command.idempotency_id IS DISTINCT FROM (selected->>'idempotency')::uuid
  OR NOT EXISTS(SELECT 1 FROM recipe_execution_transitions claimed JOIN application_events event ON event.id=claimed.event_id
    JOIN product_event_outbox outbox ON outbox.event_id=event.id
    WHERE claimed.attempt_id=original.id AND claimed.kind='claimed' AND claimed.command_id=original.command_id
      AND claimed.project_id=original.project_id AND claimed.resource_name=original.resource_name
      AND claimed.operation_id=original.id AND claimed.fingerprint=original.request_fingerprint AND claimed.before_state=original.before_state
      AND claimed.after_state->>'active_attempt'=original.id::text AND claimed.after_state->>'generation'=original.generation::text
      AND event.actor_id=original.actor_id AND event.request_id=original.request_id
      AND event.cursor=claimed.event_cursor AND event.aggregate_version=claimed.event_aggregate_version) THEN
  RAISE EXCEPTION 'immutable original Drain lineage required' USING ERRCODE='P1180'; END IF;
 -- Return only declared original comparison fields, never saved parameters or full
 -- admission/placement rows. No status/empty history is physical cleanup evidence.
 RETURN jsonb_build_object('drain',selected->'drain_claim','actor',original_command.actor_id,
  'idempotency',original_command.idempotency_id,'removal_id',selected->'removal'->'id',
  'command_key',(SELECT jsonb_agg(get_byte(decode(substr(selected->'inbox'->>'command_key',3),'hex'),n) ORDER BY n) FROM generate_series(0,31) n),
  'expected_version',selected->'removal'->'expected_version','creator',selected->'removal'->'created_by',
  'request',selected->'removal'->'request_id',
  'input_hash',(SELECT jsonb_agg(get_byte(decode(substr(selected->'removal'->>'input_hash',3),'hex'),n) ORDER BY n) FROM generate_series(0,31) n));
END $$;
REVOKE ALL ON FUNCTION load_existing_instance_closure118(uuid,uuid,uuid,bytea,uuid,jsonb,jsonb,text,text,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION load_existing_instance_closure118(uuid,uuid,uuid,bytea,uuid,jsonb,jsonb,text,text,jsonb) TO hephaestus_app;
