-- Additive142: fresh CURRENT manager publication of an ORIGINAL prepared Drain.
-- Historical1..141 and original claims remain immutable. Pending/origin JSON is
-- comparison data, never physical absence, worker cleanup or a Drain receipt.
ALTER TABLE public.agent_instance_removal_requests
 ADD COLUMN prepared_resume_command_id uuid,
 ADD CONSTRAINT instance_removal_prepared_resume_command142_fkey
  FOREIGN KEY(prepared_resume_command_id) REFERENCES public.recipe_remove_resume_admissions(command_id),
 ADD CONSTRAINT instance_removal_prepared_resume_command142_non_nil
  CHECK(prepared_resume_command_id IS NULL OR prepared_resume_command_id<>'00000000-0000-0000-0000-000000000000'::uuid);

-- Actual RecoveryRequired admission receipts retain their native lifecycle.
-- Every prior accepted alternative is preserved; no lifecycle is rewritten.
ALTER TABLE public.recipe_deployment_commands
 DROP CONSTRAINT recipe_deployment_commands_receipt_lifecycle_check,
 ADD CONSTRAINT recipe_deployment_commands_receipt_lifecycle_check
  CHECK (receipt_lifecycle IN ('installing', 'removing', 'removed', 'recovery_required'));

-- Only this exact additive key is omitted from the historical immutable wire.
CREATE FUNCTION prepared_closure_row142(p_removal public.agent_instance_removal_requests) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT to_jsonb(p_removal)-'prepared_resume_command_id';
$$;

CREATE FUNCTION prepared_drain_progress142(p_attempt uuid,p_endpoint jsonb) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE drain recipe_effect_attempts%ROWTYPE; resource recipe_deployment_resources%ROWTYPE;
 header recipe_deployment_execution_preparations%ROWTYPE; claimed recipe_execution_transitions%ROWTYPE;
 step recipe_execution_transitions%ROWTYPE; current_command recipe_deployment_commands%ROWTYPE;
 proof recipe_effect_verifications%ROWTYPE; state jsonb; endpoint_version bigint;
BEGIN
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=p_attempt;
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=drain.deployment_id AND resource_name=drain.resource_name;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=drain.deployment_id;
 IF drain.id IS NULL OR drain.action IS DISTINCT FROM 'drain' OR drain.generation<=0
  OR resource.resource_kind IS DISTINCT FROM 'instance' OR resource.ownership IS DISTINCT FROM 'owned'
  OR resource.input_hash IS DISTINCT FROM drain.input_hash OR resource.plan_json->'identity' IS DISTINCT FROM drain.claim_json->'identity'
  OR header.deployment_id IS NULL OR header.profile IS DISTINCT FROM 'runtime_named_v1'
  OR drain.claim_json IS DISTINCT FROM jsonb_build_object('command_id',drain.command_id,'deployment_id',drain.deployment_id,
    'resource',drain.resource_name,'identity',resource.plan_json->'identity','input_hash',recipe_effect_hash_json(drain.input_hash),
    'resource_version',(drain.before_state->>'version')::bigint+1,'generation',drain.generation,
    'action','drain','attempt_id',drain.id,'actor_id',drain.actor_id,'request_id',drain.request_id)
  OR jsonb_typeof(p_endpoint) IS DISTINCT FROM 'object'
  OR (SELECT array_agg(key ORDER BY key) FROM jsonb_object_keys(p_endpoint) key)
     IS DISTINCT FROM ARRAY['active_attempt','diagnostic','generation','install','last_action','removal','version']::text[]
  OR p_endpoint->>'install' IS DISTINCT FROM 'ready' OR p_endpoint->>'active_attempt' IS DISTINCT FROM drain.id::text
  OR p_endpoint->>'generation' IS DISTINCT FROM drain.generation::text
  OR p_endpoint->>'removal' NOT IN ('draining','recovery_required')
  OR (p_endpoint->>'version')::bigint<(drain.claim_json->>'resource_version')::bigint THEN
  RAISE EXCEPTION 'exact positive original Drain endpoint required' USING ERRCODE='P1420'; END IF;
 endpoint_version:=(p_endpoint->>'version')::bigint;
 IF (SELECT count(*) FROM recipe_execution_transitions WHERE attempt_id=drain.id AND kind='claimed')<>1 THEN
  RAISE EXCEPTION 'unique original claimed transition required' USING ERRCODE='P1420'; END IF;
 SELECT * INTO claimed FROM recipe_execution_transitions WHERE attempt_id=drain.id AND kind='claimed';
 IF ROW(claimed.command_id,claimed.deployment_id,claimed.project_id,claimed.resource_name,claimed.operation_id,
    claimed.actor_id,claimed.request_id,claimed.fingerprint,claimed.before_state)
   IS DISTINCT FROM ROW(drain.command_id,drain.deployment_id,drain.project_id,drain.resource_name,drain.id,
    drain.actor_id,drain.request_id,drain.request_fingerprint,drain.before_state)
  OR claimed.verification_id IS NOT NULL
  OR claimed.after_deployment_version IS DISTINCT FROM claimed.before_deployment_version+1
  OR claimed.after_state IS DISTINCT FROM recipe_effect_expected_state(drain.before_state,drain.before_state,'drain','claimed','unproven',NULL,drain.id)
  OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox product ON product.event_id=event.id
    WHERE event.id=claimed.event_id AND event.occurrence_id=claimed.id AND event.scope_kind='project'
     AND event.scope_id=drain.project_id AND event.aggregate_type='project' AND event.aggregate_id=drain.project_id
     AND event.event_type='project.changed' AND event.change_kind='updated' AND event.actor_type='user'
     AND event.actor_id=claimed.actor_id AND event.request_id=claimed.request_id
     AND event.cursor=claimed.event_cursor AND event.aggregate_version=claimed.event_aggregate_version) THEN
  RAISE EXCEPTION 'exact protected claimed event and fingerprint required' USING ERRCODE='P1420'; END IF;
 state:=claimed.after_state;
 IF p_endpoint->'last_action' IS DISTINCT FROM state->'last_action'
  OR EXISTS(SELECT 1 FROM recipe_execution_transitions t WHERE t.deployment_id=drain.deployment_id AND t.resource_name=drain.resource_name
    AND (t.before_state->>'version' IS NULL OR t.after_state->>'version' IS NULL)) THEN
  RAISE EXCEPTION 'known whole state and unchanged prior verified action required' USING ERRCODE='P1420'; END IF;
 -- Inspect the complete RESOURCE prefix, not an attempt-filtered subset that
 -- could omit a different generation or another attempt between these states.
 FOR step IN SELECT * FROM recipe_execution_transitions t WHERE t.deployment_id=drain.deployment_id
   AND t.resource_name=drain.resource_name AND t.id<>claimed.id
   AND (t.before_state->>'version')::bigint<endpoint_version
   AND (t.after_state->>'version')::bigint>(claimed.before_state->>'version')::bigint
   ORDER BY (t.before_state->>'version')::bigint,t.id LOOP
  SELECT * INTO current_command FROM recipe_deployment_commands WHERE id=step.command_id;
  SELECT * INTO proof FROM recipe_effect_verifications WHERE id=step.verification_id;
  IF step.attempt_id IS DISTINCT FROM drain.id OR step.kind NOT IN ('ambiguous','reconciled','completed')
   OR step.before_state IS DISTINCT FROM state OR octet_length(step.fingerprint)<>32
   OR (step.after_state->>'version')::bigint>endpoint_version
   OR ROW(step.deployment_id,step.project_id,step.resource_name)
      IS DISTINCT FROM ROW(drain.deployment_id,drain.project_id,drain.resource_name)
   OR current_command.operation IS DISTINCT FROM 'remove' OR current_command.deployment_id IS DISTINCT FROM drain.deployment_id
   OR current_command.project_id IS DISTINCT FROM drain.project_id OR current_command.actor_id IS DISTINCT FROM step.actor_id
   OR step.request_id='00000000-0000-0000-0000-000000000000'::uuid
   OR step.after_state->>'diagnostic' IS NULL OR step.after_state->>'diagnostic' NOT IN ('provider_failure','provider_outcome_unknown',
      'dependency_unavailable','authorization_changed','drain_pending','fencing_unproven','intent_conflict')
   OR step.after_lifecycle IS DISTINCT FROM 'recovery_required'
   OR step.after_deployment_version IS DISTINCT FROM step.before_deployment_version+1
   OR (step.kind IN ('ambiguous','completed') AND (step.operation_id IS DISTINCT FROM drain.id OR step.command_id IS DISTINCT FROM drain.command_id
      OR (state->>'version')::bigint IS DISTINCT FROM (drain.claim_json->>'resource_version')::bigint))
   OR (step.kind IN ('ambiguous','reconciled') AND step.verification_id IS NOT NULL)
   OR (step.kind='reconciled' AND (step.operation_id=drain.id OR EXISTS(SELECT 1 FROM recipe_effect_attempts WHERE id=step.operation_id)
      OR EXISTS(SELECT 1 FROM recipe_effect_verifications WHERE verification_attempt_id=step.operation_id)
      OR (SELECT count(*) FROM recipe_execution_transitions WHERE operation_id=step.operation_id)<>1
      OR (step.command_id<>drain.command_id AND NOT EXISTS(SELECT 1 FROM recipe_remove_resume_admissions admission
       WHERE admission.command_id=step.command_id AND admission.deployment_id=drain.deployment_id
        AND admission.platform_json=header.platform_json AND EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'claims') item
         WHERE item->'claim'=drain.claim_json)))))
   OR (step.kind='completed' AND (proof.id IS NULL OR proof.recorded_by IS DISTINCT FROM 'hephaestus_worker'
      OR proof.outcome IS DISTINCT FROM 'failed' OR proof.action IS DISTINCT FROM 'drain'
      OR ROW(proof.attempt_id,proof.verification_attempt_id,proof.deployment_id,proof.project_id,proof.resource_name,proof.generation,proof.input_hash)
       IS DISTINCT FROM ROW(drain.id,step.operation_id,drain.deployment_id,drain.project_id,drain.resource_name,drain.generation,drain.input_hash)
      OR proof.identity_json IS DISTINCT FROM drain.claim_json->'identity' OR proof.diagnostic IS DISTINCT FROM step.after_state->>'diagnostic'))
   OR step.after_state IS DISTINCT FROM recipe_effect_expected_state(state,drain.before_state,'drain',step.kind,
      CASE WHEN step.kind='completed' THEN 'failed' ELSE 'unproven' END,step.after_state->>'diagnostic',drain.id)
   OR step.after_state->>'active_attempt' IS DISTINCT FROM drain.id::text
   OR step.after_state->>'generation' IS DISTINCT FROM drain.generation::text
   OR (step.after_state->>'version')::bigint IS DISTINCT FROM (state->>'version')::bigint+1
   OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox product ON product.event_id=event.id
    WHERE event.id=step.event_id AND event.occurrence_id=step.id AND event.scope_kind='project' AND event.scope_id=drain.project_id
     AND event.aggregate_type='project' AND event.aggregate_id=drain.project_id AND event.event_type='project.changed'
     AND event.change_kind='updated' AND event.actor_type='user' AND event.actor_id=step.actor_id AND event.request_id=step.request_id
     AND event.cursor=step.event_cursor AND event.aggregate_version=step.event_aggregate_version) THEN
   RAISE EXCEPTION 'complete contiguous protected same-original active progress required' USING ERRCODE='P1420'; END IF;
  state:=step.after_state;
 END LOOP;
 IF state IS DISTINCT FROM p_endpoint THEN
  RAISE EXCEPTION 'exact immutable positive transition endpoint required' USING ERRCODE='P1420'; END IF;
 RETURN;
