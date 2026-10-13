-- PRIVATE118 readonly observation of exact original prepared instance creation.
-- No tables, closure mutation, verification writes, provider IO or new authority.
-- Prerequisites are the frozen preparation, activation and Ready-retention helpers.
CREATE FUNCTION prepared_instance_target118(p_attempt uuid,p_claim jsonb,p_prepared jsonb) RETURNS uuid
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE attempt recipe_effect_attempts%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
 header recipe_deployment_execution_preparations%ROWTYPE; resource recipe_deployment_resources%ROWTYPE;
BEGIN
 SELECT * INTO attempt FROM recipe_effect_attempts WHERE id=p_attempt;
 SELECT * INTO child FROM recipe_prepared_instance_imports
  WHERE deployment_id=attempt.deployment_id AND resource_name=attempt.resource_name;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=attempt.deployment_id;
 SELECT * INTO resource FROM recipe_deployment_resources
  WHERE deployment_id=attempt.deployment_id AND resource_name=attempt.resource_name;
 IF attempt.id IS NULL OR child.instance_id IS NULL OR header.deployment_id IS NULL
  OR attempt.claim_json IS DISTINCT FROM p_claim OR attempt.action IS DISTINCT FROM 'create'
  OR attempt.command_id IS DISTINCT FROM header.command_id OR attempt.actor_id IS DISTINCT FROM header.actor_id
  OR attempt.project_id IS DISTINCT FROM child.project_id OR child.command_id IS DISTINCT FROM header.command_id
  OR resource.resource_kind IS DISTINCT FROM 'instance' OR resource.ownership IS DISTINCT FROM 'owned'
  OR resource.resource_id IS DISTINCT FROM child.instance_id OR resource.revision_id IS DISTINCT FROM child.revision_id
  OR attempt.input_hash IS DISTINCT FROM resource.input_hash
  OR attempt.claim_json->'identity' IS DISTINCT FROM resource.plan_json->'identity'
  OR convert_from(child.canonical_import,'UTF8')::jsonb IS DISTINCT FROM p_prepared
  OR child.input_hash IS DISTINCT FROM sha256(child.canonical_import)
  OR header.profile IS DISTINCT FROM 'runtime_named_v1'
  OR NOT EXISTS(SELECT 1 FROM recipe_deployment_commands command
    JOIN recipe_deployments deployment ON deployment.id=command.deployment_id
    WHERE command.id=header.command_id AND command.operation='install'
      AND command.actor_id=header.actor_id AND command.input_hash=deployment.input_hash
      AND deployment.project_id=child.project_id AND header.intent_hash=deployment.input_hash)
  OR NOT EXISTS(SELECT 1 FROM recipe_execution_transitions claimed
    JOIN application_events event ON event.id=claimed.event_id
    JOIN product_event_outbox outbox ON outbox.event_id=event.id
    WHERE claimed.attempt_id=attempt.id AND claimed.kind='claimed' AND claimed.command_id=attempt.command_id
      AND claimed.project_id=attempt.project_id AND claimed.resource_name=attempt.resource_name
      AND claimed.after_state->>'active_attempt'=attempt.id::text
      AND claimed.after_state->>'generation'=attempt.generation::text
      AND event.actor_id=attempt.actor_id AND event.request_id=attempt.request_id
      AND event.cursor=claimed.event_cursor AND event.aggregate_version=claimed.event_aggregate_version) THEN
  RAISE EXCEPTION 'exact original prepared instance claim required' USING ERRCODE='P1180';
 END IF;
 RETURN child.instance_id;
END $$;

-- This accepts a truthful reconciliation under current Remove, without rewriting
-- the original Install or requiring its actor to remain authenticated/authorized.
CREATE FUNCTION prepared_instance_created118(p_instance uuid,p_attempt uuid) RETURNS boolean
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
        AND current_command.deployment_id=attempt.deployment_id AND current_command.project_id=attempt.project_id)));
$$;

