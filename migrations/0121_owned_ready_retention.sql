-- PRIVATE116: distinct Ready retention; after profiles/scalar/lifecycle/placement,
-- owned registration/provisioning/partial retention, whole preparation and firstReady.
-- Never changes published migration bytes or original installation receipts.

ALTER TABLE agent_instance_state_volumes
 ADD COLUMN ready_retention_closure_id uuid,
 ADD COLUMN ready_retention_reopen_id uuid;
CREATE TABLE volume_owned_ready_retention_closures (
 id uuid PRIMARY KEY,
 volume_id uuid NOT NULL UNIQUE REFERENCES agent_instance_state_volumes(id),
 project_id uuid NOT NULL REFERENCES projects(id),
 deployment_id uuid NOT NULL,
 resource_name text NOT NULL,
 attempt_id uuid NOT NULL REFERENCES recipe_effect_attempts(id),
 command_id uuid NOT NULL,
 operation_id uuid NOT NULL UNIQUE,
 fingerprint bytea NOT NULL CHECK(octet_length(fingerprint)=32),
 expected_deployment_version bigint NOT NULL CHECK(expected_deployment_version>=0),
 expected_resource_version bigint NOT NULL CHECK(expected_resource_version>0),
 seal_id uuid NOT NULL REFERENCES volume_creation_seals(id),
 creation_operation_id uuid NOT NULL,
 provisioning_operation_id uuid NOT NULL REFERENCES volume_owned_provisioning_operations(id),
 generation bigint NOT NULL CHECK(generation=1),
 purpose_hash bytea NOT NULL CHECK(octet_length(purpose_hash)=32),
 ready_observation_id uuid NOT NULL REFERENCES volume_owned_backing_observations(id),
 vm_provider_namespace text NOT NULL CHECK(run_cleanup_identifier_is_valid(vm_provider_namespace)),
 vm_host_id text NOT NULL CHECK(run_cleanup_identifier_is_valid(vm_host_id)),
 actor_id uuid NOT NULL REFERENCES users(id), request_id uuid NOT NULL,
 event_id uuid NOT NULL REFERENCES application_events(id),
 event_cursor bigint NOT NULL CHECK(event_cursor>0),
 event_aggregate_version bigint NOT NULL CHECK(event_aggregate_version>0),
 transaction_id bigint NOT NULL DEFAULT txid_current(),
 FOREIGN KEY(command_id,deployment_id,project_id) REFERENCES recipe_deployment_commands(id,deployment_id,project_id),
 FOREIGN KEY(deployment_id,resource_name) REFERENCES recipe_deployment_resources(deployment_id,resource_name)
);
CREATE TABLE volume_owned_ready_retention_facts (
 id uuid PRIMARY KEY,
 closure_id uuid NOT NULL REFERENCES volume_owned_ready_retention_closures(id),
 proof_json jsonb NOT NULL CHECK(jsonb_typeof(proof_json)='object' AND proof_json->>'id'=id::text),
 evidence_hash bytea NOT NULL CHECK(evidence_hash=sha256(convert_to(proof_json::text,'UTF8'))),
 event_id uuid NOT NULL REFERENCES application_events(id),
 UNIQUE(id,closure_id)
);
CREATE TABLE recipe_owned_ready_retentions (
 closure_id uuid PRIMARY KEY REFERENCES volume_owned_ready_retention_closures(id),
 fact_id uuid NOT NULL UNIQUE,
 transition_id uuid NOT NULL UNIQUE REFERENCES recipe_execution_transitions(id) DEFERRABLE INITIALLY DEFERRED,
 FOREIGN KEY(fact_id,closure_id) REFERENCES volume_owned_ready_retention_facts(id,closure_id)
);
CREATE TABLE recipe_owned_ready_reopenings (
 id uuid PRIMARY KEY,
 closure_id uuid NOT NULL UNIQUE REFERENCES recipe_owned_ready_retentions(closure_id),
 attempt_id uuid NOT NULL UNIQUE REFERENCES recipe_effect_attempts(id),
 deployment_id uuid NOT NULL REFERENCES recipe_deployments(id),
 project_id uuid NOT NULL REFERENCES projects(id),
 resource_name text NOT NULL,
 command_id uuid NOT NULL,
 actor_id uuid NOT NULL REFERENCES users(id), request_id uuid NOT NULL,
 claim_json jsonb NOT NULL,
 event_id uuid NOT NULL REFERENCES application_events(id),
 FOREIGN KEY(command_id,deployment_id,project_id) REFERENCES recipe_deployment_commands(id,deployment_id,project_id),
 FOREIGN KEY(deployment_id,resource_name) REFERENCES recipe_deployment_resources(deployment_id,resource_name)
);
CREATE TABLE recipe_owned_ready_remove_admissions (
 command_id uuid PRIMARY KEY REFERENCES recipe_deployment_commands(id),
 closure_id uuid NOT NULL REFERENCES volume_owned_ready_retention_closures(id),
 expected_version bigint NOT NULL CHECK(expected_version>=0),
 actor_id uuid NOT NULL REFERENCES users(id),request_id uuid NOT NULL,
 event_id uuid NOT NULL REFERENCES application_events(id)
);
ALTER TABLE agent_instance_state_volumes
 ADD CONSTRAINT ready_retention_closure_fk FOREIGN KEY(ready_retention_closure_id)
 REFERENCES volume_owned_ready_retention_closures(id) DEFERRABLE INITIALLY DEFERRED,
 ADD CONSTRAINT ready_retention_reopening_fk FOREIGN KEY(ready_retention_reopen_id)
 REFERENCES recipe_owned_ready_reopenings(id) DEFERRABLE INITIALLY DEFERRED;