END $$;

CREATE FUNCTION pending_instance_drain142(p_deployment uuid,p_resource text) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE resource recipe_deployment_resources%ROWTYPE; deployment recipe_deployments%ROWTYPE;
 child recipe_prepared_instance_imports%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 drain recipe_effect_attempts%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 original recipe_effect_attempts%ROWTYPE; creator recipe_deployment_commands%ROWTYPE;
 claimed recipe_execution_transitions%ROWTYPE; step recipe_execution_transitions%ROWTYPE;
 current_command recipe_deployment_commands%ROWTYPE; state jsonb; removal uuid; original_key bytea;
BEGIN
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=p_deployment AND resource_name=p_resource;
 SELECT * INTO deployment FROM recipe_deployments WHERE id=p_deployment;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE deployment_id=p_deployment AND resource_name=p_resource;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=p_deployment;
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=resource.active_attempt_id;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=drain.command_id;
 IF resource.resource_kind IS DISTINCT FROM 'instance' OR resource.ownership IS DISTINCT FROM 'owned'
  OR resource.resource_id IS DISTINCT FROM child.instance_id OR resource.revision_id IS DISTINCT FROM child.revision_id
  OR deployment.id IS NULL OR deployment.lifecycle NOT IN ('removing','recovery_required')
  OR deployment.project_id IS DISTINCT FROM child.project_id OR deployment.input_hash IS DISTINCT FROM header.intent_hash
  OR resource.install_progress IS DISTINCT FROM 'ready' OR resource.removal_progress NOT IN ('draining','recovery_required')
  OR drain.id IS NULL OR drain.action IS DISTINCT FROM 'drain' OR drain.generation<=0
  OR drain.deployment_id IS DISTINCT FROM p_deployment OR drain.project_id IS DISTINCT FROM child.project_id
  OR drain.resource_name IS DISTINCT FROM p_resource OR drain.generation IS DISTINCT FROM resource.execution_generation
  OR drain.input_hash IS DISTINCT FROM resource.input_hash OR drain.actor_id IS DISTINCT FROM command.actor_id
  OR command.operation IS DISTINCT FROM 'remove' OR command.deployment_id IS DISTINCT FROM p_deployment
  OR command.project_id IS DISTINCT FROM child.project_id OR drain.request_id='00000000-0000-0000-0000-000000000000'::uuid
  OR drain.claim_json IS DISTINCT FROM jsonb_build_object('command_id',drain.command_id,'deployment_id',p_deployment,
    'resource',p_resource,'identity',resource.plan_json->'identity','input_hash',recipe_effect_hash_json(resource.input_hash),
    'resource_version',(drain.before_state->>'version')::bigint+1,'generation',drain.generation,
    'action','drain','attempt_id',drain.id,'actor_id',drain.actor_id,'request_id',drain.request_id) THEN
  RAISE EXCEPTION 'positive immutable owned original Drain required' USING ERRCODE='P1420'; END IF;
 SELECT * INTO original FROM recipe_effect_attempts WHERE id=queued_create_original140(child.instance_id);
 SELECT * INTO creator FROM recipe_deployment_commands WHERE id=original.command_id;
 IF prepared_instance_target118(original.id,original.claim_json,convert_from(child.canonical_import,'UTF8')::jsonb)
    IS DISTINCT FROM child.instance_id OR prepared_instance_created118(child.instance_id,original.id) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'positive complete original Create required' USING ERRCODE='P1420'; END IF;
 state:=recipe_effect_resource_state(resource);
 PERFORM prepared_drain_progress142(drain.id,state);
 IF resource.removal_progress='recovery_required' AND deployment.lifecycle<>'recovery_required' THEN
  RAISE EXCEPTION 'actual recovery progress requires exact recovery lifecycle' USING ERRCODE='P1420'; END IF;
 removal:=original_drain_removal118(drain.id,child.instance_id);
 original_key:=original_drain_command_key118(drain.id,child.instance_id);
 -- Absence is checked only after every positive original root above, and never
 -- reused to qualify an already published closure.
 IF EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=child.instance_id OR id=removal)
  OR EXISTS(SELECT 1 FROM release_command_inbox WHERE command_key=original_key OR secondary_id=removal)
  OR EXISTS(SELECT 1 FROM agent_instance_events WHERE payload->>'removal_id'=removal::text) THEN
  RAISE EXCEPTION 'conflicting or already published closure is not Pending' USING ERRCODE='P1420'; END IF;
 RETURN jsonb_build_object('resource',p_resource,'drain_claim',drain.claim_json,'actor',command.actor_id,'idempotency',command.idempotency_id,
  'create_claim',original.claim_json,'create_actor',creator.actor_id,'create_idempotency',creator.idempotency_id,
  'prepared',convert_from(child.canonical_import,'UTF8')::jsonb,'observed_progress',state);
END $$;

CREATE FUNCTION prepared_closure_origin142(p_instance uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 removal agent_instance_removal_requests%ROWTYPE; inbox release_command_inbox%ROWTYPE;
 drain recipe_effect_attempts%ROWTYPE; command recipe_deployment_commands%ROWTYPE; original recipe_effect_attempts%ROWTYPE;
 creator recipe_deployment_commands%ROWTYPE; admission recipe_remove_resume_admissions%ROWTYPE;
 publisher recipe_deployment_commands%ROWTYPE; candidates uuid[]; pending jsonb; result jsonb;
BEGIN
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id;
 SELECT * INTO original FROM recipe_effect_attempts WHERE id=queued_create_original140(p_instance);
 SELECT * INTO creator FROM recipe_deployment_commands WHERE id=original.command_id;
 IF (SELECT count(*) FROM agent_instance_removal_requests WHERE instance_id=p_instance)<>1 THEN
  RAISE EXCEPTION 'one exact actual closure required' USING ERRCODE='P1420'; END IF;
 SELECT * INTO removal FROM agent_instance_removal_requests WHERE instance_id=p_instance;
 SELECT array_agg(id ORDER BY id) INTO candidates FROM recipe_effect_attempts WHERE deployment_id=child.deployment_id
  AND resource_name=child.resource_name AND action='drain' AND original_drain_removal118(id,p_instance)=removal.id;
 IF COALESCE(cardinality(candidates),0)<>1 THEN RAISE EXCEPTION 'one original Drain closure required' USING ERRCODE='P1420'; END IF;
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=candidates[1];
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=drain.command_id;
 IF drain.project_id IS DISTINCT FROM child.project_id OR command.operation IS DISTINCT FROM 'remove'
  OR command.actor_id IS DISTINCT FROM drain.actor_id OR command.project_id IS DISTINCT FROM child.project_id
  OR command.deployment_id IS DISTINCT FROM child.deployment_id
  OR drain.input_hash IS DISTINCT FROM (SELECT input_hash FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name)
  OR drain.claim_json->'identity' IS DISTINCT FROM (SELECT plan_json->'identity' FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name)
  OR (SELECT count(*) FROM recipe_execution_transitions WHERE attempt_id=drain.id AND kind='claimed')<>1
  OR NOT EXISTS(SELECT 1 FROM recipe_execution_transitions claimed JOIN application_events event ON event.id=claimed.event_id
    JOIN product_event_outbox product ON product.event_id=event.id WHERE claimed.attempt_id=drain.id AND claimed.kind='claimed'
     AND claimed.command_id=drain.command_id AND claimed.project_id=drain.project_id AND claimed.resource_name=drain.resource_name
     AND claimed.operation_id=drain.id AND claimed.fingerprint=drain.request_fingerprint AND claimed.before_state=drain.before_state
     AND claimed.after_state->>'active_attempt'=drain.id::text AND claimed.after_state->>'generation'=drain.generation::text
     AND event.actor_id=drain.actor_id AND event.request_id=drain.request_id
     AND event.cursor=claimed.event_cursor AND event.aggregate_version=claimed.event_aggregate_version)
  OR (SELECT count(*) FROM release_command_inbox WHERE secondary_id=removal.id OR command_key=original_drain_command_key118(drain.id,p_instance))<>1 THEN
  RAISE EXCEPTION 'complete immutable original and unique publisher inbox required' USING ERRCODE='P1420'; END IF;
 SELECT * INTO inbox FROM release_command_inbox WHERE secondary_id=removal.id;
 IF inbox.operation IS DISTINCT FROM 'request_instance_removal' OR inbox.command_key IS DISTINCT FROM original_drain_command_key118(drain.id,p_instance)
  OR inbox.aggregate_id IS DISTINCT FROM p_instance OR inbox.actor_id IS DISTINCT FROM removal.created_by
  OR inbox.request_id IS DISTINCT FROM removal.request_id OR inbox.input_hash IS DISTINCT FROM removal.input_hash
  OR removal.expected_version<1 OR removal.input_hash IS DISTINCT FROM sha256(convert_to(recipe_import107_json_bytes(jsonb_build_object(
    'version',1,'operation','request_instance_removal','actor',removal.created_by,'instance',p_instance,
    'removal',removal.id,'expected_version',removal.expected_version),0),'UTF8'))
  OR prepared_instance_removal118(p_instance) IS DISTINCT FROM removal.id
  OR EXISTS(SELECT 1 FROM agent_instance_volume_mount_grants grant_row WHERE instance_id=p_instance
    AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_revocations revoked WHERE revoked.grant_id=grant_row.id)) THEN
  RAISE EXCEPTION 'genuine permanent closure and actual grant withdrawal required' USING ERRCODE='P1420'; END IF;
 result:=jsonb_build_object('resource',child.resource_name,'drain_claim',drain.claim_json,'actor',command.actor_id,
  'idempotency',command.idempotency_id,'removal',prepared_closure_row142(removal),'inbox',to_jsonb(inbox));
 IF removal.prepared_resume_command_id IS NULL THEN
  IF removal.created_by IS DISTINCT FROM drain.actor_id THEN RAISE EXCEPTION 'strict original Drain publisher required' USING ERRCODE='P1420'; END IF;
  RETURN result;
 END IF;
 SELECT * INTO admission FROM recipe_remove_resume_admissions WHERE command_id=removal.prepared_resume_command_id;
 SELECT * INTO publisher FROM recipe_deployment_commands WHERE id=admission.command_id;
 IF admission.command_id IS NULL OR admission.actor_id IS DISTINCT FROM removal.created_by
  OR publisher.operation IS DISTINCT FROM 'remove' OR publisher.actor_id IS DISTINCT FROM admission.actor_id
  OR publisher.deployment_id IS DISTINCT FROM child.deployment_id OR publisher.project_id IS DISTINCT FROM child.project_id
  OR publisher.first_request_id IS DISTINCT FROM admission.request_id OR publisher.event_id IS DISTINCT FROM admission.event_id
  OR publisher.receipt_version IS DISTINCT FROM admission.expected_version OR publisher.receipt_lifecycle NOT IN ('removing','recovery_required')
  OR publisher.input_hash IS DISTINCT FROM sha256(convert_to(format('["hephaestus-recipe-remove-v1","%s",%s]',child.deployment_id,admission.expected_version),'UTF8'))
  OR admission.deployment_id IS DISTINCT FROM child.deployment_id OR admission.project_id IS DISTINCT FROM child.project_id
  OR admission.intent_hash IS DISTINCT FROM header.intent_hash OR admission.preparation_hash IS DISTINCT FROM header.input_hash
  OR admission.platform_json IS DISTINCT FROM header.platform_json
  OR NOT run_cleanup_identifier_is_valid(admission.vm_provider_namespace) OR NOT run_cleanup_identifier_is_valid(admission.vm_host_id)
  OR (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'pending') item WHERE item->>'resource'=child.resource_name)<>1
  OR EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=child.resource_name)
  OR (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'claims') item WHERE item->'claim'=drain.claim_json)<>1
  OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox product ON product.event_id=event.id
    WHERE event.id=admission.event_id AND event.actor_id=admission.actor_id AND event.request_id=admission.request_id
     AND event.scope_kind='project' AND event.scope_id=child.project_id AND event.occurrence_id=publisher.idempotency_id
     AND event.cursor=publisher.event_cursor AND event.aggregate_version=publisher.event_aggregate_version) THEN
  RAISE EXCEPTION 'exact immutable qualified fresh Remove publication link required' USING ERRCODE='P1420'; END IF;
 SELECT item INTO pending FROM jsonb_array_elements(admission.context_json->'pending') item WHERE item->>'resource'=child.resource_name;
 -- The immutable admission guard constructed this positive native progress. Its
 -- root is requalified here, without rerunning live preclosure absence/CAS.
 IF pending-ARRAY['observed_progress'] IS DISTINCT FROM jsonb_build_object('resource',child.resource_name,'drain_claim',drain.claim_json,
    'actor',command.actor_id,'idempotency',command.idempotency_id,'create_claim',original.claim_json,
    'create_actor',creator.actor_id,'create_idempotency',creator.idempotency_id,'prepared',convert_from(child.canonical_import,'UTF8')::jsonb)
  OR pending->'observed_progress'->>'active_attempt' IS DISTINCT FROM drain.id::text
  OR pending->'observed_progress'->>'generation' IS DISTINCT FROM drain.generation::text
  OR pending->'observed_progress'->>'removal' NOT IN ('draining','recovery_required') THEN
  RAISE EXCEPTION 'stored Pending must retain exact immutable original roots' USING ERRCODE='P1420'; END IF;
 PERFORM prepared_drain_progress142(drain.id,pending->'observed_progress');
 RETURN result||jsonb_build_object('publication',jsonb_build_object('kind','qualified_fresh_remove','command_id',publisher.id,
  'actor',publisher.actor_id,'idempotency',publisher.idempotency_id,'admission_request',admission.request_id,
  'publication_request',removal.request_id,'admission_version',admission.expected_version,'pending',pending));
