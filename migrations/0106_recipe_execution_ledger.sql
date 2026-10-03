-- Execution fencing is independent of provider provisioning and run lease fences.
ALTER TABLE recipe_deployment_resources ADD COLUMN execution_generation bigint NOT NULL DEFAULT 0 CHECK (execution_generation >= 0);
ALTER TABLE recipe_deployment_resources ADD COLUMN last_verified_action text CHECK (last_verified_action IN ('create','verify_external','drain','detach','retain','delete'));

CREATE TABLE recipe_effect_attempts (
    id uuid PRIMARY KEY CHECK (id <> '00000000-0000-0000-0000-000000000000'),
    command_id uuid NOT NULL,
    deployment_id uuid NOT NULL,
    project_id uuid NOT NULL,
    resource_name text NOT NULL,
    action text NOT NULL CHECK (action IN ('create','verify_external','drain','detach','retain','delete')),
    generation bigint NOT NULL CHECK (generation > 0),
    input_hash bytea NOT NULL CHECK (octet_length(input_hash)=32),
    claim_json jsonb NOT NULL,
    before_state jsonb NOT NULL,
    request_fingerprint bytea NOT NULL CHECK (octet_length(request_fingerprint)=32),
    expected_deployment_version bigint NOT NULL CHECK (expected_deployment_version >= 0),
    actor_id uuid NOT NULL REFERENCES users(id),
    request_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (deployment_id, resource_name, generation),
    UNIQUE (id, deployment_id, project_id, resource_name),
    FOREIGN KEY (command_id,deployment_id,project_id) REFERENCES recipe_deployment_commands(id,deployment_id,project_id),
    FOREIGN KEY (deployment_id,resource_name) REFERENCES recipe_deployment_resources(deployment_id,resource_name)
);
CREATE TABLE recipe_effect_verifications (
    id uuid PRIMARY KEY,
    attempt_id uuid NOT NULL,
    verification_attempt_id uuid NOT NULL CHECK (verification_attempt_id <> '00000000-0000-0000-0000-000000000000'),
    deployment_id uuid NOT NULL,
    project_id uuid NOT NULL,
    resource_name text NOT NULL,
    generation bigint NOT NULL CHECK (generation > 0),
    action text NOT NULL,
    identity_json jsonb NOT NULL,
    input_hash bytea NOT NULL CHECK (octet_length(input_hash)=32),
    outcome text NOT NULL CHECK (outcome IN ('applied','failed','not_applied','unproven')),
    diagnostic text CHECK (diagnostic IN ('provider_failure','provider_outcome_unknown','dependency_unavailable',
        'authorization_changed','drain_pending','fencing_unproven','intent_conflict')),
    recorded_by text NOT NULL DEFAULT current_user CHECK (recorded_by = 'hephaestus_worker'),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (attempt_id, verification_attempt_id),
    UNIQUE (verification_attempt_id),
    FOREIGN KEY (attempt_id,deployment_id,project_id,resource_name)
        REFERENCES recipe_effect_attempts(id,deployment_id,project_id,resource_name),
    CHECK ((outcome IN ('failed','unproven')) = (diagnostic IS NOT NULL))
);
CREATE TABLE recipe_execution_transitions (
    id uuid PRIMARY KEY,
    command_id uuid NOT NULL,
    attempt_id uuid REFERENCES recipe_effect_attempts(id),
    operation_id uuid NOT NULL CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'),
    deployment_id uuid NOT NULL,
    project_id uuid NOT NULL,
    resource_name text,
    kind text NOT NULL CHECK (kind IN ('claimed','completed','ambiguous','reconciled','finished')),
    fingerprint bytea NOT NULL CHECK (octet_length(fingerprint)=32),
    actor_id uuid NOT NULL REFERENCES users(id),
    request_id uuid NOT NULL,
    before_state jsonb,
    after_state jsonb,
    before_deployment_version bigint NOT NULL CHECK (before_deployment_version >= 0),
    after_deployment_version bigint NOT NULL CHECK (after_deployment_version = before_deployment_version + 1),
    before_lifecycle text NOT NULL,
    after_lifecycle text NOT NULL,
    verification_id uuid REFERENCES recipe_effect_verifications(id),
    event_id uuid NOT NULL,
    event_cursor bigint NOT NULL CHECK (event_cursor>0),
    event_aggregate_version bigint NOT NULL CHECK (event_aggregate_version>0),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (kind, operation_id),
    UNIQUE (deployment_id, after_deployment_version),
    FOREIGN KEY (command_id,deployment_id,project_id) REFERENCES recipe_deployment_commands(id,deployment_id,project_id),
    CHECK ((kind = 'finished' AND resource_name IS NULL AND attempt_id IS NULL AND before_state IS NULL AND after_state IS NULL)
        OR (kind <> 'finished' AND resource_name IS NOT NULL AND attempt_id IS NOT NULL AND before_state IS NOT NULL AND after_state IS NOT NULL))
);
CREATE TABLE recipe_command_results (
    command_id uuid PRIMARY KEY,
    deployment_id uuid NOT NULL,
    project_id uuid NOT NULL,
    input_hash bytea NOT NULL CHECK (octet_length(input_hash)=32),
    finish_fingerprint bytea NOT NULL CHECK (octet_length(finish_fingerprint)=32),
    lifecycle text NOT NULL CHECK (lifecycle IN ('installed','removed')),
    version bigint NOT NULL CHECK (version>0),
    event_id uuid NOT NULL,
    event_cursor bigint NOT NULL CHECK (event_cursor>0),
    event_aggregate_version bigint NOT NULL CHECK (event_aggregate_version>0),
    actor_id uuid NOT NULL REFERENCES users(id),
    request_id uuid NOT NULL,
    transition_id uuid NOT NULL UNIQUE REFERENCES recipe_execution_transitions(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (command_id,deployment_id,project_id) REFERENCES recipe_deployment_commands(id,deployment_id,project_id)
);

CREATE FUNCTION recipe_effect_resource_state(resource recipe_deployment_resources) RETURNS jsonb
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
 SELECT jsonb_build_object('install',resource.install_progress,'removal',resource.removal_progress,
    'version',resource.version,'active_attempt',resource.active_attempt_id,'diagnostic',resource.diagnostic,
    'generation',resource.execution_generation,'last_action',resource.last_verified_action)
$$;
REVOKE ALL ON FUNCTION recipe_effect_resource_state(recipe_deployment_resources) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION recipe_effect_resource_state(recipe_deployment_resources) TO hephaestus_app,hephaestus_worker;
CREATE FUNCTION recipe_effect_hash_json(bytes bytea) RETURNS jsonb LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
 SELECT to_jsonb(ARRAY(SELECT get_byte(bytes,i) FROM generate_series(0,31) i))
$$;
REVOKE ALL ON FUNCTION recipe_effect_hash_json(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION recipe_effect_hash_json(bytea) TO hephaestus_app;

-- A history of admitted removal permanently closes further installation effects.
CREATE FUNCTION recipe_cleanup_started(deployment uuid) RETURNS boolean LANGUAGE sql STABLE AS $$
 SELECT EXISTS(SELECT 1 FROM recipe_deployment_commands WHERE deployment_id=deployment AND operation='remove')
$$;
REVOKE ALL ON FUNCTION recipe_cleanup_started(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION recipe_cleanup_started(uuid) TO hephaestus_app;

CREATE FUNCTION recipe_effect_authorized(command recipe_deployment_commands) RETURNS boolean
LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
DECLARE plan recipe_deployment_resources%ROWTYPE; owner uuid; object_kind text;
BEGIN
 IF command.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
    OR check_permission('user',hephaestus_actor_id(),'can_manage','project',command.project_id::text)<>1 THEN RETURN false; END IF;
 FOR plan IN SELECT * FROM recipe_deployment_resources WHERE deployment_id=command.deployment_id LOOP
    IF plan.ownership='owned' THEN
        object_kind:=CASE WHEN plan.resource_kind='volume' THEN 'state_volume' ELSE 'agent_instance' END;
        owner:=recipe_owned_resource_project(object_kind,plan.resource_id);
        IF owner IS NOT NULL AND (owner<>command.project_id OR check_permission('user',hephaestus_actor_id(),'can_manage',object_kind,plan.resource_id::text)<>1) THEN RETURN false; END IF;
    END IF;
    IF command.operation='install' AND NOT recipe_cleanup_started(command.deployment_id) THEN
        IF plan.resource_kind='instance' AND (check_permission('user',hephaestus_actor_id(),'can_use','release_agent',
            plan.plan_json->'intent'->'release'->>'release_agent_id')<>1 OR NOT EXISTS(
                SELECT 1 FROM release_agents agent JOIN releases release ON release.id=agent.release_id
                WHERE agent.id::text=plan.plan_json->'intent'->'release'->>'release_agent_id'
                  AND release.id::text=plan.plan_json->'intent'->'release'->>'release_id' AND release.state='published')) THEN RETURN false; END IF;
        IF plan.ownership='external' THEN
            IF check_permission('user',hephaestus_actor_id(),'can_read','state_volume',plan.resource_id::text)<>1
               OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes volume WHERE volume.id=plan.resource_id
                  AND volume.project_id=command.project_id AND volume.state IN ('ready','attached')
                  AND volume.capacity_bytes=(plan.plan_json->'intent'->>'capacity_bytes')::bigint) THEN RETURN false; END IF;
            IF EXISTS(SELECT 1 FROM recipe_deployment_resources instance,
                jsonb_array_elements(instance.plan_json->'intent'->'volume_bindings') binding
                WHERE instance.deployment_id=command.deployment_id AND instance.resource_kind='instance'
                  AND binding->>'resource'=plan.resource_name)
               AND (check_permission('user',hephaestus_actor_id(),'can_attach','state_volume',plan.resource_id::text)<>1
                 OR check_permission('user',hephaestus_actor_id(),'can_grant_agent_capability','state_volume',plan.resource_id::text)<>1) THEN RETURN false; END IF;
        END IF;
    ELSIF plan.ownership='owned' THEN
        object_kind:=CASE WHEN plan.resource_kind='volume' THEN 'state_volume' ELSE 'agent_instance' END;
        owner:=recipe_owned_resource_project(object_kind,plan.resource_id);
        IF owner IS NOT NULL AND (owner<>command.project_id OR check_permission('user',hephaestus_actor_id(),'can_manage',object_kind,plan.resource_id::text)<>1) THEN RETURN false; END IF;
    END IF;
 END LOOP;
 RETURN true;
END $$;
REVOKE ALL ON FUNCTION recipe_effect_authorized(recipe_deployment_commands) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION recipe_effect_authorized(recipe_deployment_commands) TO hephaestus_app;

CREATE FUNCTION recipe_effect_attempt_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
DECLARE resource recipe_deployment_resources%ROWTYPE; command recipe_deployment_commands%ROWTYPE; deployment recipe_deployments%ROWTYPE; legal boolean;
BEGIN
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=NEW.deployment_id AND resource_name=NEW.resource_name;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=NEW.command_id;
 SELECT * INTO deployment FROM recipe_deployments WHERE id=NEW.deployment_id;
 IF NOT recipe_effect_authorized(command) OR resource.active_attempt_id IS NOT NULL
    OR NEW.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
    OR NEW.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
    OR NEW.before_state IS DISTINCT FROM recipe_effect_resource_state(resource)
    OR NEW.generation<>resource.execution_generation+1 OR NEW.input_hash<>resource.input_hash
    OR NEW.expected_deployment_version<>deployment.version OR deployment.lifecycle IN ('installed','removed')
    OR EXISTS(SELECT 1 FROM recipe_command_results WHERE command_id=NEW.command_id)
    OR NEW.claim_json IS DISTINCT FROM jsonb_build_object('command_id',NEW.command_id,'deployment_id',NEW.deployment_id,
       'resource',NEW.resource_name,'identity',resource.plan_json->'identity','input_hash',recipe_effect_hash_json(resource.input_hash),
       'resource_version',resource.version+1,'generation',NEW.generation,'action',NEW.action,'attempt_id',NEW.id,
       'actor_id',NEW.actor_id,'request_id',NEW.request_id) THEN
    RAISE EXCEPTION 'invalid effect claim identity, authority, or fence' USING ERRCODE='integrity_constraint_violation';
 END IF;
 legal:=CASE
    WHEN command.operation='install' AND NOT recipe_cleanup_started(NEW.deployment_id) THEN
       resource.install_progress IN ('pending','failed') AND
         ((resource.ownership='owned' AND NEW.action='create') OR (resource.ownership='external' AND NEW.action='verify_external'))
    WHEN command.operation='remove' AND recipe_cleanup_started(NEW.deployment_id) AND resource.ownership='owned' THEN
       (NEW.action='drain' AND resource.resource_kind='instance' AND resource.removal_progress='pending')
       OR (NEW.action='detach' AND ((resource.resource_kind='volume' AND resource.removal_progress='pending')
          OR (resource.resource_kind='instance' AND resource.removal_progress='draining' AND resource.last_verified_action='drain')))
       OR (NEW.action=resource.removal_policy AND resource.removal_progress='detached' AND resource.last_verified_action='detach')
    ELSE false END;
 IF NOT COALESCE(legal,false) THEN RAISE EXCEPTION 'illegal resource action' USING ERRCODE='integrity_constraint_violation'; END IF;
 IF command.operation='install' AND EXISTS(SELECT 1 FROM jsonb_array_elements_text(resource.plan_json->'intent'->'dependencies') dependency
    LEFT JOIN recipe_deployment_resources target ON target.deployment_id=NEW.deployment_id AND target.resource_name=dependency
    WHERE target.resource_name IS NULL OR target.install_progress<>'ready' OR target.active_attempt_id IS NOT NULL) THEN
    RAISE EXCEPTION 'install dependency is unavailable' USING ERRCODE='integrity_constraint_violation';
 END IF;
 IF command.operation='remove' AND EXISTS(SELECT 1 FROM recipe_deployment_resources target
    WHERE target.deployment_id=NEW.deployment_id AND target.plan_json->'intent'->'dependencies' ? NEW.resource_name
      AND (target.removal_progress NOT IN ('retained','deleted') OR target.active_attempt_id IS NOT NULL)) THEN
    RAISE EXCEPTION 'cleanup dependent has not finished' USING ERRCODE='integrity_constraint_violation';
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION recipe_effect_attempt_guard() FROM PUBLIC;
CREATE TRIGGER recipe_effect_attempt_guard BEFORE INSERT ON recipe_effect_attempts FOR EACH ROW EXECUTE FUNCTION recipe_effect_attempt_guard();

CREATE FUNCTION recipe_effect_verification_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
DECLARE attempt recipe_effect_attempts%ROWTYPE; resource recipe_deployment_resources%ROWTYPE;
BEGIN
 IF current_user<>'hephaestus_worker' THEN RAISE EXCEPTION 'verification requires trusted worker' USING ERRCODE='insufficient_privilege'; END IF;
 SELECT * INTO attempt FROM recipe_effect_attempts WHERE id=NEW.attempt_id;
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=attempt.deployment_id AND resource_name=attempt.resource_name;
 IF NEW.recorded_by<>'hephaestus_worker' OR NEW.generation<>attempt.generation OR NEW.action<>attempt.action
    OR NEW.identity_json IS DISTINCT FROM attempt.claim_json->'identity' OR NEW.input_hash<>attempt.input_hash
    OR resource.active_attempt_id IS DISTINCT FROM attempt.id OR resource.execution_generation<>attempt.generation
    OR (NEW.verification_attempt_id<>attempt.id AND EXISTS(SELECT 1 FROM recipe_effect_attempts WHERE id=NEW.verification_attempt_id)) THEN
    RAISE EXCEPTION 'verification differs from active exact claim' USING ERRCODE='integrity_constraint_violation';
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION recipe_effect_verification_guard() FROM PUBLIC;
CREATE TRIGGER recipe_effect_verification_guard BEFORE INSERT ON recipe_effect_verifications FOR EACH ROW EXECUTE FUNCTION recipe_effect_verification_guard();

CREATE FUNCTION recipe_effect_expected_state(before_state jsonb, original_state jsonb, action text, phase text, outcome text, diagnostic text, attempt uuid) RETURNS jsonb
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,public AS $$
DECLARE expected jsonb:=before_state; installation boolean:=action IN ('create','verify_external');
BEGIN
 IF phase='claimed' THEN
    expected:=expected||jsonb_build_object('active_attempt',attempt,'generation',(before_state->>'generation')::bigint+1,'diagnostic',NULL);
    IF installation THEN expected:=expected||jsonb_build_object('install','creating');
    ELSIF action IN ('drain','detach') THEN expected:=expected||jsonb_build_object('removal','draining'); END IF;
 ELSIF outcome='applied' THEN
    expected:=expected||jsonb_build_object('active_attempt',NULL,'diagnostic',NULL,'last_action',action);
    IF installation THEN expected:=expected||jsonb_build_object('install','ready');
    ELSE expected:=expected||jsonb_build_object('removal',CASE action WHEN 'drain' THEN 'draining' WHEN 'detach' THEN 'detached' WHEN 'retain' THEN 'retained' WHEN 'delete' THEN 'deleted' END); END IF;
 ELSIF outcome='not_applied' AND phase='reconciled' THEN
    expected:=original_state||jsonb_build_object('active_attempt',NULL,'generation',(before_state->>'generation')::bigint,'diagnostic','provider_failure');
    IF installation THEN expected:=expected||jsonb_build_object('install','failed'); END IF;
 ELSE
    expected:=expected||jsonb_build_object('diagnostic',diagnostic);
    IF installation THEN expected:=expected||jsonb_build_object('install',CASE WHEN outcome='failed' THEN 'failed' ELSE 'recovery_required' END);
    ELSE expected:=expected||jsonb_build_object('removal','recovery_required'); END IF;
 END IF;
 RETURN expected||jsonb_build_object('version',(before_state->>'version')::bigint+1);
END $$;
REVOKE ALL ON FUNCTION recipe_effect_expected_state(jsonb,jsonb,text,text,text,text,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION recipe_effect_expected_state(jsonb,jsonb,text,text,text,text,uuid) TO hephaestus_app;

CREATE FUNCTION recipe_execution_transition_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
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
REVOKE ALL ON FUNCTION recipe_execution_transition_guard() FROM PUBLIC;
CREATE TRIGGER recipe_execution_transition_guard BEFORE INSERT ON recipe_execution_transitions FOR EACH ROW EXECUTE FUNCTION recipe_execution_transition_guard();

CREATE OR REPLACE FUNCTION recipe_resource_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
DECLARE admitted_count integer;
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'resource provenance cannot be erased' USING ERRCODE='integrity_constraint_violation'; END IF;
 IF TG_OP='INSERT' THEN
    SELECT resource_count INTO admitted_count FROM recipe_deployments WHERE id=NEW.deployment_id;
    IF (SELECT count(*) FROM recipe_deployment_resources WHERE deployment_id=NEW.deployment_id)>=admitted_count
       OR NEW.install_progress<>'pending' OR NEW.version<>0 OR NEW.active_attempt_id IS NOT NULL OR NEW.diagnostic IS NOT NULL
       OR NEW.execution_generation<>0 OR NEW.last_verified_action IS NOT NULL
       OR NEW.removal_progress<>(CASE WHEN NEW.ownership='external' THEN 'retained' ELSE 'pending' END) THEN
       RAISE EXCEPTION 'invalid initial resource progress' USING ERRCODE='integrity_constraint_violation';
    END IF;
 ELSIF (to_jsonb(NEW)-ARRAY['install_progress','removal_progress','version','active_attempt_id','diagnostic','execution_generation','last_verified_action'])
       IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['install_progress','removal_progress','version','active_attempt_id','diagnostic','execution_generation','last_verified_action'])
       OR NEW.version<>OLD.version+1
       OR NOT EXISTS(SELECT 1 FROM recipe_execution_transitions transition WHERE transition.deployment_id=NEW.deployment_id
          AND transition.resource_name=NEW.resource_name AND transition.before_state=recipe_effect_resource_state(OLD)
          AND transition.after_state=recipe_effect_resource_state(NEW) AND transition.actor_id::text=hephaestus_actor_id()
          AND transition.request_id::text=current_setting('hephaestus.request_id',true)) THEN
    RAISE EXCEPTION 'resource update requires exact journaled proof' USING ERRCODE='integrity_constraint_violation';
 END IF;
 RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION recipe_deployment_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'deployment tombstones cannot be erased' USING ERRCODE='integrity_constraint_violation'; END IF;
 IF TG_OP='INSERT' THEN
    IF NEW.lifecycle<>'installing' OR NEW.version<>0 OR NOT EXISTS(SELECT 1 FROM recipe_definitions definition
       WHERE definition.project_id=NEW.project_id AND definition.recipe_id=NEW.recipe_id AND definition.recipe_version=NEW.recipe_version
         AND definition.declaration_hash=NEW.declaration_hash) THEN
       RAISE EXCEPTION 'deployment must begin at pending admission' USING ERRCODE='integrity_constraint_violation';
    END IF;
 ELSE
    IF (to_jsonb(NEW)-ARRAY['lifecycle','version']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['lifecycle','version'])
       OR NEW.version<>OLD.version+1 OR OLD.lifecycle='removed' THEN
       RAISE EXCEPTION 'immutable deployment or stale version' USING ERRCODE='integrity_constraint_violation';
    END IF;
    IF NOT EXISTS(SELECT 1 FROM recipe_execution_transitions transition WHERE transition.deployment_id=NEW.id
       AND transition.before_deployment_version=OLD.version AND transition.after_deployment_version=NEW.version
       AND transition.before_lifecycle=OLD.lifecycle AND transition.after_lifecycle=NEW.lifecycle
       AND transition.actor_id::text=hephaestus_actor_id() AND transition.request_id::text=current_setting('hephaestus.request_id',true))
       AND NOT (NEW.lifecycle='removing' AND OLD.lifecycle IN ('installing','installed','recovery_required')) THEN
       RAISE EXCEPTION 'deployment progress requires an execution transition' USING ERRCODE='integrity_constraint_violation';
    END IF;
 END IF;
 RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION recipe_deployment_commit_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
BEGIN
 IF NOT (EXISTS(SELECT 1 FROM recipe_deployment_commands command WHERE command.deployment_id=NEW.id
       AND command.project_id=NEW.project_id AND command.receipt_version=NEW.version AND command.receipt_lifecycle=NEW.lifecycle
       AND command.actor_id::text=hephaestus_actor_id() AND command.first_request_id::text=current_setting('hephaestus.request_id',true))
    OR EXISTS(SELECT 1 FROM recipe_execution_transitions transition WHERE transition.deployment_id=NEW.id
       AND transition.after_deployment_version=NEW.version AND transition.after_lifecycle=NEW.lifecycle
       AND transition.actor_id::text=hephaestus_actor_id() AND transition.request_id::text=current_setting('hephaestus.request_id',true)))
    OR (SELECT count(*) FROM recipe_deployment_resources WHERE deployment_id=NEW.id)<>NEW.resource_count
    OR NOT EXISTS(SELECT 1 FROM authorization_audit_events audit WHERE audit.actor_id::text=hephaestus_actor_id()
       AND audit.permission='can_manage' AND audit.object_type='project' AND audit.object_id=NEW.project_id
       AND audit.decision='allow' AND audit.request_id::text=current_setting('hephaestus.request_id',true)) THEN
    RAISE EXCEPTION 'deployment commit lacks exact audited ledger receipt' USING ERRCODE='integrity_constraint_violation';
 END IF;
 RETURN NULL;
END $$;

CREATE FUNCTION recipe_execution_commit_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM recipe_deployments WHERE id=NEW.deployment_id
       AND version=NEW.after_deployment_version AND lifecycle=NEW.after_lifecycle)
    OR (NEW.resource_name IS NOT NULL AND NOT EXISTS(SELECT 1 FROM recipe_deployment_resources resource
       WHERE resource.deployment_id=NEW.deployment_id AND resource.resource_name=NEW.resource_name
         AND recipe_effect_resource_state(resource)=NEW.after_state))
    OR (NEW.kind='finished' AND NOT EXISTS(SELECT 1 FROM recipe_command_results WHERE transition_id=NEW.id)) THEN
    RAISE EXCEPTION 'execution journal was not applied atomically' USING ERRCODE='integrity_constraint_violation';
 END IF;
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION recipe_execution_commit_guard() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER recipe_execution_commit_guard AFTER INSERT ON recipe_execution_transitions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION recipe_execution_commit_guard();
CREATE FUNCTION recipe_effect_claim_commit_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM recipe_execution_transitions WHERE attempt_id=NEW.id AND kind='claimed') THEN
    RAISE EXCEPTION 'attempt lacks committed claim transition' USING ERRCODE='integrity_constraint_violation';
 END IF;
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION recipe_effect_claim_commit_guard() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER recipe_effect_claim_commit_guard AFTER INSERT ON recipe_effect_attempts DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION recipe_effect_claim_commit_guard();