-- Full original relation includes the immutable whole preparation, sealed birth,
-- stable FIRST operation and its original transition/event/observation. No active
-- installer or Source permission is borrowed to authorize current Remove.
CREATE FUNCTION ready_retention_original_matches(p_volume uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT recipe_owned_first_birth_matches(p_volume) AND EXISTS(
  SELECT 1 FROM agent_instance_state_volumes volume
  JOIN volume_creation_seals seal ON seal.id=volume.creation_seal_id AND seal.volume_id=volume.id
  JOIN volume_owned_backing_purposes purpose ON purpose.volume_id=volume.id AND purpose.seal_id=seal.id
  JOIN volume_owned_provisioning_operations operation ON operation.volume_id=volume.id
    AND operation.id=recipe_owned_first_provision_id(seal.operation_id)
    AND operation.purpose_hash=purpose.purpose_hash AND operation.actor_id=seal.actor_id
    AND operation.expected_generation=0 AND operation.generation=1
  JOIN volume_owned_backing_observations observed ON observed.operation_id=operation.id
    AND observed.volume_id=volume.id AND observed.generation=1 AND observed.phase='ready'
  WHERE volume.id=p_volume AND volume.provisioning_state='ready' AND volume.provisioning_generation=1
    AND volume.host_id=purpose.host_id AND volume.host_path=purpose.canonical_path
    AND volume.state IN ('ready','attached','recovering')
    AND NOT EXISTS(SELECT 1 FROM volume_owned_birth_retirements WHERE volume_id=volume.id)
    AND NOT EXISTS(SELECT 1 FROM volume_owned_provisioning_operations other
      WHERE other.volume_id=volume.id AND other.id<>operation.id)
    AND EXISTS(SELECT 1 FROM volume_owned_transition_proofs proof
      WHERE proof.id=operation.id AND proof.operation_id=operation.id AND proof.volume_id=volume.id
        AND proof.mode='claim' AND proof.previous_generation=0 AND proof.generation=1)
    AND EXISTS(SELECT 1 FROM volume_owned_transition_proofs proof
      WHERE proof.id=observed.id AND proof.operation_id=operation.id AND proof.volume_id=volume.id
        AND proof.mode='ready' AND proof.previous_generation=1 AND proof.generation=1)
    AND EXISTS(SELECT 1 FROM application_events event WHERE event.id=operation.event_id
      AND event.occurrence_id=operation.id AND event.actor_id=seal.actor_id
      AND event.request_id=operation.request_id AND event.scope_kind='project' AND event.scope_id=seal.project_id)
    AND EXISTS(SELECT 1 FROM application_events event WHERE event.id=observed.event_id
      AND event.occurrence_id=observed.id AND event.scope_kind='project' AND event.scope_id=seal.project_id)
 );
$$;
CREATE FUNCTION ready_retention_manager(p_command uuid,p_volume uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)='hephaestus_app'
   AND EXISTS(SELECT 1 FROM recipe_deployment_commands command
     JOIN users actor ON actor.id=command.actor_id AND actor.status='active'
     JOIN agent_instance_state_volumes volume ON volume.project_id=command.project_id
     WHERE command.id=p_command AND command.operation='remove' AND volume.id=p_volume
       AND command.actor_id::text=hephaestus_actor_id()
       AND check_permission('user',hephaestus_actor_id(),'can_manage','project',volume.project_id::text)=1
       AND check_permission('user',hephaestus_actor_id(),'can_manage','state_volume',volume.id::text)=1);
$$;

-- Reserve the same exact prepared command/instance labels BEFORE Project locks.
-- Try-locks are safe for raw borrowed transactions and never confer authority.
CREATE FUNCTION reserve_ready_retention_deployment(p_deployment uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE mutex record;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app'
  OR NOT EXISTS(SELECT 1 FROM recipe_deployments deployment JOIN users actor
    ON actor.id::text=hephaestus_actor_id() AND actor.status='active'
    WHERE deployment.id=p_deployment
      AND check_permission('user',hephaestus_actor_id(),'can_manage','project',deployment.project_id::text)=1) THEN
  RAISE EXCEPTION 'Ready retention reservations require active current project manager' USING ERRCODE='42501';
 END IF;
 FOR mutex IN SELECT DISTINCT recipe_import107_command_mutex(import_command_key) AS key
   FROM recipe_prepared_instance_imports WHERE deployment_id=p_deployment ORDER BY key LOOP
  IF pg_try_advisory_xact_lock(mutex.key) IS NOT TRUE THEN
   RAISE EXCEPTION 'Ready retention command reservation held' USING ERRCODE='55P03';
  END IF;
 END LOOP;
 FOR mutex IN SELECT DISTINCT key.first_key,key.second_key
   FROM recipe_prepared_instance_imports child CROSS JOIN LATERAL recipe_import113_instance_mutex(child.instance_id) key
   WHERE child.deployment_id=p_deployment ORDER BY key.first_key,key.second_key LOOP
  IF pg_try_advisory_xact_lock(mutex.first_key,mutex.second_key) IS NOT TRUE THEN
   RAISE EXCEPTION 'Ready retention instance reservation held' USING ERRCODE='55P03';
  END IF;
 END LOOP;
END $$;

-- Caller already owns ordered Project/Deployment/Resource locks. Never wait on
-- inverse-order late locks; actual acquisitions and raw SQL share the Volume row.
CREATE FUNCTION lock_ready_retention_consumers(p_volume uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE consumer uuid;
BEGIN
 FOR consumer IN SELECT instance.id FROM agent_instances instance WHERE instance.state_volume_id=p_volume
  OR EXISTS(SELECT 1 FROM agent_instance_revision_volume_bindings binding WHERE binding.instance_id=instance.id AND binding.volume_id=p_volume)
  OR EXISTS(SELECT 1 FROM agent_instance_volume_leases lease WHERE lease.instance_id=instance.id AND lease.volume_id=p_volume)
  ORDER BY instance.id LOOP
  PERFORM 1 FROM agent_instances WHERE id=consumer FOR UPDATE NOWAIT;
  PERFORM 1 FROM runs WHERE instance_id=consumer ORDER BY id FOR UPDATE NOWAIT;
 END LOOP;
 PERFORM 1 FROM agent_instance_state_volumes WHERE id=p_volume FOR UPDATE NOWAIT;
 PERFORM 1 FROM agent_instance_volume_leases WHERE volume_id=p_volume ORDER BY id FOR UPDATE NOWAIT;
END $$;

-- Readonly global proof. It does not assert host absence independently of the
-- existing scoped cleanup owners. Unknown import/history is intentionally held.
CREATE FUNCTION ready_retention_run_terminal(execution runs,p_namespace text,p_host text) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT execution.state='cleaned_up' AND execution.volume_acquisition_closed_at IS NOT NULL
   AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id AND released_at IS NULL)
   AND CASE execution.execution_profile
    WHEN 'owned_canonical' THEN execution.vm_provider_namespace=p_namespace AND execution.vm_host_id=p_host
      AND EXISTS(SELECT 1 FROM run_cleanup_completions done JOIN run_cleanup_receipts receipt
        ON receipt.run_id=done.run_id AND receipt.generation=done.generation AND receipt.target_digest=done.target_digest
        JOIN run_cleanup_targets target ON target.run_id=receipt.run_id AND target.generation=receipt.generation
          AND target.target_digest=receipt.target_digest
        WHERE done.run_id=execution.id AND receipt.vm_provider_namespace=p_namespace
          AND receipt.vm_host_id=p_host AND receipt.vm_id=execution.vm_id
          AND target.vm_target_kind IN ('planned','persisted') AND run_cleanup_set_matches(done.run_id,done.generation,true)
          AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases history WHERE history.run_id=execution.id
            AND NOT EXISTS(SELECT 1 FROM run_cleanup_target_leases item WHERE item.run_id=done.run_id
              AND item.generation=done.generation AND item.lease_id=history.id AND item.volume_id=history.volume_id
              AND item.host_id=history.host_id AND item.fencing_token=history.fencing_token)))
    WHEN 'legacy_scalar' THEN legacy_scalar_terminal_matches(execution)
      AND ((execution.lease_id IS NULL AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id))
        OR (execution.lease_id IS NOT NULL
          AND (SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=execution.id)=1
          AND EXISTS(SELECT 1 FROM agent_instance_volume_leases history WHERE history.id=execution.lease_id
            AND history.run_id=execution.id AND history.instance_id=execution.instance_id
            AND history.volume_id=execution.volume_id AND history.fencing_token=execution.lease_fencing_token
            AND history.host_id=p_host AND history.state='released' AND history.released_at IS NOT NULL))) AND EXISTS(
      SELECT 1 FROM run_legacy_vm_placements plan WHERE plan.run_id=execution.id
        AND plan.provider_namespace::text=p_namespace AND plan.host_id=p_host AND plan.consumed_at IS NOT NULL
        AND plan.consumption_kind IN ('provision','cleanup_only') AND legacy_vm_placement_matches(execution,plan))
    ELSE false END;