END $$;

CREATE FUNCTION prepared_resume_inventory142(p_deployment uuid,p_namespace text,p_host text,p_platform jsonb) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; execution runs%ROWTYPE;
BEGIN
 IF NOT run_cleanup_identifier_is_valid(p_namespace) OR NOT run_cleanup_identifier_is_valid(p_host)
  OR NOT EXISTS(SELECT 1 FROM recipe_deployment_execution_preparations WHERE deployment_id=p_deployment
    AND profile='runtime_named_v1' AND input_hash=sha256(canonical_input) AND platform_json=p_platform) THEN
  RAISE EXCEPTION 'exact configured original runtime preparation required' USING ERRCODE='P1420'; END IF;
 FOR child IN SELECT * FROM recipe_prepared_instance_imports WHERE deployment_id=p_deployment ORDER BY resource_name LOOP
  PERFORM queued_create_original140(child.instance_id);
  IF queued_resume_associations140(child.instance_id) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'whole original association inventory required even for empty instances' USING ERRCODE='P1420'; END IF;
 END LOOP;
 FOR execution IN SELECT queued_run.* FROM runs queued_run JOIN recipe_prepared_instance_imports prepared_child
   ON prepared_child.instance_id=queued_run.instance_id WHERE prepared_child.deployment_id=p_deployment ORDER BY queued_run.id LOOP
  IF invocation119_remove_resume_matches(execution.id,p_namespace,p_host,p_platform) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'every genuine configured Invocation original required' USING ERRCODE='P1420'; END IF;
 END LOOP;
END $$;

CREATE FUNCTION prepare_resumed_prepared_instance_closure142(p_command uuid,p_create uuid,p_create_claim jsonb,p_prepared jsonb,
 p_whole bytea,p_drain jsonb,p_pending jsonb,p_namespace text,p_host text,p_platform jsonb,p_deployment_cas bigint,
 p_key bytea,p_removal uuid,p_instance_cas bigint,p_hash bytea) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE consumer uuid; original recipe_effect_attempts%ROWTYPE; drain recipe_effect_attempts%ROWTYPE;
 admission recipe_remove_resume_admissions%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 header recipe_deployment_execution_preparations%ROWTYPE; pending jsonb; winner jsonb;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app'
  OR current_setting('transaction_isolation')<>'read committed' OR current_setting('transaction_read_only')<>'off' THEN
  RAISE EXCEPTION 'owning current manager writable READ COMMITTED transaction required' USING ERRCODE='42501'; END IF;
 consumer:=prepared_instance_target118(p_create,p_create_claim,p_prepared);
 SELECT * INTO original FROM recipe_effect_attempts WHERE id=p_create;
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=(p_drain->>'attempt_id')::uuid;
 PERFORM prepared_instance_actor118(p_command,p_create);
 PERFORM remove_resume_actor118(original.deployment_id);
 SELECT * INTO admission FROM recipe_remove_resume_admissions WHERE command_id=p_command;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=p_command;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=original.deployment_id;
 IF admission.command_id IS NULL OR admission.actor_id IS DISTINCT FROM command.actor_id
  OR command.actor_id::text IS DISTINCT FROM hephaestus_actor_id() OR command.idempotency_id::text IS DISTINCT FROM current_setting('hephaestus.occurrence_id',true)
  OR admission.deployment_id IS DISTINCT FROM original.deployment_id OR admission.project_id IS DISTINCT FROM original.project_id
  OR admission.intent_hash IS DISTINCT FROM header.intent_hash OR admission.preparation_hash IS DISTINCT FROM header.input_hash
  OR admission.vm_provider_namespace IS DISTINCT FROM p_namespace OR admission.vm_host_id IS DISTINCT FROM p_host
  OR admission.platform_json IS DISTINCT FROM p_platform OR header.canonical_input IS DISTINCT FROM p_whole
  OR p_create IS DISTINCT FROM queued_create_original140(consumer)
  OR drain.claim_json IS DISTINCT FROM p_drain OR drain.action IS DISTINCT FROM 'drain'
  OR drain.deployment_id IS DISTINCT FROM original.deployment_id OR drain.resource_name IS DISTINCT FROM original.resource_name
  OR drain.project_id IS DISTINCT FROM original.project_id OR drain.input_hash IS DISTINCT FROM original.input_hash
  OR drain.claim_json->'identity' IS DISTINCT FROM original.claim_json->'identity'
  OR p_removal IS DISTINCT FROM original_drain_removal118(drain.id,consumer)
  OR p_key IS DISTINCT FROM original_drain_command_key118(drain.id,consumer)
  OR p_instance_cas IS NULL OR p_instance_cas<=0 OR p_deployment_cas IS NULL OR p_deployment_cas<=0
  OR (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'pending') item WHERE item->>'resource'=original.resource_name)<>1 THEN
  RAISE EXCEPTION 'exact admitted current command and original Pending inputs required' USING ERRCODE='P1420'; END IF;
 SELECT item INTO pending FROM jsonb_array_elements(admission.context_json->'pending') item WHERE item->>'resource'=original.resource_name;
 IF p_pending IS DISTINCT FROM pending THEN RAISE EXCEPTION 'immutable admitted Pending comparison required' USING ERRCODE='P1420'; END IF;
 PERFORM lock_prepared_closure_parents118(consumer,p_key);
 PERFORM lock_remove_resume118(original.deployment_id);
 PERFORM prepared_instance_actor118(p_command,p_create);
 PERFORM prepared_resume_inventory142(original.deployment_id,p_namespace,p_host,p_platform);
 -- A genuine winner is qualified BEFORE any live Pending-only absence or CAS.
 IF EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=consumer OR id=p_removal)
  OR EXISTS(SELECT 1 FROM release_command_inbox WHERE command_key=p_key OR secondary_id=p_removal) THEN
  winner:=prepared_closure_origin142(consumer);
  IF winner->'drain_claim' IS DISTINCT FROM p_drain OR winner->'removal'->>'id' IS DISTINCT FROM p_removal::text
   OR (winner ? 'publication' AND (winner->'publication'->'pending')-ARRAY['observed_progress'] IS DISTINCT FROM pending-ARRAY['observed_progress']) THEN
   RAISE EXCEPTION 'winner must preserve the same exact original Pending roots' USING ERRCODE='P1420'; END IF;
  RETURN true;
 END IF;
 IF p_deployment_cas IS DISTINCT FROM admission.expected_version
  OR pending_instance_drain142(original.deployment_id,original.resource_name) IS DISTINCT FROM pending
  OR NOT EXISTS(SELECT 1 FROM recipe_deployments WHERE id=original.deployment_id AND version=p_deployment_cas
    AND lifecycle IN ('removing','recovery_required'))
  OR NOT EXISTS(SELECT 1 FROM agent_instances WHERE id=consumer AND version=p_instance_cas AND state<>'removed')
  OR p_hash IS DISTINCT FROM sha256(convert_to(recipe_import107_json_bytes(jsonb_build_object('version',1,
    'operation','request_instance_removal','actor',command.actor_id,'instance',consumer,'removal',p_removal,
    'expected_version',p_instance_cas),0),'UTF8')) THEN
  RAISE EXCEPTION 'current original progress, deployment/instance CAS and publisher fingerprint required' USING ERRCODE='P1420'; END IF;
 RETURN false;
END $$;

CREATE FUNCTION guard_prepared_resume_origin142() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE admission recipe_remove_resume_admissions%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
 header recipe_deployment_execution_preparations%ROWTYPE; pending jsonb;
