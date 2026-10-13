-- Public143: protected logical tombstones for exact prepared instance histories.
-- No resource row is deleted and no recorded worker outcome proves physical absence.
-- Four explicit role fronts; all lower predicates remain owner-only.
CREATE TABLE public.recipe_prepared_instance_terminal_operations143 (
 id uuid PRIMARY KEY CONSTRAINT terminal143_id_non_nil CHECK(id<>'00000000-0000-0000-0000-000000000000'::uuid),
 instance_id uuid NOT NULL UNIQUE REFERENCES public.agent_instances(id),
 deployment_id uuid NOT NULL REFERENCES public.recipe_deployments(id),
 project_id uuid NOT NULL REFERENCES public.projects(id), resource_name text NOT NULL,
 revision_id uuid NOT NULL REFERENCES public.agent_instance_revisions(id),
 import_command_key bytea NOT NULL CONSTRAINT terminal143_import_key CHECK(octet_length(import_command_key)=32),
 preparation_hash bytea NOT NULL CONSTRAINT terminal143_preparation_hash CHECK(octet_length(preparation_hash)=32),
 create_attempt_id uuid NOT NULL REFERENCES public.recipe_effect_attempts(id),
 removal_id uuid NOT NULL UNIQUE REFERENCES public.agent_instance_removal_requests(id),
 drain_attempt_id uuid NOT NULL REFERENCES public.recipe_effect_attempts(id),
 detach_transition_id uuid NOT NULL UNIQUE REFERENCES public.recipe_execution_transitions(id),
 detach_verification_id uuid NOT NULL UNIQUE REFERENCES public.recipe_effect_verifications(id),
 delete_attempt_id uuid NOT NULL UNIQUE REFERENCES public.recipe_effect_attempts(id),
 original_json jsonb NOT NULL CONSTRAINT terminal143_original_object CHECK(jsonb_typeof(original_json)='object'),
 delete_progress jsonb NOT NULL CONSTRAINT terminal143_progress_object CHECK(jsonb_typeof(delete_progress)='object'),
 configuration_bytes bytea NOT NULL, configuration_json jsonb NOT NULL,
 configuration_hash bytea NOT NULL CONSTRAINT terminal143_configuration_hash_length CHECK(octet_length(configuration_hash)=32),
 before_instance_version bigint NOT NULL CONSTRAINT terminal143_before_version CHECK(before_instance_version>0 AND before_instance_version<9223372036854775807),
 after_instance_version bigint NOT NULL CONSTRAINT terminal143_after_version CHECK(after_instance_version=before_instance_version+1),
 before_instance jsonb NOT NULL CONSTRAINT terminal143_before_object CHECK(jsonb_typeof(before_instance)='object'),
 after_instance jsonb NOT NULL CONSTRAINT terminal143_after_object CHECK(jsonb_typeof(after_instance)='object'),
 publisher_command_id uuid NOT NULL REFERENCES public.recipe_deployment_commands(id),
 publisher_actor_id uuid NOT NULL REFERENCES public.users(id), publisher_request_id uuid NOT NULL,
 publisher_admission_id uuid REFERENCES public.recipe_remove_resume_admissions(command_id),
 event_id uuid NOT NULL UNIQUE REFERENCES public.application_events(id) DEFERRABLE INITIALLY DEFERRED,
 event_cursor bigint NOT NULL CONSTRAINT terminal143_event_cursor CHECK(event_cursor>0), event_aggregate_version bigint NOT NULL CONSTRAINT terminal143_event_version CHECK(event_aggregate_version>0),
 instance_event_id uuid NOT NULL UNIQUE REFERENCES public.agent_instance_events(id) DEFERRABLE INITIALLY DEFERRED,
 removed_at timestamptz NOT NULL DEFAULT now(),
 FOREIGN KEY(deployment_id,resource_name) REFERENCES public.recipe_deployment_resources(deployment_id,resource_name),
 CONSTRAINT terminal143_configuration_hash CHECK(configuration_hash=sha256(configuration_bytes)),
 CONSTRAINT terminal143_configuration_json CHECK(configuration_json=convert_from(configuration_bytes,'UTF8')::jsonb),
 CONSTRAINT terminal143_request_non_nil CHECK(publisher_request_id<>'00000000-0000-0000-0000-000000000000'::uuid)
);
ALTER TABLE public.recipe_prepared_instance_terminal_operations143 OWNER TO postgres;
ALTER TABLE public.recipe_prepared_instance_terminal_operations143 ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.recipe_prepared_instance_terminal_operations143 FORCE ROW LEVEL SECURITY;
REVOKE ALL ON public.recipe_prepared_instance_terminal_operations143 FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- State-independent immutable birth; never grants live use or current authority.
CREATE FUNCTION prepared_instance_birth143(p_instance uuid,p_config bytea) RETURNS uuid
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; instance agent_instances%ROWTYPE;
 activation agent_instance_execution_activations%ROWTYPE; cfg jsonb; original uuid;
BEGIN
 original:=queued_create_original140(p_instance);
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 SELECT * INTO instance FROM agent_instances WHERE id=p_instance;
 SELECT * INTO activation FROM agent_instance_execution_activations WHERE instance_id=p_instance;
 cfg:=convert_from(p_config,'UTF8')::jsonb;
 IF p_config IS NULL OR jsonb_typeof(cfg) IS DISTINCT FROM 'object'
  OR cfg->>'version' IS DISTINCT FROM '1' OR cfg->>'profile' IS DISTINCT FROM 'runtime_named_v1'
  OR NOT run_cleanup_identifier_is_valid(cfg->'vm'->>'namespace') OR NOT run_cleanup_identifier_is_valid(cfg->'vm'->>'host_id')
  OR activation.id IS NULL OR activation.revision_id IS DISTINCT FROM child.revision_id
  OR activation.deployment_id IS DISTINCT FROM child.deployment_id OR activation.resource_name IS DISTINCT FROM child.resource_name
  OR activation.config_bytes IS DISTINCT FROM p_config OR activation.config_json IS DISTINCT FROM cfg
  OR activation.config_hash IS DISTINCT FROM sha256(p_config)
  OR activation.import_command_key IS DISTINCT FROM child.import_command_key
  OR activation.creation_version IS DISTINCT FROM 1 OR activation.activated_version IS DISTINCT FROM 2
  OR activation.input_hash IS DISTINCT FROM sha256(convert_to(jsonb_build_object('version',1,'operation','activate_instance',
    'actor',activation.actor_id,'activation',activation.id,'instance',p_instance,'revision',child.revision_id,
    'creation_version',1,'activated_version',2,'config_hash',encode(sha256(p_config),'hex'),
    'prepared_hash',encode(child.input_hash,'hex'))::text,'UTF8'))
  OR (SELECT count(*) FROM agent_instance_execution_activation_audits WHERE activation_id=activation.id)<>2
  OR EXISTS(SELECT 1 FROM agent_instance_execution_activation_audits link
    LEFT JOIN authorization_audit_events audit ON audit.id=link.audit_id WHERE link.activation_id=activation.id
     AND (audit.id IS NULL OR audit.actor_id IS DISTINCT FROM activation.actor_id
      OR audit.permission IS DISTINCT FROM link.permission OR audit.object_type IS DISTINCT FROM link.object_type
      OR audit.object_id IS DISTINCT FROM link.object_id OR audit.decision IS DISTINCT FROM 'allow'
      OR audit.request_id IS DISTINCT FROM activation.request_id
      OR audit.authorization_model_version IS DISTINCT FROM activation.authorization_model_version
      OR NOT((link.permission='can_manage' AND link.object_type='agent_instance' AND link.object_id=p_instance)
        OR (link.permission='can_use' AND link.object_type='release_agent' AND link.object_id=child.release_agent_id))))
  OR NOT EXISTS(SELECT 1 FROM outbox event WHERE event.id=activation.outbox_id AND event.aggregate_type='release'
    AND event.aggregate_id=p_instance AND event.subject='hephaestus.agent_instance.activated.v1'
    AND event.event_type='agent_instance.activated.v1' AND event.payload=jsonb_build_object('activation_id',activation.id,
      'instance_id',p_instance,'revision_id',child.revision_id,'activated_version',2,'gate_open',true,'invocation_supported',false,
      'schema_version',1,'message_id',activation.outbox_id,'idempotency_key',activation.outbox_id,'request_id',NULL,'trace_id',NULL))
  OR instance.active_revision_id IS DISTINCT FROM child.revision_id OR instance.state_volume_id IS NOT NULL
  OR instance.name IS DISTINCT FROM (convert_from(child.canonical_import,'UTF8')::jsonb)->>'name'
  OR instance.volume_mode IS DISTINCT FROM 'named'
  OR cfg->'platform' IS DISTINCT FROM (SELECT platform_json FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id)
  OR NOT EXISTS(SELECT 1 FROM release_command_inbox command WHERE command.command_key=activation.command_key
    AND command.operation='activate_instance' AND command.aggregate_id=p_instance AND command.secondary_id=activation.id
    AND command.actor_id=activation.actor_id AND command.request_id=activation.request_id AND command.input_hash=activation.input_hash)
  OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox product ON product.event_id=event.id
    WHERE event.id=activation.event_id AND event.scope_kind='agent_instance' AND event.scope_id=p_instance
      AND event.aggregate_type='agent_instance' AND event.aggregate_id=p_instance AND event.event_type='agent_instance.changed'
      AND event.actor_type='user' AND event.actor_id=activation.actor_id AND event.request_id=activation.request_id
      AND event.cursor=activation.event_cursor AND event.aggregate_version=activation.event_aggregate_version)
  OR NOT EXISTS(SELECT 1 FROM agent_instance_events event WHERE event.id=activation.instance_event_id
    AND event.instance_id=p_instance AND event.revision_id=child.revision_id AND event.update_id IS NULL
    AND event.event_type='instance.activated' AND event.actor_id=activation.actor_id AND event.request_id=activation.request_id
    AND event.payload=jsonb_build_object('activation_id',activation.id,'revision_id',child.revision_id,
      'activated_version',2,'gate_open',true,'invocation_supported',false))
  OR EXISTS(SELECT 1 FROM agent_instance_execution_invocations invocation WHERE invocation.instance_id=p_instance
    AND (invocation.config_bytes IS DISTINCT FROM p_config OR invocation.config_json IS DISTINCT FROM cfg
      OR invocation.config_hash IS DISTINCT FROM sha256(p_config))) THEN
  RAISE EXCEPTION 'exact immutable prepared birth, activation and complete configuration required' USING ERRCODE='P1430'; END IF;
 RETURN original;
END $$;

CREATE FUNCTION prepared_instance_cleanup143(p_instance uuid,p_namespace text,p_host text) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT queued_resume_associations140(p_instance) IS TRUE
   AND NOT EXISTS(SELECT 1 FROM runs execution WHERE instance_id=p_instance
     AND invocation119_remove_resume_matches(execution.id,p_namespace,p_host,
       (SELECT config_json->'platform' FROM agent_instance_execution_activations WHERE instance_id=p_instance)) IS DISTINCT FROM true)
   AND prepared_instance_removal118(p_instance) IS NOT NULL
   AND NOT EXISTS(SELECT 1 FROM mailboxes WHERE instance_id=p_instance AND state='active')
   AND NOT EXISTS(SELECT 1 FROM mailbox_deliveries WHERE instance_id=p_instance AND terminal_at IS NULL)
   AND NOT EXISTS(SELECT 1 FROM mailbox_delivery_attempts attempt WHERE instance_id=p_instance
     AND (state IN ('leased','running','uncertain') OR run_id IS NULL
       OR NOT EXISTS(SELECT 1 FROM runs execution WHERE execution.id=attempt.run_id
          AND execution.instance_id=p_instance AND ready_retention_run_terminal(execution,p_namespace,p_host) IS TRUE)))
   AND NOT EXISTS(SELECT 1 FROM run_requests request WHERE instance_id=p_instance
     AND (dispatch_state='pending' OR (dispatch_state='dispatched' AND NOT EXISTS(
       SELECT 1 FROM runs execution WHERE execution.id=request.run_id AND execution.instance_id=p_instance
         AND ready_retention_run_terminal(execution,p_namespace,p_host) IS TRUE))))
   AND NOT EXISTS(SELECT 1 FROM deferred_agent_triggers WHERE instance_id=p_instance AND state='deferred')
   AND NOT EXISTS(SELECT 1 FROM agent_updates WHERE instance_id=p_instance AND state NOT IN ('activated','rejected'))
   AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_grants grant_row WHERE instance_id=p_instance
     AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_revocations revoked WHERE revoked.grant_id=grant_row.id))
   AND NOT EXISTS(SELECT 1 FROM runs execution WHERE instance_id=p_instance
     AND ready_retention_run_terminal(execution,p_namespace,p_host) IS NOT TRUE)
   AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease
     WHERE (lease.instance_id=p_instance OR lease.run_id IN (SELECT id FROM runs WHERE instance_id=p_instance))
       AND (lease.state<>'released' OR lease.released_at IS NULL OR lease.run_id IS NULL
         OR NOT EXISTS(SELECT 1 FROM runs execution WHERE execution.id=lease.run_id
           AND execution.instance_id=p_instance AND lease.instance_id=execution.instance_id
           AND ready_retention_run_terminal(execution,p_namespace,p_host) IS TRUE
           AND ((execution.execution_profile='legacy_scalar' AND execution.lease_id=lease.id
              AND execution.volume_id=lease.volume_id AND execution.lease_fencing_token=lease.fencing_token
              AND lease.host_id=p_host)
            OR (execution.execution_profile='owned_canonical' AND EXISTS(
              SELECT 1 FROM run_cleanup_target_leases item JOIN run_cleanup_completions done
                ON done.run_id=item.run_id AND done.generation=item.generation
              WHERE item.run_id=execution.id AND item.lease_id=lease.id AND item.volume_id=lease.volume_id
                AND item.host_id=lease.host_id AND item.fencing_token=lease.fencing_token))))));
