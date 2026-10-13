-- Additive read-only observation retirement. Migration 0106 remains unchanged.
-- This is cancellation of ledger observation work, never a provider success,
-- absence, detach, ownership, or deletion assertion. No external rights are needed.
ALTER TABLE recipe_execution_transitions DROP CONSTRAINT recipe_execution_transitions_kind_check;
ALTER TABLE recipe_execution_transitions ADD CONSTRAINT recipe_execution_transitions_kind_check
 CHECK (kind IN ('claimed','completed','ambiguous','reconciled','finished','observation_retired'));

CREATE OR REPLACE FUNCTION recipe_execution_transition_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
DECLARE command recipe_deployment_commands%ROWTYPE; deployment recipe_deployments%ROWTYPE; resource recipe_deployment_resources%ROWTYPE;
    attempt recipe_effect_attempts%ROWTYPE; proof recipe_effect_verifications%ROWTYPE; expected jsonb; outcome text; diagnostic text;
    expected_lifecycle text; uncertain boolean;
BEGIN
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
 IF NEW.kind='observation_retired' THEN
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