$$;
CREATE FUNCTION ready_retention_quiescent(p_closure uuid) RETURNS boolean
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
            AND transition.kind IN ('completed','reconciled') AND transition.command_id=command.id
            AND transition.after_state->>'removal' IN ('detached','retained')
            AND transition.after_state->>'active_attempt' IS NULL
            AND resource.removal_progress IN ('detached','retained','deleted')
            AND resource.active_attempt_id IS NULL))));
$$;

-- A genuinely new manager gets a new immutable Remove at current closed CAS,
-- without changing an already Removing deployment or the original closure.
CREATE FUNCTION guard_ready_retention_remove_admission() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE closure volume_owned_ready_retention_closures%ROWTYPE;
 deployment recipe_deployments%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 resource recipe_deployment_resources%ROWTYPE; object_project uuid;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'Ready Remove admission is immutable' USING ERRCODE='23000'; END IF;
 SELECT * INTO closure FROM volume_owned_ready_retention_closures WHERE id=NEW.closure_id;
 PERFORM reserve_ready_retention_deployment(closure.deployment_id);
 PERFORM 1 FROM projects WHERE id=closure.project_id FOR UPDATE;
 SELECT * INTO deployment FROM recipe_deployments WHERE id=closure.deployment_id FOR UPDATE;
 SELECT * INTO resource FROM recipe_deployment_resources
   WHERE deployment_id=closure.deployment_id AND resource_name=closure.resource_name FOR UPDATE;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=NEW.command_id;
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
CREATE TRIGGER ready_remove_admission_guard BEFORE INSERT OR UPDATE OR DELETE ON recipe_owned_ready_remove_admissions
 FOR EACH ROW EXECUTE FUNCTION guard_ready_retention_remove_admission();

CREATE FUNCTION guard_ready_retention_closure() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE resource recipe_deployment_resources%ROWTYPE; attempt recipe_effect_attempts%ROWTYPE;
 deployment recipe_deployments%ROWTYPE; seal volume_creation_seals%ROWTYPE; purpose volume_owned_backing_purposes%ROWTYPE;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'Ready retention closure is immutable' USING ERRCODE='23000'; END IF;
 IF ready_retention_manager(NEW.command_id,NEW.volume_id) IS NOT TRUE THEN
  RAISE EXCEPTION 'current active project and volume manager required' USING ERRCODE='42501';
 END IF;
 -- Raw writers have the same pre-project reservations and NOWAIT late order.
 PERFORM reserve_ready_retention_deployment(NEW.deployment_id);
 PERFORM 1 FROM projects WHERE id=NEW.project_id FOR UPDATE;
 SELECT * INTO deployment FROM recipe_deployments WHERE id=NEW.deployment_id FOR UPDATE;
 SELECT * INTO resource FROM recipe_deployment_resources
   WHERE deployment_id=NEW.deployment_id AND resource_name=NEW.resource_name FOR UPDATE;
 PERFORM lock_ready_retention_consumers(NEW.volume_id);
 SELECT * INTO attempt FROM recipe_effect_attempts WHERE id=NEW.attempt_id;
 SELECT * INTO seal FROM volume_creation_seals WHERE id=NEW.seal_id AND volume_id=NEW.volume_id;
 SELECT * INTO purpose FROM volume_owned_backing_purposes WHERE volume_id=NEW.volume_id;
 IF ready_retention_original_matches(NEW.volume_id) IS NOT TRUE
  OR ready_retention_manager(NEW.command_id,NEW.volume_id) IS NOT TRUE
  OR NEW.transaction_id<>txid_current()
  OR NEW.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
  OR NEW.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
  OR deployment.project_id IS DISTINCT FROM NEW.project_id OR deployment.lifecycle NOT IN ('removing','recovery_required')
  OR deployment.version<>NEW.expected_deployment_version OR resource.version<>NEW.expected_resource_version
  OR resource.resource_id IS DISTINCT FROM NEW.volume_id OR resource.ownership<>'owned'
  OR resource.resource_kind<>'volume' OR resource.removal_policy<>'retain' OR resource.removal_progress IN ('retained','deleted')
  OR resource.execution_generation IS DISTINCT FROM attempt.generation OR resource.input_hash IS DISTINCT FROM attempt.input_hash
  OR resource.plan_json->'identity' IS DISTINCT FROM attempt.claim_json->'identity'
  OR attempt.deployment_id IS DISTINCT FROM NEW.deployment_id OR attempt.resource_name IS DISTINCT FROM NEW.resource_name
  OR attempt.action IS DISTINCT FROM 'create' OR seal.attempt_id IS DISTINCT FROM attempt.id
  OR (resource.active_attempt_id IS DISTINCT FROM attempt.id AND
      NOT(resource.active_attempt_id IS NULL AND resource.install_progress='ready' AND recipe_owned_first_birth_completed(NEW.volume_id)))
  OR seal.project_id IS DISTINCT FROM NEW.project_id OR seal.operation_id IS DISTINCT FROM NEW.creation_operation_id
  OR purpose.seal_id IS DISTINCT FROM NEW.seal_id OR purpose.purpose_hash IS DISTINCT FROM NEW.purpose_hash
  OR NEW.provisioning_operation_id IS DISTINCT FROM recipe_owned_first_provision_id(seal.operation_id)
  OR NOT EXISTS(SELECT 1 FROM volume_owned_backing_observations observed WHERE observed.id=NEW.ready_observation_id
    AND observed.volume_id=NEW.volume_id AND observed.operation_id=NEW.provisioning_operation_id AND observed.generation=1 AND observed.phase='ready')
  OR NOT EXISTS(SELECT 1 FROM recipe_deployment_commands command WHERE command.id=NEW.command_id
    AND command.deployment_id=NEW.deployment_id AND command.project_id=NEW.project_id)
  OR NOT EXISTS(SELECT 1 FROM application_events event WHERE event.id=NEW.event_id
    AND event.occurrence_id=NEW.id AND event.scope_kind='project' AND event.scope_id=NEW.project_id
    AND event.aggregate_type='project' AND event.aggregate_id=NEW.project_id AND event.event_type='project.changed'
    AND event.actor_id=NEW.actor_id AND event.request_id=NEW.request_id
    AND event.cursor=NEW.event_cursor AND event.aggregate_version=NEW.event_aggregate_version)
  OR NOT EXISTS(SELECT 1 FROM authorization_audit_events audit WHERE audit.actor_id=NEW.actor_id AND audit.request_id=NEW.request_id
    AND audit.object_type='project' AND audit.object_id=NEW.project_id AND audit.permission='can_manage' AND audit.decision='allow')
  OR NOT EXISTS(SELECT 1 FROM authorization_audit_events audit WHERE audit.actor_id=NEW.actor_id AND audit.request_id=NEW.request_id
    AND audit.object_type='state_volume' AND audit.object_id=NEW.volume_id AND audit.permission='can_manage' AND audit.decision='allow') THEN
  RAISE EXCEPTION 'Ready closure requires exact positive original birth/current Remove/CAS/audits/event' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER ready_retention_closure_guard BEFORE INSERT OR UPDATE OR DELETE ON volume_owned_ready_retention_closures
 FOR EACH ROW EXECUTE FUNCTION guard_ready_retention_closure();