BEGIN
 IF NEW.prepared_resume_command_id IS NULL THEN RETURN NEW; END IF;
 SELECT * INTO admission FROM recipe_remove_resume_admissions WHERE command_id=NEW.prepared_resume_command_id;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=NEW.instance_id;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id;
 SELECT item INTO pending FROM jsonb_array_elements(admission.context_json->'pending') item WHERE item->>'resource'=child.resource_name;
 IF NEW.created_by IS DISTINCT FROM admission.actor_id OR NEW.created_by::text IS DISTINCT FROM hephaestus_actor_id()
  OR NEW.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
  OR prepare_resumed_prepared_instance_closure142(admission.command_id,(pending->'create_claim'->>'attempt_id')::uuid,
    pending->'create_claim',pending->'prepared',header.canonical_input,pending->'drain_claim',pending,
    admission.vm_provider_namespace,admission.vm_host_id,admission.platform_json,
    admission.expected_version,
    original_drain_command_key118((pending->'drain_claim'->>'attempt_id')::uuid,NEW.instance_id),
    NEW.id,NEW.expected_version,NEW.input_hash) IS DISTINCT FROM false THEN
  RAISE EXCEPTION 'new linked publication requires exact current fresh manager original Pending' USING ERRCODE='P1420'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER aaa_prepared_resume_origin142 BEFORE INSERT ON public.agent_instance_removal_requests
 FOR EACH ROW EXECUTE FUNCTION guard_prepared_resume_origin142();

CREATE FUNCTION validate_prepared_resume_origin142() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE admission recipe_remove_resume_admissions%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
 command recipe_deployment_commands%ROWTYPE; resource record; kind text; origin jsonb;
BEGIN
 IF NEW.prepared_resume_command_id IS NULL THEN RETURN NEW; END IF;
 SELECT * INTO admission FROM recipe_remove_resume_admissions WHERE command_id=NEW.prepared_resume_command_id;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=admission.command_id;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=NEW.instance_id;
 PERFORM remove_resume_actor118(child.deployment_id);
 PERFORM prepared_instance_actor118(command.id,(SELECT id FROM recipe_effect_attempts
   WHERE id=queued_create_original140(NEW.instance_id)));
 IF NEW.created_by::text IS DISTINCT FROM hephaestus_actor_id() OR NEW.created_by IS DISTINCT FROM command.actor_id
  OR NEW.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true) THEN
  RAISE EXCEPTION 'commit requires actual live publication actor/request and ALL current rights' USING ERRCODE='42501'; END IF;
 PERFORM prepared_resume_inventory142(child.deployment_id,admission.vm_provider_namespace,admission.vm_host_id,admission.platform_json);
 origin:=prepared_closure_origin142(NEW.instance_id);
 IF origin->'publication'->>'command_id' IS DISTINCT FROM NEW.prepared_resume_command_id::text THEN
  RAISE EXCEPTION 'complete exact linked publication required at commit' USING ERRCODE='P1420'; END IF;
 FOR resource IN SELECT * FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND ownership='owned' LOOP
  kind:=CASE resource.resource_kind WHEN 'volume' THEN 'state_volume' ELSE 'agent_instance' END;
  IF recipe_owned_resource_project(kind,resource.resource_id) IS NOT NULL AND NOT EXISTS(SELECT 1 FROM authorization_audit_events audit
    WHERE audit.actor_id=NEW.created_by AND audit.request_id=NEW.request_id AND audit.object_type=kind
     AND audit.object_id=resource.resource_id AND audit.permission='can_manage' AND audit.decision='allow') THEN
   RAISE EXCEPTION 'actual publication request ALL owned decision audits required' USING ERRCODE='42501'; END IF;
 END LOOP;
 IF NOT EXISTS(SELECT 1 FROM authorization_audit_events audit WHERE audit.actor_id=NEW.created_by AND audit.request_id=NEW.request_id
   AND audit.object_type='project' AND audit.object_id=child.project_id AND audit.permission='can_manage' AND audit.decision='allow') THEN
  RAISE EXCEPTION 'actual publication project decision audit required' USING ERRCODE='42501'; END IF;
 RETURN NEW;
END $$;
CREATE CONSTRAINT TRIGGER prepared_resume_origin_commit142 AFTER INSERT ON public.agent_instance_removal_requests
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_prepared_resume_origin142();

-- Existing context interface retains its two-key normal wire. Pending is explicit.
CREATE OR REPLACE FUNCTION remove_resume_context118(p_deployment uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE claims jsonb:='[]'; closures jsonb:='[]'; resource recipe_deployment_resources%ROWTYPE; attempt recipe_effect_attempts%ROWTYPE;
 command recipe_deployment_commands%ROWTYPE; child record; removal agent_instance_removal_requests%ROWTYPE;
 inbox release_command_inbox%ROWTYPE; drain recipe_effect_attempts%ROWTYPE; count integer;
 pending jsonb:='[]'; recovery jsonb:='[]'; origin jsonb;
BEGIN
 PERFORM remove_resume_actor118(p_deployment);
 FOR resource IN SELECT * FROM recipe_deployment_resources WHERE deployment_id=p_deployment AND active_attempt_id IS NOT NULL ORDER BY resource_name LOOP
  SELECT * INTO attempt FROM recipe_effect_attempts WHERE id=resource.active_attempt_id;
  SELECT * INTO command FROM recipe_deployment_commands WHERE id=attempt.command_id;
  IF resource.ownership IS DISTINCT FROM 'owned' OR attempt.action NOT IN ('drain','detach','retain') OR command.operation IS DISTINCT FROM 'remove'
   OR attempt.deployment_id IS DISTINCT FROM resource.deployment_id OR attempt.resource_name IS DISTINCT FROM resource.resource_name
   OR attempt.project_id IS DISTINCT FROM resource.project_id OR attempt.generation IS DISTINCT FROM resource.execution_generation
   OR attempt.input_hash IS DISTINCT FROM resource.input_hash OR attempt.claim_json->'identity' IS DISTINCT FROM resource.plan_json->'identity'
   OR NOT EXISTS(SELECT 1 FROM recipe_execution_transitions t WHERE t.attempt_id=attempt.id AND t.kind='claimed'
     AND t.command_id=attempt.command_id AND t.project_id=attempt.project_id AND t.resource_name=attempt.resource_name
     AND t.operation_id=attempt.id AND t.fingerprint=attempt.request_fingerprint AND t.before_state=attempt.before_state
     AND t.after_state->>'active_attempt'=attempt.id::text AND t.after_state->>'generation'=attempt.generation::text) THEN
   RAISE EXCEPTION 'exact original supported active removal required' USING ERRCODE='P1180'; END IF;
  IF resource.resource_kind='instance' AND attempt.action='drain' THEN
   -- Both Pending and published closures require the SAME positive current
   -- active original prefix; a closure does not erase RecoveryRequired work.
   PERFORM prepared_drain_progress142(attempt.id,recipe_effect_resource_state(resource));
   IF resource.removal_progress='recovery_required' AND NOT EXISTS(SELECT 1 FROM recipe_deployments
     WHERE id=p_deployment AND lifecycle='recovery_required') THEN
    RAISE EXCEPTION 'current closed or Pending Recovery requires actual Recovery lifecycle' USING ERRCODE='P1420'; END IF;
  END IF;
  claims:=claims||jsonb_build_array(jsonb_build_object('claim',attempt.claim_json,'actor',command.actor_id,'idempotency',command.idempotency_id));
 END LOOP;
 FOR child IN SELECT * FROM recipe_prepared_instance_imports WHERE deployment_id=p_deployment ORDER BY resource_name LOOP
  IF EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=child.instance_id) THEN
   origin:=prepared_closure_origin142(child.instance_id);
   closures:=closures||jsonb_build_array(origin);
   SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=p_deployment AND resource_name=child.resource_name;
   SELECT * INTO attempt FROM recipe_effect_attempts WHERE id=resource.active_attempt_id;
   IF resource.active_attempt_id IS NOT NULL AND attempt.action='drain' THEN
    IF origin->'drain_claim' IS DISTINCT FROM attempt.claim_json THEN
     RAISE EXCEPTION 'active published Drain must retain the exact original claim' USING ERRCODE='P1420'; END IF;
    IF resource.removal_progress='recovery_required' THEN
     recovery:=recovery||jsonb_build_array(jsonb_build_object('resource',child.resource_name,
      'drain_claim',attempt.claim_json,'observed_progress',recipe_effect_resource_state(resource)));
    END IF;
   END IF;
  ELSIF EXISTS(SELECT 1 FROM recipe_deployment_resources associated WHERE associated.deployment_id=p_deployment
     AND associated.resource_name=child.resource_name AND associated.active_attempt_id IS NOT NULL) THEN
   pending:=pending||jsonb_build_array(pending_instance_drain142(p_deployment,child.resource_name));
  END IF;
 END LOOP;
 IF jsonb_array_length(recovery)>0 THEN
  RETURN jsonb_build_object('claims',claims,'closures',closures,'recovery',recovery)
   ||CASE WHEN jsonb_array_length(pending)>0 THEN jsonb_build_object('pending',pending) ELSE '{}'::jsonb END;
 END IF;
 IF jsonb_array_length(pending)>0 THEN
  RETURN jsonb_build_object('claims',claims,'closures',closures,'pending',pending);
 END IF;
 RETURN jsonb_build_object('claims',claims,'closures',closures);
END $$;

CREATE OR REPLACE FUNCTION queued_processed_closure140(p_run uuid,p_instance uuid,p_original jsonb) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
 drain recipe_effect_attempts%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 removal agent_instance_removal_requests%ROWTYPE; inbox release_command_inbox%ROWTYPE;
 expected jsonb; cancel jsonb; admission recipe_remove_resume_admissions%ROWTYPE; recorded jsonb; closure jsonb;
BEGIN
 -- Global Run selection precedes ownership checks; no filtered absence inference.
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.id IS NULL OR execution.instance_id IS DISTINCT FROM p_instance OR execution.cancel_requested_at IS NULL THEN
  RAISE EXCEPTION 'exact original cancelled Run required' USING ERRCODE='P1180'; END IF;
 PERFORM queued_create_original140(p_instance);
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
 closure:=prepared_closure_origin142(p_instance);
 IF p_original IS DISTINCT FROM expected OR drain.id IS NULL OR removal.id IS NULL
  OR drain.action IS DISTINCT FROM 'drain' OR command.operation IS DISTINCT FROM 'remove'
  OR command.deployment_id IS DISTINCT FROM child.deployment_id OR command.project_id IS DISTINCT FROM child.project_id
  OR command.actor_id IS DISTINCT FROM drain.actor_id OR closure->'drain_claim' IS DISTINCT FROM drain.claim_json
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
    'actor',removal.created_by,'instance',p_instance,'removal',removal.id,'expected_version',removal.expected_version),0),'UTF8'))
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
  IF (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'pending') item WHERE item->>'resource'=child.resource_name)>1
   OR EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'pending') item WHERE item->>'resource'=child.resource_name
    AND (item->'drain_claim' IS DISTINCT FROM drain.claim_json OR recorded IS NOT NULL)) THEN
   RAISE EXCEPTION 'admitted Pending original conflicts with exact closure' USING ERRCODE='P1420'; END IF;
  IF recorded IS NOT NULL AND recorded IS DISTINCT FROM closure THEN
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

