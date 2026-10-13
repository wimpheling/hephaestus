-- PRIVATE additive118 worker cleanup wake for consumed canonical ownership.
-- Append after immutable113/114/115/117,119 and the four immutable118 fragments.
-- No existing function/table override, live actor rights, IO or terminal evidence.
CREATE FUNCTION require_run_cleanup_wake118() RETURNS void
LANGUAGE plpgsql STABLE SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text) IS DISTINCT FROM 'hephaestus_worker' THEN
  RAISE EXCEPTION 'cleanup wake requires actual worker role' USING ERRCODE='42501'; END IF;
END $$;
REVOKE ALL ON FUNCTION require_run_cleanup_wake118() FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- Exact UUIDv5 comparison for the unchanged owning107 Cancel derivation.
-- This helper writes/mints nothing and never substitutes a Run UUID for a VM.
CREATE FUNCTION run_cleanup_wake_cancel118(p_removal uuid,p_run uuid) RETURNS uuid
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE bytes bytea; value text;
BEGIN
 bytes:=substring(public.digest(uuid_send(p_removal)||convert_to('cancel-run:'||p_run::text,'UTF8'),'sha1') FROM 1 FOR 16);
 bytes:=set_byte(bytes,6,(get_byte(bytes,6)&15)|80); bytes:=set_byte(bytes,8,(get_byte(bytes,8)&63)|128);
 value:=encode(bytes,'hex');
 RETURN (substring(value,1,8)||'-'||substring(value,9,4)||'-'||substring(value,13,4)||'-'||substring(value,17,4)||'-'||substring(value,21,12))::uuid;