CREATE FUNCTION apply_ready_retention_closure() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 UPDATE agent_instance_state_volumes SET ready_retention_closure_id=NEW.id WHERE id=NEW.volume_id
   AND ready_retention_closure_id IS NULL AND ready_retention_reopen_id IS NULL;
 IF NOT FOUND THEN RAISE EXCEPTION 'Ready closure head conflicts' USING ERRCODE='23000'; END IF;
 RETURN NULL;
END $$;
CREATE TRIGGER ready_retention_closure_head AFTER INSERT ON volume_owned_ready_retention_closures
 FOR EACH ROW EXECUTE FUNCTION apply_ready_retention_closure();
CREATE FUNCTION guard_ready_retention_volume_head() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF TG_OP='INSERT' THEN
  IF NEW.ready_retention_closure_id IS NOT NULL OR NEW.ready_retention_reopen_id IS NOT NULL THEN
   RAISE EXCEPTION 'new volume cannot inherit Ready retention history' USING ERRCODE='23000';
  END IF;
 ELSE
  IF NEW.ready_retention_closure_id IS DISTINCT FROM OLD.ready_retention_closure_id
    AND NOT(OLD.ready_retention_closure_id IS NULL AND OLD.ready_retention_reopen_id IS NULL
      AND NEW.ready_retention_reopen_id IS NULL AND EXISTS(SELECT 1 FROM volume_owned_ready_retention_closures closure
        WHERE closure.id=NEW.ready_retention_closure_id AND closure.volume_id=NEW.id AND closure.transaction_id=txid_current())) THEN
   RAISE EXCEPTION 'Ready closure cannot be cleared or changed' USING ERRCODE='23000';
  END IF;
  IF NEW.ready_retention_reopen_id IS DISTINCT FROM OLD.ready_retention_reopen_id
    AND NOT(OLD.ready_retention_reopen_id IS NULL AND NEW.ready_retention_closure_id IS NOT NULL
      AND EXISTS(SELECT 1 FROM recipe_owned_ready_reopenings reopened WHERE reopened.id=NEW.ready_retention_reopen_id
        AND reopened.closure_id=NEW.ready_retention_closure_id)) THEN
   RAISE EXCEPTION 'Ready reuse requires exact immutable reopening receipt' USING ERRCODE='23000';
  END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER ready_retention_volume_head_guard BEFORE INSERT OR UPDATE ON agent_instance_state_volumes
 FOR EACH ROW EXECUTE FUNCTION guard_ready_retention_volume_head();

CREATE FUNCTION assert_owned_ready_retention(p_closure uuid,p_operation uuid,p_generation bigint,p_purpose bytea,
 p_namespace text,p_host text) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_worker' THEN
  RAISE EXCEPTION 'Ready retention context requires worker' USING ERRCODE='42501';
 END IF;
 IF NOT EXISTS(SELECT 1 FROM volume_owned_ready_retention_closures closure WHERE closure.id=p_closure
   AND closure.provisioning_operation_id=p_operation AND closure.generation=p_generation
   AND closure.purpose_hash=p_purpose AND closure.vm_provider_namespace=p_namespace AND closure.vm_host_id=p_host)
  OR ready_retention_quiescent(p_closure) IS NOT TRUE
  OR EXISTS(SELECT 1 FROM recipe_owned_ready_retentions WHERE closure_id=p_closure) THEN
  RAISE EXCEPTION 'Ready retention is foreign, unquiesced, completed or reopened' USING ERRCODE='23000';
 END IF;