$$;

CREATE FUNCTION prepared_instance_progress143(p_attempt uuid,p_endpoint jsonb) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE drain recipe_effect_attempts%ROWTYPE; resource recipe_deployment_resources%ROWTYPE;
 header recipe_deployment_execution_preparations%ROWTYPE; claimed recipe_execution_transitions%ROWTYPE;
 step recipe_execution_transitions%ROWTYPE; current_command recipe_deployment_commands%ROWTYPE;
 proof recipe_effect_verifications%ROWTYPE; state jsonb; endpoint_version bigint;
BEGIN
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=p_attempt;
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=drain.deployment_id AND resource_name=drain.resource_name;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=drain.deployment_id;
 IF drain.id IS NULL OR drain.action NOT IN ('detach','delete') OR drain.generation<=0
  OR resource.resource_kind IS DISTINCT FROM 'instance' OR resource.ownership IS DISTINCT FROM 'owned'
  OR resource.input_hash IS DISTINCT FROM drain.input_hash OR resource.plan_json->'identity' IS DISTINCT FROM drain.claim_json->'identity'
  OR header.deployment_id IS NULL OR header.profile IS DISTINCT FROM 'runtime_named_v1'
  OR drain.claim_json IS DISTINCT FROM jsonb_build_object('command_id',drain.command_id,'deployment_id',drain.deployment_id,
    'resource',drain.resource_name,'identity',resource.plan_json->'identity','input_hash',recipe_effect_hash_json(drain.input_hash),
    'resource_version',(drain.before_state->>'version')::bigint+1,'generation',drain.generation,
    'action',drain.action,'attempt_id',drain.id,'actor_id',drain.actor_id,'request_id',drain.request_id)
  OR jsonb_typeof(p_endpoint) IS DISTINCT FROM 'object'
  OR (SELECT array_agg(key ORDER BY key) FROM jsonb_object_keys(p_endpoint) key)
     IS DISTINCT FROM ARRAY['active_attempt','diagnostic','generation','install','last_action','removal','version']::text[]
  OR p_endpoint->>'install' IS DISTINCT FROM 'ready' OR p_endpoint->>'active_attempt' IS DISTINCT FROM drain.id::text
  OR p_endpoint->>'generation' IS DISTINCT FROM drain.generation::text
  OR p_endpoint->>'removal' NOT IN (CASE drain.action WHEN 'detach' THEN 'draining' ELSE 'detached' END,'recovery_required')
  OR (p_endpoint->>'version')::bigint<(drain.claim_json->>'resource_version')::bigint THEN
  RAISE EXCEPTION 'exact positive original Detach/Delete endpoint required' USING ERRCODE='P1430'; END IF;
 endpoint_version:=(p_endpoint->>'version')::bigint;
 IF (SELECT count(*) FROM recipe_execution_transitions WHERE attempt_id=drain.id AND kind='claimed')<>1 THEN
  RAISE EXCEPTION 'unique original claimed transition required' USING ERRCODE='P1430'; END IF;
 SELECT * INTO claimed FROM recipe_execution_transitions WHERE attempt_id=drain.id AND kind='claimed';
 IF ROW(claimed.command_id,claimed.deployment_id,claimed.project_id,claimed.resource_name,claimed.operation_id,
    claimed.actor_id,claimed.request_id,claimed.fingerprint,claimed.before_state)
   IS DISTINCT FROM ROW(drain.command_id,drain.deployment_id,drain.project_id,drain.resource_name,drain.id,
    drain.actor_id,drain.request_id,drain.request_fingerprint,drain.before_state)
  OR claimed.verification_id IS NOT NULL
  OR claimed.after_deployment_version IS DISTINCT FROM claimed.before_deployment_version+1
  OR claimed.after_state IS DISTINCT FROM recipe_effect_expected_state(drain.before_state,drain.before_state,drain.action,'claimed','unproven',NULL,drain.id)
  OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox product ON product.event_id=event.id
    WHERE event.id=claimed.event_id AND event.occurrence_id=claimed.id AND event.scope_kind='project'
     AND event.scope_id=drain.project_id AND event.aggregate_type='project' AND event.aggregate_id=drain.project_id
     AND event.event_type='project.changed' AND event.change_kind='updated' AND event.actor_type='user'
     AND event.actor_id=claimed.actor_id AND event.request_id=claimed.request_id
     AND event.cursor=claimed.event_cursor AND event.aggregate_version=claimed.event_aggregate_version) THEN
  RAISE EXCEPTION 'exact protected claimed event and fingerprint required' USING ERRCODE='P1430'; END IF;
 IF drain.before_state->>'install' IS DISTINCT FROM 'ready' OR drain.before_state->>'active_attempt' IS NOT NULL
  OR drain.before_state->>'diagnostic' IS NOT NULL
  OR drain.before_state->>'last_action' IS DISTINCT FROM (CASE drain.action WHEN 'detach' THEN 'drain' ELSE 'detach' END)
  OR drain.before_state->>'removal' IS DISTINCT FROM (CASE drain.action WHEN 'detach' THEN 'draining' ELSE 'detached' END) THEN
  RAISE EXCEPTION 'exact verified predecessor before terminal claim required' USING ERRCODE='P1430'; END IF;
 state:=claimed.after_state;
 IF p_endpoint->'last_action' IS DISTINCT FROM state->'last_action'
  OR EXISTS(SELECT 1 FROM recipe_execution_transitions t WHERE t.deployment_id=drain.deployment_id AND t.resource_name=drain.resource_name
    AND (t.before_state->>'version' IS NULL OR t.after_state->>'version' IS NULL)) THEN
  RAISE EXCEPTION 'known whole state and unchanged prior verified action required' USING ERRCODE='P1430'; END IF;
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
      OR proof.outcome IS DISTINCT FROM 'failed' OR proof.action IS DISTINCT FROM drain.action
      OR ROW(proof.attempt_id,proof.verification_attempt_id,proof.deployment_id,proof.project_id,proof.resource_name,proof.generation,proof.input_hash)
       IS DISTINCT FROM ROW(drain.id,step.operation_id,drain.deployment_id,drain.project_id,drain.resource_name,drain.generation,drain.input_hash)
      OR proof.identity_json IS DISTINCT FROM drain.claim_json->'identity' OR proof.diagnostic IS DISTINCT FROM step.after_state->>'diagnostic'))
   OR step.after_state IS DISTINCT FROM recipe_effect_expected_state(state,drain.before_state,drain.action,step.kind,
      CASE WHEN step.kind='completed' THEN 'failed' ELSE 'unproven' END,step.after_state->>'diagnostic',drain.id)
   OR step.after_state->>'active_attempt' IS DISTINCT FROM drain.id::text
   OR step.after_state->>'generation' IS DISTINCT FROM drain.generation::text
   OR (step.after_state->>'version')::bigint IS DISTINCT FROM (state->>'version')::bigint+1
   OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox product ON product.event_id=event.id
    WHERE event.id=step.event_id AND event.occurrence_id=step.id AND event.scope_kind='project' AND event.scope_id=drain.project_id
     AND event.aggregate_type='project' AND event.aggregate_id=drain.project_id AND event.event_type='project.changed'
     AND event.change_kind='updated' AND event.actor_type='user' AND event.actor_id=step.actor_id AND event.request_id=step.request_id
     AND event.cursor=step.event_cursor AND event.aggregate_version=step.event_aggregate_version) THEN
   RAISE EXCEPTION 'complete contiguous protected same-original active progress required' USING ERRCODE='P1430'; END IF;
  state:=step.after_state;
 END LOOP;
 IF state IS DISTINCT FROM p_endpoint THEN
  RAISE EXCEPTION 'exact immutable positive transition endpoint required' USING ERRCODE='P1430'; END IF;
 RETURN;
END $$;

-- Full actual Applied completion, including a genuinely different manager's
-- protected reconciliation. Saved actors are compared, never authenticated.
CREATE FUNCTION prepared_instance_phase143(p_instance uuid,p_action text,p_config bytea) RETURNS uuid
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; cfg jsonb; candidate uuid[];
 step recipe_execution_transitions%ROWTYPE; effect recipe_effect_attempts%ROWTYPE;
 proof recipe_effect_verifications%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 closure jsonb; predecessor recipe_execution_transitions%ROWTYPE; previous uuid;
BEGIN
 IF p_action NOT IN ('drain','detach','delete') OR p_action IS NULL THEN
  RAISE EXCEPTION 'known historical terminal phase required' USING ERRCODE='P1430'; END IF;
 PERFORM prepared_instance_birth143(p_instance,p_config);
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 cfg:=convert_from(p_config,'UTF8')::jsonb; closure:=prepared_closure_origin142(p_instance);
 SELECT array_agg(t.id ORDER BY t.id) INTO candidate FROM recipe_execution_transitions t
  JOIN recipe_effect_attempts attempt ON attempt.id=t.attempt_id
  JOIN recipe_effect_verifications verified ON verified.id=t.verification_id
  WHERE attempt.deployment_id=child.deployment_id AND attempt.resource_name=child.resource_name AND attempt.action=p_action
    AND t.kind IN ('completed','reconciled') AND verified.outcome='applied';
 IF COALESCE(cardinality(candidate),0)<>1 THEN
  RAISE EXCEPTION 'one actual Applied original terminal phase required' USING ERRCODE='P1430'; END IF;
 SELECT * INTO step FROM recipe_execution_transitions WHERE id=candidate[1];
 SELECT * INTO effect FROM recipe_effect_attempts WHERE id=step.attempt_id;
 SELECT * INTO proof FROM recipe_effect_verifications WHERE id=step.verification_id;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=step.command_id;
 IF p_action='drain' THEN
  IF effect.claim_json IS DISTINCT FROM closure->'drain_claim' THEN
   RAISE EXCEPTION 'exact original published Drain required' USING ERRCODE='P1430'; END IF;
  PERFORM prepared_drain_progress142(effect.id,step.before_state);
 ELSE
  PERFORM prepared_instance_progress143(effect.id,step.before_state);
 END IF;
 IF ROW(effect.project_id,step.deployment_id,step.project_id,step.resource_name)
     IS DISTINCT FROM ROW(child.project_id,child.deployment_id,child.project_id,child.resource_name)
  OR command.operation IS DISTINCT FROM 'remove' OR command.deployment_id IS DISTINCT FROM child.deployment_id
  OR command.project_id IS DISTINCT FROM child.project_id OR command.actor_id IS DISTINCT FROM step.actor_id
  OR step.request_id IS NULL OR step.request_id='00000000-0000-0000-0000-000000000000'::uuid
  OR octet_length(step.fingerprint) IS DISTINCT FROM 32
  OR step.before_lifecycle NOT IN ('removing','recovery_required') OR step.after_lifecycle NOT IN ('removing','recovery_required')
  OR step.after_deployment_version IS DISTINCT FROM step.before_deployment_version+1
  OR step.before_deployment_version<=effect.expected_deployment_version
  OR step.after_state IS DISTINCT FROM recipe_effect_expected_state(step.before_state,effect.before_state,p_action,step.kind,'applied',NULL,effect.id)
  OR step.after_state->>'active_attempt' IS NOT NULL OR step.after_state->>'last_action' IS DISTINCT FROM p_action
  OR step.after_state->>'diagnostic' IS NOT NULL
  OR proof.recorded_by IS DISTINCT FROM 'hephaestus_worker' OR proof.outcome IS DISTINCT FROM 'applied'
  OR proof.diagnostic IS NOT NULL OR proof.action IS DISTINCT FROM p_action
  OR ROW(proof.attempt_id,proof.verification_attempt_id,proof.deployment_id,proof.project_id,proof.resource_name,proof.generation,proof.input_hash)
    IS DISTINCT FROM ROW(effect.id,step.operation_id,effect.deployment_id,effect.project_id,effect.resource_name,effect.generation,effect.input_hash)
  OR proof.identity_json IS DISTINCT FROM effect.claim_json->'identity'
  OR (step.kind='completed' AND (step.command_id IS DISTINCT FROM effect.command_id OR step.operation_id IS DISTINCT FROM effect.id
    OR step.request_id IS DISTINCT FROM effect.request_id OR (step.before_state->>'version')::bigint IS DISTINCT FROM (effect.claim_json->>'resource_version')::bigint))
  OR (step.kind='reconciled' AND (step.operation_id=effect.id OR EXISTS(SELECT 1 FROM recipe_effect_attempts WHERE id=step.operation_id)
    OR (SELECT count(*) FROM recipe_execution_transitions WHERE operation_id=step.operation_id)<>1
    OR (step.command_id<>effect.command_id AND NOT EXISTS(SELECT 1 FROM recipe_remove_resume_admissions admission
      WHERE admission.command_id=step.command_id AND admission.deployment_id=effect.deployment_id
        AND admission.vm_provider_namespace=cfg->'vm'->>'namespace' AND admission.vm_host_id=cfg->'vm'->>'host_id'
        AND admission.platform_json=cfg->'platform'
        AND EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'claims') item WHERE item->'claim'=effect.claim_json)
        AND EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item=closure)))))
  OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox product ON product.event_id=event.id
    WHERE event.id=step.event_id AND event.occurrence_id=step.id AND event.scope_kind='project' AND event.scope_id=child.project_id
      AND event.aggregate_type='project' AND event.aggregate_id=child.project_id AND event.event_type='project.changed'
      AND event.change_kind='updated' AND event.schema_version=1 AND event.actor_type='user'
      AND event.actor_id=step.actor_id AND event.request_id=step.request_id
      AND event.cursor=step.event_cursor AND event.aggregate_version=step.event_aggregate_version)
  OR prepared_instance_cleanup143(p_instance,cfg->'vm'->>'namespace',cfg->'vm'->>'host_id') IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'full immutable Applied phase, proof, event and cleanup ledger required' USING ERRCODE='P1430'; END IF;
 RETURN step.id;