CREATE OR REPLACE FUNCTION queued_cancel_closure140(p_run uuid,p_instance uuid,p_original jsonb,p_cancel jsonb) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
 drain recipe_effect_attempts%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 removal agent_instance_removal_requests%ROWTYPE; inbox release_command_inbox%ROWTYPE;
 expected jsonb; cancel jsonb; admission recipe_remove_resume_admissions%ROWTYPE; recorded jsonb; closure jsonb;
BEGIN
 -- Global Run selection precedes ownership checks; no filtered absence inference.
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.id IS NULL OR execution.instance_id IS DISTINCT FROM p_instance OR execution.cancel_requested_at IS NULL THEN
  RAISE EXCEPTION 'exact original cancelled Run required' USING ERRCODE='P1180'; END IF;
 PERFORM queued_create_original140(p_instance);
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
 closure:=prepared_closure_origin142(p_instance);
 IF p_original IS DISTINCT FROM expected OR drain.id IS NULL OR removal.id IS NULL
  OR drain.action IS DISTINCT FROM 'drain' OR command.operation IS DISTINCT FROM 'remove'
  OR command.deployment_id IS DISTINCT FROM child.deployment_id OR command.project_id IS DISTINCT FROM child.project_id
  OR command.actor_id IS DISTINCT FROM drain.actor_id OR closure->'drain_claim' IS DISTINCT FROM drain.claim_json
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
    'actor',removal.created_by,'instance',p_instance,'removal',removal.id,'expected_version',removal.expected_version),0),'UTF8'))
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
  IF (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'pending') item WHERE item->>'resource'=child.resource_name)>1
   OR EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'pending') item WHERE item->>'resource'=child.resource_name
    AND (item->'drain_claim' IS DISTINCT FROM drain.claim_json OR recorded IS NOT NULL)) THEN
   RAISE EXCEPTION 'admitted Pending original conflicts with exact closure' USING ERRCODE='P1420'; END IF;
  IF recorded IS NOT NULL AND recorded IS DISTINCT FROM closure THEN
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

CREATE OR REPLACE FUNCTION queued_removal_origin140(p_instance uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; removal agent_instance_removal_requests%ROWTYPE;
 inbox release_command_inbox%ROWTYPE; drain recipe_effect_attempts%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 candidates uuid[]; original jsonb; expected jsonb; admission recipe_remove_resume_admissions%ROWTYPE; recorded jsonb; closure jsonb;
BEGIN
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 IF child.instance_id IS NULL OR (SELECT count(*) FROM agent_instance_removal_requests WHERE instance_id=p_instance)<>1 THEN
  RAISE EXCEPTION 'one genuine instance removal origin required' USING ERRCODE='P1400'; END IF;
 SELECT * INTO removal FROM agent_instance_removal_requests WHERE instance_id=p_instance;
 IF (SELECT count(*) FROM release_command_inbox WHERE operation='request_instance_removal' AND secondary_id=removal.id)<>1 THEN
  RAISE EXCEPTION 'one original removal command required' USING ERRCODE='P1400'; END IF;
 SELECT * INTO inbox FROM release_command_inbox WHERE operation='request_instance_removal' AND secondary_id=removal.id;
 SELECT array_agg(id ORDER BY id) INTO candidates FROM recipe_effect_attempts
  WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name AND action='drain'
   AND original_drain_removal118(id,p_instance)=removal.id;
 IF COALESCE(cardinality(candidates),0)<>1 THEN
  RAISE EXCEPTION 'one immutable original Drain required' USING ERRCODE='P1400'; END IF;
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=candidates[1];
 original:=jsonb_build_object('drain_attempt',drain.id,'drain_command',drain.command_id,'removal_id',removal.id,
  'command_key',recipe_effect_hash_json(inbox.command_key),'input_hash',recipe_effect_hash_json(removal.input_hash),
  'expected_version',removal.expected_version,'creator',removal.created_by,'request',removal.request_id);
 PERFORM queued_create_original140(p_instance);
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=(original->>'drain_attempt')::uuid;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=drain.command_id;
 SELECT * INTO removal FROM agent_instance_removal_requests WHERE id=(original->>'removal_id')::uuid;
 IF (SELECT count(*) FROM agent_instance_removal_requests WHERE instance_id=p_instance)<>1
  OR (SELECT count(*) FROM release_command_inbox WHERE secondary_id=removal.id AND operation='request_instance_removal')<>1 THEN
  RAISE EXCEPTION 'unique original permanent closure required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO inbox FROM release_command_inbox WHERE secondary_id=removal.id AND operation='request_instance_removal';
 expected:=jsonb_build_object('drain_attempt',drain.id,'drain_command',drain.command_id,'removal_id',removal.id,
  'command_key',recipe_effect_hash_json(inbox.command_key),'input_hash',recipe_effect_hash_json(removal.input_hash),
  'expected_version',removal.expected_version,'creator',removal.created_by,'request',removal.request_id);
 closure:=prepared_closure_origin142(p_instance);
 IF original IS DISTINCT FROM expected OR drain.id IS NULL OR removal.id IS NULL
  OR drain.action IS DISTINCT FROM 'drain' OR command.operation IS DISTINCT FROM 'remove'
  OR command.deployment_id IS DISTINCT FROM child.deployment_id OR command.project_id IS DISTINCT FROM child.project_id
  OR command.actor_id IS DISTINCT FROM drain.actor_id OR closure->'drain_claim' IS DISTINCT FROM drain.claim_json
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
    'actor',removal.created_by,'instance',p_instance,'removal',removal.id,'expected_version',removal.expected_version),0),'UTF8'))
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
  IF (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'pending') item WHERE item->>'resource'=child.resource_name)>1
   OR EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'pending') item WHERE item->>'resource'=child.resource_name
    AND (item->'drain_claim' IS DISTINCT FROM drain.claim_json OR recorded IS NOT NULL)) THEN
   RAISE EXCEPTION 'admitted Pending original conflicts with exact closure' USING ERRCODE='P1420'; END IF;
  IF recorded IS NOT NULL AND recorded IS DISTINCT FROM closure THEN
   RAISE EXCEPTION 'admitted original closure changed' USING ERRCODE='P1180'; END IF;
 END LOOP;
 RETURN original;
END $$;