END $$;
REVOKE ALL ON FUNCTION run_cleanup_wake_cancel118(uuid,uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- The original113 Create is unique by immutable install/proof lineage, never by
-- the current active attempt or the old114 phase-sensitive birth projection.
CREATE FUNCTION run_cleanup_wake_original118(p_instance uuid) RETURNS uuid
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 candidates uuid[]; original recipe_effect_attempts%ROWTYPE; document jsonb;
BEGIN
 PERFORM require_run_cleanup_wake118();
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id;
 IF child.instance_id IS NULL OR header.deployment_id IS NULL OR header.profile IS DISTINCT FROM 'runtime_named_v1'
  OR header.input_hash IS DISTINCT FROM sha256(header.canonical_input)
  OR child.input_hash IS DISTINCT FROM sha256(child.canonical_import)
  OR child.command_id IS DISTINCT FROM header.command_id OR child.project_id IS DISTINCT FROM header.project_id
  OR header.instance_count NOT BETWEEN 1 AND 64
  OR header.instance_count IS DISTINCT FROM (SELECT count(*)::integer FROM recipe_prepared_instance_imports WHERE deployment_id=child.deployment_id)
  OR qualified_invocation_import_lineage_matches(p_instance) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'complete immutable named113 import required' USING ERRCODE='P1180'; END IF;
 document:=convert_from(header.canonical_input,'UTF8')::jsonb;
 IF document->>'version' IS DISTINCT FROM '1' OR document->>'profile' IS DISTINCT FROM header.profile
  OR document->>'deployment_id' IS DISTINCT FROM header.deployment_id::text
  OR document->>'project_id' IS DISTINCT FROM header.project_id::text
  OR document->'intent_hash' IS DISTINCT FROM recipe_effect_hash_json(header.intent_hash)
  OR document->'platform' IS DISTINCT FROM header.platform_json
  OR document->'command' IS DISTINCT FROM (SELECT jsonb_build_object('id',id,'actor_id',actor_id,
    'idempotency_id',idempotency_id,'operation',operation) FROM recipe_deployment_commands WHERE id=header.command_id)
  OR jsonb_typeof(document->'instances') IS DISTINCT FROM 'array'
  OR jsonb_array_length(document->'instances') IS DISTINCT FROM header.instance_count
  OR header.instance_count IS DISTINCT FROM (SELECT count(*)::integer FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_kind='instance')
  OR EXISTS(SELECT 1 FROM recipe_prepared_instance_imports sibling WHERE sibling.deployment_id=child.deployment_id AND (
    sibling.ordinal>=header.instance_count OR sibling.project_id IS DISTINCT FROM header.project_id
    OR sibling.command_id IS DISTINCT FROM header.command_id OR sibling.input_hash IS DISTINCT FROM sha256(sibling.canonical_import)
    OR convert_from(sibling.canonical_import,'UTF8')::jsonb IS DISTINCT FROM document->'instances'->sibling.ordinal
    OR document->'instances'->sibling.ordinal->>'resource' IS DISTINCT FROM sibling.resource_name
    OR document->'instances'->sibling.ordinal->>'operation_id' IS DISTINCT FROM sibling.operation_id::text
    OR document->'instances'->sibling.ordinal->'command_key' IS DISTINCT FROM recipe_effect_hash_json(sibling.import_command_key)
    OR document->'instances'->sibling.ordinal->>'instance_id' IS DISTINCT FROM sibling.instance_id::text
    OR document->'instances'->sibling.ordinal->>'revision_id' IS DISTINCT FROM sibling.revision_id::text
    OR document->'instances'->sibling.ordinal->>'project_id' IS DISTINCT FROM sibling.project_id::text
    OR document->'instances'->sibling.ordinal->'source'->'pin' IS DISTINCT FROM jsonb_build_object('release_id',sibling.release_id,'release_agent_id',sibling.release_agent_id)
    OR document->'instances'->sibling.ordinal->'source'->'platform' IS DISTINCT FROM header.platform_json))
  OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox product ON product.event_id=event.id
    WHERE event.id=header.event_id AND event.cursor=header.event_cursor AND event.aggregate_version=header.event_aggregate_version
     AND event.scope_kind='project' AND event.scope_id=header.project_id AND event.aggregate_type='project' AND event.aggregate_id=header.project_id
     AND event.event_type='project.changed' AND event.change_kind='updated' AND event.actor_type='user'
     AND event.actor_id=header.actor_id AND event.request_id=header.request_id) THEN
  RAISE EXCEPTION 'complete original113 bytes and committed preparation required' USING ERRCODE='P1180'; END IF;
 SELECT array_agg(DISTINCT attempt.id ORDER BY attempt.id) INTO candidates
  FROM recipe_effect_attempts attempt JOIN recipe_effect_verifications proof ON proof.attempt_id=attempt.id
  JOIN recipe_execution_transitions transition ON transition.attempt_id=attempt.id AND transition.verification_id=proof.id
  JOIN recipe_deployment_commands current_command ON current_command.id=transition.command_id
  WHERE attempt.deployment_id=child.deployment_id AND attempt.resource_name=child.resource_name
   AND attempt.command_id=header.command_id AND attempt.action='create' AND attempt.actor_id=header.actor_id
   AND attempt.project_id=child.project_id
   AND proof.recorded_by='hephaestus_worker' AND proof.action='create' AND proof.outcome='applied' AND proof.diagnostic IS NULL
   AND proof.deployment_id=attempt.deployment_id AND proof.project_id=attempt.project_id
   AND proof.resource_name=attempt.resource_name AND proof.generation=attempt.generation
   AND proof.input_hash=attempt.input_hash AND proof.identity_json=attempt.claim_json->'identity'
   AND transition.kind IN ('completed','reconciled') AND transition.operation_id=proof.verification_attempt_id
   AND transition.project_id=attempt.project_id AND transition.resource_name=attempt.resource_name
   AND transition.after_state->>'install'='ready' AND transition.after_state->>'active_attempt' IS NULL
   AND ((current_command.id=attempt.command_id AND current_command.operation='install')
    OR (transition.kind='reconciled' AND current_command.operation='remove'
      AND current_command.deployment_id=attempt.deployment_id AND current_command.project_id=attempt.project_id));
 IF COALESCE(cardinality(candidates),0)<>1 THEN
  RAISE EXCEPTION 'one committed original Create proof required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO original FROM recipe_effect_attempts WHERE id=candidates[1];
 IF NOT EXISTS(SELECT 1 FROM recipe_execution_transitions claimed JOIN application_events event ON event.id=claimed.event_id
    JOIN product_event_outbox product ON product.event_id=event.id
    WHERE claimed.attempt_id=original.id AND claimed.kind='claimed' AND claimed.command_id=original.command_id
     AND claimed.project_id=original.project_id AND claimed.resource_name=original.resource_name
     AND claimed.operation_id=original.id AND claimed.fingerprint=original.request_fingerprint AND claimed.before_state=original.before_state
     AND claimed.after_state->>'active_attempt'=original.id::text AND claimed.after_state->>'generation'=original.generation::text
     AND event.actor_id=original.actor_id AND event.request_id=original.request_id
     AND event.cursor=claimed.event_cursor AND event.aggregate_version=claimed.event_aggregate_version) THEN
  RAISE EXCEPTION 'committed immutable original Create claim required' USING ERRCODE='P1180'; END IF;
 IF prepared_instance_target118(original.id,original.claim_json,convert_from(child.canonical_import,'UTF8')::jsonb) IS DISTINCT FROM p_instance THEN
  RAISE EXCEPTION 'exact original prepared target required' USING ERRCODE='P1180'; END IF;
 RETURN original.id;