END $$;
CREATE FUNCTION guard_owned_ready_retention_fact() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE closure volume_owned_ready_retention_closures%ROWTYPE; prior volume_owned_backing_observations%ROWTYPE;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'Ready retention facts are immutable' USING ERRCODE='23000'; END IF;
 SELECT * INTO closure FROM volume_owned_ready_retention_closures WHERE id=NEW.closure_id;
 PERFORM assert_owned_ready_retention(closure.id,closure.provisioning_operation_id,closure.generation,closure.purpose_hash,
   closure.vm_provider_namespace,closure.vm_host_id);
 PERFORM 1 FROM agent_instance_state_volumes WHERE id=closure.volume_id FOR UPDATE NOWAIT;
 IF ready_retention_quiescent(closure.id) IS NOT TRUE THEN RAISE EXCEPTION 'Ready retention lost global quiescence' USING ERRCODE='23000'; END IF;
 SELECT * INTO prior FROM volume_owned_backing_observations WHERE id=closure.ready_observation_id;
 IF NEW.proof_json-'id' IS DISTINCT FROM prior.proof_json-'id'
  OR NOT EXISTS(SELECT 1 FROM application_events event WHERE event.id=NEW.event_id AND event.occurrence_id=NEW.id
    AND event.scope_kind='project' AND event.scope_id=closure.project_id AND event.aggregate_type='project'
    AND event.aggregate_id=closure.project_id AND event.event_type='project.changed'
    AND event.actor_type='system' AND event.actor_id IS NULL) THEN
  RAISE EXCEPTION 'Ready retention fact differs from immutable positive backing/geometry or worker event' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER ready_retention_fact_guard BEFORE INSERT OR UPDATE OR DELETE ON volume_owned_ready_retention_facts
 FOR EACH ROW EXECUTE FUNCTION guard_owned_ready_retention_fact();

-- All original birth writes remain fenced forever, even after explicit reuse.
CREATE FUNCTION reject_closed_ready_birth_writes() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE volume agent_instance_state_volumes%ROWTYPE;
BEGIN
 SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=NEW.volume_id FOR UPDATE NOWAIT;
 IF volume.ready_retention_closure_id IS NOT NULL THEN
  RAISE EXCEPTION 'original owned birth is closed by Ready retention' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER aab_ready_retention_original_operation BEFORE INSERT ON volume_owned_provisioning_operations FOR EACH ROW EXECUTE FUNCTION reject_closed_ready_birth_writes();
CREATE TRIGGER aab_ready_retention_original_transition BEFORE INSERT ON volume_owned_transition_proofs FOR EACH ROW EXECUTE FUNCTION reject_closed_ready_birth_writes();
CREATE TRIGGER aab_ready_retention_original_observation BEFORE INSERT ON volume_owned_backing_observations FOR EACH ROW EXECUTE FUNCTION reject_closed_ready_birth_writes();
CREATE TRIGGER aab_ready_retention_original_partial BEFORE INSERT ON volume_owned_partial_receipts FOR EACH ROW EXECUTE FUNCTION reject_closed_ready_birth_writes();
-- Freeze late original recipe creation evidence too. Distinct ReadyRetention is
-- the only new disposition of that attempt; it never fabricates Install Applied.
CREATE FUNCTION reject_closed_ready_recipe_writes() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF EXISTS(SELECT 1 FROM volume_owned_ready_retention_closures closure
   WHERE closure.attempt_id=NEW.attempt_id)
   AND NOT(TG_TABLE_NAME='recipe_execution_transitions' AND to_jsonb(NEW)->>'kind'='creation_retained_ready') THEN
  RAISE EXCEPTION 'original owned Create evidence is closed by Ready retention' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER aab_ready_retention_original_verification BEFORE INSERT ON recipe_effect_verifications
 FOR EACH ROW EXECUTE FUNCTION reject_closed_ready_recipe_writes();
CREATE TRIGGER aab_ready_retention_original_recipe_transition BEFORE INSERT ON recipe_execution_transitions
 FOR EACH ROW EXECUTE FUNCTION reject_closed_ready_recipe_writes();

CREATE FUNCTION guard_closed_ready_birth_progress() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF OLD.ready_retention_closure_id IS NOT NULL AND ROW(NEW.host_id,NEW.host_path,NEW.provisioning_generation,
   NEW.provisioning_state,NEW.capacity_bytes,NEW.filesystem_uuid,NEW.creation_seal_id,NEW.creation_operation_id)
   IS DISTINCT FROM ROW(OLD.host_id,OLD.host_path,OLD.provisioning_generation,OLD.provisioning_state,
   OLD.capacity_bytes,OLD.filesystem_uuid,OLD.creation_seal_id,OLD.creation_operation_id) THEN
  RAISE EXCEPTION 'closed Ready birth cannot acquire new provisioning authority' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER ready_retention_closed_progress BEFORE UPDATE ON agent_instance_state_volumes FOR EACH ROW EXECUTE FUNCTION guard_closed_ready_birth_progress();

