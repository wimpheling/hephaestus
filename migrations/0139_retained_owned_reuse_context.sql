-- Additive retained-reuse worker context. The pre-completion Remove assertion
-- remains unchanged: completed retention needs a distinct read-only assertion.
-- This grants neither reopening nor mount/write/current-actor authority.
CREATE FUNCTION public.assert_retained_owned_reuse_context(
 p_closure uuid,p_operation uuid,p_generation bigint,p_purpose bytea,
 p_namespace text,p_host text,p_retention_operation uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_worker' THEN
  RAISE EXCEPTION 'Retained reuse context requires worker' USING ERRCODE='42501';
 END IF;
 -- Every joined identity is unique. Require exactly one complete positive chain;
 -- unfinished Ready retention and a foreign receipt cannot qualify an old birth.
 IF (SELECT count(*) FROM public.volume_owned_ready_retention_closures closure
  JOIN public.agent_instance_state_volumes volume ON volume.id=closure.volume_id
  JOIN public.volume_creation_seals seal ON seal.id=closure.seal_id AND seal.volume_id=volume.id
  JOIN public.volume_owned_backing_purposes purpose ON purpose.volume_id=volume.id AND purpose.seal_id=seal.id
  JOIN public.volume_owned_provisioning_operations operation ON operation.id=closure.provisioning_operation_id
    AND operation.volume_id=volume.id AND operation.purpose_hash=purpose.purpose_hash
  JOIN public.volume_owned_backing_observations observed ON observed.id=closure.ready_observation_id
    AND observed.operation_id=operation.id AND observed.volume_id=volume.id
  JOIN public.recipe_owned_ready_retentions retained ON retained.closure_id=closure.id
  JOIN public.volume_owned_ready_retention_facts fact ON fact.id=retained.fact_id AND fact.closure_id=closure.id
  JOIN public.recipe_execution_transitions transition ON transition.id=retained.transition_id
    AND transition.ready_retention_fact_id=fact.id AND transition.attempt_id=closure.attempt_id
    AND transition.deployment_id=closure.deployment_id AND transition.project_id=closure.project_id
    AND transition.resource_name=closure.resource_name
  JOIN public.recipe_deployment_commands command ON command.id=transition.command_id
    AND command.deployment_id=closure.deployment_id AND command.project_id=closure.project_id
  JOIN public.recipe_deployment_resources resource ON resource.deployment_id=closure.deployment_id
    AND resource.resource_name=closure.resource_name AND resource.project_id=closure.project_id
  JOIN public.application_events fact_event ON fact_event.id=fact.event_id
  JOIN public.application_events transition_event ON transition_event.id=transition.event_id
  WHERE closure.id=p_closure AND closure.provisioning_operation_id=p_operation
    AND closure.generation=p_generation AND closure.purpose_hash=p_purpose
    AND closure.vm_provider_namespace=p_namespace AND closure.vm_host_id=p_host
    AND volume.ready_retention_closure_id=closure.id AND volume.ready_retention_reopen_id IS NULL
    AND NOT EXISTS(SELECT 1 FROM public.recipe_owned_ready_reopenings reopened WHERE reopened.closure_id=closure.id)
    AND closure.generation=1 AND closure.project_id=volume.project_id
    AND closure.attempt_id=seal.attempt_id AND closure.creation_operation_id=seal.operation_id
    AND volume.creation_seal_id=seal.id AND volume.creation_operation_id=seal.operation_id
    AND purpose.project_id=closure.project_id AND purpose.purpose_hash=closure.purpose_hash
    AND operation.id=public.recipe_owned_first_provision_id(seal.operation_id)
    AND operation.expected_generation=0 AND operation.generation=1 AND operation.actor_id=seal.actor_id
    AND observed.generation=1 AND observed.phase='ready'
    AND fact.proof_json-'id'=observed.proof_json-'id'
    AND fact.evidence_hash=sha256(convert_to(fact.proof_json::text,'UTF8'))
    AND fact_event.occurrence_id=fact.id AND fact_event.scope_kind='project' AND fact_event.scope_id=closure.project_id
    AND fact_event.aggregate_type='project' AND fact_event.aggregate_id=closure.project_id
    AND fact_event.event_type='project.changed' AND fact_event.actor_type='system' AND fact_event.actor_id IS NULL
    AND transition.kind='creation_retained_ready' AND transition.operation_id=p_retention_operation
    AND transition.verification_id IS NULL AND transition.partial_retention_receipt_id IS NULL
    AND command.operation='remove' AND command.actor_id=transition.actor_id
    AND transition.after_state=public.recipe_effect_resource_state(resource)
    AND resource.resource_id=volume.id AND resource.resource_kind='volume' AND resource.ownership='owned'
    AND resource.removal_policy='retain' AND resource.install_progress='ready'
    AND resource.removal_progress='retained' AND resource.active_attempt_id IS NULL
    AND transition_event.occurrence_id=transition.id AND transition_event.scope_kind='project'
    AND transition_event.scope_id=closure.project_id AND transition_event.aggregate_type='project'
    AND transition_event.aggregate_id=closure.project_id AND transition_event.event_type='project.changed'
    AND transition_event.actor_id=transition.actor_id AND transition_event.request_id=transition.request_id
    AND transition_event.cursor=transition.event_cursor AND transition_event.aggregate_version=transition.event_aggregate_version
    AND public.recipe_owned_first_birth_completed(volume.id) IS TRUE
    -- Reuse the complete private lease/Run/consumer predicate, without exposing it.
    AND public.ready_retention_quiescent(closure.id) IS TRUE)<>1 THEN
  RAISE EXCEPTION 'Retained reuse requires exact completed retention, completed original birth and closed quiescent head'
    USING ERRCODE='23000';
 END IF;
END $$;
REVOKE ALL ON FUNCTION public.assert_retained_owned_reuse_context(uuid,uuid,bigint,bytea,text,text,uuid)
 FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION public.assert_retained_owned_reuse_context(uuid,uuid,bigint,bytea,text,text,uuid)
 TO hephaestus_worker;