END $$;
REVOKE ALL ON FUNCTION run_cleanup_wake_original118(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION run_cleanup_wake_closure118(p_run uuid,p_instance uuid,p_original jsonb) RETURNS void
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
 -- Existing release outbox uses aggregate_type=release with this exact Run ID;
 -- transport enrichment is compared separately from the canonical Cancel DTO.
 IF (SELECT count(*) FROM outbox WHERE aggregate_type='release' AND aggregate_id=p_run
    AND subject='heph.run.command.cancel.v1' AND event_type='run.cancel_requested.v1'
    AND payload->'command_id'=cancel->'command_id' AND payload->'run_id'=cancel->'run_id' AND payload->'reason'=cancel->'reason')<>1
  OR NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(cancel->>'command_id')::uuid
    AND command_type='cancel_run' AND payload=cancel AND processed_at IS NOT NULL) THEN
  RAISE EXCEPTION 'exact committed and owning-processed original Cancel required' USING ERRCODE='P1180'; END IF;
END $$;
REVOKE ALL ON FUNCTION run_cleanup_wake_closure118(uuid,uuid,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- Immutable producer checks intentionally omit dispatch/hook phases and live
-- permissions. Retained actor/request IDs are original data, never principals.
CREATE FUNCTION run_cleanup_wake_producer118(execution runs,p_config bytea) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE start jsonb;
BEGIN
 PERFORM require_run_cleanup_wake118();
 IF execution.execution_profile IS DISTINCT FROM 'owned_canonical'
  OR execution.vm_binding_kind IS DISTINCT FROM 'planned'
  OR NOT run_cleanup_identifier_is_valid(execution.vm_id)
  OR NOT run_cleanup_identifier_is_valid(execution.vm_provider_namespace)
  OR NOT run_cleanup_identifier_is_valid(execution.vm_host_id)
  OR profiled_run_pins_match(execution,false) IS DISTINCT FROM true THEN RETURN false; END IF;
 IF execution.run_kind='invocation' THEN
  RETURN inspect_qualified_invocation117(execution.id) IS NOT DISTINCT FROM true
   AND EXISTS(SELECT 1 FROM run_invocation_consumptions consumed JOIN agent_instance_execution_invocations receipt ON receipt.id=consumed.invocation_id
    WHERE consumed.run_id=execution.id AND consumed.config_bytes=p_config
     AND ROW(consumed.vm_id,consumed.vm_provider_namespace,consumed.vm_host_id)
      IS NOT DISTINCT FROM ROW(execution.vm_id,execution.vm_provider_namespace,execution.vm_host_id));
 END IF;
 start:=jsonb_build_object('command_id',execution.command_id,'run_id',execution.id,'instance_id',execution.instance_id,
  'instance_revision_id',execution.instance_revision_id,'release_id',execution.release_id,'release_agent_id',execution.release_agent_id,
  'attachment_id',execution.attachment_id,'kind',execution.run_kind,'requires_state',execution.requires_state);
 IF NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=execution.command_id AND command_type='start_run'
   AND payload=start AND processed_at IS NOT NULL) THEN RETURN false; END IF;
 IF execution.run_kind='normal' THEN
  RETURN NOT execution.vm_plan_admission_required
   AND NOT EXISTS(SELECT 1 FROM mailbox_delivery_attempts WHERE run_id=execution.id)
   AND NOT EXISTS(SELECT 1 FROM run_vm_plan_admissions WHERE run_id=execution.id)
   AND EXISTS(SELECT 1 FROM run_requests request WHERE request.run_id=execution.id AND request.command_id=execution.command_id
    AND request.request_kind='instance_normal'
    AND ROW(request.instance_id,request.instance_revision_id,request.release_id,request.release_agent_id,request.attachment_id,request.requires_state)
     IS NOT DISTINCT FROM ROW(execution.instance_id,execution.instance_revision_id,execution.release_id,execution.release_agent_id,execution.attachment_id,execution.requires_state));
 ELSIF execution.run_kind='update' THEN
  RETURN execution.vm_plan_admission_required AND execution.attachment_id IS NULL
   AND EXISTS(SELECT 1 FROM agent_updates update JOIN run_vm_plan_admissions admission ON admission.update_id=update.id
    JOIN release_command_inbox command ON command.command_key=admission.producer_command_key
    WHERE admission.run_id=execution.id AND admission.command_id=execution.command_id
     AND admission.instance_id=execution.instance_id AND admission.instance_revision_id=execution.instance_revision_id
     AND update.hook_run_id=execution.id AND update.instance_id=execution.instance_id AND update.candidate_revision_id=execution.instance_revision_id
     AND command.operation='begin_update_hook' AND command.aggregate_id=update.id AND command.secondary_id=execution.id
     AND command.actor_id=admission.actor_id AND command.request_id=admission.request_id
     AND admission.consumed_at IS NOT NULL AND admission.consumption_kind IN ('provision','cleanup')
     AND ROW(admission.vm_id,admission.vm_provider_namespace,admission.vm_host_id)
      IS NOT DISTINCT FROM ROW(execution.vm_id,execution.vm_provider_namespace,execution.vm_host_id));
 END IF;
 RETURN false;