CREATE FUNCTION ready_retention_lease_reopening_matches(p_reopen uuid,p_instance uuid,p_volume uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT EXISTS(SELECT 1 FROM recipe_owned_ready_reopenings reopened
  JOIN recipe_owned_ready_retentions retained ON retained.closure_id=reopened.closure_id
  JOIN volume_owned_ready_retention_closures closure ON closure.id=retained.closure_id AND closure.volume_id=p_volume
  JOIN recipe_prepared_instance_imports child ON child.deployment_id=reopened.deployment_id AND child.project_id=reopened.project_id
  WHERE reopened.id=p_reopen AND child.instance_id=p_instance
    AND EXISTS(SELECT 1 FROM jsonb_array_elements(convert_from(child.canonical_import,'UTF8')::jsonb->'volumes') selection
      WHERE selection->>'volume_id'=p_volume::text));
$$;
CREATE FUNCTION guard_ready_retention_lease() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE volume agent_instance_state_volumes%ROWTYPE;
BEGIN
 -- Always serialize positive mapped births, even BEFORE a closure becomes visible.
 -- Updating the Volume head makes REPEATABLE READ races abort, not miss a closure.
 IF EXISTS(SELECT 1 FROM recipe_owned_volume_preparations WHERE volume_id=NEW.volume_id) THEN
  SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=NEW.volume_id FOR UPDATE NOWAIT;
  IF NEW.released_at IS NULL AND volume.ready_retention_closure_id IS NOT NULL
    AND (volume.ready_retention_reopen_id IS NULL OR
      ready_retention_lease_reopening_matches(volume.ready_retention_reopen_id,NEW.instance_id,NEW.volume_id) IS NOT TRUE) THEN
   RAISE EXCEPTION 'Ready retention admission remains closed' USING ERRCODE='23000';
  END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER aab_ready_retention_lease BEFORE INSERT OR UPDATE ON agent_instance_volume_leases
 FOR EACH ROW EXECUTE FUNCTION guard_ready_retention_lease();
-- Composite first-birth policy: no inactive/manager flag can substitute for a
-- completed original Install or explicit reuse of distinct ReadyRetention proof.
CREATE OR REPLACE FUNCTION guard_recipe_owned_first_birth_lease() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE volume agent_instance_state_volumes%ROWTYPE;
BEGIN
 IF EXISTS(SELECT 1 FROM recipe_owned_volume_preparations WHERE volume_id=NEW.volume_id) THEN
  SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=NEW.volume_id FOR UPDATE NOWAIT;
  IF recipe_owned_first_birth_completed(NEW.volume_id) IS NOT TRUE
    AND (volume.ready_retention_reopen_id IS NULL OR
      ready_retention_lease_reopening_matches(volume.ready_retention_reopen_id,NEW.instance_id,NEW.volume_id) IS NOT TRUE) THEN
   RAISE EXCEPTION 'original protected owned Create must complete before runtime lease admission' USING ERRCODE='23000';
  END IF;
 END IF;
 RETURN NEW;
END $$;

ALTER TABLE recipe_execution_transitions ADD COLUMN ready_retention_fact_id uuid REFERENCES volume_owned_ready_retention_facts(id);
ALTER TABLE recipe_execution_transitions DROP CONSTRAINT recipe_execution_transitions_kind_check;
ALTER TABLE recipe_execution_transitions ADD CONSTRAINT recipe_execution_transitions_kind_check
 CHECK(kind IN ('claimed','completed','ambiguous','reconciled','finished','observation_retired','creation_retained_partial','creation_retained_ready'));
CREATE FUNCTION assert_ready_retention_transition(t recipe_execution_transitions) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE closure volume_owned_ready_retention_closures%ROWTYPE; fact volume_owned_ready_retention_facts%ROWTYPE;
 resource recipe_deployment_resources%ROWTYPE; expected jsonb; expected_lifecycle text;
BEGIN
 SELECT * INTO fact FROM volume_owned_ready_retention_facts WHERE id=t.ready_retention_fact_id;
 SELECT * INTO closure FROM volume_owned_ready_retention_closures WHERE id=fact.closure_id;
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=t.deployment_id AND resource_name=t.resource_name;
 PERFORM lock_ready_retention_consumers(closure.volume_id);
 IF closure.id IS NULL OR ready_retention_manager(t.command_id,closure.volume_id) IS NOT TRUE
  OR ready_retention_quiescent(closure.id) IS NOT TRUE
  OR t.project_id IS DISTINCT FROM closure.project_id OR t.deployment_id IS DISTINCT FROM closure.deployment_id
  OR t.resource_name IS DISTINCT FROM closure.resource_name OR t.attempt_id IS DISTINCT FROM closure.attempt_id
  OR t.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
  OR t.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
  OR t.before_state IS DISTINCT FROM recipe_effect_resource_state(resource)
  OR resource.execution_generation IS DISTINCT FROM (SELECT generation FROM recipe_effect_attempts WHERE id=closure.attempt_id)
  OR resource.removal_progress IN ('retained','deleted') OR resource.removal_policy<>'retain'
  OR resource.resource_id IS DISTINCT FROM closure.volume_id OR resource.ownership<>'owned' OR resource.resource_kind<>'volume'
  OR t.verification_id IS NOT NULL OR t.partial_retention_receipt_id IS NOT NULL
  OR t.operation_id=closure.attempt_id
  OR (t.command_id=closure.command_id AND ROW(t.operation_id,t.fingerprint,t.before_deployment_version,(t.before_state->>'version')::bigint)
      IS DISTINCT FROM ROW(closure.operation_id,closure.fingerprint,closure.expected_deployment_version,closure.expected_resource_version))
  OR (t.command_id<>closure.command_id AND (t.operation_id=closure.operation_id
      OR NOT EXISTS(SELECT 1 FROM recipe_owned_ready_remove_admissions admission
         WHERE admission.command_id=t.command_id AND admission.closure_id=closure.id
           AND admission.expected_version=t.before_deployment_version)))
  OR EXISTS(SELECT 1 FROM recipe_effect_attempts WHERE id=t.operation_id)
  OR EXISTS(SELECT 1 FROM recipe_effect_verifications WHERE verification_attempt_id=t.operation_id)
  OR EXISTS(SELECT 1 FROM recipe_execution_transitions WHERE operation_id=t.operation_id)
  OR NOT EXISTS(SELECT 1 FROM recipe_owned_ready_retentions retained
    WHERE retained.closure_id=closure.id AND retained.fact_id=fact.id AND retained.transition_id=t.id) THEN
  RAISE EXCEPTION 'Ready retention requires own current Remove, exact original closure and quiescent positive fact' USING ERRCODE='23000';
 END IF;
 expected:=t.before_state||jsonb_build_object('install','ready','removal','retained','active_attempt',NULL,
   'diagnostic',NULL,'last_action','retain','version',resource.version+1);
 expected_lifecycle:=CASE WHEN EXISTS(SELECT 1 FROM recipe_deployment_resources other
   WHERE other.deployment_id=t.deployment_id AND other.resource_name<>t.resource_name AND other.active_attempt_id IS NOT NULL
     AND (other.install_progress IN ('failed','recovery_required') OR other.removal_progress='recovery_required'))
     THEN 'recovery_required' ELSE 'removing' END;
 IF t.after_state IS DISTINCT FROM expected OR t.after_lifecycle<>expected_lifecycle THEN
  RAISE EXCEPTION 'ReadyRetention has distinct physical Ready/current Remove semantics; original Install is not completed' USING ERRCODE='23000';
 END IF;
END $$;
CREATE FUNCTION guard_recipe_ready_retention_link() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE closure volume_owned_ready_retention_closures%ROWTYPE;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'Ready retained disposition is immutable' USING ERRCODE='23000'; END IF;
 SELECT * INTO closure FROM volume_owned_ready_retention_closures WHERE id=NEW.closure_id;
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app'
   OR check_permission('user',hephaestus_actor_id(),'can_manage','project',closure.project_id::text) IS DISTINCT FROM 1
   OR check_permission('user',hephaestus_actor_id(),'can_manage','state_volume',closure.volume_id::text) IS DISTINCT FROM 1
   OR NOT EXISTS(SELECT 1 FROM users WHERE id::text=hephaestus_actor_id() AND status='active')
   OR ready_retention_quiescent(closure.id) IS NOT TRUE THEN
  RAISE EXCEPTION 'current manager and permanently closed quiescent Ready birth required' USING ERRCODE='42501';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER recipe_ready_retention_link BEFORE INSERT OR UPDATE OR DELETE ON recipe_owned_ready_retentions
 FOR EACH ROW EXECUTE FUNCTION guard_recipe_ready_retention_link();
CREATE FUNCTION validate_recipe_ready_retention_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM recipe_execution_transitions transition
  JOIN volume_owned_ready_retention_closures closure ON closure.id=NEW.closure_id
  JOIN volume_owned_ready_retention_facts fact ON fact.id=NEW.fact_id AND fact.closure_id=closure.id
  JOIN recipe_deployment_resources resource ON resource.deployment_id=closure.deployment_id AND resource.resource_name=closure.resource_name
  JOIN recipe_deployments deployment ON deployment.id=resource.deployment_id
  WHERE transition.id=NEW.transition_id AND transition.kind='creation_retained_ready'
    AND transition.ready_retention_fact_id=fact.id AND transition.attempt_id=closure.attempt_id
    AND transition.deployment_id=closure.deployment_id AND transition.project_id=closure.project_id
    AND transition.resource_name=closure.resource_name AND transition.after_state=recipe_effect_resource_state(resource)
    AND transition.after_deployment_version=deployment.version AND transition.after_lifecycle=deployment.lifecycle
    AND resource.install_progress='ready' AND resource.removal_progress='retained' AND resource.active_attempt_id IS NULL
    AND EXISTS(SELECT 1 FROM authorization_audit_events audit WHERE audit.actor_id=transition.actor_id
      AND audit.request_id=transition.request_id AND audit.object_type='state_volume' AND audit.object_id=closure.volume_id
      AND audit.permission='can_manage' AND audit.decision='allow')) THEN
  RAISE EXCEPTION 'ReadyRetention must atomically apply exact disposition/transition/progress/audits' USING ERRCODE='23000';
 END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER recipe_ready_retention_commit AFTER INSERT ON recipe_owned_ready_retentions
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_recipe_ready_retention_commit();

