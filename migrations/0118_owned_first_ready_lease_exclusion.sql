-- PRIVATE113 successor, after complete113 preparation and coherent110/111/112.
-- No schema publication. Only positively recipe-mapped births are affected.
-- Runtime acquisition remains governed by every existing scalar/named guard.

-- Native UUIDv5 uses the published Rust mapping unchanged. A preexisting
-- extension in another schema is held: never relocate/drop an operator extension.
DO $$
BEGIN
 IF EXISTS(SELECT 1 FROM pg_catalog.pg_extension extension
   JOIN pg_catalog.pg_namespace namespace ON namespace.oid=extension.extnamespace
   WHERE extension.extname='pgcrypto' AND namespace.nspname<>'public') THEN
  RAISE EXCEPTION 'first-birth verifier requires pgcrypto in public; incompatible existing namespace' USING ERRCODE='55000';
 END IF;
END $$;
CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;
DO $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_proc procedure
   JOIN pg_catalog.pg_namespace namespace ON namespace.oid=procedure.pronamespace
   JOIN pg_catalog.pg_depend dependency ON dependency.objid=procedure.oid
     AND dependency.classid='pg_catalog.pg_proc'::regclass AND dependency.deptype='e'
   JOIN pg_catalog.pg_extension extension ON extension.oid=dependency.refobjid
   WHERE namespace.nspname='public' AND procedure.oid='public.digest(bytea,text)'::regprocedure
      AND extension.extname='pgcrypto') THEN
  RAISE EXCEPTION 'first-birth verifier requires native pgcrypto public.digest(bytea,text)' USING ERRCODE='55000';
 END IF;
END $$;
CREATE FUNCTION recipe_owned_first_provision_id(p_creation_operation uuid) RETURNS uuid
LANGUAGE plpgsql IMMUTABLE STRICT PARALLEL SAFE SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE bytes bytea;
BEGIN
 bytes:=substring(public.digest(uuid_send(p_creation_operation)
   ||convert_to('heph-owned-first-provision-v1','UTF8')||decode('00','hex'),'sha1') FROM 1 FOR 16);
 bytes:=set_byte(bytes,6,(get_byte(bytes,6)&15)|80);
 bytes:=set_byte(bytes,8,(get_byte(bytes,8)&63)|128);
 RETURN encode(bytes,'hex')::uuid;