CREATE OR REPLACE FUNCTION guard_remove_resume_admission118() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE deployment recipe_deployments%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 header recipe_deployment_execution_preparations%ROWTYPE; resource record; kind text;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'Remove resume admission is immutable' USING ERRCODE='23000'; END IF;
 PERFORM lock_remove_resume118(NEW.deployment_id);
 PERFORM remove_resume_actor118(NEW.deployment_id);
 SELECT * INTO deployment FROM recipe_deployments WHERE id=NEW.deployment_id;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=NEW.command_id;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=NEW.deployment_id;
 PERFORM prepared_resume_inventory142(NEW.deployment_id,NEW.vm_provider_namespace,NEW.vm_host_id,NEW.platform_json);
 IF deployment.id IS NULL OR deployment.lifecycle NOT IN ('removing','recovery_required') OR deployment.version IS DISTINCT FROM NEW.expected_version
  OR (deployment.lifecycle='recovery_required'
   AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(NEW.context_json->'pending') item
    WHERE item->'observed_progress'->>'removal'='recovery_required')
   AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(NEW.context_json->'recovery') item
    WHERE item->'observed_progress'->>'removal'='recovery_required'))
  OR command.operation IS DISTINCT FROM 'remove' OR command.deployment_id IS DISTINCT FROM NEW.deployment_id OR command.project_id IS DISTINCT FROM NEW.project_id
  OR deployment.project_id IS DISTINCT FROM NEW.project_id OR deployment.input_hash IS DISTINCT FROM NEW.intent_hash
  OR header.input_hash IS DISTINCT FROM NEW.preparation_hash OR header.platform_json IS DISTINCT FROM NEW.platform_json
  OR NEW.actor_id::text IS DISTINCT FROM hephaestus_actor_id() OR NEW.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
  OR command.actor_id IS DISTINCT FROM NEW.actor_id OR command.first_request_id IS DISTINCT FROM NEW.request_id OR command.event_id IS DISTINCT FROM NEW.event_id
  OR command.idempotency_id::text IS DISTINCT FROM current_setting('hephaestus.occurrence_id',true)
  OR command.receipt_version IS DISTINCT FROM NEW.expected_version OR command.receipt_lifecycle IS DISTINCT FROM deployment.lifecycle
  OR command.input_hash IS DISTINCT FROM sha256(convert_to(format('["hephaestus-recipe-remove-v1","%s",%s]',deployment.id,NEW.expected_version),'UTF8'))
  OR EXISTS(SELECT 1 FROM recipe_prepared_instance_imports associated WHERE associated.deployment_id=NEW.deployment_id
    AND queued_resume_associations140(associated.instance_id) IS DISTINCT FROM true)
  OR EXISTS(SELECT 1 FROM runs execution JOIN recipe_prepared_instance_imports child ON child.instance_id=execution.instance_id
    WHERE child.deployment_id=NEW.deployment_id AND (
      (execution.execution_profile='owned_canonical' AND execution.vm_provider_namespace=NEW.vm_provider_namespace
       AND execution.vm_host_id=NEW.vm_host_id AND EXISTS(SELECT 1 FROM run_vm_plan_admissions plan
         WHERE plan.run_id=execution.id AND plan.consumed_at IS NOT NULL AND plan.vm_id=execution.vm_id
           AND plan.vm_provider_namespace=execution.vm_provider_namespace AND plan.vm_host_id=execution.vm_host_id))
      OR invocation119_remove_resume_matches(execution.id,NEW.vm_provider_namespace,NEW.vm_host_id,NEW.platform_json) IS NOT DISTINCT FROM true
      OR (execution.execution_profile='legacy_scalar' AND EXISTS(SELECT 1 FROM run_legacy_vm_placements plan
         WHERE plan.run_id=execution.id AND plan.consumed_at IS NOT NULL
           AND plan.provider_namespace::text=NEW.vm_provider_namespace AND plan.host_id=NEW.vm_host_id
           AND legacy_vm_placement_matches(execution,plan)))) IS DISTINCT FROM true)
  OR NEW.context_json IS DISTINCT FROM remove_resume_context118(NEW.deployment_id)
  OR NOT EXISTS(SELECT 1 FROM application_events e JOIN product_event_outbox o ON o.event_id=e.id WHERE e.id=NEW.event_id
    AND e.actor_id=NEW.actor_id AND e.request_id=NEW.request_id AND e.scope_kind='project' AND e.scope_id=NEW.project_id
    AND e.occurrence_id=command.idempotency_id AND e.cursor=command.event_cursor AND e.aggregate_version=command.event_aggregate_version)
  OR NOT EXISTS(SELECT 1 FROM authorization_audit_events a WHERE a.actor_id=NEW.actor_id AND a.request_id=NEW.request_id
    AND a.object_type='project' AND a.object_id=NEW.project_id AND a.permission='can_manage' AND a.decision='allow') THEN
  RAISE EXCEPTION 'fresh exact current manager command/CAS/preparation required' USING ERRCODE='23000'; END IF;
 FOR resource IN SELECT * FROM recipe_deployment_resources WHERE deployment_id=NEW.deployment_id AND ownership='owned' LOOP
  kind:=CASE resource.resource_kind WHEN 'volume' THEN 'state_volume' ELSE 'agent_instance' END;
  IF recipe_owned_resource_project(kind,resource.resource_id) IS NOT NULL AND NOT EXISTS(SELECT 1 FROM authorization_audit_events a
   WHERE a.actor_id=NEW.actor_id AND a.request_id=NEW.request_id AND a.object_type=kind AND a.object_id=resource.resource_id
    AND a.permission='can_manage' AND a.decision='allow') THEN RAISE EXCEPTION 'all owned decision audits required' USING ERRCODE='42501'; END IF;
 END LOOP;
 RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION load_existing_instance_closure118(p_command uuid,p_deployment uuid,p_project uuid,p_intent_hash bytea,
 p_attempt uuid,p_claim jsonb,p_prepared jsonb,p_namespace text,p_host text,p_platform jsonb) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE admission recipe_remove_resume_admissions%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 command recipe_deployment_commands%ROWTYPE; consumer uuid; fresh jsonb; selected jsonb; recorded jsonb;
 original recipe_effect_attempts%ROWTYPE; original_command recipe_deployment_commands%ROWTYPE; count integer;
 recovery_entry record; recovery_original recipe_effect_attempts%ROWTYPE;
 recovery_child recipe_prepared_instance_imports%ROWTYPE; recovery_origin jsonb; has_recovery boolean:=false;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app' THEN
  RAISE EXCEPTION 'current manager actor pool required' USING ERRCODE='42501'; END IF;
 PERFORM remove_resume_actor118(p_deployment);
 PERFORM prepared_instance_actor118(p_command,p_attempt);
 consumer:=prepared_instance_target118(p_attempt,p_claim,p_prepared);
 SELECT * INTO admission FROM recipe_remove_resume_admissions WHERE command_id=p_command;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=p_deployment;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=p_command;
 IF admission.context_json ? 'recovery' AND (jsonb_typeof(admission.context_json->'recovery') IS DISTINCT FROM 'array'
   OR jsonb_array_length(admission.context_json->'recovery')=0) THEN
  RAISE EXCEPTION 'explicit nonempty closed Recovery comparison array required' USING ERRCODE='P1420'; END IF;
 -- Only immutable admission endpoints are requalified here. New admission
 -- independently checks the current active claim/gen/State through context118.
 FOR recovery_entry IN
  SELECT item AS comparison,false AS closed FROM jsonb_array_elements(admission.context_json->'pending') item
   WHERE item->'observed_progress'->>'removal'='recovery_required'
  UNION ALL
  SELECT item AS comparison,true AS closed FROM jsonb_array_elements(admission.context_json->'recovery') item
 LOOP
  SELECT * INTO recovery_original FROM recipe_effect_attempts WHERE id=(recovery_entry.comparison->'drain_claim'->>'attempt_id')::uuid;
  SELECT * INTO recovery_child FROM recipe_prepared_instance_imports WHERE deployment_id=p_deployment
   AND resource_name=recovery_entry.comparison->>'resource';
  IF recovery_original.id IS NULL OR recovery_child.instance_id IS NULL
   OR recovery_original.claim_json IS DISTINCT FROM recovery_entry.comparison->'drain_claim'
   OR recovery_original.deployment_id IS DISTINCT FROM p_deployment OR recovery_original.project_id IS DISTINCT FROM p_project
   OR recovery_original.resource_name IS DISTINCT FROM recovery_entry.comparison->>'resource'
   OR recovery_entry.comparison->'observed_progress'->>'removal' IS DISTINCT FROM 'recovery_required'
   OR (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'claims') item
     WHERE item->'claim'=recovery_original.claim_json)<>1
   OR (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'claims') item
     WHERE item->'claim'->>'resource'=recovery_original.resource_name)<>1 THEN
   RAISE EXCEPTION 'stored Recovery binds one exact original instance Drain claim' USING ERRCODE='P1420'; END IF;
  PERFORM queued_create_original140(recovery_child.instance_id);
  PERFORM prepared_drain_progress142(recovery_original.id,recovery_entry.comparison->'observed_progress');
  IF recovery_entry.closed THEN
   IF (SELECT array_agg(key ORDER BY key) FROM jsonb_object_keys(recovery_entry.comparison) key)
      IS DISTINCT FROM ARRAY['drain_claim','observed_progress','resource']::text[]
    OR (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'recovery') item
      WHERE item->>'resource'=recovery_original.resource_name)<>1
    OR (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'closures') item
      WHERE item->>'resource'=recovery_original.resource_name)<>1
    OR EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'pending') item
      WHERE item->>'resource'=recovery_original.resource_name) THEN
    RAISE EXCEPTION 'stored closed Recovery requires exact disjoint original closure inventory' USING ERRCODE='P1420'; END IF;
   recovery_origin:=prepared_closure_origin142(recovery_child.instance_id);
   IF NOT EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'closures') item
     WHERE item=recovery_origin AND item->'drain_claim'=recovery_original.claim_json) THEN
    RAISE EXCEPTION 'stored closed Recovery must retain the positively qualified stable origin' USING ERRCODE='P1420'; END IF;
  ELSE
   IF (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'pending') item
      WHERE item->>'resource'=recovery_original.resource_name)<>1
    OR EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'closures') item
      WHERE item->>'resource'=recovery_original.resource_name)
    OR EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'recovery') item
      WHERE item->>'resource'=recovery_original.resource_name) THEN
    RAISE EXCEPTION 'stored Pending Recovery remains exact and disjoint after publication' USING ERRCODE='P1420'; END IF;
  END IF;
  has_recovery:=true;
 END LOOP;
 IF admission.command_id IS NULL OR admission.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
  OR NULLIF(current_setting('hephaestus.request_id',true),'') IS NULL
  OR command.operation IS DISTINCT FROM 'remove' OR command.actor_id IS DISTINCT FROM admission.actor_id
  OR command.first_request_id IS DISTINCT FROM admission.request_id OR command.event_id IS DISTINCT FROM admission.event_id
  OR command.idempotency_id::text IS DISTINCT FROM current_setting('hephaestus.occurrence_id',true)
  OR command.receipt_version IS DISTINCT FROM admission.expected_version OR command.receipt_lifecycle NOT IN ('removing','recovery_required')
  OR (command.receipt_lifecycle='recovery_required' AND NOT has_recovery)
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
 IF EXISTS(SELECT 1 FROM recipe_prepared_instance_imports associated WHERE associated.deployment_id=p_deployment
    AND queued_resume_associations140(associated.instance_id) IS DISTINCT FROM true)
  OR EXISTS(SELECT 1 FROM runs execution JOIN recipe_prepared_instance_imports child ON child.instance_id=execution.instance_id
    WHERE child.deployment_id=p_deployment AND (
      (execution.execution_profile='owned_canonical' AND execution.vm_provider_namespace=admission.vm_provider_namespace
       AND execution.vm_host_id=admission.vm_host_id AND EXISTS(SELECT 1 FROM run_vm_plan_admissions plan
         WHERE plan.run_id=execution.id AND plan.consumed_at IS NOT NULL AND plan.vm_id=execution.vm_id
           AND plan.vm_provider_namespace=execution.vm_provider_namespace AND plan.vm_host_id=execution.vm_host_id))
      OR invocation119_remove_resume_matches(execution.id,admission.vm_provider_namespace,admission.vm_host_id,admission.platform_json) IS NOT DISTINCT FROM true
      OR (execution.execution_profile='legacy_scalar' AND EXISTS(SELECT 1 FROM run_legacy_vm_placements plan
         WHERE plan.run_id=execution.id AND plan.consumed_at IS NOT NULL
           AND plan.provider_namespace::text=admission.vm_provider_namespace AND plan.host_id=admission.vm_host_id
           AND legacy_vm_placement_matches(execution,plan)))) IS DISTINCT FROM true) THEN
  RAISE EXCEPTION 'unknown or foreign current runtime history held' USING ERRCODE='P1180'; END IF;
 PERFORM prepared_resume_inventory142(p_deployment,p_namespace,p_host,p_platform);
 fresh:=remove_resume_context118(p_deployment);
 SELECT count(*) INTO count FROM jsonb_array_elements(fresh->'closures') item WHERE item->>'resource'=p_claim->>'resource';
 IF count<>1 THEN RAISE EXCEPTION 'one exact original closure required' USING ERRCODE='P1180'; END IF;
 SELECT item INTO selected FROM jsonb_array_elements(fresh->'closures') item WHERE item->>'resource'=p_claim->>'resource';
 SELECT count(*) INTO count FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=p_claim->>'resource';
 IF count>1 THEN RAISE EXCEPTION 'one admitted original closure required' USING ERRCODE='P1180'; END IF;
 SELECT item INTO recorded FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=p_claim->>'resource';
 IF recorded IS NOT NULL THEN
  IF selected IS DISTINCT FROM recorded OR EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'pending') item
    WHERE item->>'resource'=p_claim->>'resource') THEN
   RAISE EXCEPTION 'original closure changed after admission' USING ERRCODE='P1180'; END IF;
 ELSE
  IF (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'pending') item WHERE item->>'resource'=p_claim->>'resource')<>1 THEN
   RAISE EXCEPTION 'one explicitly admitted Pending original required' USING ERRCODE='P1420'; END IF;
  SELECT item INTO recorded FROM jsonb_array_elements(admission.context_json->'pending') item WHERE item->>'resource'=p_claim->>'resource';
  IF recorded->'drain_claim' IS DISTINCT FROM selected->'drain_claim'
   OR recorded->'create_claim' IS DISTINCT FROM p_claim OR recorded->'prepared' IS DISTINCT FROM p_prepared
   OR (selected ? 'publication' AND (selected->'publication'->'pending')-ARRAY['observed_progress']
      IS DISTINCT FROM recorded-ARRAY['observed_progress']) THEN
   RAISE EXCEPTION 'actual published winner must retain immutable Pending roots' USING ERRCODE='P1420'; END IF;
 END IF;
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
  'input_hash',(SELECT jsonb_agg(get_byte(decode(substr(selected->'removal'->>'input_hash',3),'hex'),n) ORDER BY n) FROM generate_series(0,31) n))
  ||CASE WHEN selected ? 'publication' THEN jsonb_build_object('publication',selected->'publication') ELSE '{}'::jsonb END;
END $$;