CREATE FUNCTION guard_recipe_ready_reopening() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE closure volume_owned_ready_retention_closures%ROWTYPE; claim recipe_effect_attempts%ROWTYPE;
 resource recipe_deployment_resources%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'Ready reopening is immutable' USING ERRCODE='23000'; END IF;
 SELECT * INTO closure FROM volume_owned_ready_retention_closures WHERE id=NEW.closure_id;
 PERFORM reserve_ready_retention_deployment(NEW.deployment_id);
 PERFORM 1 FROM projects WHERE id=NEW.project_id FOR UPDATE;
 PERFORM 1 FROM recipe_deployments WHERE id=NEW.deployment_id FOR UPDATE;
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=NEW.deployment_id AND resource_name=NEW.resource_name FOR UPDATE;
 PERFORM lock_ready_retention_consumers(closure.volume_id);
 SELECT * INTO claim FROM recipe_effect_attempts WHERE id=NEW.attempt_id;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=NEW.command_id;
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app'
  OR NEW.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
  OR NEW.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
  OR NOT EXISTS(SELECT 1 FROM users WHERE id=NEW.actor_id AND status='active')
  OR check_permission('user',hephaestus_actor_id(),'can_manage','state_volume',closure.volume_id::text) IS DISTINCT FROM 1
  OR recipe_effect_authorized(command) IS NOT TRUE OR command.operation<>'install'
  OR recipe_cleanup_started(NEW.deployment_id) OR NEW.project_id IS DISTINCT FROM closure.project_id
  OR ready_retention_quiescent(closure.id) IS NOT TRUE OR recipe_owned_first_birth_completed(closure.volume_id) IS NOT TRUE
  -- This initial bounded reopening accepts completed106 birth. Unfinished Ready
  -- retention keeps its distinct full positive fact but remains held for reuse.
  OR claim.action IS DISTINCT FROM 'verify_external' OR claim.command_id IS DISTINCT FROM NEW.command_id
  OR claim.deployment_id IS DISTINCT FROM NEW.deployment_id OR claim.project_id IS DISTINCT FROM NEW.project_id
  OR claim.resource_name IS DISTINCT FROM NEW.resource_name OR claim.claim_json IS DISTINCT FROM NEW.claim_json
  OR resource.resource_id IS DISTINCT FROM closure.volume_id OR resource.resource_kind<>'volume' OR resource.ownership<>'external'
  OR resource.active_attempt_id IS DISTINCT FROM claim.id OR resource.execution_generation IS DISTINCT FROM claim.generation
  OR resource.input_hash IS DISTINCT FROM claim.input_hash OR resource.plan_json->'identity' IS DISTINCT FROM claim.claim_json->'identity'
  OR (resource.plan_json->'intent'->>'capacity_bytes')::bigint IS DISTINCT FROM
    (SELECT capacity_bytes FROM agent_instance_state_volumes WHERE id=closure.volume_id)
  OR NOT EXISTS(SELECT 1 FROM recipe_deployment_execution_preparations whole
    WHERE whole.deployment_id=NEW.deployment_id AND whole.project_id=NEW.project_id AND whole.command_id=NEW.command_id
      AND whole.version=1 AND whole.profile='runtime_named_v1')
  OR NOT EXISTS(SELECT 1 FROM application_events event WHERE event.id=NEW.event_id
    AND event.occurrence_id=NEW.id AND event.actor_id=NEW.actor_id AND event.request_id=NEW.request_id
    AND event.scope_kind='project' AND event.scope_id=NEW.project_id AND event.event_type='project.changed')
  OR NOT EXISTS(SELECT 1 FROM authorization_audit_events audit WHERE audit.actor_id=NEW.actor_id
    AND audit.request_id=NEW.request_id AND audit.object_type='state_volume' AND audit.object_id=closure.volume_id
    AND audit.permission='can_manage' AND audit.decision='allow') THEN
  RAISE EXCEPTION 'explicit reuse needs current manager, fresh exact external Install/Source/preparation and retained completed birth' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER recipe_ready_reopening_guard BEFORE INSERT OR UPDATE OR DELETE ON recipe_owned_ready_reopenings
 FOR EACH ROW EXECUTE FUNCTION guard_recipe_ready_reopening();
CREATE FUNCTION apply_recipe_ready_reopening() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 UPDATE agent_instance_state_volumes SET ready_retention_reopen_id=NEW.id
  WHERE ready_retention_closure_id=NEW.closure_id AND ready_retention_reopen_id IS NULL;
 IF NOT FOUND THEN RAISE EXCEPTION 'reopening head conflicts' USING ERRCODE='23000'; END IF;
 RETURN NULL;