END $$;
REVOKE ALL ON FUNCTION run_cleanup_wake_producer118(runs,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- Complete frozen named selections reconstructed from immutable bindings.
CREATE FUNCTION run_cleanup_wake_selections118(execution runs) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 WITH identity AS (SELECT jsonb_build_object('run',execution.id,'instance',execution.instance_id,
  'revision',execution.instance_revision_id,'release',execution.release_id,'release_agent',execution.release_agent_id,
  'project',consumer.project_id) value,agent.runtime_contract_hash contract_hash
  FROM agent_instances consumer JOIN release_agents agent ON agent.id=execution.release_agent_id WHERE consumer.id=execution.instance_id)
 SELECT jsonb_build_object('identity',identity.value,'selections',COALESCE((SELECT jsonb_agg(jsonb_build_object(
  'identity',identity.value,'scope',jsonb_build_object('instance_id',binding.instance_id,'revision_id',binding.instance_revision_id,
   'release_agent_id',binding.release_agent_id,'slot',binding.slot_key,'volume_id',binding.volume_id,'access_mode',binding.access_mode,
   'release_contract_hash',recipe_effect_hash_json(identity.contract_hash)),
  'declaration',jsonb_build_object('slot',binding.slot_key,'guest_path',binding.guest_path,'access_mode',binding.access_mode,
   'required',binding.slot_required,'minimum_capacity_bytes',binding.minimum_capacity_bytes),'origin',binding.provenance) ORDER BY binding.slot_key)
  FROM agent_instance_revision_volume_bindings binding WHERE binding.instance_revision_id=execution.instance_revision_id),'[]'::jsonb)) FROM identity
$$;
REVOKE ALL ON FUNCTION run_cleanup_wake_selections118(runs) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION run_cleanup_wake_set118(execution runs,p_released boolean) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE selection jsonb; number integer;
BEGIN
 PERFORM require_run_cleanup_wake118();
 SELECT count(*)::integer INTO number FROM agent_instance_volume_leases WHERE run_id=execution.id;
 -- Consumed pre-acquisition work has no fabricated selection or lease history.
 IF number=0 AND execution.volume_selection_snapshot IS NULL THEN
  RETURN num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)=0
   AND NOT EXISTS(SELECT 1 FROM run_events WHERE run_id=execution.id AND event_type IN ('run.starting','run.running'));
 END IF;
 selection:=run_cleanup_wake_selections118(execution);
 RETURN execution.volume_selection_snapshot IS NOT DISTINCT FROM selection
  AND jsonb_array_length(selection->'selections') BETWEEN 0 AND 32
  AND number=jsonb_array_length(selection->'selections')
  AND num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)=0
  AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease
   LEFT JOIN agent_instance_state_volumes volume ON volume.id=lease.volume_id
   LEFT JOIN agent_instance_revision_volume_bindings binding ON binding.instance_revision_id=execution.instance_revision_id AND binding.slot_key=lease.slot_key
   WHERE lease.run_id=execution.id AND (volume.id IS NULL OR binding.volume_id IS DISTINCT FROM lease.volume_id
    OR ROW(lease.instance_id,lease.selected_revision_id,lease.selected_release_id,lease.selected_release_agent_id,
      lease.selected_project_id,lease.selection_origin,lease.access_mode,lease.guest_path,lease.slot_required,lease.minimum_capacity_bytes)
     IS DISTINCT FROM ROW(execution.instance_id,execution.instance_revision_id,execution.release_id,execution.release_agent_id,
      volume.project_id,binding.provenance,binding.access_mode,binding.guest_path,binding.slot_required,binding.minimum_capacity_bytes)
    OR lease.release_contract_hash IS DISTINCT FROM (SELECT runtime_contract_hash FROM release_agents WHERE id=execution.release_agent_id)
    OR lease.update_id IS DISTINCT FROM (CASE WHEN execution.run_kind='update' THEN (SELECT id FROM agent_updates WHERE hook_run_id=execution.id) ELSE NULL::uuid END)
    OR lease.host_id IS DISTINCT FROM execution.vm_host_id
    OR (p_released AND (lease.state IS DISTINCT FROM 'released' OR lease.released_at IS NULL))
    OR (NOT p_released AND (lease.state NOT IN ('active','recovering') OR lease.released_at IS NOT NULL
      OR volume.lease_generation IS DISTINCT FROM lease.fencing_token OR volume.host_id IS DISTINCT FROM lease.host_id
      OR volume.state NOT IN ('ready','attached','recovering') OR volume.provisioning_state IS DISTINCT FROM 'ready'
      OR EXISTS(SELECT 1 FROM agent_instance_volume_leases other WHERE other.volume_id=lease.volume_id AND other.id<>lease.id AND other.released_at IS NULL)))));
 -- Released comparison deliberately ignores the current volume's generation,
 -- holder and readiness: a newer consumer cannot be reset by original replay.