END $$;

CREATE FUNCTION prepared_instance_detached143(p_instance uuid,p_config bytea) RETURNS uuid
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE drain recipe_execution_transitions%ROWTYPE; detach recipe_execution_transitions%ROWTYPE; attempt recipe_effect_attempts%ROWTYPE;
BEGIN
 SELECT * INTO drain FROM recipe_execution_transitions WHERE id=prepared_instance_phase143(p_instance,'drain',p_config);
 SELECT * INTO detach FROM recipe_execution_transitions WHERE id=prepared_instance_phase143(p_instance,'detach',p_config);
 SELECT * INTO attempt FROM recipe_effect_attempts WHERE id=detach.attempt_id;
 IF attempt.before_state IS DISTINCT FROM drain.after_state
  OR attempt.generation IS DISTINCT FROM (drain.after_state->>'generation')::bigint+1 THEN
  RAISE EXCEPTION 'Detach must follow actual Applied Drain without intervening state' USING ERRCODE='P1430'; END IF;
 RETURN detach.id;
END $$;

-- Closed wire, immutable comparison only. No current actor or live State is
-- inferred here: each accepting front independently validates those boundaries.
CREATE FUNCTION prepared_instance_original143(p_request jsonb,p_config bytea) RETURNS uuid
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 resource recipe_deployment_resources%ROWTYPE; create_effect recipe_effect_attempts%ROWTYPE;
 effect recipe_effect_attempts%ROWTYPE; predecessor recipe_execution_transitions%ROWTYPE;
 command recipe_deployment_commands%ROWTYPE; cfg jsonb; consumer uuid;
BEGIN
 IF jsonb_typeof(p_request) IS DISTINCT FROM 'object'
  OR (SELECT array_agg(key ORDER BY key) FROM jsonb_object_keys(p_request) key) IS DISTINCT FROM
    ARRAY['before_instance_version','closure','configuration','create_claim','current_command','delete_or_detach_claim',
      'deployment','intent_hash','prepared','project','resource','version']::text[]
  OR p_request->>'version' IS DISTINCT FROM '1' THEN
  RAISE EXCEPTION 'exact version1 terminal comparison wire required' USING ERRCODE='P1430'; END IF;
 SELECT * INTO effect FROM recipe_effect_attempts WHERE id=(p_request->'delete_or_detach_claim'->>'attempt_id')::uuid;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE deployment_id=(p_request->>'deployment')::uuid
  AND resource_name=p_request->>'resource';
 consumer:=child.instance_id;
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id;
 SELECT * INTO create_effect FROM recipe_effect_attempts WHERE id=prepared_instance_birth143(consumer,p_config);
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=(p_request->'current_command'->>'id')::uuid;
 cfg:=convert_from(p_config,'UTF8')::jsonb;
 IF effect.id IS NULL OR effect.action NOT IN ('detach','delete') OR child.instance_id IS NULL
  OR effect.claim_json IS DISTINCT FROM p_request->'delete_or_detach_claim'
  OR effect.deployment_id IS DISTINCT FROM child.deployment_id OR effect.resource_name IS DISTINCT FROM child.resource_name
  OR effect.project_id IS DISTINCT FROM child.project_id OR p_request->>'project' IS DISTINCT FROM child.project_id::text
  OR resource.resource_kind IS DISTINCT FROM 'instance' OR resource.ownership IS DISTINCT FROM 'owned'
  OR resource.removal_policy IS DISTINCT FROM 'delete' OR resource.resource_id IS DISTINCT FROM consumer
  OR effect.input_hash IS DISTINCT FROM resource.input_hash OR effect.claim_json->'identity' IS DISTINCT FROM resource.plan_json->'identity'
  OR create_effect.claim_json IS DISTINCT FROM p_request->'create_claim'
  OR convert_from(child.canonical_import,'UTF8')::jsonb IS DISTINCT FROM p_request->'prepared'
  OR recipe_effect_hash_json(header.intent_hash) IS DISTINCT FROM p_request->'intent_hash'
  OR cfg IS DISTINCT FROM p_request->'configuration'
  OR prepared_closure_origin142(consumer) IS DISTINCT FROM p_request->'closure'
  OR command.operation IS DISTINCT FROM 'remove' OR command.deployment_id IS DISTINCT FROM child.deployment_id
  OR command.project_id IS DISTINCT FROM child.project_id
  OR p_request->'current_command' IS DISTINCT FROM jsonb_build_object('id',command.id,'actor_id',command.actor_id,
    'idempotency_id',command.idempotency_id,'operation',command.operation)
  OR effect.id=create_effect.id OR effect.id=(p_request->'closure'->'drain_claim'->>'attempt_id')::uuid
  OR NOT EXISTS(SELECT 1 FROM recipe_deployment_commands original WHERE original.id=effect.command_id
    AND original.operation='remove' AND original.deployment_id=effect.deployment_id
    AND original.project_id=effect.project_id AND original.actor_id=effect.actor_id)
  OR (effect.action='delete' AND (jsonb_typeof(p_request->'before_instance_version') IS DISTINCT FROM 'number'
    OR (p_request->>'before_instance_version')::bigint<=0 OR (p_request->>'before_instance_version')::bigint>=9223372036854775807))
  OR (effect.action='detach' AND p_request->'before_instance_version' IS DISTINCT FROM 'null'::jsonb) THEN
  RAISE EXCEPTION 'whole immutable terminal original, policy, command and configuration required' USING ERRCODE='P1430'; END IF;
 IF effect.action='detach' THEN
  SELECT * INTO predecessor FROM recipe_execution_transitions WHERE id=prepared_instance_phase143(consumer,'drain',p_config);
 ELSE
  SELECT * INTO predecessor FROM recipe_execution_transitions WHERE id=prepared_instance_detached143(consumer,p_config);
 END IF;
 IF effect.before_state IS DISTINCT FROM predecessor.after_state
  OR effect.generation IS DISTINCT FROM (predecessor.after_state->>'generation')::bigint+1 THEN
  RAISE EXCEPTION 'actual immediate verified terminal predecessor required' USING ERRCODE='P1430'; END IF;
 -- Validate the positive initial claimed endpoint even when later progress exists.
 PERFORM prepared_instance_progress143(effect.id,(SELECT after_state FROM recipe_execution_transitions
   WHERE attempt_id=effect.id AND kind='claimed'));
 RETURN consumer;
END $$;

CREATE FUNCTION prepared_instance_terminal143(p_operation uuid) RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE saved recipe_prepared_instance_terminal_operations143%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
 instance agent_instances%ROWTYPE; effect recipe_effect_attempts%ROWTYPE; detach recipe_execution_transitions%ROWTYPE;
 publisher recipe_deployment_commands%ROWTYPE; admission recipe_remove_resume_admissions%ROWTYPE;
 cfg jsonb; consumer uuid; current_resource recipe_deployment_resources%ROWTYPE; final_step recipe_execution_transitions%ROWTYPE;
BEGIN
 SELECT * INTO saved FROM recipe_prepared_instance_terminal_operations143 WHERE id=p_operation;
 IF saved.id IS NULL THEN RETURN false; END IF;
 consumer:=prepared_instance_original143(saved.original_json,saved.configuration_bytes);
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=consumer;
 SELECT * INTO instance FROM agent_instances WHERE id=consumer;
 SELECT * INTO effect FROM recipe_effect_attempts WHERE id=saved.delete_attempt_id;
 SELECT * INTO publisher FROM recipe_deployment_commands WHERE id=saved.publisher_command_id;
 SELECT * INTO admission FROM recipe_remove_resume_admissions WHERE command_id=saved.publisher_admission_id;
 SELECT * INTO detach FROM recipe_execution_transitions WHERE id=prepared_instance_detached143(consumer,saved.configuration_bytes);
 cfg:=convert_from(saved.configuration_bytes,'UTF8')::jsonb;
 PERFORM prepared_instance_progress143(effect.id,saved.delete_progress);
 SELECT * INTO current_resource FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name;
 IF current_resource.active_attempt_id=effect.id THEN
  PERFORM prepared_instance_progress143(effect.id,recipe_effect_resource_state(current_resource));
 ELSE
  SELECT * INTO final_step FROM recipe_execution_transitions WHERE id=prepared_instance_phase143(consumer,'delete',saved.configuration_bytes);
  IF current_resource.active_attempt_id IS NOT NULL OR recipe_effect_resource_state(current_resource) IS DISTINCT FROM final_step.after_state THEN
   RAISE EXCEPTION 'terminal history must retain exact original active or Applied Delete state' USING ERRCODE='P1430'; END IF;
 END IF;
 IF ROW(saved.instance_id,saved.deployment_id,saved.project_id,saved.resource_name,saved.revision_id,saved.import_command_key,
    saved.create_attempt_id,saved.removal_id,saved.drain_attempt_id,saved.detach_transition_id,saved.detach_verification_id)
   IS DISTINCT FROM ROW(consumer,child.deployment_id,child.project_id,child.resource_name,child.revision_id,child.import_command_key,
    (saved.original_json->'create_claim'->>'attempt_id')::uuid,(saved.original_json->'closure'->'removal'->>'id')::uuid,
    (saved.original_json->'closure'->'drain_claim'->>'attempt_id')::uuid,detach.id,detach.verification_id)
  OR effect.action IS DISTINCT FROM 'delete' OR effect.claim_json IS DISTINCT FROM saved.original_json->'delete_or_detach_claim'
  OR saved.preparation_hash IS DISTINCT FROM (SELECT input_hash FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id)
  OR saved.configuration_json IS DISTINCT FROM cfg OR saved.configuration_hash IS DISTINCT FROM sha256(saved.configuration_bytes)
  OR saved.before_instance_version IS DISTINCT FROM (saved.original_json->>'before_instance_version')::bigint
  OR saved.after_instance_version IS DISTINCT FROM saved.before_instance_version+1
  OR saved.before_instance->>'id' IS DISTINCT FROM consumer::text
  OR saved.before_instance->>'state' IS DISTINCT FROM 'active' OR saved.before_instance->>'run_gate_open' IS DISTINCT FROM 'false'
  OR saved.before_instance->>'version' IS DISTINCT FROM saved.before_instance_version::text
  OR saved.before_instance->'removed_at' IS DISTINCT FROM 'null'::jsonb
  OR saved.after_instance IS DISTINCT FROM saved.before_instance||jsonb_build_object('state','removed','run_gate_open',false,
    'version',saved.after_instance_version,'removed_at',saved.removed_at,'updated_at',saved.removed_at)
  OR to_jsonb(instance) IS DISTINCT FROM saved.after_instance
  OR publisher.operation IS DISTINCT FROM 'remove' OR publisher.actor_id IS DISTINCT FROM saved.publisher_actor_id
  OR publisher.deployment_id IS DISTINCT FROM child.deployment_id OR publisher.project_id IS DISTINCT FROM child.project_id
  OR saved.original_json->'current_command' IS DISTINCT FROM jsonb_build_object('id',publisher.id,'actor_id',publisher.actor_id,
    'idempotency_id',publisher.idempotency_id,'operation',publisher.operation)
  OR (publisher.id=effect.command_id AND saved.publisher_admission_id IS NOT NULL)
  OR (publisher.id<>effect.command_id AND (saved.publisher_admission_id IS DISTINCT FROM publisher.id
    OR admission.deployment_id IS DISTINCT FROM child.deployment_id OR admission.project_id IS DISTINCT FROM child.project_id
    OR admission.actor_id IS DISTINCT FROM publisher.actor_id OR admission.request_id IS DISTINCT FROM publisher.first_request_id
    OR admission.vm_provider_namespace IS DISTINCT FROM cfg->'vm'->>'namespace'
    OR admission.vm_host_id IS DISTINCT FROM cfg->'vm'->>'host_id' OR admission.platform_json IS DISTINCT FROM cfg->'platform'
    OR NOT EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'claims') item WHERE item->'claim'=effect.claim_json)
    OR NOT EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item=saved.original_json->'closure')))
  OR prepared_instance_cleanup143(consumer,cfg->'vm'->>'namespace',cfg->'vm'->>'host_id') IS DISTINCT FROM true
  OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox product ON product.event_id=event.id
    WHERE event.id=saved.event_id AND event.occurrence_id=saved.id AND event.scope_kind='agent_instance' AND event.scope_id=consumer
      AND event.aggregate_type='agent_instance' AND event.aggregate_id=consumer AND event.event_type='agent_instance.changed'
      AND event.change_kind='state_changed' AND event.safe_state='removed' AND event.related_id_one=child.project_id
      AND event.schema_version=1 AND event.actor_type='user' AND event.actor_id=saved.publisher_actor_id
      AND event.request_id=saved.publisher_request_id AND event.cursor=saved.event_cursor AND event.aggregate_version=saved.event_aggregate_version)
  OR NOT EXISTS(SELECT 1 FROM agent_instance_events event WHERE event.id=saved.instance_event_id AND event.instance_id=consumer
    AND event.revision_id=child.revision_id AND event.update_id IS NULL AND event.event_type='instance.removed'
    AND event.actor_id=saved.publisher_actor_id AND event.request_id=saved.publisher_request_id
    AND event.payload=jsonb_build_object('terminal_operation_id',saved.id,'delete_attempt_id',effect.id,
      'before_version',saved.before_instance_version,'after_version',saved.after_instance_version,'removal_id',saved.removal_id)) THEN
  RETURN false;
 END IF;
 RETURN true;
