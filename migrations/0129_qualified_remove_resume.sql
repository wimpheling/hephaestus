-- PRIVATE additive118: fresh current manager continuation of original removal.
-- All frozen observer/closure/retention functions retain their original contracts.
CREATE TABLE recipe_remove_resume_admissions (
 command_id uuid PRIMARY KEY REFERENCES recipe_deployment_commands(id),
 deployment_id uuid NOT NULL REFERENCES recipe_deployments(id),
 project_id uuid NOT NULL REFERENCES projects(id),
 expected_version bigint NOT NULL CHECK(expected_version>0),
 actor_id uuid NOT NULL REFERENCES users(id), request_id uuid NOT NULL,
 event_id uuid NOT NULL REFERENCES application_events(id),
 intent_hash bytea NOT NULL CHECK(octet_length(intent_hash)=32),
 preparation_hash bytea NOT NULL CHECK(octet_length(preparation_hash)=32),
 vm_provider_namespace text NOT NULL, vm_host_id text NOT NULL,
 platform_json jsonb NOT NULL, context_json jsonb NOT NULL,
 CHECK(vm_provider_namespace ~ '^[A-Za-z0-9._:-]{1,128}$' AND vm_provider_namespace NOT IN ('.','..')),
 CHECK(vm_host_id ~ '^[A-Za-z0-9._:-]{1,128}$' AND vm_host_id NOT IN ('.','..'))
);
CREATE FUNCTION remove_resume_actor118(p_deployment uuid) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE actor uuid; project uuid; owned record; owning_project uuid;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app' THEN
  RAISE EXCEPTION 'current manager actor pool required' USING ERRCODE='42501'; END IF;
 actor:=NULLIF(hephaestus_actor_id(),'')::uuid;
 SELECT project_id INTO project FROM recipe_deployments WHERE id=p_deployment;
 IF actor IS NULL OR project IS NULL OR NOT EXISTS(SELECT 1 FROM users WHERE id=actor AND status='active')
  OR check_permission('user',actor::text,'can_manage','project',project::text) IS DISTINCT FROM 1 THEN
  RAISE EXCEPTION 'active current project manager required' USING ERRCODE='42501'; END IF;
 FOR owned IN SELECT * FROM recipe_deployment_resources WHERE deployment_id=p_deployment AND ownership='owned' ORDER BY resource_name LOOP
  owning_project:=recipe_owned_resource_project(CASE owned.resource_kind WHEN 'volume' THEN 'state_volume' ELSE 'agent_instance' END,owned.resource_id);
  IF owning_project IS NOT NULL AND (owning_project IS DISTINCT FROM project OR check_permission('user',actor::text,'can_manage',
   CASE owned.resource_kind WHEN 'volume' THEN 'state_volume' ELSE 'agent_instance' END,owned.resource_id::text) IS DISTINCT FROM 1) THEN
   RAISE EXCEPTION 'every materialized owned cleanup permission required' USING ERRCODE='42501'; END IF;
 END LOOP;