END $$;
REVOKE ALL ON FUNCTION run_cleanup_wake_set118(runs,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION qualify_run_cleanup_wake118(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea,p_preparing boolean) RETURNS runs
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; target run_cleanup_targets%ROWTYPE;
BEGIN
 PERFORM require_run_cleanup_wake118();
 PERFORM run_cleanup_wake_closure118(p_run,p_instance,p_original);
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.vm_provider_namespace IS DISTINCT FROM p_namespace OR execution.vm_host_id IS DISTINCT FROM p_host
  OR run_cleanup_wake_producer118(execution,p_config) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'positive consumed canonical producer and actual scope required' USING ERRCODE='P1180'; END IF;
 -- Genuine113 imports are named; positive Legacy requires legacy mode. Keep the
 -- mismatch held rather than adopting an old or invented placement association.
 IF execution.execution_profile IS DISTINCT FROM 'owned_canonical' THEN
  RAISE EXCEPTION 'Legacy113 cleanup wake association unsupported' USING ERRCODE='P1180'; END IF;
 SELECT * INTO target FROM run_cleanup_targets WHERE run_id=p_run;
 IF target.run_id IS NULL THEN
  IF NOT p_preparing OR execution.state='cleaned_up' OR run_cleanup_wake_set118(execution,false) IS DISTINCT FROM true
   OR EXISTS(SELECT 1 FROM run_cleanup_receipts WHERE run_id=p_run)
   OR EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=p_run) THEN
   RAISE EXCEPTION 'owning preparation or exact existing target required' USING ERRCODE='P1180'; END IF;
  RETURN execution;
 END IF;
 IF execution.volume_acquisition_closed_at IS NULL
  OR ROW(target.instance_id,target.instance_revision_id,target.vm_target_kind,target.vm_provider_namespace,target.vm_host_id,target.vm_id)
   IS DISTINCT FROM ROW(execution.instance_id,execution.instance_revision_id,execution.vm_binding_kind,execution.vm_provider_namespace,execution.vm_host_id,execution.vm_id)
  OR target.target_digest IS DISTINCT FROM run_cleanup_target_digest(p_run,target.generation)
  OR target.lease_count IS DISTINCT FROM (SELECT count(*)::integer FROM run_cleanup_target_leases WHERE run_id=p_run)
  OR EXISTS(SELECT 1 FROM run_cleanup_target_leases WHERE run_id=p_run AND generation<>target.generation) THEN
  RAISE EXCEPTION 'global original target or acquisition closure contradicts Run' USING ERRCODE='P1180'; END IF;
 IF execution.state='cleaned_up' THEN
  IF run_cleanup_wake_set118(execution,true) IS DISTINCT FROM true
   OR run_cleanup_set_matches(p_run,target.generation,true) IS DISTINCT FROM true
   OR NOT EXISTS(SELECT 1 FROM run_cleanup_receipts receipt JOIN run_cleanup_completions done
    ON done.run_id=receipt.run_id AND done.generation=receipt.generation AND done.target_digest=receipt.target_digest
    WHERE receipt.run_id=p_run AND receipt.generation=target.generation AND receipt.target_digest=target.target_digest
     AND ROW(receipt.vm_provider_namespace,receipt.vm_host_id,receipt.vm_id)
      IS NOT DISTINCT FROM ROW(target.vm_provider_namespace,target.vm_host_id,target.vm_id)
     AND receipt.observation_kind IN ('destroyed','authoritatively_absent')) THEN
   RAISE EXCEPTION 'exact original receipt and released completion required' USING ERRCODE='P1180'; END IF;
 ELSE
  IF run_cleanup_wake_set118(execution,false) IS DISTINCT FROM true
   OR run_cleanup_set_matches(p_run,target.generation,false) IS DISTINCT FROM true
   OR EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=p_run) THEN
   RAISE EXCEPTION 'whole original global held set required' USING ERRCODE='P1180'; END IF;
 END IF;
 RETURN execution;