END $$;

-- Actor accepts only its REAL middleware command/request, never a saved actor.
CREATE FUNCTION prepared_instance_current143(p_request jsonb,p_config bytea) RETURNS uuid
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE consumer uuid; effect recipe_effect_attempts%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 cfg jsonb; admission recipe_remove_resume_admissions%ROWTYPE;
BEGIN
 consumer:=prepared_instance_original143(p_request,p_config);
 SELECT * INTO effect FROM recipe_effect_attempts WHERE id=(p_request->'delete_or_detach_claim'->>'attempt_id')::uuid;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=(p_request->'current_command'->>'id')::uuid;
 PERFORM prepared_instance_actor118(command.id,effect.id);
 PERFORM remove_resume_actor118(effect.deployment_id);
 cfg:=convert_from(p_config,'UTF8')::jsonb;
 PERFORM prepared_resume_inventory142(effect.deployment_id,cfg->'vm'->>'namespace',cfg->'vm'->>'host_id',cfg->'platform');
 IF command.id<>effect.command_id THEN
  SELECT * INTO admission FROM recipe_remove_resume_admissions WHERE command_id=command.id;
  IF (qualified_remove_resume_claim118(command.id,effect.claim_json,cfg->'vm'->>'namespace',cfg->'vm'->>'host_id',cfg->'platform') IS DISTINCT FROM true
     AND NOT(effect.action='delete' AND EXISTS(SELECT 1 FROM recipe_prepared_instance_terminal_operations143 terminal
       JOIN recipe_deployment_resources progress ON progress.deployment_id=terminal.deployment_id AND progress.resource_name=terminal.resource_name
       WHERE terminal.instance_id=consumer AND terminal.delete_attempt_id=effect.id AND progress.active_attempt_id IS NULL
        AND prepared_instance_terminal143(terminal.id) IS TRUE)
       AND admission.actor_id=command.actor_id AND admission.request_id=command.first_request_id
       AND admission.vm_provider_namespace=cfg->'vm'->>'namespace' AND admission.vm_host_id=cfg->'vm'->>'host_id'
       AND admission.platform_json=cfg->'platform'
       AND EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'terminal') historical WHERE historical->'delete_claim'=effect.claim_json)))
    OR NOT EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item=p_request->'closure') THEN
   RAISE EXCEPTION 'exact fresh current Delete or Detach admission and original closure required' USING ERRCODE='P1430'; END IF;
 END IF;
 RETURN consumer;
END $$;

CREATE FUNCTION observe_prepared_instance_detach143(p_request jsonb,p_config bytea) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE consumer uuid; effect recipe_effect_attempts%ROWTYPE; resource recipe_deployment_resources%ROWTYPE; cfg jsonb;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_worker'
   OR current_setting('transaction_isolation')<>'repeatable read' OR current_setting('transaction_read_only')<>'on' THEN
  RAISE EXCEPTION 'independent worker repeatable-read readonly transaction required' USING ERRCODE='42501'; END IF;
 consumer:=prepared_instance_original143(p_request,p_config);
 SELECT * INTO effect FROM recipe_effect_attempts WHERE id=(p_request->'delete_or_detach_claim'->>'attempt_id')::uuid;
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=effect.deployment_id AND resource_name=effect.resource_name;
 cfg:=convert_from(p_config,'UTF8')::jsonb;
 IF effect.action IS DISTINCT FROM 'detach' OR resource.active_attempt_id IS DISTINCT FROM effect.id
  OR resource.execution_generation IS DISTINCT FROM effect.generation
  OR NOT EXISTS(SELECT 1 FROM agent_instances WHERE id=consumer AND state='active' AND NOT run_gate_open AND removed_at IS NULL)
  OR prepared_instance_cleanup143(consumer,cfg->'vm'->>'namespace',cfg->'vm'->>'host_id') IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'exact active Detach and complete closed canonical cleanup required' USING ERRCODE='P1430'; END IF;
 PERFORM prepared_instance_progress143(effect.id,recipe_effect_resource_state(resource));
 RETURN jsonb_build_object('version',1,'request',p_request,'birth_proven',true,'cleanup_ready',true,
   'removal_id',p_request->'closure'->'removal'->'id','inventory',prepared_instance_inventory118(consumer));
END $$;