CREATE FUNCTION prepared_instance_removal118(p_instance uuid) RETURNS uuid
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT removal.id FROM agent_instance_removal_requests removal
 JOIN agent_instances consumer ON consumer.id=removal.instance_id
 WHERE removal.instance_id=p_instance AND NOT consumer.run_gate_open
   AND EXISTS(SELECT 1 FROM release_command_inbox command
     WHERE command.operation='request_instance_removal' AND command.aggregate_id=consumer.id
       AND command.secondary_id=removal.id AND command.actor_id=removal.created_by
       AND command.request_id=removal.request_id AND command.input_hash=removal.input_hash)
   AND EXISTS(SELECT 1 FROM agent_instance_events event
     WHERE event.instance_id=consumer.id AND event.event_type='instance.removal_requested'
       AND event.actor_id=removal.created_by AND event.request_id=removal.request_id
       AND event.payload=jsonb_build_object('removal_id',removal.id,'cleanup_complete',false))
   AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
     WHERE event.aggregate_type='agent_instance' AND event.aggregate_id=consumer.id
       AND event.scope_kind='agent_instance' AND event.scope_id=consumer.id
       AND event.actor_id=removal.created_by AND event.request_id=removal.request_id);
$$;

CREATE FUNCTION prepared_instance_inventory118(p_instance uuid) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 WITH relevant_volumes AS (
  SELECT state_volume_id id FROM agent_instances WHERE id=p_instance AND state_volume_id IS NOT NULL
  UNION SELECT volume_id FROM agent_instance_revision_volume_bindings WHERE instance_id=p_instance
  UNION SELECT volume_id FROM agent_instance_volume_leases
    WHERE instance_id=p_instance OR run_id IN (SELECT id FROM runs WHERE instance_id=p_instance)
 ), work AS (
  SELECT 'mailbox' kind,id FROM mailboxes WHERE instance_id=p_instance
  UNION ALL SELECT 'delivery',event_id FROM mailbox_deliveries WHERE instance_id=p_instance
  UNION ALL SELECT 'delivery_attempt',id FROM mailbox_delivery_attempts WHERE instance_id=p_instance
  UNION ALL SELECT 'run_request',id FROM run_requests WHERE instance_id=p_instance
  UNION ALL SELECT 'deferred_trigger',id FROM deferred_agent_triggers WHERE instance_id=p_instance
  UNION ALL SELECT 'update',id FROM agent_updates WHERE instance_id=p_instance
 ) SELECT jsonb_build_object(
  'runs',COALESCE((SELECT jsonb_agg(id ORDER BY id) FROM runs WHERE instance_id=p_instance),'[]'::jsonb),
  'leases',COALESCE((SELECT jsonb_agg(id ORDER BY id) FROM agent_instance_volume_leases
     WHERE instance_id=p_instance OR run_id IN (SELECT id FROM runs WHERE instance_id=p_instance)),'[]'::jsonb),
  'mount_grants',COALESCE((SELECT jsonb_agg(id ORDER BY id) FROM agent_instance_volume_mount_grants WHERE instance_id=p_instance),'[]'::jsonb),
  'volumes',COALESCE((SELECT jsonb_agg(id ORDER BY id) FROM relevant_volumes),'[]'::jsonb),
  'consumers',COALESCE((SELECT jsonb_agg(id ORDER BY id) FROM agent_instances consumer
    WHERE consumer.id=p_instance OR consumer.state_volume_id IN (SELECT id FROM relevant_volumes)
      OR EXISTS(SELECT 1 FROM agent_instance_revision_volume_bindings binding
          WHERE binding.instance_id=consumer.id AND binding.volume_id IN (SELECT id FROM relevant_volumes))
      OR EXISTS(SELECT 1 FROM agent_instance_volume_leases lease
          WHERE lease.instance_id=consumer.id AND lease.volume_id IN (SELECT id FROM relevant_volumes))),'[]'::jsonb),
  'work',COALESCE((SELECT jsonb_agg(jsonb_build_object('kind',kind,'id',id) ORDER BY kind,id) FROM work),'[]'::jsonb));
$$;

CREATE FUNCTION prepared_instance_drained118(p_instance uuid,p_attempt uuid,p_namespace text,p_host text) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT prepared_instance_created118(p_instance,p_attempt) IS TRUE
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