END $$;
REVOKE ALL ON FUNCTION qualify_run_cleanup_wake118(uuid,uuid,jsonb,text,text,bytea,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- Readonly facade: STABLE, no locks/writes, original actor IDs are comparison data.
CREATE FUNCTION load_run_cleanup_wake118(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea) RETURNS SETOF runs
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 RETURN NEXT qualify_run_cleanup_wake118(p_run,p_instance,p_original,p_namespace,p_host,p_config,false);
END $$;
REVOKE ALL ON FUNCTION load_run_cleanup_wake118(uuid,uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION load_run_cleanup_wake118(uuid,uuid,jsonb,text,text,bytea) TO hephaestus_worker;

-- The owner may wait on this complete, immutable sorted reservation projection
-- only before parent rows. The raw locking facade below always uses TRY/NOWAIT.
CREATE FUNCTION run_cleanup_wake_reservations118(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea)
RETURNS TABLE(lock_class integer,command_mutex bigint,first_mutex integer,second_mutex integer)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; execution runs%ROWTYPE; extra bytea;
BEGIN
 PERFORM require_run_cleanup_wake118();
 execution:=qualify_run_cleanup_wake118(p_run,p_instance,p_original,p_namespace,p_host,p_config,true);
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
REVOKE ALL ON FUNCTION run_cleanup_wake_reservations118(uuid,uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION run_cleanup_wake_reservations118(uuid,uuid,jsonb,text,text,bytea) TO hephaestus_worker;

CREATE FUNCTION lock_run_cleanup_wake118(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE early record; child recipe_prepared_instance_imports%ROWTYPE; extra bytea;
BEGIN
 PERFORM require_run_cleanup_wake118();
 FOR early IN SELECT * FROM run_cleanup_wake_reservations118(p_run,p_instance,p_original,p_namespace,p_host,p_config)
   ORDER BY lock_class,command_mutex,first_mutex,second_mutex LOOP
  IF early.lock_class=0 THEN
   IF pg_try_advisory_xact_lock(early.command_mutex) IS DISTINCT FROM true THEN
    RAISE EXCEPTION 'original command reservation held' USING ERRCODE='55P03'; END IF;
  ELSE
   IF pg_try_advisory_xact_lock(early.first_mutex,early.second_mutex) IS DISTINCT FROM true THEN
    RAISE EXCEPTION 'original instance reservation held' USING ERRCODE='55P03'; END IF;
  END IF;
 END LOOP;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 SELECT command_key INTO extra FROM release_command_inbox WHERE secondary_id=(p_original->>'removal_id')::uuid AND operation='request_instance_removal';
 -- These existing helpers contain raw ordering only, with no actor/live eligibility.
 -- They reserve whole113 before Project -> Deployment -> all Resources, then
 -- Instance -> Run -> sorted Volumes/leases and remaining late rows NOWAIT.
 PERFORM lock_prepared_closure_parents118(p_instance,extra);
 PERFORM lock_prepared_closure_participants118(p_instance,child.project_id);
 PERFORM run_id FROM run_cleanup_targets WHERE run_id=p_run FOR UPDATE NOWAIT;
 PERFORM lease_id FROM run_cleanup_target_leases WHERE run_id=p_run ORDER BY volume_id,lease_id FOR UPDATE NOWAIT;
 PERFORM run_id FROM run_cleanup_receipts WHERE run_id=p_run ORDER BY generation FOR UPDATE NOWAIT;
 PERFORM run_id FROM run_cleanup_completions WHERE run_id=p_run ORDER BY generation FOR UPDATE NOWAIT;
 PERFORM qualify_run_cleanup_wake118(p_run,p_instance,p_original,p_namespace,p_host,p_config,true);
 -- Closure/target writes are the existing transaction tail in the owning Rust
 -- adapter after this requalification. No state, outcome, receipt or cancellation.
END $$;
REVOKE ALL ON FUNCTION lock_run_cleanup_wake118(uuid,uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION lock_run_cleanup_wake118(uuid,uuid,jsonb,text,text,bytea) TO hephaestus_worker;