CREATE FUNCTION delete_prepared_instance143(p_request jsonb,p_config bytea)
RETURNS TABLE(terminal_operation_id uuid,after_instance_version bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE consumer uuid; effect recipe_effect_attempts%ROWTYPE; resource recipe_deployment_resources%ROWTYPE;
 child recipe_prepared_instance_imports%ROWTYPE; instance agent_instances%ROWTYPE;
 detach recipe_execution_transitions%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 saved recipe_prepared_instance_terminal_operations143%ROWTYPE; cfg jsonb; event record;
 operation uuid; domain_event uuid; terminal_at timestamptz; before_row jsonb; after_row jsonb; request uuid;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app'
  OR current_setting('transaction_isolation')<>'read committed' OR current_setting('transaction_read_only')<>'off' THEN
  RAISE EXCEPTION 'owning current actor writable read-committed transaction required' USING ERRCODE='42501'; END IF;
 -- Complete sorted original reservations precede ALL parent and late locks.
 PERFORM lock_remove_resume118((p_request->>'deployment')::uuid);
 consumer:=prepared_instance_current143(p_request,p_config);
 SELECT * INTO effect FROM recipe_effect_attempts WHERE id=(p_request->'delete_or_detach_claim'->>'attempt_id')::uuid;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=(p_request->'current_command'->>'id')::uuid;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=consumer;
 SELECT * INTO saved FROM recipe_prepared_instance_terminal_operations143 WHERE instance_id=consumer OR delete_attempt_id=effect.id;
 -- Winner replay precedes live absence/CAS checks; the first publisher and rows
 -- are immutable. Current authorization was independently renewed above.
 IF saved.id IS NOT NULL THEN
  IF saved.delete_attempt_id IS DISTINCT FROM effect.id OR saved.configuration_bytes IS DISTINCT FROM p_config
    OR saved.original_json-ARRAY['current_command'] IS DISTINCT FROM p_request-ARRAY['current_command']
    OR prepared_instance_terminal143(saved.id) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'exact immutable tombstone replay required' USING ERRCODE='P1430'; END IF;
  RETURN QUERY SELECT saved.id,saved.after_instance_version; RETURN;
 END IF;
 SELECT * INTO instance FROM agent_instances WHERE id=consumer;
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=effect.deployment_id AND resource_name=effect.resource_name;
 SELECT * INTO detach FROM recipe_execution_transitions WHERE id=prepared_instance_detached143(consumer,p_config);
 cfg:=convert_from(p_config,'UTF8')::jsonb;
 request:=NULLIF(current_setting('hephaestus.request_id',true),'')::uuid;
 IF effect.action IS DISTINCT FROM 'delete' OR resource.active_attempt_id IS DISTINCT FROM effect.id
  OR resource.execution_generation IS DISTINCT FROM effect.generation
  OR instance.state IS DISTINCT FROM 'active' OR instance.run_gate_open IS DISTINCT FROM false OR instance.removed_at IS NOT NULL
  OR instance.version IS DISTINCT FROM (p_request->>'before_instance_version')::bigint
  OR instance.version<=0 OR instance.version>=9223372036854775807
  OR prepared_instance_cleanup143(consumer,cfg->'vm'->>'namespace',cfg->'vm'->>'host_id') IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'actual active Delete, historical Detach, full cleanup and explicit instance CAS required' USING ERRCODE='P1430'; END IF;
 PERFORM prepared_instance_progress143(effect.id,recipe_effect_resource_state(resource));
 -- Repeat ALL current rights after complete locks and immediately before writes.
 PERFORM prepared_instance_current143(p_request,p_config);
 operation:=gen_random_uuid(); domain_event:=gen_random_uuid(); terminal_at:=now(); before_row:=to_jsonb(instance);
 after_row:=before_row||jsonb_build_object('state','removed','run_gate_open',false,'version',instance.version+1,
   'removed_at',terminal_at,'updated_at',terminal_at);
 SELECT * INTO event FROM append_application_event(operation,'agent_instance',consumer,'agent_instance',consumer,
  'agent_instance.changed','state_changed','removed',child.project_id,NULL);
 INSERT INTO recipe_prepared_instance_terminal_operations143(id,instance_id,deployment_id,project_id,resource_name,
  revision_id,import_command_key,preparation_hash,create_attempt_id,removal_id,drain_attempt_id,
  detach_transition_id,detach_verification_id,delete_attempt_id,original_json,delete_progress,
  configuration_bytes,configuration_json,configuration_hash,before_instance_version,after_instance_version,
  before_instance,after_instance,publisher_command_id,publisher_actor_id,publisher_request_id,publisher_admission_id,
  event_id,event_cursor,event_aggregate_version,instance_event_id,removed_at)
 VALUES(operation,consumer,child.deployment_id,child.project_id,child.resource_name,child.revision_id,child.import_command_key,
  (SELECT input_hash FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id),
  (p_request->'create_claim'->>'attempt_id')::uuid,(p_request->'closure'->'removal'->>'id')::uuid,
  (p_request->'closure'->'drain_claim'->>'attempt_id')::uuid,detach.id,detach.verification_id,effect.id,
  p_request,recipe_effect_resource_state(resource),p_config,cfg,sha256(p_config),instance.version,instance.version+1,
  before_row,after_row,command.id,command.actor_id,request,CASE WHEN command.id<>effect.command_id THEN command.id END,
  event.event_id,event.cursor,event.aggregate_version,domain_event,terminal_at);
 UPDATE agent_instances target SET state='removed',run_gate_open=false,version=instance.version+1,
  removed_at=terminal_at,updated_at=terminal_at WHERE target.id=consumer AND target.version=instance.version;
 IF NOT FOUND THEN RAISE EXCEPTION 'exact instance tombstone CAS lost' USING ERRCODE='40001'; END IF;
 INSERT INTO agent_instance_events(id,instance_id,revision_id,event_type,actor_id,request_id,payload)
 VALUES(domain_event,consumer,child.revision_id,'instance.removed',command.actor_id,request,
  jsonb_build_object('terminal_operation_id',operation,'delete_attempt_id',effect.id,'before_version',instance.version,
   'after_version',instance.version+1,'removal_id',(p_request->'closure'->'removal'->>'id')::uuid));
 IF prepared_instance_terminal143(operation) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'tombstone full record validation failed' USING ERRCODE='P1430'; END IF;
 RETURN QUERY SELECT operation,instance.version+1;
END $$;

CREATE FUNCTION observe_prepared_instance_deleted143(p_request jsonb,p_config bytea)
RETURNS TABLE(terminal_operation_id uuid,after_instance_version bigint,terminal_history jsonb)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE consumer uuid; saved recipe_prepared_instance_terminal_operations143%ROWTYPE; history jsonb;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_worker'
  OR current_setting('transaction_isolation')<>'repeatable read' OR current_setting('transaction_read_only')<>'on' THEN
  RAISE EXCEPTION 'independent worker repeatable-read readonly transaction required' USING ERRCODE='42501'; END IF;
 consumer:=prepared_instance_original143(p_request,p_config);
 SELECT * INTO saved FROM recipe_prepared_instance_terminal_operations143 WHERE instance_id=consumer;
 IF saved.id IS NULL OR saved.configuration_bytes IS DISTINCT FROM p_config
  OR saved.original_json-ARRAY['current_command'] IS DISTINCT FROM p_request-ARRAY['current_command']
  OR prepared_instance_terminal143(saved.id) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'exact positively qualified committed tombstone required' USING ERRCODE='P1430'; END IF;
 history:=prepared_instance_delete_context143(consumer);
 IF history IS NULL OR jsonb_typeof(history->'observed_progress') IS DISTINCT FROM 'object'
  OR history-ARRAY['observed_progress'] IS DISTINCT FROM jsonb_build_object(
   'resource',saved.resource_name,'delete_claim',saved.original_json->'delete_or_detach_claim',
   'before_instance_version',saved.before_instance_version,'terminal_operation_id',saved.id,
   'after_instance_version',saved.after_instance_version,
   'publisher',jsonb_build_object('command_id',saved.publisher_command_id,'actor_id',saved.publisher_actor_id,
    'request_id',saved.publisher_request_id,'admission_command_id',saved.publisher_admission_id)) THEN
  RAISE EXCEPTION 'exact positively qualified committed terminal history required' USING ERRCODE='P1430'; END IF;
 RETURN QUERY SELECT saved.id,saved.after_instance_version,history;
END $$;

-- One exact instance Delete history, not a work claim or current authority.
CREATE FUNCTION prepared_instance_delete_context143(p_instance uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; resource recipe_deployment_resources%ROWTYPE;
 effect recipe_effect_attempts%ROWTYPE; saved recipe_prepared_instance_terminal_operations143%ROWTYPE;
 activation agent_instance_execution_activations%ROWTYPE; instance agent_instances%ROWTYPE;
 candidates uuid[]; request jsonb; current_command recipe_deployment_commands%ROWTYPE; result jsonb;
BEGIN
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name;
 SELECT array_agg(id ORDER BY id) INTO candidates FROM recipe_effect_attempts WHERE deployment_id=child.deployment_id
   AND resource_name=child.resource_name AND action='delete';
 IF COALESCE(cardinality(candidates),0)=0 THEN RETURN NULL; END IF;
 IF cardinality(candidates)<>1 OR resource.resource_kind IS DISTINCT FROM 'instance'
  OR resource.ownership IS DISTINCT FROM 'owned' OR resource.removal_policy IS DISTINCT FROM 'delete' THEN
  RAISE EXCEPTION 'one original Owned Delete history required' USING ERRCODE='P1430'; END IF;
 SELECT * INTO effect FROM recipe_effect_attempts WHERE id=candidates[1];
 SELECT * INTO saved FROM recipe_prepared_instance_terminal_operations143 WHERE instance_id=p_instance OR delete_attempt_id=effect.id;
 SELECT * INTO instance FROM agent_instances WHERE id=p_instance;
 SELECT * INTO activation FROM agent_instance_execution_activations WHERE instance_id=p_instance;
 SELECT * INTO current_command FROM recipe_deployment_commands WHERE id=effect.command_id;
 IF saved.id IS NOT NULL THEN
  IF saved.instance_id IS DISTINCT FROM p_instance OR saved.delete_attempt_id IS DISTINCT FROM effect.id
   OR prepared_instance_terminal143(saved.id) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'exact committed original Delete graph required' USING ERRCODE='P1430'; END IF;
  request:=saved.original_json;
 ELSE
  IF resource.active_attempt_id IS DISTINCT FROM effect.id OR resource.execution_generation IS DISTINCT FROM effect.generation
   OR instance.state IS DISTINCT FROM 'active' OR instance.removed_at IS NOT NULL OR instance.run_gate_open IS DISTINCT FROM false
   OR instance.version<=0 OR instance.version>=9223372036854775807 THEN
   RAISE EXCEPTION 'unwritten Delete requires positive original active state and live bounded CAS' USING ERRCODE='P1430'; END IF;
  request:=jsonb_build_object('version',1,'project',child.project_id,'deployment',child.deployment_id,
    'intent_hash',(SELECT recipe_effect_hash_json(intent_hash) FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id),
    'resource',child.resource_name,'create_claim',(SELECT claim_json FROM recipe_effect_attempts WHERE id=queued_create_original140(p_instance)),
    'prepared',convert_from(child.canonical_import,'UTF8')::jsonb,'closure',prepared_closure_origin142(p_instance),
    'delete_or_detach_claim',effect.claim_json,'current_command',jsonb_build_object('id',current_command.id,'actor_id',current_command.actor_id,
     'idempotency_id',current_command.idempotency_id,'operation',current_command.operation),
    'configuration',activation.config_json,'before_instance_version',instance.version);
  PERFORM prepared_instance_original143(request,activation.config_bytes);
 END IF;
 IF resource.active_attempt_id=effect.id THEN
  PERFORM prepared_instance_progress143(effect.id,recipe_effect_resource_state(resource));
 END IF;
 result:=jsonb_build_object('resource',child.resource_name,'delete_claim',effect.claim_json,
   'observed_progress',recipe_effect_resource_state(resource),'before_instance_version',request->'before_instance_version',
   'terminal_operation_id',saved.id,'after_instance_version',saved.after_instance_version,
   'publisher',CASE WHEN saved.id IS NOT NULL THEN jsonb_build_object('command_id',saved.publisher_command_id,
     'actor_id',saved.publisher_actor_id,'request_id',saved.publisher_request_id,'admission_command_id',saved.publisher_admission_id) ELSE NULL END);
 RETURN result;
END $$;

-- Stored active endpoints remain positively qualified after a later Applied
-- completion. Current admission separately observes actual live state/context.
CREATE FUNCTION prepared_instance_terminal_context143(p_deployment uuid,p_context jsonb,p_namespace text,p_host text,p_platform jsonb) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE item jsonb; child recipe_prepared_instance_imports%ROWTYPE; effect recipe_effect_attempts%ROWTYPE;
 activation agent_instance_execution_activations%ROWTYPE; saved recipe_prepared_instance_terminal_operations143%ROWTYPE;
 current_data jsonb; recovery boolean:=false;
BEGIN
 IF p_context ? 'detach' THEN
  IF jsonb_typeof(p_context->'detach') IS DISTINCT FROM 'array' OR jsonb_array_length(p_context->'detach')=0
   OR jsonb_array_length(p_context->'detach')>(SELECT count(*) FROM recipe_prepared_instance_imports WHERE deployment_id=p_deployment)
   OR EXISTS(SELECT 1 FROM jsonb_array_elements(p_context->'detach') entry GROUP BY entry->>'resource' HAVING count(*)<>1) THEN
   RAISE EXCEPTION 'bounded unique explicit Detach Recovery inventory required' USING ERRCODE='P1430'; END IF;
  FOR item IN SELECT value FROM jsonb_array_elements(p_context->'detach') LOOP
   SELECT * INTO child FROM recipe_prepared_instance_imports WHERE deployment_id=p_deployment AND resource_name=item->>'resource';
   SELECT * INTO effect FROM recipe_effect_attempts WHERE id=(item->'detach_claim'->>'attempt_id')::uuid;
   SELECT * INTO activation FROM agent_instance_execution_activations WHERE instance_id=child.instance_id;
   IF jsonb_typeof(item) IS DISTINCT FROM 'object'
    OR (SELECT array_agg(key ORDER BY key) FROM jsonb_object_keys(item) key) IS DISTINCT FROM ARRAY['detach_claim','observed_progress','resource']::text[]
    OR child.instance_id IS NULL OR effect.action IS DISTINCT FROM 'detach' OR effect.claim_json IS DISTINCT FROM item->'detach_claim'
    OR effect.deployment_id IS DISTINCT FROM p_deployment OR effect.resource_name IS DISTINCT FROM child.resource_name
    OR item->'observed_progress'->>'removal' IS DISTINCT FROM 'recovery_required'
    OR NOT EXISTS(SELECT 1 FROM recipe_deployment_resources resource WHERE resource.deployment_id=p_deployment
      AND resource.resource_name=child.resource_name AND resource.resource_kind='instance' AND resource.ownership='owned' AND resource.removal_policy='delete')
    OR activation.config_json->'vm' IS DISTINCT FROM jsonb_build_object('namespace',p_namespace,'host_id',p_host)
    OR activation.config_json->'platform' IS DISTINCT FROM p_platform
    OR effect.before_state IS DISTINCT FROM (SELECT after_state FROM recipe_execution_transitions
       WHERE id=prepared_instance_phase143(child.instance_id,'drain',activation.config_bytes))
    OR (SELECT count(*) FROM jsonb_array_elements(p_context->'claims') claimed WHERE claimed->'claim'=effect.claim_json)<>1
    OR (SELECT count(*) FROM jsonb_array_elements(p_context->'closures') original WHERE original=prepared_closure_origin142(child.instance_id))<>1
    OR EXISTS(SELECT 1 FROM jsonb_array_elements(p_context->'pending') other WHERE other->>'resource'=child.resource_name)
    OR EXISTS(SELECT 1 FROM jsonb_array_elements(p_context->'recovery') other WHERE other->>'resource'=child.resource_name)
    OR EXISTS(SELECT 1 FROM jsonb_array_elements(p_context->'terminal') other WHERE other->>'resource'=child.resource_name) THEN
    RAISE EXCEPTION 'exact disjoint original configured Detach Recovery history required' USING ERRCODE='P1430'; END IF;
   PERFORM prepared_instance_progress143(effect.id,item->'observed_progress');
   recovery:=true;
  END LOOP;
 END IF;
 IF NOT(p_context ? 'terminal') THEN RETURN recovery; END IF;
 IF jsonb_typeof(p_context->'terminal') IS DISTINCT FROM 'array' OR jsonb_array_length(p_context->'terminal')=0
  OR jsonb_array_length(p_context->'terminal')>(SELECT count(*) FROM recipe_prepared_instance_imports WHERE deployment_id=p_deployment)
  OR EXISTS(SELECT 1 FROM jsonb_array_elements(p_context->'terminal') entry GROUP BY entry->>'resource' HAVING count(*)<>1) THEN
  RAISE EXCEPTION 'bounded unique explicit terminal comparison inventory required' USING ERRCODE='P1430'; END IF;
 FOR item IN SELECT value FROM jsonb_array_elements(p_context->'terminal') LOOP
  IF jsonb_typeof(item) IS DISTINCT FROM 'object'
   OR (SELECT array_agg(key ORDER BY key) FROM jsonb_object_keys(item) key) IS DISTINCT FROM
    ARRAY['after_instance_version','before_instance_version','delete_claim','observed_progress','publisher','resource','terminal_operation_id']::text[] THEN
   RAISE EXCEPTION 'exact declared terminal history keys required' USING ERRCODE='P1430'; END IF;
  SELECT * INTO child FROM recipe_prepared_instance_imports WHERE deployment_id=p_deployment AND resource_name=item->>'resource';
  SELECT * INTO effect FROM recipe_effect_attempts WHERE id=(item->'delete_claim'->>'attempt_id')::uuid;
  SELECT * INTO activation FROM agent_instance_execution_activations WHERE instance_id=child.instance_id;
  IF child.instance_id IS NULL OR effect.action IS DISTINCT FROM 'delete' OR effect.claim_json IS DISTINCT FROM item->'delete_claim'
   OR effect.deployment_id IS DISTINCT FROM child.deployment_id OR effect.resource_name IS DISTINCT FROM child.resource_name
   OR activation.config_json->'vm' IS DISTINCT FROM jsonb_build_object('namespace',p_namespace,'host_id',p_host)
   OR activation.config_json->'platform' IS DISTINCT FROM p_platform
   OR (SELECT count(*) FROM jsonb_array_elements(p_context->'closures') original WHERE original=prepared_closure_origin142(child.instance_id))<>1
   OR EXISTS(SELECT 1 FROM jsonb_array_elements(p_context->'pending') pending WHERE pending->>'resource'=child.resource_name)
   OR EXISTS(SELECT 1 FROM jsonb_array_elements(p_context->'recovery') drain WHERE drain->>'resource'=child.resource_name)
   OR jsonb_typeof(item->'before_instance_version') IS DISTINCT FROM 'number'
   OR (item->>'before_instance_version')::bigint<=0 OR (item->>'before_instance_version')::bigint>=9223372036854775807 THEN
   RAISE EXCEPTION 'exact disjoint original Delete, configured closure and CAS required' USING ERRCODE='P1430'; END IF;
  IF effect.before_state IS DISTINCT FROM (SELECT after_state FROM recipe_execution_transitions
    WHERE id=prepared_instance_detached143(child.instance_id,activation.config_bytes)) THEN
   RAISE EXCEPTION 'stored Delete requires its actual immediate completed Detach predecessor' USING ERRCODE='P1430'; END IF;
  IF item->'observed_progress'->>'active_attempt' IS NOT NULL THEN
   PERFORM prepared_instance_progress143(effect.id,item->'observed_progress');
   IF (SELECT count(*) FROM jsonb_array_elements(p_context->'claims') claimed WHERE claimed->'claim'=effect.claim_json)<>1 THEN
    RAISE EXCEPTION 'active terminal history must retain the exact active claim' USING ERRCODE='P1430'; END IF;
   recovery:=recovery OR item->'observed_progress'->>'removal'='recovery_required';
  ELSE
   IF EXISTS(SELECT 1 FROM jsonb_array_elements(p_context->'claims') claimed WHERE claimed->'claim'->>'resource'=child.resource_name)
    OR item->'observed_progress' IS DISTINCT FROM (SELECT after_state FROM recipe_execution_transitions
      WHERE id=prepared_instance_phase143(child.instance_id,'delete',activation.config_bytes)) THEN
    RAISE EXCEPTION 'inactive terminal history needs actual unique Applied Delete and no active claim' USING ERRCODE='P1430'; END IF;
  END IF;
  SELECT * INTO saved FROM recipe_prepared_instance_terminal_operations143 WHERE instance_id=child.instance_id;
  IF item->'terminal_operation_id' IS DISTINCT FROM 'null'::jsonb THEN
   IF saved.id IS NULL OR item->>'terminal_operation_id' IS DISTINCT FROM saved.id::text
    OR item->>'before_instance_version' IS DISTINCT FROM saved.before_instance_version::text
    OR item->>'after_instance_version' IS DISTINCT FROM saved.after_instance_version::text
    OR item->'publisher' IS DISTINCT FROM jsonb_build_object('command_id',saved.publisher_command_id,
      'actor_id',saved.publisher_actor_id,'request_id',saved.publisher_request_id,'admission_command_id',saved.publisher_admission_id)
    OR prepared_instance_terminal143(saved.id) IS DISTINCT FROM true THEN
    RAISE EXCEPTION 'exact saved terminal CAS and publisher graph required' USING ERRCODE='P1430'; END IF;
  ELSE
   -- A historically uncommitted admission remains metadata after publication.
   -- It never proves current absence or authorizes replacing a committed row.
   IF item->'after_instance_version' IS DISTINCT FROM 'null'::jsonb OR item->'publisher' IS DISTINCT FROM 'null'::jsonb
    OR item->'observed_progress'->>'active_attempt' IS DISTINCT FROM effect.id::text
    OR (saved.id IS NOT NULL AND item->>'before_instance_version' IS DISTINCT FROM saved.before_instance_version::text) THEN
    RAISE EXCEPTION 'unwritten comparison must retain exact bounded original before CAS' USING ERRCODE='P1430'; END IF;
  END IF;
 END LOOP;
 RETURN recovery;
END $$;

CREATE FUNCTION load_prepared_instance_delete_continuation143(p_current uuid,p_original jsonb,p_config bytea) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE command recipe_deployment_commands%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
 actual jsonb; request jsonb; consumer uuid; admission recipe_remove_resume_admissions%ROWTYPE;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app'
  OR current_setting('transaction_isolation')<>'repeatable read' OR current_setting('transaction_read_only')<>'on' THEN
  RAISE EXCEPTION 'current actor repeatable-read readonly continuation required' USING ERRCODE='42501'; END IF;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=p_current;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE deployment_id=(p_original->>'deployment')::uuid
   AND resource_name=p_original->>'resource';
 PERFORM remove_resume_actor118(command.deployment_id);
 actual:=prepared_instance_delete_context143(child.instance_id);
 IF actual IS NULL OR actual->'delete_claim' IS DISTINCT FROM p_original->'delete_or_detach_claim'
  OR (p_original->'before_instance_version' IS DISTINCT FROM 'null'::jsonb
    AND p_original->'before_instance_version' IS DISTINCT FROM actual->'before_instance_version') THEN
  RAISE EXCEPTION 'exact original Delete and positively observed before CAS required' USING ERRCODE='P1430'; END IF;
 request:=p_original||jsonb_build_object('current_command',jsonb_build_object('id',command.id,'actor_id',command.actor_id,
   'idempotency_id',command.idempotency_id,'operation',command.operation),'before_instance_version',actual->'before_instance_version');
 consumer:=prepared_instance_current143(request,p_config);
 RETURN jsonb_build_object('version',1,'request',request,'history',actual);
END $$;

CREATE OR REPLACE FUNCTION remove_resume_context118(p_deployment uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE claims jsonb:='[]'; closures jsonb:='[]'; resource recipe_deployment_resources%ROWTYPE; attempt recipe_effect_attempts%ROWTYPE;
 command recipe_deployment_commands%ROWTYPE; child record; removal agent_instance_removal_requests%ROWTYPE;
 inbox release_command_inbox%ROWTYPE; drain recipe_effect_attempts%ROWTYPE; count integer;
 pending jsonb:='[]'; recovery jsonb:='[]'; origin jsonb; terminal jsonb:='[]'; history jsonb; result jsonb; detach jsonb:='[]'; activation agent_instance_execution_activations%ROWTYPE; predecessor recipe_execution_transitions%ROWTYPE;
BEGIN
 PERFORM remove_resume_actor118(p_deployment);
 FOR resource IN SELECT * FROM recipe_deployment_resources WHERE deployment_id=p_deployment AND active_attempt_id IS NOT NULL ORDER BY resource_name LOOP
  SELECT * INTO attempt FROM recipe_effect_attempts WHERE id=resource.active_attempt_id;
  SELECT * INTO command FROM recipe_deployment_commands WHERE id=attempt.command_id;
  IF resource.ownership IS DISTINCT FROM 'owned' OR (attempt.action NOT IN ('drain','detach','retain') AND NOT(attempt.action='delete' AND resource.resource_kind='instance' AND resource.removal_policy='delete')) OR command.operation IS DISTINCT FROM 'remove'
   OR attempt.deployment_id IS DISTINCT FROM resource.deployment_id OR attempt.resource_name IS DISTINCT FROM resource.resource_name
   OR attempt.project_id IS DISTINCT FROM resource.project_id OR attempt.generation IS DISTINCT FROM resource.execution_generation
   OR attempt.input_hash IS DISTINCT FROM resource.input_hash OR attempt.claim_json->'identity' IS DISTINCT FROM resource.plan_json->'identity'
   OR NOT EXISTS(SELECT 1 FROM recipe_execution_transitions t WHERE t.attempt_id=attempt.id AND t.kind='claimed'
     AND t.command_id=attempt.command_id AND t.project_id=attempt.project_id AND t.resource_name=attempt.resource_name
     AND t.operation_id=attempt.id AND t.fingerprint=attempt.request_fingerprint AND t.before_state=attempt.before_state
     AND t.after_state->>'active_attempt'=attempt.id::text AND t.after_state->>'generation'=attempt.generation::text) THEN
   RAISE EXCEPTION 'exact original supported active removal required' USING ERRCODE='P1180'; END IF;
  IF resource.resource_kind='instance' AND attempt.action='delete' THEN
   history:=prepared_instance_delete_context143(resource.resource_id);
   IF history IS NULL OR history->'delete_claim' IS DISTINCT FROM attempt.claim_json
     OR history->'observed_progress' IS DISTINCT FROM recipe_effect_resource_state(resource) THEN
    RAISE EXCEPTION 'exact active original Delete history required' USING ERRCODE='P1430'; END IF;
  END IF;
  IF resource.resource_kind='instance' AND resource.removal_policy='delete' AND attempt.action='detach'
    AND resource.removal_progress='recovery_required' THEN
   SELECT * INTO activation FROM agent_instance_execution_activations WHERE instance_id=resource.resource_id;
   SELECT * INTO predecessor FROM recipe_execution_transitions WHERE id=prepared_instance_phase143(resource.resource_id,'drain',activation.config_bytes);
   IF attempt.before_state IS DISTINCT FROM predecessor.after_state
     OR attempt.generation IS DISTINCT FROM (predecessor.after_state->>'generation')::bigint+1
     OR NOT EXISTS(SELECT 1 FROM recipe_deployments WHERE id=p_deployment AND lifecycle='recovery_required') THEN
    RAISE EXCEPTION 'actual completed Drain before original Detach Recovery required' USING ERRCODE='P1430'; END IF;
   PERFORM prepared_instance_progress143(attempt.id,recipe_effect_resource_state(resource));
   detach:=detach||jsonb_build_array(jsonb_build_object('resource',resource.resource_name,'detach_claim',attempt.claim_json,
     'observed_progress',recipe_effect_resource_state(resource)));
  END IF;
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
 FOR child IN SELECT * FROM recipe_prepared_instance_imports WHERE deployment_id=p_deployment ORDER BY resource_name LOOP
  history:=prepared_instance_delete_context143(child.instance_id);
  IF history IS NOT NULL THEN terminal:=terminal||jsonb_build_array(history); END IF;
 END LOOP;
 IF jsonb_array_length(terminal)>0 OR jsonb_array_length(detach)>0 THEN
  result:=jsonb_build_object('claims',claims,'closures',closures);
  IF jsonb_array_length(terminal)>0 THEN result:=result||jsonb_build_object('terminal',terminal); END IF;
  IF jsonb_array_length(detach)>0 THEN result:=result||jsonb_build_object('detach',detach); END IF;
  IF jsonb_array_length(pending)>0 THEN result:=result||jsonb_build_object('pending',pending); END IF;
  IF jsonb_array_length(recovery)>0 THEN result:=result||jsonb_build_object('recovery',recovery); END IF;
  RETURN result;
 END IF;
 IF jsonb_array_length(recovery)>0 THEN
  RETURN jsonb_build_object('claims',claims,'closures',closures,'recovery',recovery)
   ||CASE WHEN jsonb_array_length(pending)>0 THEN jsonb_build_object('pending',pending) ELSE '{}'::jsonb END;
 END IF;
 IF jsonb_array_length(pending)>0 THEN
  RETURN jsonb_build_object('claims',claims,'closures',closures,'pending',pending);
 END IF;
 RETURN jsonb_build_object('claims',claims,'closures',closures);
END $$;

CREATE OR REPLACE FUNCTION qualified_remove_resume_claim118(p_command uuid,p_claim jsonb,p_namespace text,p_host text,p_platform jsonb) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE admission recipe_remove_resume_admissions%ROWTYPE; command recipe_deployment_commands%ROWTYPE; effect recipe_effect_attempts%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE; resource recipe_deployment_resources%ROWTYPE; cfg bytea; history jsonb;
BEGIN
 SELECT * INTO admission FROM recipe_remove_resume_admissions WHERE command_id=p_command;
 IF admission.command_id IS NULL THEN RETURN false; END IF;
 PERFORM remove_resume_actor118(admission.deployment_id);
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=p_command;
 SELECT * INTO effect FROM recipe_effect_attempts WHERE id=(p_claim->>'attempt_id')::uuid;
 IF effect.action='delete' THEN
  SELECT * INTO child FROM recipe_prepared_instance_imports WHERE deployment_id=effect.deployment_id AND resource_name=effect.resource_name;
  SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=effect.deployment_id AND resource_name=effect.resource_name;
  SELECT config_bytes INTO cfg FROM agent_instance_execution_activations WHERE instance_id=child.instance_id;
  PERFORM prepared_instance_terminal_context143(admission.deployment_id,admission.context_json,p_namespace,p_host,p_platform);
  history:=prepared_instance_delete_context143(child.instance_id);
  RETURN command.actor_id::text=hephaestus_actor_id() AND command.operation='remove'
   AND command.idempotency_id::text=current_setting('hephaestus.occurrence_id',true)
   AND admission.vm_provider_namespace=p_namespace AND admission.vm_host_id=p_host AND admission.platform_json=p_platform
   AND effect.claim_json=p_claim AND effect.deployment_id=admission.deployment_id AND effect.command_id<>p_command
   AND resource.resource_kind='instance' AND resource.ownership='owned' AND resource.removal_policy='delete'
   AND resource.active_attempt_id=effect.id AND resource.execution_generation=effect.generation
   AND history->'delete_claim'=p_claim
   AND EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'claims') selected WHERE selected->'claim'=p_claim)
   AND EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'terminal') selected WHERE selected->'delete_claim'=p_claim)
   AND EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'closures') original WHERE original=prepared_closure_origin142(child.instance_id));
 END IF;
 RETURN command.actor_id::text=hephaestus_actor_id()
  AND admission.vm_provider_namespace=p_namespace AND admission.vm_host_id=p_host AND admission.platform_json=p_platform
  AND EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'claims') selected WHERE selected->'claim'=p_claim)
  AND EXISTS(SELECT 1 FROM recipe_effect_attempts old JOIN recipe_deployment_resources r ON r.deployment_id=old.deployment_id AND r.resource_name=old.resource_name
   WHERE old.id=(p_claim->>'attempt_id')::uuid AND old.claim_json=p_claim AND old.deployment_id=admission.deployment_id
    AND old.command_id<>p_command AND old.action IN ('drain','detach','retain') AND r.ownership='owned'
    AND r.input_hash=old.input_hash AND r.plan_json->'identity'=old.claim_json->'identity');
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
 PERFORM prepared_instance_terminal_context143(NEW.deployment_id,NEW.context_json,NEW.vm_provider_namespace,NEW.vm_host_id,NEW.platform_json);
 PERFORM prepared_resume_inventory142(NEW.deployment_id,NEW.vm_provider_namespace,NEW.vm_host_id,NEW.platform_json);
 IF deployment.id IS NULL OR deployment.lifecycle NOT IN ('removing','recovery_required') OR deployment.version IS DISTINCT FROM NEW.expected_version
  OR (deployment.lifecycle='recovery_required'
   AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(NEW.context_json->'pending') item
    WHERE item->'observed_progress'->>'removal'='recovery_required')
   AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(NEW.context_json->'recovery') item
    WHERE item->'observed_progress'->>'removal'='recovery_required')
   AND NOT prepared_instance_terminal_context143(NEW.deployment_id,NEW.context_json,NEW.vm_provider_namespace,NEW.vm_host_id,NEW.platform_json))
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
                    WHERE item->'observed_progress'->>'removal' = 'recovery_required')
                OR EXISTS(SELECT 1 FROM jsonb_array_elements(context->'terminal') item
                    WHERE item->'observed_progress'->>'removal' = 'recovery_required')
                OR EXISTS(SELECT 1 FROM jsonb_array_elements(context->'detach') item
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
 IF prepared_instance_terminal_context143(p_deployment,admission.context_json,p_namespace,p_host,p_platform) THEN
  has_recovery:=true;
 END IF;
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

CREATE FUNCTION guard_prepared_instance_terminal143() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE consumer uuid; instance agent_instances%ROWTYPE; effect recipe_effect_attempts%ROWTYPE;
 resource recipe_deployment_resources%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
BEGIN
 IF TG_OP<>'INSERT' THEN
  RAISE EXCEPTION 'terminal operation and its original publisher are immutable' USING ERRCODE='23000'; END IF;
 PERFORM lock_remove_resume118(NEW.deployment_id);
 consumer:=prepared_instance_current143(NEW.original_json,NEW.configuration_bytes);
 SELECT * INTO instance FROM agent_instances WHERE id=consumer;
 SELECT * INTO effect FROM recipe_effect_attempts WHERE id=NEW.delete_attempt_id;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=NEW.publisher_command_id;
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=NEW.deployment_id AND resource_name=NEW.resource_name;
 IF consumer IS DISTINCT FROM NEW.instance_id OR effect.action IS DISTINCT FROM 'delete'
  OR effect.claim_json IS DISTINCT FROM NEW.original_json->'delete_or_detach_claim'
  OR resource.active_attempt_id IS DISTINCT FROM effect.id OR resource.execution_generation IS DISTINCT FROM effect.generation
  OR recipe_effect_resource_state(resource) IS DISTINCT FROM NEW.delete_progress
  OR command.id::text IS DISTINCT FROM NEW.original_json->'current_command'->>'id'
  OR command.actor_id IS DISTINCT FROM NEW.publisher_actor_id
  OR NEW.removed_at IS DISTINCT FROM now()
  OR NEW.publisher_actor_id::text IS DISTINCT FROM hephaestus_actor_id()
  OR NEW.publisher_request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
  OR instance.state IS DISTINCT FROM 'active' OR instance.run_gate_open IS DISTINCT FROM false OR instance.removed_at IS NOT NULL
  OR instance.version IS DISTINCT FROM NEW.before_instance_version OR to_jsonb(instance) IS DISTINCT FROM NEW.before_instance
  OR NEW.original_json->>'before_instance_version' IS DISTINCT FROM NEW.before_instance_version::text
  OR NEW.after_instance IS DISTINCT FROM NEW.before_instance||jsonb_build_object('state','removed','run_gate_open',false,
    'version',NEW.after_instance_version,'removed_at',NEW.removed_at,'updated_at',NEW.removed_at) THEN
  RAISE EXCEPTION 'current original active Delete, exact locked before/after and publisher required' USING ERRCODE='P1430'; END IF;
 PERFORM prepared_instance_progress143(effect.id,NEW.delete_progress);
 RETURN NEW;
END $$;
CREATE TRIGGER aaa_prepared_instance_terminal_guard143 BEFORE INSERT OR UPDATE OR DELETE
 ON public.recipe_prepared_instance_terminal_operations143 FOR EACH ROW EXECUTE FUNCTION guard_prepared_instance_terminal143();

CREATE FUNCTION validate_prepared_instance_terminal143() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 -- A deferred validator renews actual current authorization independently of
 -- recorded history. An actor/role/request change cannot commit this publication.
 PERFORM prepared_instance_current143(NEW.original_json,NEW.configuration_bytes);
 IF NEW.publisher_actor_id::text IS DISTINCT FROM hephaestus_actor_id()
   OR NEW.publisher_request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
   OR prepared_instance_terminal143(NEW.id) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'exact authorized terminal row, publisher, event and full history required at commit' USING ERRCODE='P1430'; END IF;
 RETURN NEW;
END $$;
CREATE CONSTRAINT TRIGGER prepared_instance_terminal_commit143 AFTER INSERT
 ON public.recipe_prepared_instance_terminal_operations143 DEFERRABLE INITIALLY DEFERRED
 FOR EACH ROW EXECUTE FUNCTION validate_prepared_instance_terminal143();

CREATE OR REPLACE FUNCTION enforce_instance_volume_lifecycle() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE admission agent_instance_execution_activations%ROWTYPE; terminal recipe_prepared_instance_terminal_operations143%ROWTYPE;
BEGIN
 SELECT * INTO terminal FROM recipe_prepared_instance_terminal_operations143 WHERE instance_id=NEW.id;
 IF TG_OP='UPDATE' AND OLD.state='removed' AND terminal.id IS NOT NULL THEN
  RAISE EXCEPTION 'qualified tombstone cannot be mutated or reopened' USING ERRCODE='23000'; END IF;
 IF TG_OP='UPDATE' AND NEW.volume_mode IS DISTINCT FROM OLD.volume_mode THEN
   RAISE EXCEPTION 'instance volume mode is immutable' USING ERRCODE='23000';
 END IF;
 SELECT * INTO admission FROM agent_instance_execution_activations WHERE instance_id=NEW.id;
 IF NEW.volume_mode='named' AND (NEW.state_volume_id IS NOT NULL OR
      (NEW.run_gate_open AND NOT named_volume_dispatch_supported(NEW.id)
        AND NOT(admission.id IS NOT NULL AND admission.revision_id=NEW.active_revision_id AND NEW.version>=2))) THEN
   RAISE EXCEPTION 'named runtime dispatch is unsupported' USING ERRCODE='23000';
 END IF;
 IF TG_OP='UPDATE' AND admission.id IS NOT NULL THEN
   IF NEW.version<OLD.version OR NEW.version<2 THEN
     RAISE EXCEPTION 'activation version cannot be rewound' USING ERRCODE='23000';
   END IF;
   IF NEW.run_gate_open AND NOT OLD.run_gate_open AND
      (OLD.version IS DISTINCT FROM 1 OR NEW.version IS DISTINCT FROM 2
       OR admission.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
       OR admission.request_id IS DISTINCT FROM NULLIF(current_setting('hephaestus.request_id',true),'')::uuid) THEN
     RAISE EXCEPTION 'activation receipt cannot reopen a closed gate' USING ERRCODE='23000';
   END IF;
 END IF;
 IF TG_OP='UPDATE' AND NEW.volume_mode='named' AND OLD.active_revision_id IS NOT NULL
    AND NEW.active_revision_id IS DISTINCT FROM OLD.active_revision_id THEN
   RAISE EXCEPTION 'typed revision updates are unsupported' USING ERRCODE='23000';
 END IF;
 IF EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=NEW.id)
    AND (NEW.run_gate_open OR (TG_OP='UPDATE' AND NEW.active_revision_id IS DISTINCT FROM OLD.active_revision_id) OR (NEW.state='removed' AND NOT(TG_OP='UPDATE' AND OLD.state='active' AND terminal.id IS NOT NULL
      AND terminal.before_instance=to_jsonb(OLD) AND terminal.after_instance=to_jsonb(NEW)
      AND terminal.before_instance_version=OLD.version AND terminal.after_instance_version=NEW.version
      AND terminal.publisher_actor_id::text=hephaestus_actor_id()
      AND terminal.publisher_request_id::text=current_setting('hephaestus.request_id',true)
      AND prepared_instance_original143(terminal.original_json,terminal.configuration_bytes)=NEW.id))) THEN
   RAISE EXCEPTION 'permanent closure forbids reopening or unproven tombstone' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION prepared_instance_created118(p_instance uuid,p_attempt uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT qualified_recipe_instance_birth_matches(p_instance) AND EXISTS(
  SELECT 1 FROM recipe_effect_attempts attempt
   JOIN recipe_effect_verifications proof ON proof.attempt_id=attempt.id
   JOIN recipe_execution_transitions transition ON transition.attempt_id=attempt.id AND transition.verification_id=proof.id
   JOIN recipe_deployment_commands current_command ON current_command.id=transition.command_id
   JOIN recipe_prepared_instance_imports child ON child.deployment_id=attempt.deployment_id AND child.resource_name=attempt.resource_name
  WHERE attempt.id=p_attempt AND child.instance_id=p_instance AND attempt.command_id=child.command_id
    AND proof.recorded_by='hephaestus_worker' AND proof.action='create' AND proof.outcome='applied' AND proof.diagnostic IS NULL
    AND proof.deployment_id=attempt.deployment_id AND proof.project_id=attempt.project_id
    AND proof.resource_name=attempt.resource_name AND proof.generation=attempt.generation
    AND proof.input_hash=attempt.input_hash AND proof.identity_json=attempt.claim_json->'identity'
    AND transition.kind IN ('completed','reconciled') AND transition.operation_id=proof.verification_attempt_id
    AND transition.project_id=attempt.project_id AND transition.resource_name=attempt.resource_name
    AND transition.after_state->>'install'='ready' AND transition.after_state->>'active_attempt' IS NULL
    AND ((current_command.id=attempt.command_id AND current_command.operation='install')
      OR (transition.kind='reconciled' AND current_command.operation='remove'
        AND current_command.deployment_id=attempt.deployment_id AND current_command.project_id=attempt.project_id)))
 OR EXISTS(SELECT 1 FROM recipe_prepared_instance_terminal_operations143 terminal
  WHERE terminal.instance_id=p_instance AND terminal.create_attempt_id=p_attempt
    AND prepared_instance_terminal143(terminal.id) IS TRUE);
$$;

CREATE OR REPLACE FUNCTION inspect_prepared_instance_safety118(p_command uuid,p_attempt uuid,p_claim jsonb,p_prepared jsonb,p_namespace text,p_host text)
RETURNS TABLE(birth_proven boolean,removal_id uuid,cleanup_ready boolean,inventory jsonb)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE consumer uuid;
BEGIN
 PERFORM prepared_instance_actor118(p_command,p_attempt);
 consumer:=prepared_instance_target118(p_attempt,p_claim,p_prepared);
 RETURN QUERY SELECT (qualified_recipe_instance_birth_matches(consumer) OR EXISTS(
   SELECT 1 FROM recipe_prepared_instance_terminal_operations143 terminal WHERE terminal.instance_id=consumer
     AND terminal.create_attempt_id=p_attempt AND prepared_instance_terminal143(terminal.id) IS TRUE)),prepared_instance_removal118(consumer),
  prepared_instance_drained118(consumer,p_attempt,p_namespace,p_host),prepared_instance_inventory118(consumer);
END $$;

CREATE OR REPLACE FUNCTION observe_prepared_instance_safety118(p_attempt uuid,p_claim jsonb,p_prepared jsonb,p_namespace text,p_host text)
RETURNS TABLE(birth_proven boolean,removal_id uuid,cleanup_ready boolean,inventory jsonb)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE consumer uuid;
BEGIN
 -- Explicit role before any protected preparation lookup; no PUBLIC/app grant.
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_worker' THEN
  RAISE EXCEPTION 'independent worker pool required' USING ERRCODE='42501'; END IF;
 consumer:=prepared_instance_target118(p_attempt,p_claim,p_prepared);
 RETURN QUERY SELECT (qualified_recipe_instance_birth_matches(consumer) OR EXISTS(
   SELECT 1 FROM recipe_prepared_instance_terminal_operations143 terminal WHERE terminal.instance_id=consumer
     AND terminal.create_attempt_id=p_attempt AND prepared_instance_terminal143(terminal.id) IS TRUE)),prepared_instance_removal118(consumer),
  prepared_instance_drained118(consumer,p_attempt,p_namespace,p_host),prepared_instance_inventory118(consumer);
END $$;

CREATE OR REPLACE FUNCTION ready_retention_quiescent(p_closure uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT EXISTS(SELECT 1 FROM volume_owned_ready_retention_closures closure
  JOIN agent_instance_state_volumes volume ON volume.id=closure.volume_id
  WHERE closure.id=p_closure AND volume.ready_retention_closure_id=closure.id AND volume.ready_retention_reopen_id IS NULL
    AND ready_retention_original_matches(volume.id) AND volume.state='ready'
    AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease WHERE lease.volume_id=volume.id
      AND (lease.released_at IS NULL OR lease.state<>'released' OR lease.run_id IS NULL
        OR NOT EXISTS(SELECT 1 FROM runs execution WHERE execution.id=lease.run_id
          AND execution.instance_id=lease.instance_id
          AND ready_retention_run_terminal(execution,closure.vm_provider_namespace,closure.vm_host_id)
          AND ((execution.execution_profile='legacy_scalar' AND execution.lease_id=lease.id
             AND execution.volume_id=lease.volume_id AND execution.lease_fencing_token=lease.fencing_token)
            OR (execution.execution_profile='owned_canonical' AND EXISTS(
              SELECT 1 FROM run_cleanup_target_leases item JOIN run_cleanup_completions done
                ON done.run_id=item.run_id AND done.generation=item.generation
              WHERE item.run_id=execution.id AND item.lease_id=lease.id
                AND item.volume_id=lease.volume_id AND item.host_id=lease.host_id AND item.fencing_token=lease.fencing_token))))))
    AND NOT EXISTS(SELECT 1 FROM agent_instances consumer WHERE
      (consumer.state_volume_id=volume.id
        OR EXISTS(SELECT 1 FROM agent_instance_revision_volume_bindings binding WHERE binding.instance_id=consumer.id AND binding.volume_id=volume.id)
        OR EXISTS(SELECT 1 FROM agent_instance_volume_leases lease WHERE lease.instance_id=consumer.id AND lease.volume_id=volume.id))
      AND (consumer.run_gate_open
        OR NOT EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=consumer.id)
        OR EXISTS(SELECT 1 FROM mailboxes WHERE instance_id=consumer.id AND state='active')
        OR EXISTS(SELECT 1 FROM mailbox_deliveries WHERE instance_id=consumer.id AND terminal_at IS NULL)
        OR EXISTS(SELECT 1 FROM run_requests WHERE instance_id=consumer.id AND dispatch_state='pending')
        OR EXISTS(SELECT 1 FROM deferred_agent_triggers WHERE instance_id=consumer.id AND state='deferred')
        OR EXISTS(SELECT 1 FROM agent_updates WHERE instance_id=consumer.id AND state NOT IN ('activated','rejected'))
        OR EXISTS(SELECT 1 FROM runs execution WHERE execution.instance_id=consumer.id
          AND ready_retention_run_terminal(execution,closure.vm_provider_namespace,closure.vm_host_id) IS NOT TRUE)
        -- A raw107 import is not a qualified drained recipe consumer. Require its
        -- genuine exact original113 birth and actual applied Remove/Detach proof.
        OR NOT EXISTS(SELECT 1 FROM recipe_prepared_instance_imports child
          JOIN recipe_deployment_resources resource ON resource.deployment_id=child.deployment_id AND resource.resource_name=child.resource_name
          JOIN recipe_effect_attempts attempt ON attempt.deployment_id=resource.deployment_id AND attempt.resource_name=resource.resource_name AND attempt.action='detach'
          JOIN recipe_deployment_commands command ON command.id=attempt.command_id AND command.operation='remove'
          JOIN recipe_effect_verifications proof ON proof.attempt_id=attempt.id AND proof.outcome='applied' AND proof.action='detach' AND proof.recorded_by='hephaestus_worker'
          JOIN recipe_execution_transitions transition ON transition.verification_id=proof.id AND transition.attempt_id=attempt.id
          WHERE child.instance_id=consumer.id AND child.project_id=volume.project_id
            AND resource.resource_id=consumer.id AND resource.resource_kind='instance'
            AND EXISTS(SELECT 1 FROM recipe_effect_attempts original
              JOIN recipe_effect_verifications created ON created.attempt_id=original.id
                AND created.outcome='applied' AND created.action='create' AND created.recorded_by='hephaestus_worker'
              JOIN recipe_execution_transitions installed ON installed.attempt_id=original.id AND installed.verification_id=created.id
              WHERE original.deployment_id=child.deployment_id AND original.resource_name=child.resource_name
                AND original.command_id=child.command_id AND original.action='create'
                AND original.claim_json->'identity'->>'id'=consumer.id::text
                AND installed.kind IN ('completed','reconciled') AND installed.after_state->>'install'='ready'
                AND installed.after_state->>'active_attempt' IS NULL)
            AND transition.kind IN ('completed','reconciled') AND (transition.command_id=command.id OR (transition.kind='reconciled' AND EXISTS(
              SELECT 1 FROM recipe_prepared_instance_terminal_operations143 terminal
              WHERE terminal.instance_id=consumer.id AND terminal.detach_transition_id=transition.id
                AND terminal.detach_verification_id=proof.id
                AND terminal.configuration_json->'vm'=jsonb_build_object('namespace',closure.vm_provider_namespace,'host_id',closure.vm_host_id)
                AND prepared_instance_terminal143(terminal.id) IS TRUE
                AND prepared_instance_detached143(consumer.id,terminal.configuration_bytes)=transition.id)))
            AND transition.after_state->>'removal' IN ('detached','retained')
            AND transition.after_state->>'active_attempt' IS NULL
            AND resource.removal_progress IN ('detached','retained','deleted')
            AND resource.active_attempt_id IS NULL))));