END $$;
-- Keys are immutable comparison data. Only the owning Rust transaction blocks
-- on this complete ordered set, before parent locks, then independently rechecks.
CREATE FUNCTION remove_resume_reservations118(p_deployment uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE commands jsonb; instances jsonb;
BEGIN
 PERFORM remove_resume_actor118(p_deployment);
 IF NOT EXISTS(SELECT 1 FROM recipe_deployment_execution_preparations WHERE deployment_id=p_deployment AND profile='runtime_named_v1') THEN
  RAISE EXCEPTION 'complete original preparation required' USING ERRCODE='P1180'; END IF;
 SELECT COALESCE(jsonb_agg(key ORDER BY key),'[]'::jsonb) INTO commands FROM (
  SELECT DISTINCT recipe_import107_command_mutex(import_command_key) key FROM recipe_prepared_instance_imports WHERE deployment_id=p_deployment
 ) ordered_commands;
 SELECT COALESCE(jsonb_agg(jsonb_build_array(first_key,second_key) ORDER BY first_key,second_key),'[]'::jsonb) INTO instances FROM (
  SELECT DISTINCT keys.first_key,keys.second_key FROM recipe_prepared_instance_imports child
   CROSS JOIN LATERAL recipe_import113_instance_mutex(child.instance_id) keys WHERE child.deployment_id=p_deployment
 ) ordered_instances;
 RETURN jsonb_build_object('commands',commands,'instances',instances);
END $$;
-- Raw/public callers can only TRY; no caller-selected blocking mode exists.
CREATE FUNCTION reserve_remove_resume118(p_deployment uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE key bigint; pair record;
BEGIN
 PERFORM remove_resume_reservations118(p_deployment);
 FOR key IN SELECT DISTINCT recipe_import107_command_mutex(import_command_key) FROM recipe_prepared_instance_imports
   WHERE deployment_id=p_deployment ORDER BY 1 LOOP
  IF pg_try_advisory_xact_lock(key) IS DISTINCT FROM true THEN RAISE EXCEPTION 'original command reservation held' USING ERRCODE='55P03'; END IF;
 END LOOP;
 FOR pair IN SELECT DISTINCT keys.first_key,keys.second_key FROM recipe_prepared_instance_imports child
   CROSS JOIN LATERAL recipe_import113_instance_mutex(child.instance_id) keys WHERE child.deployment_id=p_deployment ORDER BY 1,2 LOOP
  IF pg_try_advisory_xact_lock(pair.first_key,pair.second_key) IS DISTINCT FROM true THEN RAISE EXCEPTION 'original instance reservation held' USING ERRCODE='55P03'; END IF;
 END LOOP;
END $$;
CREATE FUNCTION lock_remove_resume118(p_deployment uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE project uuid; consumers uuid[]; volumes uuid[];
BEGIN
 PERFORM reserve_remove_resume118(p_deployment);
 SELECT project_id INTO project FROM recipe_deployments WHERE id=p_deployment;
 PERFORM id FROM projects WHERE id=project FOR NO KEY UPDATE NOWAIT;
 PERFORM id FROM recipe_deployments WHERE id=p_deployment FOR UPDATE NOWAIT;
 PERFORM resource_name FROM recipe_deployment_resources WHERE deployment_id=p_deployment ORDER BY resource_name FOR UPDATE NOWAIT;
 SELECT COALESCE(array_agg(instance_id ORDER BY instance_id),'{}'::uuid[]) INTO consumers FROM recipe_prepared_instance_imports WHERE deployment_id=p_deployment;
 SELECT COALESCE(array_agg(id ORDER BY id),'{}'::uuid[]) INTO volumes FROM (
  SELECT resource_id id FROM recipe_deployment_resources WHERE deployment_id=p_deployment AND resource_kind='volume'
  UNION SELECT state_volume_id FROM agent_instances WHERE id=ANY(consumers) AND state_volume_id IS NOT NULL
  UNION SELECT volume_id FROM agent_instance_revision_volume_bindings WHERE instance_id=ANY(consumers)
  UNION SELECT volume_id FROM agent_instance_volume_leases WHERE instance_id=ANY(consumers) OR run_id IN (SELECT id FROM runs WHERE instance_id=ANY(consumers))
 ) relevant;
 -- Whole-deployment late participants use ONE global order, never per-child loops.
 PERFORM id FROM agent_instances WHERE id=ANY(consumers) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM runs WHERE instance_id=ANY(consumers) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM agent_instance_state_volumes WHERE id=ANY(volumes) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM agent_instance_volume_leases WHERE volume_id=ANY(volumes) OR instance_id=ANY(consumers)
  OR run_id IN (SELECT id FROM runs WHERE instance_id=ANY(consumers)) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM slot_key FROM agent_instance_revision_volume_bindings WHERE instance_id=ANY(consumers) ORDER BY instance_revision_id,slot_key FOR UPDATE NOWAIT;
 PERFORM id FROM agent_instance_volume_mount_grants WHERE instance_id=ANY(consumers) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM agent_instance_volume_mount_revocations WHERE grant_id IN (SELECT id FROM agent_instance_volume_mount_grants WHERE instance_id=ANY(consumers)) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM mailboxes WHERE instance_id=ANY(consumers) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM event_id FROM mailbox_deliveries WHERE instance_id=ANY(consumers) ORDER BY event_id FOR UPDATE NOWAIT;
 PERFORM id FROM mailbox_delivery_attempts WHERE mailbox_id IN (SELECT id FROM mailboxes WHERE instance_id=ANY(consumers)) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM run_requests WHERE instance_id=ANY(consumers) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM deferred_agent_triggers WHERE instance_id=ANY(consumers) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM agent_updates WHERE instance_id=ANY(consumers) ORDER BY id FOR UPDATE NOWAIT;
 PERFORM scope_id FROM application_event_scopes WHERE (scope_kind='project' AND scope_id=project)
  OR (scope_kind='agent_instance' AND scope_id=ANY(consumers)) ORDER BY scope_kind,scope_id FOR UPDATE NOWAIT;
END $$;
-- Versioned UUIDv5 derivation matches the unchanged Core118 original Drain key.
CREATE FUNCTION original_drain_removal118(p_attempt uuid,p_instance uuid) RETURNS uuid
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE bytes bytea; value text;
BEGIN
 bytes:=substring(public.digest(uuid_send(p_attempt)||convert_to('hephaestus-prepared-instance-removal-v1','UTF8')||decode('00','hex')||uuid_send(p_instance),'sha1') FROM 1 FOR 16);
 bytes:=set_byte(bytes,6,(get_byte(bytes,6)&15)|80);
 bytes:=set_byte(bytes,8,(get_byte(bytes,8)&63)|128);
 value:=encode(bytes,'hex'); RETURN (substring(value,1,8)||'-'||substring(value,9,4)||'-'||substring(value,13,4)||'-'||substring(value,17,4)||'-'||substring(value,21,12))::uuid;
END $$;
CREATE FUNCTION original_drain_command_key118(p_attempt uuid,p_instance uuid) RETURNS bytea
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT sha256(int8send(octet_length('prepared_instance_remove_v1')::bigint)||convert_to('prepared_instance_remove_v1','UTF8')
  ||int8send(16)||uuid_send(p_instance)||int8send(16)||uuid_send(p_attempt));
$$;
CREATE FUNCTION remove_resume_context118(p_deployment uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE claims jsonb:='[]'; closures jsonb:='[]'; resource record; attempt recipe_effect_attempts%ROWTYPE;
 command recipe_deployment_commands%ROWTYPE; child record; removal agent_instance_removal_requests%ROWTYPE;
 inbox release_command_inbox%ROWTYPE; drain recipe_effect_attempts%ROWTYPE; count integer;
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
  claims:=claims||jsonb_build_array(jsonb_build_object('claim',attempt.claim_json,'actor',command.actor_id,'idempotency',command.idempotency_id));
 END LOOP;
 FOR child IN SELECT * FROM recipe_prepared_instance_imports WHERE deployment_id=p_deployment ORDER BY resource_name LOOP
  SELECT * INTO removal FROM agent_instance_removal_requests WHERE id=prepared_instance_removal118(child.instance_id);
  IF removal.id IS NULL THEN
   IF EXISTS(SELECT 1 FROM recipe_deployment_resources r WHERE r.deployment_id=p_deployment AND r.resource_name=child.resource_name AND r.active_attempt_id IS NOT NULL)
    OR EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=child.instance_id) THEN
    RAISE EXCEPTION 'selected old work requires exact original permanent closure' USING ERRCODE='P1180'; END IF;
   -- Unclaimed work may later obtain its first genuine Drain/closure. This does
   -- not grant IO, infer absence, or replace an existing unqualified request.
   CONTINUE;
  END IF;
  SELECT count(*) INTO count FROM recipe_effect_attempts a JOIN recipe_deployment_commands c ON c.id=a.command_id
   WHERE a.deployment_id=p_deployment AND a.resource_name=child.resource_name AND a.action='drain' AND c.operation='remove'
    AND a.actor_id=removal.created_by
    AND original_drain_removal118(a.id,child.instance_id)=removal.id;
  IF count<>1 THEN RAISE EXCEPTION 'unique original Drain closure required' USING ERRCODE='P1180'; END IF;
  SELECT a.* INTO drain FROM recipe_effect_attempts a WHERE a.deployment_id=p_deployment AND a.resource_name=child.resource_name
   AND a.action='drain' AND original_drain_removal118(a.id,child.instance_id)=removal.id;
  SELECT * INTO command FROM recipe_deployment_commands WHERE id=drain.command_id;
  SELECT count(*) INTO count FROM release_command_inbox WHERE secondary_id=removal.id AND operation='request_instance_removal';
  IF count<>1 THEN RAISE EXCEPTION 'unique original closure inbox required' USING ERRCODE='P1180'; END IF;
  SELECT * INTO inbox FROM release_command_inbox WHERE secondary_id=removal.id AND operation='request_instance_removal';
  IF inbox.command_key IS DISTINCT FROM original_drain_command_key118(drain.id,child.instance_id) OR inbox.aggregate_id IS DISTINCT FROM child.instance_id
   OR inbox.actor_id IS DISTINCT FROM removal.created_by OR inbox.request_id IS DISTINCT FROM removal.request_id
   OR inbox.input_hash IS DISTINCT FROM removal.input_hash
   OR removal.input_hash IS DISTINCT FROM sha256(convert_to(recipe_import107_json_bytes(jsonb_build_object('version',1,'operation','request_instance_removal',
    'actor',drain.actor_id,'instance',child.instance_id,'removal',removal.id,'expected_version',removal.expected_version),0),'UTF8')) THEN
   RAISE EXCEPTION 'exact original closure fingerprint required' USING ERRCODE='P1180'; END IF;
  closures:=closures||jsonb_build_array(jsonb_build_object('resource',child.resource_name,'drain_claim',drain.claim_json,
   'actor',command.actor_id,'idempotency',command.idempotency_id,'removal',to_jsonb(removal),'inbox',to_jsonb(inbox)));
 END LOOP;
 RETURN jsonb_build_object('claims',claims,'closures',closures);
END $$;
CREATE FUNCTION guard_remove_resume_admission118() RETURNS trigger
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
 IF deployment.id IS NULL OR deployment.lifecycle<>'removing' OR deployment.version IS DISTINCT FROM NEW.expected_version
  OR command.operation IS DISTINCT FROM 'remove' OR command.deployment_id IS DISTINCT FROM NEW.deployment_id OR command.project_id IS DISTINCT FROM NEW.project_id
  OR deployment.project_id IS DISTINCT FROM NEW.project_id OR deployment.input_hash IS DISTINCT FROM NEW.intent_hash
  OR header.input_hash IS DISTINCT FROM NEW.preparation_hash OR header.platform_json IS DISTINCT FROM NEW.platform_json
  OR NEW.actor_id::text IS DISTINCT FROM hephaestus_actor_id() OR NEW.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
  OR command.actor_id IS DISTINCT FROM NEW.actor_id OR command.first_request_id IS DISTINCT FROM NEW.request_id OR command.event_id IS DISTINCT FROM NEW.event_id
  OR command.idempotency_id::text IS DISTINCT FROM current_setting('hephaestus.occurrence_id',true)
  OR command.receipt_version IS DISTINCT FROM NEW.expected_version OR command.receipt_lifecycle<>'removing'
  OR command.input_hash IS DISTINCT FROM sha256(convert_to(format('["hephaestus-recipe-remove-v1","%s",%s]',deployment.id,NEW.expected_version),'UTF8'))
  OR EXISTS(SELECT 1 FROM runs execution JOIN recipe_prepared_instance_imports child ON child.instance_id=execution.instance_id
    WHERE child.deployment_id=NEW.deployment_id AND (
      (execution.execution_profile='owned_canonical' AND execution.vm_provider_namespace=NEW.vm_provider_namespace
       AND execution.vm_host_id=NEW.vm_host_id AND EXISTS(SELECT 1 FROM run_vm_plan_admissions plan
         WHERE plan.run_id=execution.id AND plan.consumed_at IS NOT NULL AND plan.vm_id=execution.vm_id
           AND plan.vm_provider_namespace=execution.vm_provider_namespace AND plan.vm_host_id=execution.vm_host_id))
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
CREATE TRIGGER remove_resume_admission118 BEFORE INSERT OR UPDATE OR DELETE ON recipe_remove_resume_admissions
 FOR EACH ROW EXECUTE FUNCTION guard_remove_resume_admission118();
CREATE FUNCTION qualified_remove_resume_claim118(p_command uuid,p_claim jsonb,p_namespace text,p_host text,p_platform jsonb) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE admission recipe_remove_resume_admissions%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
BEGIN
 SELECT * INTO admission FROM recipe_remove_resume_admissions WHERE command_id=p_command;
 IF admission.command_id IS NULL THEN RETURN false; END IF;
 PERFORM remove_resume_actor118(admission.deployment_id);
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=p_command;
 RETURN command.actor_id::text=hephaestus_actor_id()
  AND admission.vm_provider_namespace=p_namespace AND admission.vm_host_id=p_host AND admission.platform_json=p_platform
  AND EXISTS(SELECT 1 FROM jsonb_array_elements(admission.context_json->'claims') selected WHERE selected->'claim'=p_claim)
  AND EXISTS(SELECT 1 FROM recipe_effect_attempts old JOIN recipe_deployment_resources r ON r.deployment_id=old.deployment_id AND r.resource_name=old.resource_name
   WHERE old.id=(p_claim->>'attempt_id')::uuid AND old.claim_json=p_claim AND old.deployment_id=admission.deployment_id
    AND old.command_id<>p_command AND old.action IN ('drain','detach','retain') AND r.ownership='owned'
    AND r.input_hash=old.input_hash AND r.plan_json->'identity'=old.claim_json->'identity');
END $$;
-- Add exactly one independently-qualified cross-Remove alternative. Existing
-- same-command and original Install reconciliation branches remain byte-identical.
CREATE OR REPLACE FUNCTION recipe_execution_transition_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
DECLARE command recipe_deployment_commands%ROWTYPE; deployment recipe_deployments%ROWTYPE; resource recipe_deployment_resources%ROWTYPE;
    attempt recipe_effect_attempts%ROWTYPE; proof recipe_effect_verifications%ROWTYPE; expected jsonb; outcome text; diagnostic text;
    expected_lifecycle text; uncertain boolean; partial volume_owned_partial_receipts%ROWTYPE; seal volume_creation_seals%ROWTYPE;
BEGIN
 IF (NEW.kind='creation_retained_ready') IS DISTINCT FROM (NEW.ready_retention_fact_id IS NOT NULL) THEN
  RAISE EXCEPTION 'Ready fact belongs only to distinct ReadyRetention' USING ERRCODE='23000';
 END IF;
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
 IF NEW.kind='creation_retained_ready' THEN
  PERFORM assert_ready_retention_transition(NEW);
  RETURN NEW;
 END IF;
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
       OR NOT EXISTS(SELECT 1 FROM recipe_owned_volume_preparations prepared
          WHERE prepared.attempt_id=attempt.id AND prepared.deployment_id=deployment.id
            AND prepared.project_id=resource.project_id AND prepared.resource_name=resource.resource_name
            AND prepared.volume_id=resource.resource_id AND prepared.filesystem_uuid=resource.filesystem_uuid
            AND prepared.scope_id=seal.scope_id AND prepared.effect_id=seal.effect_id
            AND prepared.registration_operation_id=seal.operation_id
            AND prepared.creation_json=partial.proof_json->'original_creation'
            AND prepared.capacity_bytes=seal.capacity_bytes
            AND EXISTS(SELECT 1 FROM application_events preparation_event JOIN application_events birth_event
               ON birth_event.id=seal.event_id WHERE preparation_event.id=prepared.event_id
                 AND preparation_event.cursor<birth_event.cursor)) OR seal.command_id IS DISTINCT FROM attempt.command_id
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
       AND (EXISTS(SELECT 1 FROM recipe_deployment_commands original WHERE original.id=attempt.command_id AND original.operation='install') OR EXISTS(SELECT 1 FROM recipe_remove_resume_admissions resume WHERE resume.command_id=NEW.command_id AND qualified_remove_resume_claim118(NEW.command_id,attempt.claim_json,resume.vm_provider_namespace,resume.vm_host_id,resume.platform_json)))))
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
-- A second distinct Remove at the current closed CAS cannot commit an orphan
-- generic receipt; its exact generic or existing116 admission must commit too.
CREATE FUNCTION require_remove_resume_admission118() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF NEW.operation='remove' AND EXISTS(SELECT 1 FROM recipe_deployment_commands prior
   WHERE prior.deployment_id=NEW.deployment_id AND prior.operation='remove' AND prior.id<>NEW.id AND prior.event_cursor<NEW.event_cursor)
  AND NOT EXISTS(SELECT 1 FROM recipe_remove_resume_admissions WHERE command_id=NEW.id)
  AND NOT EXISTS(SELECT 1 FROM recipe_owned_ready_remove_admissions WHERE command_id=NEW.id) THEN
  RAISE EXCEPTION 'fresh removing continuation needs exact immutable admission' USING ERRCODE='23000'; END IF;
 RETURN NEW;
END $$;
CREATE CONSTRAINT TRIGGER qualified_remove_resume_commit118 AFTER INSERT ON recipe_deployment_commands
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION require_remove_resume_admission118();
ALTER TABLE recipe_remove_resume_admissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE recipe_remove_resume_admissions FORCE ROW LEVEL SECURITY;
GRANT SELECT,INSERT ON recipe_remove_resume_admissions TO hephaestus_app;
GRANT SELECT ON recipe_remove_resume_admissions TO hephaestus_worker;
CREATE POLICY remove_resume_actor118 ON recipe_remove_resume_admissions TO hephaestus_app
 USING(actor_id::text=hephaestus_actor_id()) WITH CHECK(actor_id::text=hephaestus_actor_id());
CREATE POLICY remove_resume_worker118 ON recipe_remove_resume_admissions FOR SELECT TO hephaestus_worker USING(true);
REVOKE ALL ON FUNCTION remove_resume_actor118(uuid),remove_resume_reservations118(uuid),reserve_remove_resume118(uuid),lock_remove_resume118(uuid),original_drain_removal118(uuid,uuid),original_drain_command_key118(uuid,uuid),require_remove_resume_admission118(),
 remove_resume_context118(uuid),guard_remove_resume_admission118(),qualified_remove_resume_claim118(uuid,jsonb,text,text,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION remove_resume_reservations118(uuid),reserve_remove_resume118(uuid),lock_remove_resume118(uuid),remove_resume_context118(uuid),
 qualified_remove_resume_claim118(uuid,jsonb,text,text,jsonb) TO hephaestus_app;