END $$;
REVOKE ALL ON FUNCTION recipe_owned_first_provision_id(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION recipe_owned_first_birth_matches(p_volume uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT EXISTS(
  SELECT 1 FROM recipe_owned_volume_preparations prepared
  JOIN recipe_effect_attempts attempt ON attempt.id=prepared.attempt_id
  JOIN recipe_deployment_commands command ON command.id=attempt.command_id
  JOIN recipe_deployment_resources resource ON resource.deployment_id=prepared.deployment_id
     AND resource.resource_name=prepared.resource_name
  JOIN recipe_deployment_execution_preparations whole ON whole.deployment_id=prepared.deployment_id
  JOIN volume_creation_seals seal ON seal.volume_id=prepared.volume_id
  JOIN agent_instance_state_volumes volume ON volume.id=seal.volume_id
  WHERE prepared.volume_id=p_volume AND prepared.mapping_version=1
   AND prepared.project_id=resource.project_id AND prepared.project_id=seal.project_id
   AND prepared.project_id=whole.project_id AND prepared.project_id=command.project_id
   AND prepared.deployment_id=attempt.deployment_id AND prepared.resource_name=attempt.resource_name
   AND whole.command_id=command.id AND whole.actor_id=attempt.actor_id
   AND command.actor_id=attempt.actor_id AND attempt.project_id=prepared.project_id
   AND whole.intent_hash=seal.intent_hash AND command.input_hash=whole.intent_hash
   AND whole.version=1 AND whole.profile='runtime_named_v1' AND whole.input_hash=sha256(whole.canonical_input)
   AND convert_from(whole.canonical_input,'UTF8')::jsonb->'platform'=whole.platform_json
   AND command.operation='install' AND attempt.action='create'
   AND resource.resource_kind='volume' AND resource.ownership='owned'
   AND resource.resource_id=prepared.volume_id AND resource.filesystem_uuid=prepared.filesystem_uuid
   AND resource.input_hash=attempt.input_hash
   AND resource.plan_json->'identity'=attempt.claim_json->'identity'
   AND (resource.plan_json->'intent'->>'capacity_bytes')::bigint=prepared.capacity_bytes
   -- Canonical Rust bytes need not use PostgreSQL JSON spacing: compare value plus digest.
   AND convert_from(prepared.canonical_claim,'UTF8')::jsonb=attempt.claim_json
   AND prepared.claim_digest=sha256(prepared.canonical_claim)
   AND seal.scope_id=prepared.scope_id AND seal.operation_id=prepared.registration_operation_id
   AND seal.effect_id=prepared.effect_id AND seal.attempt_id=attempt.id
   AND seal.command_id=command.id AND seal.actor_id=attempt.actor_id
   AND seal.origin_request_id=attempt.request_id
   AND seal.resource_version=(attempt.claim_json->>'resource_version')::bigint
   AND seal.generation=attempt.generation AND seal.input_hash=attempt.input_hash
   AND seal.capacity_bytes=prepared.capacity_bytes AND seal.filesystem_uuid=prepared.filesystem_uuid
   AND volume.creation_seal_id=seal.id AND volume.creation_operation_id=seal.operation_id
   AND volume.project_id=seal.project_id AND volume.capacity_bytes=seal.capacity_bytes
   AND volume.filesystem_uuid=seal.filesystem_uuid
 );
$$;
-- No active-claim/lifecycle/source predicate replaces actual immutable completion.
-- Retain and later external reuse remain legal only after genuine first completion.
CREATE FUNCTION recipe_owned_first_birth_completed(p_volume uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT recipe_owned_first_birth_matches(p_volume) AND EXISTS(
  SELECT 1 FROM recipe_owned_volume_preparations prepared
  JOIN recipe_effect_attempts attempt ON attempt.id=prepared.attempt_id
  JOIN recipe_effect_verifications proof ON proof.attempt_id=attempt.id
  JOIN recipe_execution_transitions transition ON transition.verification_id=proof.id
  JOIN volume_creation_seals seal ON seal.volume_id=prepared.volume_id
  JOIN volume_owned_backing_purposes purpose ON purpose.volume_id=seal.volume_id
      AND purpose.seal_id=seal.id AND purpose.project_id=seal.project_id
  JOIN volume_owned_provisioning_operations operation ON operation.volume_id=purpose.volume_id
      AND operation.purpose_hash=purpose.purpose_hash AND operation.expected_generation=0 AND operation.generation=1
  JOIN volume_owned_backing_observations observation ON observation.operation_id=operation.id
      AND observation.volume_id=operation.volume_id AND observation.generation=operation.generation
  WHERE prepared.volume_id=p_volume AND proof.recorded_by='hephaestus_worker'
    AND proof.outcome='applied' AND proof.action='create' AND proof.generation=attempt.generation
    AND proof.deployment_id=prepared.deployment_id AND proof.project_id=prepared.project_id
    AND proof.resource_name=prepared.resource_name AND proof.input_hash=attempt.input_hash
    AND proof.identity_json=attempt.claim_json->'identity'
    AND transition.kind IN ('completed','reconciled') AND transition.attempt_id=attempt.id
    AND transition.command_id=attempt.command_id AND transition.deployment_id=prepared.deployment_id
    AND transition.project_id=prepared.project_id AND transition.resource_name=prepared.resource_name
    AND transition.operation_id=proof.verification_attempt_id
    AND transition.after_state->>'install'='ready' AND transition.after_state->>'last_action'='create'
    AND transition.after_state->>'active_attempt' IS NULL
    AND (transition.after_state->>'generation')::bigint=attempt.generation
    AND operation.id=recipe_owned_first_provision_id(seal.operation_id)
    AND operation.actor_id=seal.actor_id AND observation.phase='ready'
    AND EXISTS(SELECT 1 FROM volume_owned_transition_proofs claim_proof
      WHERE claim_proof.volume_id=seal.volume_id AND claim_proof.operation_id=operation.id
        AND claim_proof.mode='claim' AND claim_proof.generation=1 AND claim_proof.previous_generation=0)
    AND EXISTS(SELECT 1 FROM volume_owned_transition_proofs ready_proof
      WHERE ready_proof.id=observation.id AND ready_proof.volume_id=seal.volume_id
        AND ready_proof.operation_id=operation.id AND ready_proof.mode='ready'
        AND ready_proof.generation=1 AND ready_proof.previous_generation=1)
    AND EXISTS(SELECT 1 FROM application_events admission
      WHERE admission.id=operation.event_id AND admission.occurrence_id=operation.id
        AND admission.scope_kind='project' AND admission.scope_id=seal.project_id
        AND admission.aggregate_type='project' AND admission.aggregate_id=seal.project_id
        AND admission.event_type='project.changed' AND admission.actor_id=seal.actor_id
        AND admission.request_id=operation.request_id)
    AND EXISTS(SELECT 1 FROM application_events ready_event
      WHERE ready_event.id=observation.event_id AND ready_event.occurrence_id=observation.id
        AND ready_event.scope_kind='project' AND ready_event.scope_id=seal.project_id
        AND ready_event.aggregate_type='project' AND ready_event.aggregate_id=seal.project_id
        AND ready_event.event_type='project.changed')
 );
$$;

CREATE FUNCTION guard_recipe_owned_first_birth_lease() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF EXISTS(SELECT 1 FROM recipe_owned_volume_preparations WHERE volume_id=NEW.volume_id)
    AND recipe_owned_first_birth_completed(NEW.volume_id) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'original protected owned Create must complete before runtime lease admission' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
-- Both scalar and named admission use the SAME canonical lease table. Applying
-- this BEFORE INSERT guard includes raw actor/worker SQL and run_id NULL history.
CREATE TRIGGER aaa_recipe_owned_first_birth_lease BEFORE INSERT OR UPDATE OF volume_id
 ON agent_instance_volume_leases FOR EACH ROW EXECUTE FUNCTION guard_recipe_owned_first_birth_lease();

CREATE FUNCTION assert_recipe_owned_first_ready(p_volume uuid,p_operation uuid,p_generation bigint,p_purpose bytea)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_worker' THEN
  RAISE EXCEPTION 'first Ready verification requires worker role' USING ERRCODE='42501';
 END IF;
 IF recipe_owned_first_birth_matches(p_volume) IS DISTINCT FROM true
    OR recipe_owned_first_birth_completed(p_volume) IS NOT FALSE
    OR NOT EXISTS(SELECT 1 FROM recipe_owned_volume_preparations prepared
      JOIN recipe_effect_attempts attempt ON attempt.id=prepared.attempt_id
      JOIN recipe_deployment_commands command ON command.id=attempt.command_id
      JOIN recipe_deployments deployment ON deployment.id=prepared.deployment_id
      JOIN recipe_deployment_resources resource ON resource.deployment_id=prepared.deployment_id
         AND resource.resource_name=prepared.resource_name
      JOIN recipe_deployment_execution_preparations whole ON whole.deployment_id=deployment.id
      WHERE prepared.volume_id=p_volume AND command.operation='install' AND attempt.action='create'
        AND resource.active_attempt_id=attempt.id AND resource.execution_generation=attempt.generation
        AND resource.install_progress IN ('creating','recovery_required')
        AND resource.removal_progress='pending'
        AND deployment.lifecycle IN ('installing','recovery_required')
        AND deployment.project_id=prepared.project_id AND deployment.input_hash=whole.intent_hash
        AND command.id=whole.command_id AND command.input_hash=whole.intent_hash
        AND whole.version=1 AND whole.profile='runtime_named_v1'
        AND NOT EXISTS(SELECT 1 FROM recipe_deployment_commands removal
           WHERE removal.deployment_id=deployment.id AND removal.operation='remove'))
    OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE volume_id=p_volume)
    OR EXISTS(SELECT 1 FROM volume_owned_birth_retirements WHERE volume_id=p_volume)
    OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes volume
      JOIN volume_owned_backing_purposes purpose ON purpose.volume_id=volume.id
      JOIN volume_owned_provisioning_operations operation ON operation.volume_id=volume.id
      JOIN volume_owned_backing_observations observed ON observed.operation_id=operation.id
         AND observed.volume_id=volume.id AND observed.generation=operation.generation
      WHERE volume.id=p_volume AND volume.state='ready' AND volume.provisioning_state='ready'
        AND volume.provisioning_generation=1 AND purpose.purpose_hash=p_purpose
        AND operation.id=p_operation AND operation.purpose_hash=p_purpose
        AND operation.id=recipe_owned_first_provision_id(volume.creation_operation_id)
        AND operation.expected_generation=0 AND operation.generation=1 AND p_generation=1
        AND observed.phase='ready'
        AND NOT EXISTS(SELECT 1 FROM volume_owned_provisioning_operations extra
           WHERE extra.volume_id=volume.id AND extra.id<>p_operation)) THEN
  RAISE EXCEPTION 'first Ready verification requires unfinished exact birth and global zero lease history' USING ERRCODE='23000';
 END IF;
END $$;
REVOKE ALL ON FUNCTION recipe_owned_first_birth_matches(uuid),recipe_owned_first_birth_completed(uuid),
 guard_recipe_owned_first_birth_lease(),assert_recipe_owned_first_ready(uuid,uuid,bigint,bytea)
 FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION assert_recipe_owned_first_ready(uuid,uuid,bigint,bytea) TO hephaestus_worker;