$$;

ALTER FUNCTION public.prepared_instance_birth143(uuid,bytea) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepared_instance_birth143(uuid,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.prepared_instance_cleanup143(uuid,text,text) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepared_instance_cleanup143(uuid,text,text) FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.prepared_instance_progress143(uuid,jsonb) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepared_instance_progress143(uuid,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.prepared_instance_phase143(uuid,text,bytea) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepared_instance_phase143(uuid,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.prepared_instance_detached143(uuid,bytea) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepared_instance_detached143(uuid,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.prepared_instance_original143(jsonb,bytea) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepared_instance_original143(jsonb,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.prepared_instance_terminal143(uuid) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepared_instance_terminal143(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.prepared_instance_current143(jsonb,bytea) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepared_instance_current143(jsonb,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.observe_prepared_instance_detach143(jsonb,bytea) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.observe_prepared_instance_detach143(jsonb,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.delete_prepared_instance143(jsonb,bytea) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.delete_prepared_instance143(jsonb,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.observe_prepared_instance_deleted143(jsonb,bytea) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.observe_prepared_instance_deleted143(jsonb,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.prepared_instance_delete_context143(uuid) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepared_instance_delete_context143(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.prepared_instance_terminal_context143(uuid,jsonb,text,text,jsonb) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.prepared_instance_terminal_context143(uuid,jsonb,text,text,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.load_prepared_instance_delete_continuation143(uuid,jsonb,bytea) OWNER TO postgres;
REVOKE ALL ON FUNCTION public.load_prepared_instance_delete_continuation143(uuid,jsonb,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.guard_prepared_instance_terminal143() OWNER TO postgres;
REVOKE ALL ON FUNCTION public.guard_prepared_instance_terminal143() FROM PUBLIC,hephaestus_app,hephaestus_worker;

ALTER FUNCTION public.validate_prepared_instance_terminal143() OWNER TO postgres;
REVOKE ALL ON FUNCTION public.validate_prepared_instance_terminal143() FROM PUBLIC,hephaestus_app,hephaestus_worker;

GRANT EXECUTE ON FUNCTION public.observe_prepared_instance_detach143(jsonb,bytea),
 public.observe_prepared_instance_deleted143(jsonb,bytea) TO hephaestus_worker;
GRANT EXECUTE ON FUNCTION public.delete_prepared_instance143(jsonb,bytea),
 public.load_prepared_instance_delete_continuation143(uuid,jsonb,bytea) TO hephaestus_app;

-- Associate a genuinely configured current Remove with the exact prior Ready closure.
CREATE OR REPLACE FUNCTION guard_ready_retention_remove_admission() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE closure volume_owned_ready_retention_closures%ROWTYPE;
 deployment recipe_deployments%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 resource recipe_deployment_resources%ROWTYPE; object_project uuid;
 admission recipe_remove_resume_admissions%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 current_context jsonb;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'Ready Remove admission is immutable' USING ERRCODE='23000'; END IF;
 SELECT * INTO closure FROM volume_owned_ready_retention_closures WHERE id=NEW.closure_id;
 SELECT * INTO admission FROM recipe_remove_resume_admissions WHERE command_id=NEW.command_id;
 IF admission.command_id IS NOT NULL THEN
  PERFORM reserve_remove_resume118(closure.deployment_id);
 END IF;
 PERFORM reserve_ready_retention_deployment(closure.deployment_id);
 PERFORM 1 FROM projects WHERE id=closure.project_id FOR UPDATE;
 SELECT * INTO deployment FROM recipe_deployments WHERE id=closure.deployment_id FOR UPDATE;
 SELECT * INTO resource FROM recipe_deployment_resources
   WHERE deployment_id=closure.deployment_id AND resource_name=closure.resource_name FOR UPDATE;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=NEW.command_id;
 IF admission.command_id IS NULL THEN
 IF closure.id IS NULL OR ready_retention_original_matches(closure.volume_id) IS NOT TRUE
  OR ready_retention_manager(NEW.command_id,closure.volume_id) IS NOT TRUE
  OR NEW.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
  OR NEW.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
  OR command.actor_id IS DISTINCT FROM NEW.actor_id OR command.first_request_id IS DISTINCT FROM NEW.request_id
  OR command.deployment_id IS DISTINCT FROM closure.deployment_id OR command.project_id IS DISTINCT FROM closure.project_id
  OR command.event_id IS DISTINCT FROM NEW.event_id OR command.receipt_lifecycle<>'removing'
  OR command.receipt_version IS DISTINCT FROM NEW.expected_version OR deployment.version IS DISTINCT FROM NEW.expected_version
  OR deployment.lifecycle<>'removing' OR resource.removal_progress IN ('retained','deleted')
  OR command.input_hash IS DISTINCT FROM sha256(convert_to(format('["hephaestus-recipe-remove-v1","%s",%s]',deployment.id,NEW.expected_version),'UTF8')) THEN
  RAISE EXCEPTION 'fresh Ready Remove needs exact original closure/current manager and current Removing CAS' USING ERRCODE='23000';
 END IF;
 ELSE
  -- Association request is current authorization; the command/admission first
  -- request and event remain immutable historical comparison data.
  PERFORM remove_resume_actor118(closure.deployment_id);
  SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=closure.deployment_id;
  IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app'
   OR current_setting('transaction_isolation')<>'read committed' OR current_setting('transaction_read_only')<>'off'
   OR closure.id IS NULL OR ready_retention_original_matches(closure.volume_id) IS NOT TRUE
   OR ready_retention_manager(NEW.command_id,closure.volume_id) IS NOT TRUE
   OR NEW.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
   OR NEW.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
   OR command.actor_id IS DISTINCT FROM NEW.actor_id OR command.operation IS DISTINCT FROM 'remove'
   OR command.idempotency_id::text IS DISTINCT FROM current_setting('hephaestus.occurrence_id',true)
   OR command.deployment_id IS DISTINCT FROM closure.deployment_id OR command.project_id IS DISTINCT FROM closure.project_id
   OR command.event_id IS DISTINCT FROM NEW.event_id OR command.receipt_lifecycle NOT IN ('removing','recovery_required')
   OR deployment.version IS DISTINCT FROM NEW.expected_version OR NEW.expected_version<=0
   OR deployment.lifecycle NOT IN ('removing','recovery_required') OR resource.removal_progress IN ('retained','deleted')
   OR resource.resource_kind IS DISTINCT FROM 'volume' OR resource.resource_id IS DISTINCT FROM closure.volume_id
   OR resource.ownership IS DISTINCT FROM 'owned' OR resource.removal_policy IS DISTINCT FROM 'retain'
   OR NOT EXISTS(SELECT 1 FROM recipe_effect_attempts original WHERE original.id=closure.attempt_id
     AND original.action='create' AND original.deployment_id=closure.deployment_id
     AND original.project_id=closure.project_id AND original.resource_name=closure.resource_name
     AND original.claim_json->'identity'->>'id'=closure.volume_id::text)
   OR admission.deployment_id IS DISTINCT FROM deployment.id OR admission.project_id IS DISTINCT FROM deployment.project_id
   OR admission.actor_id IS DISTINCT FROM command.actor_id OR admission.request_id IS DISTINCT FROM command.first_request_id
   OR admission.event_id IS DISTINCT FROM command.event_id OR admission.expected_version IS DISTINCT FROM command.receipt_version
   OR admission.expected_version<=0 OR admission.expected_version>NEW.expected_version
   OR (command.receipt_lifecycle='recovery_required'
    AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'pending') item WHERE item->'observed_progress'->>'removal'='recovery_required')
    AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'recovery') item WHERE item->'observed_progress'->>'removal'='recovery_required')
    AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'detach') item WHERE item->'observed_progress'->>'removal'='recovery_required')
    AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'terminal') item WHERE item->'observed_progress'->>'removal'='recovery_required'))
   OR admission.intent_hash IS DISTINCT FROM deployment.input_hash
   OR admission.vm_provider_namespace IS DISTINCT FROM closure.vm_provider_namespace OR admission.vm_host_id IS DISTINCT FROM closure.vm_host_id
   OR header.project_id IS DISTINCT FROM deployment.project_id OR header.intent_hash IS DISTINCT FROM deployment.input_hash
   OR header.profile IS DISTINCT FROM 'runtime_named_v1' OR header.input_hash IS DISTINCT FROM sha256(header.canonical_input)
   OR admission.preparation_hash IS DISTINCT FROM header.input_hash OR admission.platform_json IS DISTINCT FROM header.platform_json
   OR command.input_hash IS DISTINCT FROM sha256(convert_to(format('["hephaestus-recipe-remove-v1","%s",%s]',deployment.id,admission.expected_version),'UTF8'))
   OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
     WHERE event.id=command.event_id AND event.actor_id=command.actor_id AND event.request_id=command.first_request_id
       AND event.scope_kind='project' AND event.scope_id=command.project_id AND event.occurrence_id=command.idempotency_id
       AND event.cursor=command.event_cursor AND event.aggregate_version=command.event_aggregate_version)
   OR NOT EXISTS(SELECT 1 FROM authorization_audit_events audit WHERE audit.actor_id=NEW.actor_id
     AND audit.request_id=NEW.request_id AND audit.object_type='project' AND audit.object_id=closure.project_id
     AND audit.permission='can_manage' AND audit.decision='allow') THEN
   RAISE EXCEPTION 'configured Ready association requires immutable first admission and current exact CAS/actor' USING ERRCODE='23000';
  END IF;
  current_context:=remove_resume_context118(closure.deployment_id);
  PERFORM prepared_resume_inventory142(closure.deployment_id,admission.vm_provider_namespace,admission.vm_host_id,admission.platform_json);
  PERFORM prepared_instance_terminal_context143(closure.deployment_id,current_context,admission.vm_provider_namespace,admission.vm_host_id,admission.platform_json);
  IF (deployment.lifecycle='recovery_required'
    AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(current_context->'pending') item WHERE item->'observed_progress'->>'removal'='recovery_required')
    AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(current_context->'recovery') item WHERE item->'observed_progress'->>'removal'='recovery_required')
    AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(current_context->'detach') item WHERE item->'observed_progress'->>'removal'='recovery_required')
    AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(current_context->'terminal') item WHERE item->'observed_progress'->>'removal'='recovery_required')) THEN
   RAISE EXCEPTION 'configured Ready association requires complete current original progress' USING ERRCODE='23000';
  END IF;
 END IF;
 -- This is a generic Remove command: repeat ALL materialized owned-resource
 -- cleanup permissions, matching the existing service without old SourceUse.
 FOR resource IN SELECT * FROM recipe_deployment_resources
   WHERE deployment_id=closure.deployment_id AND ownership='owned' ORDER BY resource_name LOOP
  object_project:=recipe_owned_resource_project(CASE resource.resource_kind
    WHEN 'volume' THEN 'state_volume' ELSE 'agent_instance' END,resource.resource_id);
  IF object_project IS NOT NULL AND (object_project<>closure.project_id
    OR check_permission('user',hephaestus_actor_id(),'can_manage',CASE resource.resource_kind
       WHEN 'volume' THEN 'state_volume' ELSE 'agent_instance' END,resource.resource_id::text) IS DISTINCT FROM 1
    OR NOT EXISTS(SELECT 1 FROM authorization_audit_events audit WHERE audit.actor_id=NEW.actor_id
       AND audit.request_id=NEW.request_id AND audit.object_type=CASE resource.resource_kind
         WHEN 'volume' THEN 'state_volume' ELSE 'agent_instance' END
       AND audit.object_id=resource.resource_id AND audit.permission='can_manage' AND audit.decision='allow')) THEN
    RAISE EXCEPTION 'fresh Ready Remove requires all owned cleanup permissions' USING ERRCODE='42501';
  END IF;
 END LOOP;
 RETURN NEW;
END $$;