-- The deferred require_remove_resume_admission118 trigger still requires a
-- configured immutable admission at commit; this receipt is not that admission.
CREATE OR REPLACE FUNCTION recipe_command_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
DECLARE qualified_recovery boolean := false; context jsonb;
BEGIN
    -- Do not evaluate this owner-qualified front for the unchanged Install or
    -- normal Removing paths. The trigger remains SECURITY INVOKER; this is the
    -- existing app-executable current-manager context front, not a private call.
    IF NEW.operation = 'remove' AND NEW.receipt_lifecycle = 'recovery_required' THEN
        context := remove_resume_context118(NEW.deployment_id);
        qualified_recovery := EXISTS (SELECT 1 FROM recipe_deployment_commands prior
            WHERE prior.id <> NEW.id AND prior.deployment_id = NEW.deployment_id
              AND prior.project_id = NEW.project_id AND prior.operation = 'remove')
            AND EXISTS (SELECT 1 FROM recipe_deployment_execution_preparations preparation
                WHERE preparation.deployment_id = NEW.deployment_id
                  AND preparation.project_id = NEW.project_id AND preparation.profile = 'runtime_named_v1'
                  AND preparation.input_hash = sha256(preparation.canonical_input))
            AND (EXISTS (SELECT 1 FROM jsonb_array_elements(context->'pending') item
                WHERE item->'observed_progress'->>'removal' = 'recovery_required')
                OR EXISTS (SELECT 1 FROM jsonb_array_elements(context->'recovery') item
                    WHERE item->'observed_progress'->>'removal' = 'recovery_required'));
    END IF;
    IF NEW.actor_id::text <> hephaestus_actor_id()
       OR NEW.first_request_id::text <> current_setting('hephaestus.request_id', true)
       OR NEW.idempotency_id::text <> current_setting('hephaestus.occurrence_id', true)
       OR NOT EXISTS (SELECT 1 FROM application_events event
          WHERE event.id = NEW.event_id AND event.scope_kind = 'project'
            AND event.scope_id = NEW.project_id AND event.aggregate_type = 'project'
            AND event.aggregate_id = NEW.project_id AND event.event_type = 'project.changed'
            AND event.actor_id = NEW.actor_id AND event.request_id = NEW.first_request_id
            AND event.occurrence_id = NEW.idempotency_id AND event.cursor = NEW.event_cursor
            AND event.aggregate_version = NEW.event_aggregate_version)
       OR NOT EXISTS (SELECT 1 FROM recipe_deployments deployment
          WHERE deployment.id = NEW.deployment_id AND deployment.version = NEW.receipt_version
            AND deployment.lifecycle = NEW.receipt_lifecycle)
       OR (NEW.operation = 'install' AND NEW.receipt_lifecycle <> 'installing')
       OR (NEW.operation = 'remove' AND NEW.receipt_lifecycle <> 'removing' AND NOT qualified_recovery) THEN
        RAISE EXCEPTION 'recipe command requires exact committed event provenance' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;