CREATE FUNCTION recipe_command_result_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM recipe_execution_transitions transition JOIN recipe_deployment_commands command ON command.id=transition.command_id
       WHERE transition.id=NEW.transition_id AND transition.kind='finished' AND command.id=NEW.command_id
         AND transition.deployment_id=NEW.deployment_id AND transition.project_id=NEW.project_id
         AND transition.after_lifecycle=NEW.lifecycle AND transition.after_deployment_version=NEW.version
         AND transition.fingerprint=NEW.finish_fingerprint AND command.input_hash=NEW.input_hash
         AND transition.event_id=NEW.event_id AND transition.event_cursor=NEW.event_cursor
         AND transition.event_aggregate_version=NEW.event_aggregate_version
         AND transition.actor_id=NEW.actor_id AND transition.request_id=NEW.request_id) THEN
    RAISE EXCEPTION 'terminal result differs from committed exact transition' USING ERRCODE='integrity_constraint_violation';
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION recipe_command_result_guard() FROM PUBLIC;
CREATE TRIGGER recipe_command_result_guard BEFORE INSERT ON recipe_command_results FOR EACH ROW EXECUTE FUNCTION recipe_command_result_guard();

DO $$
DECLARE ledger_table text;
BEGIN
 FOREACH ledger_table IN ARRAY ARRAY['recipe_effect_attempts','recipe_execution_transitions','recipe_command_results','recipe_effect_verifications'] LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY',ledger_table);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY',ledger_table);
    EXECUTE format('GRANT SELECT ON %I TO hephaestus_app,hephaestus_worker',ledger_table);
    EXECUTE format('CREATE POLICY recipe_execution_read ON %I FOR SELECT TO hephaestus_app USING
       (check_permission(''user'',hephaestus_actor_id(),''can_manage'',''project'',project_id::text)=1)',ledger_table);
    EXECUTE format('CREATE POLICY recipe_execution_worker_read ON %I FOR SELECT TO hephaestus_worker USING (true)',ledger_table);
    EXECUTE format('CREATE TRIGGER recipe_execution_immutable BEFORE UPDATE OR DELETE ON %I FOR EACH ROW EXECUTE FUNCTION recipe_ledger_immutable()',ledger_table);
    IF ledger_table<>'recipe_effect_verifications' THEN
       EXECUTE format('GRANT INSERT ON %I TO hephaestus_app',ledger_table);
       EXECUTE format('CREATE POLICY recipe_execution_insert ON %I FOR INSERT TO hephaestus_app WITH CHECK
          (actor_id::text=hephaestus_actor_id() AND request_id::text=current_setting(''hephaestus.request_id'',true)
           AND check_permission(''user'',hephaestus_actor_id(),''can_manage'',''project'',project_id::text)=1)',ledger_table);
    END IF;
 END LOOP;
 FOREACH ledger_table IN ARRAY ARRAY['recipe_deployments','recipe_deployment_commands','recipe_deployment_resources'] LOOP
    EXECUTE format('GRANT SELECT ON %I TO hephaestus_worker',ledger_table);
    EXECUTE format('CREATE POLICY recipe_worker_evidence_read ON %I FOR SELECT TO hephaestus_worker USING (true)',ledger_table);
 END LOOP;
END $$;
REVOKE INSERT,UPDATE,DELETE ON recipe_effect_verifications FROM hephaestus_app;
GRANT INSERT ON recipe_effect_verifications TO hephaestus_worker;
CREATE POLICY recipe_verification_worker_insert ON recipe_effect_verifications FOR INSERT TO hephaestus_worker WITH CHECK (recorded_by='hephaestus_worker');
GRANT UPDATE ON recipe_deployment_resources TO hephaestus_app;
CREATE POLICY recipe_resource_execution_update ON recipe_deployment_resources FOR UPDATE TO hephaestus_app
USING (check_permission('user',hephaestus_actor_id(),'can_manage','project',project_id::text)=1)
WITH CHECK (check_permission('user',hephaestus_actor_id(),'can_manage','project',project_id::text)=1);