CREATE FUNCTION prepared_instance_actor118(p_command uuid,p_attempt uuid) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE actor uuid; command recipe_deployment_commands%ROWTYPE; original recipe_effect_attempts%ROWTYPE;
 owned record; owning_project uuid;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app' THEN
  RAISE EXCEPTION 'current manager actor pool required' USING ERRCODE='42501'; END IF;
 actor:=NULLIF(hephaestus_actor_id(),'')::uuid;
 IF actor IS NULL OR NOT EXISTS(SELECT 1 FROM users WHERE id=actor AND status='active') THEN
  RAISE EXCEPTION 'active current manager required' USING ERRCODE='42501'; END IF;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=p_command;
 SELECT * INTO original FROM recipe_effect_attempts WHERE id=p_attempt;
 IF command.id IS NULL OR command.operation<>'remove' OR command.actor_id IS DISTINCT FROM actor
  OR command.deployment_id IS DISTINCT FROM original.deployment_id
  OR command.project_id IS DISTINCT FROM original.project_id
  OR command.idempotency_id::text IS DISTINCT FROM current_setting('hephaestus.occurrence_id',true)
  OR NULLIF(current_setting('hephaestus.request_id',true),'') IS NULL
  OR check_permission('user',actor::text,'can_manage','project',command.project_id::text) IS DISTINCT FROM 1
  OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
    WHERE event.id=command.event_id AND event.scope_kind='project' AND event.scope_id=command.project_id
      AND event.aggregate_type='project' AND event.aggregate_id=command.project_id AND event.actor_type='user'
      AND event.actor_id=actor AND event.request_id=command.first_request_id
      AND event.occurrence_id=command.idempotency_id AND event.cursor=command.event_cursor
      AND event.aggregate_version=command.event_aggregate_version) THEN
  RAISE EXCEPTION 'exact current admitted Remove required' USING ERRCODE='42501'; END IF;
 FOR owned IN SELECT resource_kind,resource_id FROM recipe_deployment_resources
     WHERE deployment_id=command.deployment_id AND ownership='owned' LOOP
  owning_project:=recipe_owned_resource_project(CASE owned.resource_kind WHEN 'volume' THEN 'state_volume' ELSE 'agent_instance' END,owned.resource_id);
  IF owning_project IS NOT NULL AND (owning_project IS DISTINCT FROM command.project_id
    OR check_permission('user',actor::text,'can_manage',CASE owned.resource_kind WHEN 'volume' THEN 'state_volume' ELSE 'agent_instance' END,owned.resource_id::text) IS DISTINCT FROM 1) THEN
   RAISE EXCEPTION 'current cleanup permission for every owned resource required' USING ERRCODE='42501'; END IF;
 END LOOP;
END $$;

CREATE FUNCTION inspect_prepared_instance_safety118(p_command uuid,p_attempt uuid,p_claim jsonb,p_prepared jsonb,p_namespace text,p_host text)
RETURNS TABLE(birth_proven boolean,removal_id uuid,cleanup_ready boolean,inventory jsonb)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE consumer uuid;
BEGIN
 PERFORM prepared_instance_actor118(p_command,p_attempt);
 consumer:=prepared_instance_target118(p_attempt,p_claim,p_prepared);
 RETURN QUERY SELECT qualified_recipe_instance_birth_matches(consumer),prepared_instance_removal118(consumer),
  prepared_instance_drained118(consumer,p_attempt,p_namespace,p_host),prepared_instance_inventory118(consumer);
END $$;
CREATE FUNCTION observe_prepared_instance_safety118(p_attempt uuid,p_claim jsonb,p_prepared jsonb,p_namespace text,p_host text)
RETURNS TABLE(birth_proven boolean,removal_id uuid,cleanup_ready boolean,inventory jsonb)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE consumer uuid;
BEGIN
 -- Explicit role before any protected preparation lookup; no PUBLIC/app grant.
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_worker' THEN
  RAISE EXCEPTION 'independent worker pool required' USING ERRCODE='42501'; END IF;
 consumer:=prepared_instance_target118(p_attempt,p_claim,p_prepared);
 RETURN QUERY SELECT qualified_recipe_instance_birth_matches(consumer),prepared_instance_removal118(consumer),
  prepared_instance_drained118(consumer,p_attempt,p_namespace,p_host),prepared_instance_inventory118(consumer);
END $$;
REVOKE ALL ON FUNCTION prepared_instance_target118(uuid,jsonb,jsonb),prepared_instance_created118(uuid,uuid),
 prepared_instance_removal118(uuid),prepared_instance_inventory118(uuid),prepared_instance_drained118(uuid,uuid,text,text),
 prepared_instance_actor118(uuid,uuid),inspect_prepared_instance_safety118(uuid,uuid,jsonb,jsonb,text,text),
 observe_prepared_instance_safety118(uuid,jsonb,jsonb,text,text) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION inspect_prepared_instance_safety118(uuid,uuid,jsonb,jsonb,text,text) TO hephaestus_app;
GRANT EXECUTE ON FUNCTION observe_prepared_instance_safety118(uuid,jsonb,jsonb,text,text) TO hephaestus_worker;