-- The original seven replacement ACLs/owners are inherited exactly.
ALTER FUNCTION public.prepared_closure_row142(public.agent_instance_removal_requests) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepared_closure_row142(public.agent_instance_removal_requests) FROM PUBLIC,hephaestus_app,hephaestus_worker;
ALTER FUNCTION public.pending_instance_drain142(uuid,text) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.pending_instance_drain142(uuid,text) FROM PUBLIC,hephaestus_app,hephaestus_worker;
ALTER FUNCTION public.prepared_closure_origin142(uuid) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepared_closure_origin142(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;
ALTER FUNCTION public.prepared_resume_inventory142(uuid,text,text,jsonb) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepared_resume_inventory142(uuid,text,text,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;
ALTER FUNCTION public.guard_prepared_resume_origin142() OWNER TO postgres;
REVOKE ALL ON FUNCTION public.guard_prepared_resume_origin142() FROM PUBLIC,hephaestus_app,hephaestus_worker;
ALTER FUNCTION public.validate_prepared_resume_origin142() OWNER TO postgres;
REVOKE ALL ON FUNCTION public.validate_prepared_resume_origin142() FROM PUBLIC,hephaestus_app,hephaestus_worker;
ALTER FUNCTION public.prepare_resumed_prepared_instance_closure142(uuid,uuid,jsonb,jsonb,bytea,jsonb,jsonb,text,text,jsonb,bigint,bytea,uuid,bigint,bytea) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepare_resumed_prepared_instance_closure142(uuid,uuid,jsonb,jsonb,bytea,jsonb,jsonb,text,text,jsonb,bigint,bytea,uuid,bigint,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION public.prepare_resumed_prepared_instance_closure142(uuid,uuid,jsonb,jsonb,bytea,jsonb,jsonb,text,text,jsonb,bigint,bytea,uuid,bigint,bytea) TO hephaestus_app;
ALTER FUNCTION public.prepared_drain_progress142(uuid,jsonb) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepared_drain_progress142(uuid,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- Additive worker RRRO empty-secret cleanup facade. Frozen137 consumed semantics
-- and all140 queued qualification/target/receipt helpers remain unchanged.
CREATE FUNCTION verify_invocation142_empty_secret_cleanup(p_run uuid,p_config bytea,p_namespace text,p_host text)
RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER
SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution public.runs%ROWTYPE; revision public.agent_instance_revisions%ROWTYPE;
 owner uuid; original jsonb;
BEGIN
 PERFORM public.require_invocation117_worker();
 IF current_setting('transaction_isolation') IS DISTINCT FROM 'repeatable read'
  OR current_setting('transaction_read_only') IS DISTINCT FROM 'on' THEN
  RAISE EXCEPTION 'Invocation empty-secret cleanup requires owning REPEATABLE READ read-only transaction'
   USING ERRCODE='23000'; END IF;
 IF p_run IS NULL OR p_run='00000000-0000-0000-0000-000000000000'::uuid
  OR p_config IS NULL OR p_namespace IS NULL OR p_host IS NULL THEN
  RAISE EXCEPTION 'qualified Invocation empty-secret cleanup context is unavailable' USING ERRCODE='23000'; END IF;
 -- Any117 history stays on the original consumed verifier, including malformed
 -- or conflicting consumption. Absence alone never permits queued cleanup.
 IF EXISTS(SELECT 1 FROM public.run_invocation_consumptions WHERE run_id=p_run) THEN
  RETURN public.verify_invocation119_empty_secret_cleanup(p_run,p_config,p_namespace,p_host);
 END IF;
 -- Discover original comparison DATA using the same selector as the configured
 -- repository, then require the actual prepared target rather than a candidate.
 IF public.ordinary_queued_cancel_selected140(p_run) THEN
  SELECT context.instance_id,context.original INTO STRICT owner,original
   FROM public.discover_ordinary_queued_cancel_context140(p_run,p_namespace,p_host,p_config) context;
  SELECT prepared.* INTO STRICT execution
   FROM public.load_ordinary_queued_cancel_prepared140(p_run,original,p_namespace,p_host,p_config) prepared;
 ELSE
  SELECT context.instance_id,context.original INTO STRICT owner,original
   FROM public.discover_queued_run_cleanup_context119(p_run,p_namespace,p_host,p_config) context;
  SELECT prepared.* INTO STRICT execution
   FROM public.load_invocation119_cleanup_only(p_run,owner,original,p_namespace,p_host,p_config) prepared;
 END IF;
 SELECT * INTO revision FROM public.agent_instance_revisions WHERE id=execution.instance_revision_id;
 IF execution.id IS DISTINCT FROM p_run OR execution.instance_id IS DISTINCT FROM owner
  OR execution.state NOT IN ('cancelled','cleaning_up','cleaned_up')
  OR revision.id IS NULL OR revision.instance_id IS DISTINCT FROM execution.instance_id
  OR jsonb_typeof(revision.secret_bindings) IS DISTINCT FROM 'array'
  OR revision.secret_bindings IS DISTINCT FROM '[]'::jsonb
  OR public.invocation119_cleanup_secret_history_empty(p_run,execution.instance_id,execution.instance_revision_id)
   IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'qualified Invocation empty-secret cleanup context is unavailable' USING ERRCODE='23000'; END IF;
 -- The strict loaders above retain phase-specific scoped receipt/completion,
 -- current processed Cancel, configured115 pins and full zero-history proofs.
 -- The private137 helper additionally checks the complete crosslinked secret
 -- history and immutable revision; its worker ACL remains unchanged.
 RETURN jsonb_build_object('decision','empty','run_id',p_run);
END $$;
ALTER FUNCTION verify_invocation142_empty_secret_cleanup(uuid,bytea,text,text) OWNER TO postgres;
REVOKE ALL ON FUNCTION verify_invocation142_empty_secret_cleanup(uuid,bytea,text,text)
 FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION verify_invocation142_empty_secret_cleanup(uuid,bytea,text,text) TO hephaestus_worker;

-- Only the actual current prepared closure publisher may stamp a never-consumed
-- Invocation. An old closure/outbox is not authority for a later app update.
-- Worker ordinary cancellation below retains its exact Source140 path.
CREATE OR REPLACE FUNCTION validate_ordinary_queued_cancel_commit140() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE; p_run uuid;
 child recipe_prepared_instance_imports%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 removal agent_instance_removal_requests%ROWTYPE; resource recipe_deployment_resources%ROWTYPE;
 deployment recipe_deployments%ROWTYPE; drain recipe_effect_attempts%ROWTYPE;
 origin jsonb; publisher uuid; original uuid; cancel jsonb;
 inventory_child recipe_prepared_instance_imports%ROWTYPE; inventory_run runs%ROWTYPE;
 inventory_receipt agent_instance_execution_invocations%ROWTYPE; ordinary jsonb; item jsonb;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)='hephaestus_app'
  AND TG_TABLE_NAME='runs' AND TG_OP='UPDATE' THEN
  IF OLD.run_kind='invocation' AND NOT EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=OLD.id) THEN
   IF current_setting('transaction_isolation') IS DISTINCT FROM 'read committed'
    OR current_setting('transaction_read_only') IS DISTINCT FROM 'off'
    OR to_jsonb(NEW)-ARRAY['cancel_requested_at','updated_at'] IS DISTINCT FROM to_jsonb(OLD)-ARRAY['cancel_requested_at','updated_at']
    OR OLD.state NOT IN ('queued','cancelled','cleaning_up')
    OR NEW.cancel_requested_at IS DISTINCT FROM COALESCE(OLD.cancel_requested_at,transaction_timestamp())
    OR NEW.updated_at IS DISTINCT FROM transaction_timestamp() THEN
    RAISE EXCEPTION 'app closure permits only the exact cancellation stamp' USING ERRCODE='42501'; END IF;
   SELECT * INTO execution FROM runs WHERE id=NEW.id;
   SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=NEW.id;
   SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=NEW.instance_id;
   SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id;
   SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name;
   SELECT * INTO deployment FROM recipe_deployments WHERE id=child.deployment_id;
   origin:=prepared_closure_origin142(NEW.instance_id);
   SELECT * INTO removal FROM agent_instance_removal_requests WHERE id=(origin->'removal'->>'id')::uuid;
   SELECT * INTO drain FROM recipe_effect_attempts WHERE id=(origin->'drain_claim'->>'attempt_id')::uuid;
   original:=queued_create_original140(NEW.instance_id);
   publisher:=COALESCE(removal.prepared_resume_command_id,drain.command_id);
   PERFORM prepared_instance_actor118(publisher,original);
   IF removal.id IS NULL OR receipt.id IS NULL OR child.instance_id IS NULL
    OR removal.created_by::text IS DISTINCT FROM hephaestus_actor_id()
    OR removal.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
    OR removal.created_at IS DISTINCT FROM transaction_timestamp()
    OR to_jsonb(execution) IS DISTINCT FROM to_jsonb(NEW)
    OR receipt.instance_id IS DISTINCT FROM child.instance_id OR receipt.deployment_id IS DISTINCT FROM child.deployment_id
    OR receipt.resource_name IS DISTINCT FROM child.resource_name
    OR prepared_instance_created118(child.instance_id,original) IS DISTINCT FROM true
    OR deployment.lifecycle NOT IN ('removing','recovery_required')
    OR deployment.project_id IS DISTINCT FROM child.project_id OR deployment.input_hash IS DISTINCT FROM header.intent_hash
    OR resource.install_progress IS DISTINCT FROM 'ready' OR resource.removal_progress NOT IN ('draining','recovery_required')
    OR resource.active_attempt_id IS DISTINCT FROM drain.id OR resource.execution_generation IS DISTINCT FROM drain.generation
    OR qualified_invocation_graph_matches(receipt) IS DISTINCT FROM true
    OR queued_ordinary_zero140(execution) IS DISTINCT FROM true
    OR receipt.config_hash IS DISTINCT FROM sha256(receipt.config_bytes)
    OR receipt.config_json IS DISTINCT FROM convert_from(receipt.config_bytes,'UTF8')::jsonb
    OR receipt.config_json->'vm' IS DISTINCT FROM jsonb_build_object('namespace',execution.vm_provider_namespace,'host_id',execution.vm_host_id)
    OR receipt.config_json->'platform' IS DISTINCT FROM header.platform_json THEN
    RAISE EXCEPTION 'actual current prepared closure and complete queued original required' USING ERRCODE='42501'; END IF;
   PERFORM prepared_drain_progress142(drain.id,recipe_effect_resource_state(resource));
   -- Keep the complete original inventory. Only this newly published manager
   -- closure may stamp genuine queued work before its owning Cancel is consumed.
   IF NOT run_cleanup_identifier_is_valid(execution.vm_provider_namespace)
    OR NOT run_cleanup_identifier_is_valid(execution.vm_host_id)
    OR NOT EXISTS(SELECT 1 FROM recipe_deployment_execution_preparations
      WHERE deployment_id=child.deployment_id AND profile='runtime_named_v1'
       AND input_hash=sha256(canonical_input) AND platform_json=header.platform_json) THEN
    RAISE EXCEPTION 'exact configured original runtime preparation required' USING ERRCODE='42501'; END IF;
   FOR inventory_child IN SELECT * FROM recipe_prepared_instance_imports
     WHERE deployment_id=child.deployment_id ORDER BY resource_name LOOP
    PERFORM queued_create_original140(inventory_child.instance_id);
    IF queued_resume_associations140(inventory_child.instance_id) IS DISTINCT FROM true THEN
     RAISE EXCEPTION 'whole original association inventory required even for empty instances' USING ERRCODE='42501'; END IF;
   END LOOP;
   FOR inventory_run IN SELECT queued_run.* FROM runs queued_run JOIN recipe_prepared_instance_imports prepared_child
     ON prepared_child.instance_id=queued_run.instance_id WHERE prepared_child.deployment_id=child.deployment_id ORDER BY queued_run.id LOOP
    IF inventory_run.instance_id=NEW.instance_id AND inventory_run.state='queued' AND inventory_run.state_version=0
     AND EXISTS(SELECT 1 FROM control_requests WHERE run_id=inventory_run.id)
     AND NOT EXISTS(SELECT 1 FROM command_inbox inbox JOIN control_requests control ON control.id=inbox.command_id
       WHERE control.run_id=inventory_run.id) THEN
     ordinary:=queued_ordinary_original140(inventory_run.id,NULL);
     SELECT * INTO inventory_receipt FROM agent_instance_execution_invocations WHERE run_id=inventory_run.id;
     IF inventory_receipt.id IS NULL OR inventory_receipt.instance_id IS DISTINCT FROM inventory_run.instance_id
      OR inventory_receipt.deployment_id IS DISTINCT FROM child.deployment_id
      OR inventory_run.execution_profile IS DISTINCT FROM 'owned_canonical' OR inventory_run.run_kind IS DISTINCT FROM 'invocation'
      OR inventory_run.vm_binding_kind IS DISTINCT FROM 'planned' OR inventory_run.vm_plan_admission_required IS DISTINCT FROM false
      OR ROW(inventory_run.vm_provider_namespace,inventory_run.vm_host_id)
        IS DISTINCT FROM ROW(execution.vm_provider_namespace,execution.vm_host_id)
      OR inventory_run.vm_id IS DISTINCT FROM inventory_run.id::text
      OR inventory_run.outcome IS NOT NULL OR num_nonnulls(inventory_run.exit_code,inventory_run.exit_signal,inventory_run.failure)<>0
      OR inventory_run.cancel_requested_at IS DISTINCT FROM transaction_timestamp() OR inventory_run.updated_at IS DISTINCT FROM transaction_timestamp()
      OR inventory_run.volume_acquisition_closed_at IS NOT NULL
      OR inventory_receipt.config_hash IS DISTINCT FROM sha256(inventory_receipt.config_bytes)
      OR inventory_receipt.config_json IS DISTINCT FROM convert_from(inventory_receipt.config_bytes,'UTF8')::jsonb
      OR inventory_receipt.config_json->>'version' IS DISTINCT FROM '1'
      OR inventory_receipt.config_json->>'profile' IS DISTINCT FROM 'runtime_named_v1'
      OR inventory_receipt.config_json->'vm' IS DISTINCT FROM jsonb_build_object('namespace',execution.vm_provider_namespace,'host_id',execution.vm_host_id)
      OR inventory_receipt.config_json->'volume'->>'host_id' IS DISTINCT FROM execution.vm_host_id
      OR inventory_receipt.config_json->'platform' IS DISTINCT FROM header.platform_json
      OR qualified_invocation_graph_matches(inventory_receipt) IS DISTINCT FROM true
      OR queued_drain_zero140(inventory_run) IS DISTINCT FROM true
      OR queued_ordinary_events140(inventory_run,ordinary,false) IS DISTINCT FROM true
      OR ordinary->'drain'->>'removal_id' IS DISTINCT FROM removal.id::text
      OR (SELECT count(*) FROM control_requests WHERE run_id=inventory_run.id)<>jsonb_array_length(ordinary->'controls')
      OR EXISTS(SELECT 1 FROM control_requests WHERE run_id=inventory_run.id
        AND (kind IS DISTINCT FROM 'cancel_run' OR state IS DISTINCT FROM 'completed' OR proposal_id IS NOT NULL
          OR processed_at IS NULL OR diagnostics IS DISTINCT FROM '[]'::jsonb))
      OR NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=inventory_receipt.start_command_id
        AND command_type='start_run' AND payload=qualified_invocation_start_json(inventory_receipt) AND processed_at IS NULL)
      OR (SELECT count(*) FROM outbox WHERE payload->>'command_id'=inventory_receipt.start_command_id::text)<>1
      OR (SELECT count(*) FROM outbox WHERE aggregate_id=inventory_run.id OR payload->>'run_id'=inventory_run.id::text)
        <>1+jsonb_array_length(ordinary->'cancels')+jsonb_array_length(ordinary->'controls')
      OR (SELECT count(*) FROM command_inbox WHERE payload->>'run_id'=inventory_run.id::text)<>1
      OR EXISTS(SELECT 1 FROM run_cleanup_targets WHERE run_id=inventory_run.id)
      OR EXISTS(SELECT 1 FROM run_cleanup_target_leases WHERE run_id=inventory_run.id)
      OR EXISTS(SELECT 1 FROM run_cleanup_receipts WHERE run_id=inventory_run.id)
      OR EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=inventory_run.id) THEN
      RAISE EXCEPTION 'complete genuine queued ordinary original required for current closure stamp' USING ERRCODE='42501'; END IF;
     FOR item IN SELECT value FROM jsonb_array_elements(ordinary->'cancels') LOOP
      IF EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(item->>'command_id')::uuid)
       OR (SELECT count(*) FROM outbox WHERE payload->>'command_id'=item->>'command_id')
        <>(CASE WHEN EXISTS(SELECT 1 FROM jsonb_array_elements(ordinary->'controls') admitted
          WHERE admitted->>'id'=item->>'command_id') THEN 2 ELSE 1 END) THEN
       RAISE EXCEPTION 'exact unprocessed owning Cancel envelopes required for current closure stamp' USING ERRCODE='42501'; END IF;
     END LOOP;
    ELSIF invocation119_remove_resume_matches(inventory_run.id,execution.vm_provider_namespace,execution.vm_host_id,header.platform_json)
      IS DISTINCT FROM true THEN
     RAISE EXCEPTION 'every genuine configured Invocation original required' USING ERRCODE='42501'; END IF;
   END LOOP;
   cancel:=jsonb_build_object('command_id',run_cleanup_wake_cancel118(removal.id,NEW.id),'run_id',NEW.id,'reason','instance removal requested');
   IF (SELECT count(*) FROM outbox WHERE payload->>'command_id'=cancel->>'command_id')<>1
    OR (SELECT count(*) FROM outbox WHERE aggregate_type='release' AND aggregate_id=NEW.id
      AND subject='heph.run.command.cancel.v1' AND event_type='run.cancel_requested.v1'
      AND payload=cancel||jsonb_build_object('schema_version',1,'message_id',id,'idempotency_key',id,'request_id',NULL,'trace_id',NULL))<>1 THEN
    RAISE EXCEPTION 'exact real closure Cancel outbox required' USING ERRCODE='42501'; END IF;
   RETURN NULL;
  END IF;
 END IF;
 IF TG_TABLE_NAME='runs' THEN p_run:=NEW.id;
 ELSE
  IF jsonb_typeof(NEW.payload->'run_id') IS DISTINCT FROM 'string' THEN RETURN NULL; END IF;
  BEGIN p_run:=(NEW.payload->>'run_id')::uuid;
  EXCEPTION WHEN invalid_text_representation THEN RETURN NULL; END;
 END IF;
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.run_kind IS DISTINCT FROM 'invocation'
  OR EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=p_run) THEN RETURN NULL; END IF;
 PERFORM require_invocation117_worker();
 IF NOT ordinary_queued_cancel_selected140(p_run) THEN RETURN NULL; END IF;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 PERFORM ordinary_queued_cancel_qualify140(p_run,ordinary_queued_cancel_original140(p_run,NULL),
  execution.vm_provider_namespace,execution.vm_host_id,receipt.config_bytes,'preparing');
 RETURN NULL;
END $$;