END $$;
CREATE TRIGGER recipe_ready_reopening_head AFTER INSERT ON recipe_owned_ready_reopenings
 FOR EACH ROW EXECUTE FUNCTION apply_recipe_ready_reopening();

-- Preserve every previous transition branch verbatim; dispatch only the new
-- distinctly proven ReadyRetention branch through its dedicated validator.
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
       AND EXISTS(SELECT 1 FROM recipe_deployment_commands original WHERE original.id=attempt.command_id AND original.operation='install')))
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

ALTER TABLE volume_owned_ready_retention_closures ENABLE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_ready_retention_closures FORCE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_ready_retention_facts ENABLE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_ready_retention_facts FORCE ROW LEVEL SECURITY;
ALTER TABLE recipe_owned_ready_retentions ENABLE ROW LEVEL SECURITY;
ALTER TABLE recipe_owned_ready_retentions FORCE ROW LEVEL SECURITY;
ALTER TABLE recipe_owned_ready_reopenings ENABLE ROW LEVEL SECURITY;
ALTER TABLE recipe_owned_ready_reopenings FORCE ROW LEVEL SECURITY;
GRANT SELECT,INSERT ON volume_owned_ready_retention_closures,recipe_owned_ready_retentions,recipe_owned_ready_reopenings TO hephaestus_app;
GRANT SELECT ON volume_owned_ready_retention_facts TO hephaestus_app;
GRANT SELECT ON volume_owned_ready_retention_closures,recipe_owned_ready_retentions,recipe_owned_ready_reopenings TO hephaestus_worker;
GRANT SELECT,INSERT ON volume_owned_ready_retention_facts TO hephaestus_worker;
CREATE POLICY ready_closure_actor_read ON volume_owned_ready_retention_closures FOR SELECT TO hephaestus_app
 USING(check_permission('user',hephaestus_actor_id(),'can_manage','project',project_id::text)=1);
CREATE POLICY ready_closure_actor_insert ON volume_owned_ready_retention_closures FOR INSERT TO hephaestus_app
 WITH CHECK(ready_retention_manager(command_id,volume_id));
CREATE POLICY ready_closure_worker ON volume_owned_ready_retention_closures FOR SELECT TO hephaestus_worker USING(true);
CREATE POLICY ready_fact_worker ON volume_owned_ready_retention_facts TO hephaestus_worker USING(true) WITH CHECK(true);
CREATE POLICY ready_fact_actor ON volume_owned_ready_retention_facts FOR SELECT TO hephaestus_app USING(EXISTS(
 SELECT 1 FROM volume_owned_ready_retention_closures closure WHERE closure.id=closure_id
 AND check_permission('user',hephaestus_actor_id(),'can_manage','project',closure.project_id::text)=1));
CREATE POLICY ready_disposition_worker ON recipe_owned_ready_retentions FOR SELECT TO hephaestus_worker USING(true);
CREATE POLICY ready_disposition_actor ON recipe_owned_ready_retentions TO hephaestus_app USING(EXISTS(
 SELECT 1 FROM volume_owned_ready_retention_closures closure WHERE closure.id=closure_id
 AND check_permission('user',hephaestus_actor_id(),'can_manage','project',closure.project_id::text)=1)) WITH CHECK(EXISTS(
 SELECT 1 FROM volume_owned_ready_retention_closures closure WHERE closure.id=closure_id
 AND check_permission('user',hephaestus_actor_id(),'can_manage','project',closure.project_id::text)=1));
CREATE POLICY ready_reopening_worker ON recipe_owned_ready_reopenings FOR SELECT TO hephaestus_worker USING(true);
CREATE POLICY ready_reopening_actor ON recipe_owned_ready_reopenings TO hephaestus_app
 USING(check_permission('user',hephaestus_actor_id(),'can_manage','project',project_id::text)=1)
 WITH CHECK(check_permission('user',hephaestus_actor_id(),'can_manage','project',project_id::text)=1);

-- Explicit least-privilege function surface; no PUBLIC access to internal history.
REVOKE ALL ON FUNCTION ready_retention_original_matches(uuid),ready_retention_manager(uuid,uuid),
 reserve_ready_retention_deployment(uuid),lock_ready_retention_consumers(uuid),
 ready_retention_run_terminal(runs,text,text),ready_retention_quiescent(uuid),
 assert_owned_ready_retention(uuid,uuid,bigint,bytea,text,text),
 ready_retention_lease_reopening_matches(uuid,uuid,uuid),assert_ready_retention_transition(recipe_execution_transitions)
 FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION reserve_ready_retention_deployment(uuid),ready_retention_manager(uuid,uuid),
 lock_ready_retention_consumers(uuid),ready_retention_quiescent(uuid),assert_ready_retention_transition(recipe_execution_transitions)
 TO hephaestus_app;
GRANT EXECUTE ON FUNCTION assert_owned_ready_retention(uuid,uuid,bigint,bytea,text,text) TO hephaestus_worker;
REVOKE ALL ON FUNCTION guard_ready_retention_closure(),apply_ready_retention_closure(),guard_ready_retention_volume_head(),
 guard_owned_ready_retention_fact(),reject_closed_ready_birth_writes(),reject_closed_ready_recipe_writes(),guard_closed_ready_birth_progress(),
 guard_ready_retention_lease(),guard_recipe_ready_retention_link(),validate_recipe_ready_retention_commit(),
 guard_recipe_ready_reopening(),apply_recipe_ready_reopening() FROM PUBLIC;

ALTER TABLE recipe_owned_ready_remove_admissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE recipe_owned_ready_remove_admissions FORCE ROW LEVEL SECURITY;
GRANT SELECT,INSERT ON recipe_owned_ready_remove_admissions TO hephaestus_app;
GRANT SELECT ON recipe_owned_ready_remove_admissions TO hephaestus_worker;
CREATE POLICY ready_remove_actor ON recipe_owned_ready_remove_admissions TO hephaestus_app
 USING(EXISTS(SELECT 1 FROM volume_owned_ready_retention_closures closure WHERE closure.id=closure_id
   AND check_permission('user',hephaestus_actor_id(),'can_manage','project',closure.project_id::text)=1))
 WITH CHECK(actor_id::text=hephaestus_actor_id());
CREATE POLICY ready_remove_worker ON recipe_owned_ready_remove_admissions FOR SELECT TO hephaestus_worker USING(true);
REVOKE ALL ON FUNCTION guard_ready_retention_remove_admission() FROM PUBLIC;
