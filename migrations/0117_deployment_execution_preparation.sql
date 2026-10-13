-- PRIVATE113 PROPOSAL: protected writer and typed preparation, no native/publication claim.
-- Follows coherent108-112. Frozen106/112 functions and histories are unchanged.
-- Rust reconstructs the bounded typed record and rederives tagged UUIDv5 IDs
-- and ReleaseCommandKey::derive before any effect. SQL does not compute UUIDv5.
-- Current configured platform/version/profile is trusted server composition:
-- future adapter rechecks exact equality before replay/new IO; SQL labels do
-- not grant source/runtime/provider authority. No named dispatch activation.

CREATE TABLE recipe_deployment_execution_preparations (
 deployment_id uuid PRIMARY KEY,
 project_id uuid NOT NULL,
 command_id uuid NOT NULL UNIQUE,
 actor_id uuid NOT NULL REFERENCES users(id),
 version integer NOT NULL CHECK(version=1),
 profile text NOT NULL CHECK(profile='runtime_named_v1'),
 authorization_model_version text NOT NULL CHECK(octet_length(authorization_model_version) BETWEEN 1 AND 256
   AND authorization_model_version=btrim(authorization_model_version) AND authorization_model_version !~ '[[:cntrl:]]'),
 intent_hash bytea NOT NULL CHECK(octet_length(intent_hash)=32),
 platform_json jsonb NOT NULL CHECK(jsonb_typeof(platform_json)='object'),
 instance_count integer NOT NULL CHECK(instance_count BETWEEN 0 AND 64),
 canonical_input bytea NOT NULL CHECK(octet_length(canonical_input) BETWEEN 1 AND 1048576),
 input_hash bytea NOT NULL CHECK(input_hash=sha256(canonical_input)),
 request_id uuid NOT NULL CHECK(request_id<>'00000000-0000-0000-0000-000000000000'),
 event_id uuid NOT NULL UNIQUE REFERENCES application_events(id),
 event_cursor bigint NOT NULL CHECK(event_cursor>0),
 event_aggregate_version bigint NOT NULL CHECK(event_aggregate_version>0),
 transaction_id bigint NOT NULL DEFAULT txid_current(),
 created_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(deployment_id,project_id,command_id),
 FOREIGN KEY(command_id,deployment_id,project_id) REFERENCES recipe_deployment_commands(id,deployment_id,project_id),
 FOREIGN KEY(deployment_id,project_id) REFERENCES recipe_deployments(id,project_id)
);

CREATE TABLE recipe_prepared_instance_imports (
 deployment_id uuid NOT NULL,
 project_id uuid NOT NULL,
 command_id uuid NOT NULL,
 resource_name text NOT NULL,
 ordinal integer NOT NULL CHECK(ordinal BETWEEN 0 AND 63),
 operation_id uuid NOT NULL UNIQUE CHECK(operation_id<>'00000000-0000-0000-0000-000000000000'),
 import_command_key bytea NOT NULL UNIQUE CHECK(octet_length(import_command_key)=32),
 instance_id uuid NOT NULL UNIQUE CHECK(instance_id<>'00000000-0000-0000-0000-000000000000'),
 revision_id uuid NOT NULL UNIQUE CHECK(revision_id<>'00000000-0000-0000-0000-000000000000'),
 release_id uuid NOT NULL REFERENCES releases(id),
 release_agent_id uuid NOT NULL,
 canonical_import bytea NOT NULL CHECK(octet_length(canonical_import) BETWEEN 1 AND 1048576),
 input_hash bytea NOT NULL CHECK(input_hash=sha256(canonical_import)),
 transaction_id bigint NOT NULL DEFAULT txid_current(),
 PRIMARY KEY(deployment_id,resource_name),
 UNIQUE(deployment_id,ordinal),
 FOREIGN KEY(deployment_id,project_id,command_id) REFERENCES recipe_deployment_execution_preparations(deployment_id,project_id,command_id),
 FOREIGN KEY(deployment_id,resource_name) REFERENCES recipe_deployment_resources(deployment_id,resource_name),
 FOREIGN KEY(release_agent_id,release_id) REFERENCES release_agents(id,release_id)
);

-- This 113-specific relation captures only IDs returned by the protected writer's
-- fresh audit INSERTs. It does not change the historical/global audit schema.
CREATE TABLE recipe_execution_preparation_audits (
 deployment_id uuid NOT NULL REFERENCES recipe_deployment_execution_preparations(deployment_id),
 permission text NOT NULL CHECK(permission IN ('can_manage','can_use','can_read','can_attach','can_grant_agent_capability')),
 object_type text NOT NULL CHECK(object_type IN ('project','release_agent','state_volume')),
 object_id uuid NOT NULL,
 audit_id uuid NOT NULL UNIQUE REFERENCES authorization_audit_events(id),
 PRIMARY KEY(deployment_id,permission,object_type,object_id)
);
CREATE FUNCTION reject_recipe_preparation_audit_rewrite() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 RAISE EXCEPTION 'preparation audit links are immutable' USING ERRCODE='23000';
END $$;
CREATE TRIGGER recipe_preparation_audit_immutable BEFORE UPDATE OR DELETE
 ON recipe_execution_preparation_audits FOR EACH ROW EXECUTE FUNCTION reject_recipe_preparation_audit_rewrite();

-- Current authority for preparation/new execution replay. Historical inspection
-- uses the manager SELECT policies below and never calls this Install helper.
-- Source/volume checks use the existing FGA vocabulary, not recorded grants.
CREATE FUNCTION recipe_execution_preparation_authorized(command recipe_deployment_commands) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE deployment recipe_deployments%ROWTYPE; resource recipe_deployment_resources%ROWTYPE;
BEGIN
 IF command.id IS NULL OR command.operation IS DISTINCT FROM 'install'
    OR command.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
    OR NOT EXISTS(SELECT 1 FROM users WHERE id=command.actor_id AND status='active')
    OR check_permission('user',hephaestus_actor_id(),'can_manage','project',command.project_id::text) IS DISTINCT FROM 1 THEN
   RETURN false;
 END IF;
 SELECT * INTO deployment FROM recipe_deployments WHERE id=command.deployment_id;
 IF deployment.id IS NULL OR deployment.project_id IS DISTINCT FROM command.project_id
    OR deployment.created_by IS DISTINCT FROM command.actor_id
    OR deployment.first_request_id IS DISTINCT FROM command.first_request_id
    OR command.receipt_version IS DISTINCT FROM 0 OR command.receipt_lifecycle IS DISTINCT FROM 'installing'
    OR command.input_hash IS DISTINCT FROM deployment.input_hash
    OR (SELECT count(*) FROM recipe_deployment_commands
        WHERE deployment_id=command.deployment_id AND operation='install')<>1
    OR recipe_cleanup_started(command.deployment_id) THEN RETURN false; END IF;
 FOR resource IN SELECT * FROM recipe_deployment_resources WHERE deployment_id=command.deployment_id LOOP
   IF resource.resource_kind='instance' THEN
     IF check_permission('user',hephaestus_actor_id(),'can_use','release_agent',
         resource.plan_json->'intent'->'release'->>'release_agent_id') IS DISTINCT FROM 1
        OR NOT EXISTS(SELECT 1 FROM release_agents agent JOIN releases release ON release.id=agent.release_id
           WHERE agent.id=(resource.plan_json->'intent'->'release'->>'release_agent_id')::uuid
             AND release.id=(resource.plan_json->'intent'->'release'->>'release_id')::uuid
             AND release.state='published') THEN RETURN false; END IF;
   ELSIF resource.ownership='external' THEN
     IF check_permission('user',hephaestus_actor_id(),'can_read','state_volume',resource.resource_id::text) IS DISTINCT FROM 1
        OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes volume WHERE volume.id=resource.resource_id
           AND volume.project_id=command.project_id AND volume.state IN ('ready','attached')
           AND volume.provisioning_state='ready'
           AND volume.capacity_bytes=(resource.plan_json->'intent'->>'capacity_bytes')::bigint) THEN RETURN false; END IF;
     IF EXISTS(SELECT 1 FROM recipe_deployment_resources instance
         CROSS JOIN LATERAL jsonb_array_elements(instance.plan_json->'intent'->'volume_bindings') binding
         WHERE instance.deployment_id=command.deployment_id AND instance.resource_kind='instance'
           AND binding->>'resource'=resource.resource_name)
        AND (check_permission('user',hephaestus_actor_id(),'can_attach','state_volume',resource.resource_id::text) IS DISTINCT FROM 1
          OR check_permission('user',hephaestus_actor_id(),'can_grant_agent_capability','state_volume',resource.resource_id::text) IS DISTINCT FROM 1) THEN
       RETURN false;
     END IF;
   END IF;
 END LOOP;
 RETURN true;
END $$;
REVOKE ALL ON FUNCTION recipe_execution_preparation_authorized(recipe_deployment_commands) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION recipe_execution_preparation_authorized(recipe_deployment_commands) TO hephaestus_app;

-- Header insertion is before EVERY new effect, including external verification.
-- No preparation backfill is allowed for historic claims or existing owned IDs.
CREATE FUNCTION guard_recipe_execution_preparation() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE command recipe_deployment_commands%ROWTYPE; deployment recipe_deployments%ROWTYPE; input jsonb;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'execution preparation is immutable' USING ERRCODE='23000'; END IF;
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app'
    OR NEW.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
    OR NEW.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
    OR NEW.transaction_id<>txid_current() THEN
   RAISE EXCEPTION 'execution preparation requires current original actor' USING ERRCODE='42501';
 END IF;
 SELECT * INTO deployment FROM recipe_deployments WHERE id=NEW.deployment_id FOR UPDATE;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=NEW.command_id;
 IF command.id IS NULL OR command.operation<>'install' OR command.actor_id<>NEW.actor_id
    OR command.deployment_id<>NEW.deployment_id OR command.project_id<>NEW.project_id
    OR deployment.created_by IS DISTINCT FROM NEW.actor_id
    OR deployment.first_request_id IS DISTINCT FROM command.first_request_id
    OR command.receipt_version IS DISTINCT FROM 0 OR command.receipt_lifecycle IS DISTINCT FROM 'installing'
    OR (SELECT count(*) FROM recipe_deployment_commands
        WHERE deployment_id=NEW.deployment_id AND operation='install')<>1
    OR deployment.project_id<>NEW.project_id OR deployment.input_hash<>NEW.intent_hash
    OR deployment.lifecycle<>'installing' OR command.input_hash<>NEW.intent_hash
    OR recipe_cleanup_started(NEW.deployment_id) OR recipe_execution_preparation_authorized(command) IS DISTINCT FROM true
    OR check_permission('user',hephaestus_actor_id(),'can_manage','project',NEW.project_id::text) IS DISTINCT FROM 1
    OR EXISTS(SELECT 1 FROM recipe_effect_attempts WHERE deployment_id=NEW.deployment_id)
    OR EXISTS(SELECT 1 FROM recipe_owned_volume_preparations WHERE deployment_id=NEW.deployment_id)
    OR EXISTS(SELECT 1 FROM recipe_deployment_resources resource WHERE resource.deployment_id=NEW.deployment_id
        AND (resource.install_progress<>'pending' OR resource.version<>0 OR resource.active_attempt_id IS NOT NULL
          OR resource.execution_generation<>0
          OR (resource.ownership='owned' AND resource.resource_kind='volume'
             AND EXISTS(SELECT 1 FROM agent_instance_state_volumes WHERE id=resource.resource_id))
          OR (resource.resource_kind='instance' AND EXISTS(SELECT 1 FROM agent_instances WHERE id=resource.resource_id)))) THEN
   RAISE EXCEPTION 'preparation must precede every effect and match live admitted intent' USING ERRCODE='23000';
 END IF;
 input:=convert_from(NEW.canonical_input,'UTF8')::jsonb;
 IF jsonb_typeof(input) IS DISTINCT FROM 'object' OR input->>'version' IS DISTINCT FROM '1'
    OR input->>'profile' IS DISTINCT FROM NEW.profile OR input->>'deployment_id' IS DISTINCT FROM NEW.deployment_id::text
    OR input->>'project_id' IS DISTINCT FROM NEW.project_id::text OR input->'intent_hash' IS DISTINCT FROM recipe_effect_hash_json(NEW.intent_hash)
    OR input->'command' IS DISTINCT FROM jsonb_build_object('id',command.id,'actor_id',command.actor_id,
       'idempotency_id',command.idempotency_id,'operation',command.operation)
    OR input->'platform' IS DISTINCT FROM NEW.platform_json
    OR jsonb_typeof(NEW.platform_json->'version') IS DISTINCT FROM 'string'
    OR octet_length(NEW.platform_json->>'version') NOT BETWEEN 1 AND 128
    OR (NEW.platform_json->>'version') IS DISTINCT FROM btrim(NEW.platform_json->>'version')
    OR (NEW.platform_json->>'version') ~ '[[:cntrl:]]'
    OR COALESCE((NEW.platform_json->'policy'->>'vcpus')::integer,0)<=0
    OR COALESCE((NEW.platform_json->'policy'->>'memory_mib')::bigint,0)<=0
    OR jsonb_typeof(input->'instances') IS DISTINCT FROM 'array'
    OR jsonb_array_length(input->'instances')<>NEW.instance_count
    OR NEW.instance_count<>(SELECT count(*) FROM recipe_deployment_resources
        WHERE deployment_id=NEW.deployment_id AND resource_kind='instance') THEN
   RAISE EXCEPTION 'preparation header must match complete typed input' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER recipe_execution_preparation_guard BEFORE INSERT OR UPDATE OR DELETE
 ON recipe_deployment_execution_preparations FOR EACH ROW EXECUTE FUNCTION guard_recipe_execution_preparation();

CREATE FUNCTION guard_recipe_prepared_import() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE header recipe_deployment_execution_preparations%ROWTYPE; resource recipe_deployment_resources%ROWTYPE;
 input jsonb; source jsonb; agent release_agents%ROWTYPE;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'prepared import is immutable' USING ERRCODE='23000'; END IF;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=NEW.deployment_id;
 SELECT * INTO resource FROM recipe_deployment_resources WHERE deployment_id=NEW.deployment_id AND resource_name=NEW.resource_name;
 SELECT * INTO agent FROM release_agents WHERE id=NEW.release_agent_id;
 input:=convert_from(NEW.canonical_import,'UTF8')::jsonb; source:=input->'source';
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app'
    OR header.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
    OR header.transaction_id<>txid_current() OR NEW.transaction_id<>txid_current()
    OR header.command_id<>NEW.command_id OR header.project_id<>NEW.project_id
    OR resource.resource_kind IS DISTINCT FROM 'instance' OR resource.ownership<>'owned'
    OR resource.resource_id<>NEW.instance_id OR resource.revision_id<>NEW.revision_id
    OR EXISTS(SELECT 1 FROM agent_instances WHERE id=NEW.instance_id)
    OR EXISTS(SELECT 1 FROM release_command_inbox WHERE command_key=NEW.import_command_key)
    OR input IS DISTINCT FROM (convert_from(header.canonical_input,'UTF8')::jsonb)->'instances'->NEW.ordinal
    OR input->>'resource' IS DISTINCT FROM NEW.resource_name OR input->>'operation_id' IS DISTINCT FROM NEW.operation_id::text
    OR input->'command_key' IS DISTINCT FROM recipe_effect_hash_json(NEW.import_command_key)
    OR input->>'instance_id' IS DISTINCT FROM NEW.instance_id::text OR input->>'revision_id' IS DISTINCT FROM NEW.revision_id::text
    OR input->>'project_id' IS DISTINCT FROM NEW.project_id::text OR input->>'name' IS DISTINCT FROM ('recipe-'||NEW.instance_id::text)
    OR input->'parameters' IS DISTINCT FROM resource.plan_json->'intent'->'parameters'
    OR source->'pin' IS DISTINCT FROM resource.plan_json->'intent'->'release'
    OR source->'pin' IS DISTINCT FROM jsonb_build_object('release_id',NEW.release_id,'release_agent_id',NEW.release_agent_id)
    OR source->'platform' IS DISTINCT FROM header.platform_json
    OR agent.id IS NULL OR agent.release_id<>NEW.release_id
    OR source->'runtime_contract_hash' IS DISTINCT FROM recipe_effect_hash_json(agent.runtime_contract_hash)
    OR source->>'image' IS DISTINCT FROM agent.runtime_contract->>'image_reference'
    OR source->'selected_policy' IS DISTINCT FROM agent.runtime_contract->'policy_ceiling'
    OR NOT EXISTS(SELECT 1 FROM releases WHERE id=NEW.release_id AND state='published')
    OR check_permission('user',hephaestus_actor_id(),'can_use','release_agent',NEW.release_agent_id::text) IS DISTINCT FROM 1
    OR jsonb_typeof(input->'volumes') IS DISTINCT FROM 'array'
    OR jsonb_array_length(input->'volumes')<>jsonb_array_length(resource.plan_json->'intent'->'volume_bindings')
    OR EXISTS(SELECT 1 FROM jsonb_array_elements(input->'volumes') selection
       GROUP BY selection->>'slot' HAVING count(*)<>1)
    OR EXISTS(SELECT 1 FROM jsonb_array_elements(input->'volumes') selection WHERE NOT EXISTS(
       SELECT 1 FROM jsonb_array_elements(resource.plan_json->'intent'->'volume_bindings') binding
       JOIN recipe_deployment_resources volume ON volume.deployment_id=NEW.deployment_id
         AND volume.resource_name=binding->>'resource' AND volume.resource_kind='volume'
       WHERE selection->>'slot'=binding->>'slot' AND selection->'guest_path'=binding->'guest_path'
         AND selection->'access_mode'=binding->'access_mode' AND selection->>'volume_id'=volume.resource_id::text
         AND (selection->>'grant_id')::uuid<>'00000000-0000-0000-0000-000000000000')) THEN
   RAISE EXCEPTION 'import must exactly match complete original plan and published observations' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER recipe_prepared_import_guard BEFORE INSERT OR UPDATE OR DELETE
 ON recipe_prepared_instance_imports FOR EACH ROW EXECUTE FUNCTION guard_recipe_prepared_import();

-- Deferred header+child checks cover zero imports and every sibling: no partial
-- graph can commit. The original event is its stable preparation receipt.
CREATE FUNCTION validate_recipe_execution_preparation_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE header recipe_deployment_execution_preparations%ROWTYPE;
BEGIN
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=NEW.deployment_id;
 IF header.instance_count<>(SELECT count(*) FROM recipe_prepared_instance_imports WHERE deployment_id=header.deployment_id)
    OR EXISTS(SELECT 1 FROM recipe_deployment_resources resource WHERE resource.deployment_id=header.deployment_id
       AND resource.resource_kind='instance' AND NOT EXISTS(SELECT 1 FROM recipe_prepared_instance_imports child
         WHERE child.deployment_id=resource.deployment_id AND child.resource_name=resource.resource_name))
    OR EXISTS(SELECT 1 FROM recipe_prepared_instance_imports child
       CROSS JOIN LATERAL jsonb_array_elements((convert_from(child.canonical_import,'UTF8')::jsonb)->'volumes') selection
       WHERE child.deployment_id=header.deployment_id GROUP BY selection->>'grant_id' HAVING count(*)<>1)
    OR NOT EXISTS(SELECT 1 FROM application_events event WHERE event.id=header.event_id
       AND event.scope_kind='project' AND event.scope_id=header.project_id
       AND event.aggregate_type='project' AND event.aggregate_id=header.project_id AND event.event_type='project.changed'
       AND event.change_kind='updated' AND event.actor_type='user'
       AND event.related_id_one=(SELECT organization_id FROM projects WHERE id=header.project_id)
       AND event.related_id_two IS NULL
       AND event.actor_id=header.actor_id AND event.request_id=header.request_id
       AND event.cursor=header.event_cursor AND event.aggregate_version=header.event_aggregate_version)
    OR NOT EXISTS(SELECT 1 FROM recipe_execution_preparation_audits link
       JOIN authorization_audit_events audit ON audit.id=link.audit_id
       WHERE link.deployment_id=header.deployment_id
         AND link.permission=audit.permission AND link.object_type=audit.object_type AND link.object_id=audit.object_id
         AND audit.authorization_model_version=header.authorization_model_version
         AND audit.actor_id=header.actor_id
       AND audit.request_id=header.request_id AND audit.permission='can_manage' AND audit.object_type='project'
       AND audit.object_id=header.project_id AND audit.decision='allow')
    OR EXISTS(SELECT 1 FROM recipe_prepared_instance_imports child WHERE child.deployment_id=header.deployment_id
       AND NOT EXISTS(SELECT 1 FROM recipe_execution_preparation_audits link
       JOIN authorization_audit_events audit ON audit.id=link.audit_id
       WHERE link.deployment_id=header.deployment_id
         AND link.permission=audit.permission AND link.object_type=audit.object_type AND link.object_id=audit.object_id
         AND audit.authorization_model_version=header.authorization_model_version
         AND audit.actor_id=header.actor_id
         AND audit.request_id=header.request_id AND audit.object_type='release_agent'
         AND audit.object_id=child.release_agent_id AND audit.permission='can_use' AND audit.decision='allow'))
    OR EXISTS(SELECT 1 FROM recipe_deployment_resources volume WHERE volume.deployment_id=header.deployment_id
       AND volume.ownership='external' AND NOT EXISTS(SELECT 1 FROM recipe_execution_preparation_audits link
         JOIN authorization_audit_events audit ON audit.id=link.audit_id
         WHERE link.deployment_id=header.deployment_id
           AND link.permission=audit.permission AND link.object_type=audit.object_type AND link.object_id=audit.object_id
           AND audit.authorization_model_version=header.authorization_model_version
         AND audit.actor_id=header.actor_id AND audit.request_id=header.request_id
           AND audit.object_type='state_volume' AND audit.object_id=volume.resource_id
           AND audit.permission='can_read' AND audit.decision='allow'))
    OR EXISTS(SELECT 1 FROM recipe_deployment_resources volume WHERE volume.deployment_id=header.deployment_id
       AND volume.ownership='external' AND EXISTS(SELECT 1 FROM recipe_deployment_resources instance
         CROSS JOIN LATERAL jsonb_array_elements(instance.plan_json->'intent'->'volume_bindings') binding
         WHERE instance.deployment_id=header.deployment_id AND instance.resource_kind='instance'
           AND binding->>'resource'=volume.resource_name)
       AND EXISTS(SELECT 1 FROM (VALUES('can_attach'),('can_grant_agent_capability')) required(permission)
         WHERE NOT EXISTS(SELECT 1 FROM recipe_execution_preparation_audits link
       JOIN authorization_audit_events audit ON audit.id=link.audit_id
       WHERE link.deployment_id=header.deployment_id
         AND link.permission=audit.permission AND link.object_type=audit.object_type AND link.object_id=audit.object_id
         AND audit.authorization_model_version=header.authorization_model_version
         AND audit.actor_id=header.actor_id
           AND audit.request_id=header.request_id AND audit.object_type='state_volume'
           AND audit.object_id=volume.resource_id AND audit.permission=required.permission AND audit.decision='allow'))) THEN
   RAISE EXCEPTION 'whole preparation requires every import plus exact committed event and allowed audit' USING ERRCODE='23000';
 END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER recipe_execution_preparation_commit AFTER INSERT
 ON recipe_deployment_execution_preparations DEFERRABLE INITIALLY DEFERRED
 FOR EACH ROW EXECUTE FUNCTION validate_recipe_execution_preparation_commit();
CREATE CONSTRAINT TRIGGER recipe_prepared_import_commit AFTER INSERT
 ON recipe_prepared_instance_imports DEFERRABLE INITIALLY DEFERRED
 FOR EACH ROW EXECUTE FUNCTION validate_recipe_execution_preparation_commit();

-- Additional predicates leave historical claims/mappings intact. They cannot
-- authorize resumed IO with missing preparation. Remove/cleanup remains legal.
CREATE FUNCTION require_recipe_whole_preparation_before_effect() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE command recipe_deployment_commands%ROWTYPE;
BEGIN
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=NEW.command_id;
 IF command.operation='install' AND (recipe_execution_preparation_authorized(command) IS DISTINCT FROM true
    OR NOT EXISTS(SELECT 1 FROM recipe_deployment_execution_preparations header
    JOIN recipe_deployments deployment ON deployment.id=header.deployment_id
    WHERE header.deployment_id=NEW.deployment_id AND header.command_id=NEW.command_id
      AND header.project_id=NEW.project_id AND header.actor_id=command.actor_id
      AND header.intent_hash=deployment.input_hash)) THEN
   RAISE EXCEPTION 'whole preparation and current original Install authority must precede effect claims' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER recipe_whole_preparation_before_claim BEFORE INSERT ON recipe_effect_attempts
 FOR EACH ROW EXECUTE FUNCTION require_recipe_whole_preparation_before_effect();

CREATE FUNCTION require_recipe_whole_preparation_before_provider_mapping() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE command recipe_deployment_commands%ROWTYPE;
BEGIN
 SELECT original.* INTO command FROM recipe_deployment_commands original
   JOIN recipe_effect_attempts attempt ON attempt.command_id=original.id WHERE attempt.id=NEW.attempt_id;
 IF recipe_execution_preparation_authorized(command) IS DISTINCT FROM true
    OR NOT EXISTS(SELECT 1 FROM recipe_effect_attempts attempt
    JOIN recipe_deployment_execution_preparations header ON header.deployment_id=attempt.deployment_id
      AND header.command_id=attempt.command_id AND header.actor_id=attempt.actor_id
    JOIN recipe_deployments deployment ON deployment.id=header.deployment_id
    WHERE attempt.id=NEW.attempt_id AND header.deployment_id=NEW.deployment_id
      AND header.project_id=NEW.project_id AND header.intent_hash=deployment.input_hash) THEN
   RAISE EXCEPTION 'whole preparation must precede original provider mapping' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER recipe_whole_preparation_before_mapping BEFORE INSERT ON recipe_owned_volume_preparations
 FOR EACH ROW EXECUTE FUNCTION require_recipe_whole_preparation_before_provider_mapping();

ALTER TABLE recipe_deployment_execution_preparations ENABLE ROW LEVEL SECURITY;
ALTER TABLE recipe_deployment_execution_preparations FORCE ROW LEVEL SECURITY;
ALTER TABLE recipe_prepared_instance_imports ENABLE ROW LEVEL SECURITY;
ALTER TABLE recipe_prepared_instance_imports FORCE ROW LEVEL SECURITY;
ALTER TABLE recipe_execution_preparation_audits ENABLE ROW LEVEL SECURITY;
ALTER TABLE recipe_execution_preparation_audits FORCE ROW LEVEL SECURITY;
REVOKE ALL ON recipe_deployment_execution_preparations,recipe_prepared_instance_imports,
 recipe_execution_preparation_audits FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT SELECT ON recipe_deployment_execution_preparations,recipe_prepared_instance_imports,
 recipe_execution_preparation_audits TO hephaestus_app;
CREATE POLICY recipe_execution_preparation_read ON recipe_deployment_execution_preparations FOR SELECT TO hephaestus_app
 USING(check_permission('user',hephaestus_actor_id(),'can_manage','project',project_id::text)=1);
CREATE POLICY recipe_prepared_import_read ON recipe_prepared_instance_imports FOR SELECT TO hephaestus_app
 USING(check_permission('user',hephaestus_actor_id(),'can_manage','project',project_id::text)=1);
CREATE POLICY recipe_preparation_audit_read ON recipe_execution_preparation_audits FOR SELECT TO hephaestus_app
 USING(EXISTS(SELECT 1 FROM recipe_deployment_execution_preparations header
    WHERE header.deployment_id=recipe_execution_preparation_audits.deployment_id));
REVOKE ALL ON FUNCTION guard_recipe_execution_preparation(),guard_recipe_prepared_import(),
 validate_recipe_execution_preparation_commit(),require_recipe_whole_preparation_before_effect(),
 require_recipe_whole_preparation_before_provider_mapping(),reject_recipe_preparation_audit_rewrite() FROM PUBLIC;

-- Input is produced/reconstructed by the owning Rust adapter, never client JSON.
-- The SQL boundary validates stored identity/current FGA and published facts;
-- tagged UUIDv5 and ReleaseCommandKey derivations remain Rust checks. Model
-- version is bounded audit metadata bound from AUTHORIZATION_MODEL_VERSION,
-- not authority. No event/audit UUID or historical receipt is an input.
CREATE FUNCTION recipe_preparation_digest(value jsonb) RETURNS bytea
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE element jsonb; result bytea:=''::bytea; octet integer;
BEGIN
 IF jsonb_typeof(value) IS DISTINCT FROM 'array' OR jsonb_array_length(value)<>32 THEN
   RAISE EXCEPTION 'preparation digest must contain 32 octets' USING ERRCODE='23000';
 END IF;
 FOR element IN SELECT * FROM jsonb_array_elements(value) LOOP
   IF jsonb_typeof(element) IS DISTINCT FROM 'number' THEN
     RAISE EXCEPTION 'preparation digest octet must be numeric' USING ERRCODE='23000';
   END IF;
   octet:=(element#>>'{}')::integer;
   IF octet NOT BETWEEN 0 AND 255 OR to_jsonb(octet) IS DISTINCT FROM element THEN
     RAISE EXCEPTION 'preparation digest octet out of range' USING ERRCODE='23000';
   END IF;
   result:=result||decode(lpad(to_hex(octet),2,'0'),'hex');
 END LOOP;
 RETURN result;
END $$;
REVOKE ALL ON FUNCTION recipe_preparation_digest(jsonb) FROM PUBLIC;

-- Transient serialization ONLY, never persisted scope/ownership/provenance.
-- Actual signed bigint keys match existing107's first8-byte i64 framing.
-- Instance SHA256(tag-NUL-UUIDbytes) uses two-int PG lock space, disjoint from
-- bigint command space; collisions serialize and do not establish equality.
CREATE FUNCTION recipe_import107_command_mutex(key bytea) RETURNS bigint
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF key IS NULL OR octet_length(key)<>32 THEN
   RAISE EXCEPTION 'import command mutex needs exact key bytes' USING ERRCODE='23000';
 END IF;
 RETURN ('x'||substr(encode(key,'hex'),1,16))::bit(64)::bigint;
END $$;
REVOKE ALL ON FUNCTION recipe_import107_command_mutex(bytea) FROM PUBLIC;

CREATE FUNCTION recipe_import113_instance_mutex(id uuid) RETURNS TABLE(first_key integer,second_key integer)
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT ('x'||substr(encode(digest,'hex'),1,8))::bit(32)::integer,
        ('x'||substr(encode(digest,'hex'),9,8))::bit(32)::integer
 FROM (SELECT sha256(convert_to('hephaestus-recipe113-instance-birth-v1','UTF8')||decode('00','hex')||uuid_send(id)) digest) value
$$;
REVOKE ALL ON FUNCTION recipe_import113_instance_mutex(uuid) FROM PUBLIC;

CREATE FUNCTION reserve_recipe_execution_preparation(
 p_deployment_id uuid,p_command_id uuid,p_canonical_input bytea,p_canonical_imports bytea[]
) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE original recipe_deployment_commands%ROWTYPE; input jsonb; child jsonb; index integer; mutex record;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text) IS DISTINCT FROM 'hephaestus_app'
    OR current_setting('hephaestus.subject_type',true) IS DISTINCT FROM 'user'
    OR NULLIF(current_setting('hephaestus.request_id',true),'')::uuid IS NULL
    OR p_canonical_input IS NULL OR octet_length(p_canonical_input) NOT BETWEEN 1 AND 1048576
    OR p_canonical_imports IS NULL OR COALESCE(array_ndims(p_canonical_imports),1)<>1
    OR cardinality(p_canonical_imports)>64
    OR (SELECT COALESCE(sum(octet_length(bytes)),0) FROM unnest(p_canonical_imports) bytes)>1048576 THEN
   RAISE EXCEPTION 'preparation reservation requires bounded actor input' USING ERRCODE='42501';
 END IF;
 SELECT * INTO original FROM recipe_deployment_commands WHERE id=p_command_id;
 IF original.deployment_id IS DISTINCT FROM p_deployment_id
    OR recipe_execution_preparation_authorized(original) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'preparation reservation requires current original Install' USING ERRCODE='42501';
 END IF;
 input:=convert_from(p_canonical_input,'UTF8')::jsonb;
 IF input->>'deployment_id' IS DISTINCT FROM p_deployment_id::text
    OR input->>'project_id' IS DISTINCT FROM original.project_id::text
    OR input->'command' IS DISTINCT FROM jsonb_build_object('id',original.id,'actor_id',original.actor_id,
       'idempotency_id',original.idempotency_id,'operation',original.operation)
    OR jsonb_typeof(input->'instances') IS DISTINCT FROM 'array'
    OR jsonb_array_length(input->'instances')<>cardinality(p_canonical_imports)
    OR cardinality(p_canonical_imports)<>(SELECT count(*) FROM recipe_deployment_resources
       WHERE deployment_id=p_deployment_id AND resource_kind='instance') THEN
   RAISE EXCEPTION 'reservation must cover the exact complete instance set' USING ERRCODE='23000';
 END IF;
 FOR index IN SELECT generate_subscripts(p_canonical_imports,1) LOOP
   IF p_canonical_imports[index] IS NULL OR octet_length(p_canonical_imports[index]) NOT BETWEEN 1 AND 1048576 THEN
     RAISE EXCEPTION 'reservation needs bounded complete children' USING ERRCODE='23000';
   END IF;
   child:=convert_from(p_canonical_imports[index],'UTF8')::jsonb;
   IF child IS DISTINCT FROM input->'instances'->(index-array_lower(p_canonical_imports,1))
      OR NOT EXISTS(SELECT 1 FROM recipe_deployment_resources resource
        WHERE deployment_id=p_deployment_id AND resource_name=child->>'resource'
          AND resource.resource_kind='instance' AND resource.ownership='owned'
          AND resource.resource_id::text=child->>'instance_id' AND resource.revision_id::text=child->>'revision_id') THEN
     RAISE EXCEPTION 'reservation labels do not match immutable resources' USING ERRCODE='23000';
   END IF;
 END LOOP;
 -- Raw callers may already hold Project: never wait and form a lock cycle.
 -- Trusted Rust waits in this same order BEFORE any row lock, then these
 -- try-locks are reentrant. No new authority is inferred from held locks.
 -- Dedup/sort the actual signed mutex values, not labels or unsigned digests.
 FOR mutex IN SELECT DISTINCT recipe_import107_command_mutex(recipe_preparation_digest(
     (convert_from(bytes,'UTF8')::jsonb)->'command_key')) key
     FROM unnest(p_canonical_imports) bytes ORDER BY key LOOP
   IF pg_try_advisory_xact_lock(mutex.key) IS DISTINCT FROM true THEN
     RAISE EXCEPTION 'preparation command reservation is held' USING ERRCODE='55P03';
   END IF;
 END LOOP;
 FOR mutex IN SELECT DISTINCT key.first_key,key.second_key
     FROM unnest(p_canonical_imports) bytes
     CROSS JOIN LATERAL recipe_import113_instance_mutex(((convert_from(bytes,'UTF8')::jsonb)->>'instance_id')::uuid) key
     ORDER BY key.first_key,key.second_key LOOP
   IF pg_try_advisory_xact_lock(mutex.first_key,mutex.second_key) IS DISTINCT FROM true THEN
     RAISE EXCEPTION 'preparation instance reservation is held' USING ERRCODE='55P03';
   END IF;
 END LOOP;
END $$;
REVOKE ALL ON FUNCTION reserve_recipe_execution_preparation(uuid,uuid,bytea,bytea[]) FROM PUBLIC,hephaestus_worker;
GRANT EXECUTE ON FUNCTION reserve_recipe_execution_preparation(uuid,uuid,bytea,bytea[]) TO hephaestus_app;

CREATE FUNCTION write_recipe_execution_preparation(
 p_deployment_id uuid,p_command_id uuid,p_canonical_input bytea,
 p_canonical_imports bytea[],p_authorization_model_version text
) RETURNS TABLE(event_id uuid,event_cursor bigint,event_aggregate_version bigint,replayed boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE
 original recipe_deployment_commands%ROWTYPE;
 deployment recipe_deployments%ROWTYPE;
 existing recipe_deployment_execution_preparations%ROWTYPE;
 input jsonb; child jsonb; source jsonb; required record; agent release_agents%ROWTYPE;
 organization uuid; current_request uuid; captured_audit_id uuid;
 captured_event_id uuid; captured_cursor bigint; captured_version bigint;
 index integer; ordinal integer; import_count integer; is_replay boolean;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app'
    OR current_setting('hephaestus.subject_type',true) IS DISTINCT FROM 'user' THEN
   RAISE EXCEPTION 'preparation writer is actor only' USING ERRCODE='42501';
 END IF;
 current_request:=NULLIF(current_setting('hephaestus.request_id',true),'')::uuid;
 IF current_request IS NULL OR current_request='00000000-0000-0000-0000-000000000000'
    OR p_authorization_model_version IS NULL OR octet_length(p_authorization_model_version) NOT BETWEEN 1 AND 256
    OR p_authorization_model_version<>btrim(p_authorization_model_version)
    OR p_authorization_model_version ~ '[[:cntrl:]]'
    OR p_canonical_input IS NULL OR octet_length(p_canonical_input) NOT BETWEEN 1 AND 1048576
    OR p_canonical_imports IS NULL OR COALESCE(array_ndims(p_canonical_imports),1)<>1
    OR cardinality(p_canonical_imports)>64
    OR (SELECT COALESCE(sum(octet_length(bytes)),0) FROM unnest(p_canonical_imports) bytes)>1048576 THEN
   RAISE EXCEPTION 'preparation writer needs bounded canonical input and current provenance' USING ERRCODE='23000';
 END IF;
 SELECT * INTO original FROM recipe_deployment_commands WHERE id=p_command_id;
 IF original.deployment_id IS DISTINCT FROM p_deployment_id
    OR recipe_execution_preparation_authorized(original) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'preparation requires exact original Install and current rights' USING ERRCODE='42501';
 END IF;
 PERFORM reserve_recipe_execution_preparation(p_deployment_id,p_command_id,p_canonical_input,p_canonical_imports);
 -- Match the adapter's project -> deployment order; serialize first creation.
 SELECT organization_id INTO organization FROM projects WHERE id=original.project_id FOR NO KEY UPDATE;
 SELECT * INTO deployment FROM recipe_deployments WHERE id=p_deployment_id FOR UPDATE;
 IF recipe_execution_preparation_authorized(original) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'preparation rights changed while waiting' USING ERRCODE='42501';
 END IF;
 input:=convert_from(p_canonical_input,'UTF8')::jsonb;
 import_count:=cardinality(p_canonical_imports);
 IF jsonb_typeof(input->'instances') IS DISTINCT FROM 'array'
    OR jsonb_array_length(input->'instances')<>import_count THEN
   RAISE EXCEPTION 'preparation child count mismatch' USING ERRCODE='23000';
 END IF;
 -- Check current published source facts even on replay: existing bytes are not
 -- reusable authority. The adapter also hashes authored JSON and checks policy.
 FOR index IN SELECT generate_subscripts(p_canonical_imports,1) LOOP
   ordinal:=index-array_lower(p_canonical_imports,1);
   IF p_canonical_imports[index] IS NULL OR octet_length(p_canonical_imports[index]) NOT BETWEEN 1 AND 1048576 THEN
     RAISE EXCEPTION 'prepared child must be bounded' USING ERRCODE='23000';
   END IF;
   child:=convert_from(p_canonical_imports[index],'UTF8')::jsonb; source:=child->'source';
   SELECT * INTO agent FROM release_agents WHERE id=(source->'pin'->>'release_agent_id')::uuid;
   IF child IS DISTINCT FROM input->'instances'->ordinal OR agent.id IS NULL
      OR agent.release_id IS DISTINCT FROM (source->'pin'->>'release_id')::uuid
      OR source->'runtime_contract_hash' IS DISTINCT FROM recipe_effect_hash_json(agent.runtime_contract_hash)
      OR source->>'image' IS DISTINCT FROM agent.runtime_contract->>'image_reference'
      OR source->'selected_policy' IS DISTINCT FROM agent.runtime_contract->'policy_ceiling'
      OR source->'platform' IS DISTINCT FROM input->'platform'
      OR NOT EXISTS(SELECT 1 FROM releases WHERE id=agent.release_id AND state='published')
      OR check_permission('user',hephaestus_actor_id(),'can_use','release_agent',agent.id::text) IS DISTINCT FROM 1 THEN
     RAISE EXCEPTION 'prepared source observation is not current published input' USING ERRCODE='23000';
   END IF;
 END LOOP;
 SELECT * INTO existing FROM recipe_deployment_execution_preparations WHERE deployment_id=p_deployment_id;
 is_replay:=existing.deployment_id IS NOT NULL;
 IF is_replay THEN
   IF existing.command_id IS DISTINCT FROM p_command_id OR existing.actor_id IS DISTINCT FROM original.actor_id
      OR existing.canonical_input IS DISTINCT FROM p_canonical_input OR existing.instance_count<>import_count
      OR EXISTS(SELECT 1 FROM recipe_prepared_instance_imports stored WHERE stored.deployment_id=p_deployment_id
         AND stored.canonical_import IS DISTINCT FROM p_canonical_imports[array_lower(p_canonical_imports,1)+stored.ordinal]) THEN
     RAISE EXCEPTION 'changed original execution preparation' USING ERRCODE='23000';
   END IF;
   captured_event_id:=existing.event_id; captured_cursor:=existing.event_cursor;
   captured_version:=existing.event_aggregate_version;
 ELSE
   SELECT event.event_id,event.cursor,event.aggregate_version
     INTO captured_event_id,captured_cursor,captured_version
     FROM append_application_event(gen_random_uuid(),'project',original.project_id,
       'project',original.project_id,'project.changed','updated',NULL,organization,NULL) event;
   INSERT INTO recipe_deployment_execution_preparations(
     deployment_id,project_id,command_id,actor_id,version,profile,authorization_model_version,intent_hash,platform_json,
     instance_count,canonical_input,input_hash,request_id,event_id,event_cursor,event_aggregate_version)
   VALUES(p_deployment_id,original.project_id,p_command_id,original.actor_id,
     (input->>'version')::integer,input->>'profile',p_authorization_model_version,deployment.input_hash,input->'platform',
     import_count,p_canonical_input,sha256(p_canonical_input),current_request,
     captured_event_id,captured_cursor,captured_version);
   FOR index IN SELECT generate_subscripts(p_canonical_imports,1) LOOP
     ordinal:=index-array_lower(p_canonical_imports,1);
     child:=convert_from(p_canonical_imports[index],'UTF8')::jsonb; source:=child->'source';
     INSERT INTO recipe_prepared_instance_imports(
       deployment_id,project_id,command_id,resource_name,ordinal,operation_id,import_command_key,
       instance_id,revision_id,release_id,release_agent_id,canonical_import,input_hash)
     VALUES(p_deployment_id,original.project_id,p_command_id,child->>'resource',ordinal,
       (child->>'operation_id')::uuid,recipe_preparation_digest(child->'command_key'),
       (child->>'instance_id')::uuid,(child->>'revision_id')::uuid,
       (source->'pin'->>'release_id')::uuid,(source->'pin'->>'release_agent_id')::uuid,
       p_canonical_imports[index],sha256(p_canonical_imports[index]));
   END LOOP;
 END IF;
 -- Each operation, including replay, creates fresh allows after live checks.
 -- Only first creation links those exact newly inserted IDs to its header.
 FOR required IN
   SELECT 'can_manage'::text permission,'project'::text object_type,original.project_id object_id
   UNION
   SELECT 'can_use','release_agent',(resource.plan_json->'intent'->'release'->>'release_agent_id')::uuid
     FROM recipe_deployment_resources resource WHERE resource.deployment_id=p_deployment_id AND resource.resource_kind='instance'
   UNION
   SELECT 'can_read','state_volume',resource.resource_id FROM recipe_deployment_resources resource
     WHERE resource.deployment_id=p_deployment_id AND resource.ownership='external'
   UNION
   SELECT permission.name,'state_volume',resource.resource_id
     FROM recipe_deployment_resources resource
     CROSS JOIN (VALUES('can_attach'),('can_grant_agent_capability')) permission(name)
     WHERE resource.deployment_id=p_deployment_id AND resource.ownership='external'
       AND EXISTS(SELECT 1 FROM recipe_deployment_resources instance
         CROSS JOIN LATERAL jsonb_array_elements(instance.plan_json->'intent'->'volume_bindings') binding
         WHERE instance.deployment_id=p_deployment_id AND instance.resource_kind='instance'
           AND binding->>'resource'=resource.resource_name)
 LOOP
   IF check_permission('user',hephaestus_actor_id(),required.permission,required.object_type,required.object_id::text) IS DISTINCT FROM 1 THEN
     RAISE EXCEPTION 'preparation authority changed before allow audit' USING ERRCODE='42501';
   END IF;
   INSERT INTO authorization_audit_events(id,actor_id,permission,object_type,object_id,decision,request_id,authorization_model_version)
     VALUES(gen_random_uuid(),original.actor_id,required.permission,required.object_type,required.object_id,'allow',
       current_request,p_authorization_model_version) RETURNING id INTO captured_audit_id;
   IF NOT is_replay THEN
     INSERT INTO recipe_execution_preparation_audits(deployment_id,permission,object_type,object_id,audit_id)
       VALUES(p_deployment_id,required.permission,required.object_type,required.object_id,captured_audit_id);
   END IF;
 END LOOP;
 RETURN QUERY SELECT captured_event_id,captured_cursor,captured_version,is_replay;
END $$;
REVOKE ALL ON FUNCTION write_recipe_execution_preparation(uuid,uuid,bytea,bytea[],text) FROM PUBLIC,hephaestus_worker;
GRANT EXECUTE ON FUNCTION write_recipe_execution_preparation(uuid,uuid,bytea,bytea[],text) TO hephaestus_app;

-- Pending bounded adapter/integration work before publication:
-- * derive/compare all Rust IDs and full canonical imports on insertion and hydration;
-- * fresh original actor/source/external rights BEFORE replay, exact config drift hold;
-- * protected replay writes fresh allow audits for current request, preserves original
--   header/event/linked-audit IDs; no new general attempt/proof framework;
-- * owning registration/provider/instance invocation must load exact preparation
--   AND actual active claim before IO (generic providers remain recipe-neutral);
-- * actual113 upgrade/rawSQL/whole-graph rollback/concurrency/native import tests.
-- No historical backfill, opaque provider receipt, physical absence or grant is
-- inferred from this header, child input, UUID, SourceObservation or event.

-- Actual107 instance birth boundary. Ordinary instances without a positive113
-- child retain their exact public107 behavior. No transaction crosses a call.
-- These functions encode ONLY the bounded integral/string/bool/null import-v1
-- domain used by release_volume_instance_inputs::import_hash. JSON objects in
-- that json! Value are serde_json's C/UTF8-lexical BTreeMap order, not struct
-- field order. Unsupported numbers/depth/size/NUL input fail rather than round.
CREATE FUNCTION recipe_import107_json_bytes(value jsonb,depth integer DEFAULT 0) RETURNS text
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE result text;
BEGIN
 IF value IS NULL OR depth IS NULL OR depth NOT BETWEEN 0 AND 16
    OR octet_length(value::text)>1048576 THEN
   RAISE EXCEPTION 'unsupported bounded import JSON' USING ERRCODE='23000';
 END IF;
 CASE jsonb_typeof(value)
 WHEN 'object' THEN
   SELECT '{'||COALESCE(string_agg(to_json(key)::text||':'||recipe_import107_json_bytes(item,depth+1),',' ORDER BY key COLLATE "C"),'')||'}'
     INTO result FROM jsonb_each(value) pair(key,item);
 WHEN 'array' THEN
   SELECT '['||COALESCE(string_agg(recipe_import107_json_bytes(item,depth+1),',' ORDER BY ordinal),'')||']'
     INTO result FROM jsonb_array_elements(value) WITH ORDINALITY pair(item,ordinal);
 WHEN 'number' THEN
   IF value::text !~ '^-?(0|[1-9][0-9]*)$' OR (value::text)::numeric NOT BETWEEN -9223372036854775808 AND 18446744073709551615 THEN
     RAISE EXCEPTION 'import JSON needs exact integral numbers' USING ERRCODE='23000';
   END IF;
   result:=value::text;
 WHEN 'string' THEN result:=value::text;
 WHEN 'boolean' THEN result:=value::text;
 WHEN 'null' THEN result:='null';
 ELSE RAISE EXCEPTION 'unsupported import JSON kind' USING ERRCODE='23000';
 END CASE;
 IF octet_length(result)>1048576 THEN RAISE EXCEPTION 'import JSON exceeds bound' USING ERRCODE='23000'; END IF;
 RETURN result;
END $$;
REVOKE ALL ON FUNCTION recipe_import107_json_bytes(jsonb,integer) FROM PUBLIC;

-- Exact ParameterDocument::parameter_hash on supported64-bit Heph targets:
-- each sorted name/type/value has an8-byte big-endian usize length prefix.
-- Strings use RAW UTF8; integers use signed8-byte big-endian; bool is01/00.
-- This is NOT import_hash's JSON algorithm (empty map hashes empty bytes).
CREATE FUNCTION recipe_import107_parameter_hash(value jsonb) RETURNS bytea
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE entry record; stream bytea:=decode('','hex'); name_bytes bytea; kind bytea; item bytea;
BEGIN
 IF value IS NULL OR jsonb_typeof(value) IS DISTINCT FROM 'object'
    OR octet_length(value::text)>1048576 THEN
   RAISE EXCEPTION 'unsupported bounded parameter document' USING ERRCODE='23000';
 END IF;
 FOR entry IN SELECT fields.key,fields.item FROM jsonb_each(value) fields(key,item) ORDER BY fields.key COLLATE "C" LOOP
   name_bytes:=convert_to(entry.key,'UTF8');
   CASE jsonb_typeof(entry.item)
   WHEN 'string' THEN kind:=convert_to('string','UTF8'); item:=convert_to(entry.item#>>'{}','UTF8');
   WHEN 'number' THEN
     IF entry.item::text !~ '^-?(0|[1-9][0-9]*)$'
        OR (entry.item::text)::numeric NOT BETWEEN -9223372036854775808 AND 9223372036854775807 THEN
       RAISE EXCEPTION 'parameter integer must be exact signed64-bit' USING ERRCODE='23000';
     END IF;
     kind:=convert_to('integer','UTF8'); item:=int8send((entry.item::text)::bigint);
   WHEN 'boolean' THEN kind:=convert_to('boolean','UTF8'); item:=CASE WHEN entry.item='true'::jsonb THEN decode('01','hex') ELSE decode('00','hex') END;
   ELSE RAISE EXCEPTION 'unsupported parameter type' USING ERRCODE='23000';
   END CASE;
   stream:=stream||int8send(octet_length(name_bytes)::bigint)||name_bytes||
       int8send(octet_length(kind)::bigint)||kind||int8send(octet_length(item)::bigint)||item;
   IF octet_length(stream)>1048576 THEN RAISE EXCEPTION 'parameter hash input exceeds bound' USING ERRCODE='23000'; END IF;
 END LOOP;
 RETURN sha256(stream);
END $$;
REVOKE ALL ON FUNCTION recipe_import107_parameter_hash(jsonb) FROM PUBLIC;

CREATE FUNCTION recipe_import107_input_hash(child recipe_prepared_instance_imports,actor uuid) RETURNS bytea
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT sha256(convert_to(recipe_import107_json_bytes(jsonb_build_object(
   'version',1,'operation','import_agent_with_volumes','actor',actor,
   'instance',child.instance_id,'revision',child.revision_id,'release',child.release_id,
   'agent',child.release_agent_id,'project',child.project_id,'name',input->>'name',
   'parameters',input->'parameters','selected_policy',input->'source'->'selected_policy',
   'platform_policy',input->'source'->'platform'->'policy',
   'platform_policy_version',input->'source'->'platform'->>'version','volumes',input->'volumes'),0),'UTF8'))
 FROM (SELECT convert_from(child.canonical_import,'UTF8')::jsonb input) document
$$;
REVOKE ALL ON FUNCTION recipe_import107_input_hash(recipe_prepared_instance_imports,uuid) FROM PUBLIC;

CREATE FUNCTION guard_recipe_instance_birth() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 command recipe_deployment_commands%ROWTYPE; deployment recipe_deployments%ROWTYPE;
 resource recipe_deployment_resources%ROWTYPE; attempt recipe_effect_attempts%ROWTYPE;
 input jsonb; agent release_agents%ROWTYPE; mutex record;
BEGIN
 SELECT * INTO mutex FROM recipe_import113_instance_mutex(NEW.id);
 IF pg_try_advisory_xact_lock(mutex.first_key,mutex.second_key) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'instance birth reservation is held' USING ERRCODE='55P03';
 END IF;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=NEW.id;
 IF child.instance_id IS NULL THEN RETURN NEW; END IF;
 -- Same serialization as preparation and Remove, acquired in the actual107
 -- insertion transaction after its unchanged command advisory lock.
 PERFORM 1 FROM projects WHERE id=child.project_id FOR NO KEY UPDATE;
 SELECT * INTO deployment FROM recipe_deployments WHERE id=child.deployment_id FOR UPDATE;
 SELECT * INTO resource FROM recipe_deployment_resources
   WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name FOR UPDATE;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=child.command_id;
 SELECT * INTO attempt FROM recipe_effect_attempts WHERE id=resource.active_attempt_id;
 SELECT * INTO agent FROM release_agents WHERE id=child.release_agent_id;
 input:=convert_from(child.canonical_import,'UTF8')::jsonb;
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text) IS DISTINCT FROM 'hephaestus_app'
    OR current_setting('hephaestus.subject_type',true) IS DISTINCT FROM 'user'
    OR NULLIF(current_setting('hephaestus.request_id',true),'')::uuid IS NULL
    OR recipe_execution_preparation_authorized(command) IS DISTINCT FROM true
    OR deployment.id IS NULL OR deployment.lifecycle NOT IN ('installing','recovery_required')
    OR header.command_id IS DISTINCT FROM command.id OR header.actor_id IS DISTINCT FROM command.actor_id
    OR header.intent_hash IS DISTINCT FROM deployment.input_hash
    OR header.input_hash IS DISTINCT FROM sha256(header.canonical_input)
    OR header.instance_count IS DISTINCT FROM (SELECT count(*)::integer FROM recipe_prepared_instance_imports WHERE deployment_id=child.deployment_id)
    OR resource.ownership IS DISTINCT FROM 'owned' OR resource.resource_kind IS DISTINCT FROM 'instance'
    OR resource.resource_id IS DISTINCT FROM child.instance_id OR resource.revision_id IS DISTINCT FROM child.revision_id
    OR resource.install_progress NOT IN ('creating','recovery_required')
    OR attempt.id IS NULL OR attempt.command_id IS DISTINCT FROM command.id
    OR attempt.deployment_id IS DISTINCT FROM deployment.id OR attempt.project_id IS DISTINCT FROM child.project_id
    OR attempt.resource_name IS DISTINCT FROM child.resource_name OR attempt.action IS DISTINCT FROM 'create'
    OR attempt.actor_id IS DISTINCT FROM command.actor_id OR attempt.generation IS DISTINCT FROM resource.execution_generation
    OR attempt.input_hash IS DISTINCT FROM resource.input_hash
    OR attempt.claim_json IS DISTINCT FROM jsonb_build_object('command_id',command.id,'deployment_id',deployment.id,
       'resource',child.resource_name,'identity',resource.plan_json->'identity','input_hash',recipe_effect_hash_json(resource.input_hash),
       'resource_version',(attempt.before_state->>'version')::bigint+1,'generation',attempt.generation,'action','create',
       'attempt_id',attempt.id,'actor_id',attempt.actor_id,'request_id',attempt.request_id)
    OR NOT EXISTS(SELECT 1 FROM recipe_execution_transitions transition WHERE transition.attempt_id=attempt.id
       AND transition.kind='claimed' AND transition.after_state->>'active_attempt'=attempt.id::text)
    OR NEW.project_id IS DISTINCT FROM child.project_id OR NEW.created_by IS DISTINCT FROM command.actor_id
    OR NEW.name IS DISTINCT FROM input->>'name' OR NEW.family_id IS DISTINCT FROM agent.family_id
    OR NEW.volume_mode IS DISTINCT FROM 'named' OR NEW.run_gate_open IS DISTINCT FROM false
    OR NEW.state IS DISTINCT FROM 'active' OR NEW.state_volume_id IS NOT NULL OR NEW.active_revision_id IS NOT NULL
    OR input->'source'->'runtime_contract_hash' IS DISTINCT FROM recipe_effect_hash_json(agent.runtime_contract_hash)
    OR input->'source'->>'image' IS DISTINCT FROM agent.runtime_contract->>'image_reference'
    OR input->'source'->'selected_policy' IS DISTINCT FROM agent.runtime_contract->'policy_ceiling'
    OR EXISTS(SELECT 1 FROM release_command_inbox WHERE command_key=child.import_command_key) THEN
   RAISE EXCEPTION 'recipe instance birth needs exact active original Create and open current Install' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_recipe_instance_birth() FROM PUBLIC;
CREATE TRIGGER recipe_instance_birth_guard BEFORE INSERT ON agent_instances
 FOR EACH ROW EXECUTE FUNCTION guard_recipe_instance_birth();

CREATE FUNCTION validate_recipe_instance_birth_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 instance agent_instances%ROWTYPE; revision agent_instance_revisions%ROWTYPE;
 command release_command_inbox%ROWTYPE; input jsonb; policy jsonb; expected_policy_hash bytea; actual_request uuid;
BEGIN
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=NEW.id;
 IF child.instance_id IS NULL THEN RETURN NULL; END IF;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id;
 SELECT * INTO instance FROM agent_instances WHERE id=child.instance_id;
 SELECT * INTO revision FROM agent_instance_revisions WHERE id=child.revision_id;
 SELECT * INTO command FROM release_command_inbox WHERE command_key=child.import_command_key;
 input:=convert_from(child.canonical_import,'UTF8')::jsonb; policy:=input->'source'->'selected_policy';
 actual_request:=NULLIF(current_setting('hephaestus.request_id',true),'')::uuid;
 -- insert_revision hashes RuntimePolicy as a struct (vcpus,memory_mib,network),
 -- unlike import_hash's json! Value. Do not substitute C-sorted object bytes.
 expected_policy_hash:=sha256(convert_to('{"vcpus":'||recipe_import107_json_bytes(policy->'vcpus',0)||
   ',"memory_mib":'||recipe_import107_json_bytes(policy->'memory_mib',0)||
   ',"network":'||recipe_import107_json_bytes(policy->'network',0)||'}','UTF8'));
 IF NOT EXISTS(SELECT 1 FROM recipe_deployment_commands original WHERE original.id=header.command_id
       AND recipe_execution_preparation_authorized(original) IS TRUE)
    OR command.command_key IS NULL OR command.operation IS DISTINCT FROM 'import_agent_with_volumes'
    OR command.aggregate_id IS DISTINCT FROM child.instance_id OR command.secondary_id IS DISTINCT FROM child.revision_id
    OR command.actor_id IS DISTINCT FROM header.actor_id OR command.request_id IS DISTINCT FROM actual_request
    OR command.input_hash IS DISTINCT FROM recipe_import107_input_hash(child,header.actor_id)
    OR instance.active_revision_id IS DISTINCT FROM child.revision_id OR instance.created_by IS DISTINCT FROM header.actor_id
    OR instance.project_id IS DISTINCT FROM child.project_id OR instance.name IS DISTINCT FROM input->>'name'
    OR instance.volume_mode IS DISTINCT FROM 'named' OR instance.run_gate_open IS DISTINCT FROM false
    OR instance.state_volume_id IS NOT NULL OR instance.state IS DISTINCT FROM 'active'
    OR revision.instance_id IS DISTINCT FROM child.instance_id OR revision.release_agent_id IS DISTINCT FROM child.release_agent_id
    OR revision.created_by IS DISTINCT FROM header.actor_id OR revision.parameters IS DISTINCT FROM input->'parameters'
    OR revision.parameter_hash IS DISTINCT FROM recipe_import107_parameter_hash(input->'parameters')
    OR revision.resource_selection IS DISTINCT FROM policy OR revision.network_restriction IS DISTINCT FROM jsonb_build_object('network',policy->'network')
    OR revision.effective_runtime_policy IS DISTINCT FROM policy OR revision.effective_policy_hash IS DISTINCT FROM expected_policy_hash
    OR revision.platform_policy_version IS DISTINCT FROM input->'source'->'platform'->>'version'
    OR revision.secret_bindings IS DISTINCT FROM '[]'::jsonb OR revision.diagnostics IS DISTINCT FROM '[]'::jsonb
    OR revision.publication_mode IS DISTINCT FROM 'proposal' OR revision.publication_repository_binding_id IS NOT NULL
    OR revision.runnable IS DISTINCT FROM true
    OR (SELECT count(*) FROM agent_instance_revision_volume_bindings WHERE instance_revision_id=child.revision_id)<>jsonb_array_length(input->'volumes')
    OR (SELECT count(*) FROM agent_instance_volume_mount_grants WHERE instance_revision_id=child.revision_id)<>jsonb_array_length(input->'volumes')
    OR EXISTS(SELECT 1 FROM jsonb_array_elements(input->'volumes') selection WHERE NOT EXISTS(
       SELECT 1 FROM agent_instance_revision_volume_bindings binding JOIN agent_instance_volume_mount_grants grant_row
         ON grant_row.instance_revision_id=binding.instance_revision_id AND grant_row.slot_key=binding.slot_key
       WHERE binding.instance_revision_id=child.revision_id AND binding.instance_id=child.instance_id
         AND binding.project_id=child.project_id AND binding.release_agent_id=child.release_agent_id
         AND binding.slot_key=selection->>'slot' AND binding.volume_id=(selection->>'volume_id')::uuid
         AND binding.access_mode=selection->>'access_mode' AND binding.guest_path=selection->>'guest_path'
         AND binding.created_by=header.actor_id AND grant_row.id=(selection->>'grant_id')::uuid
         AND grant_row.instance_id=child.instance_id AND grant_row.release_agent_id=child.release_agent_id
         AND grant_row.volume_id=binding.volume_id AND grant_row.access_mode=binding.access_mode
         AND grant_row.created_by=header.actor_id AND grant_row.request_id=actual_request
         AND recipe_effect_hash_json(grant_row.release_contract_hash)=input->'source'->'runtime_contract_hash'))
    OR NOT EXISTS(SELECT 1 FROM agent_instance_events event WHERE event.instance_id=child.instance_id
       AND event.revision_id=child.revision_id AND event.update_id IS NULL AND event.event_type='instance.created'
       AND event.actor_id=header.actor_id AND event.request_id=actual_request
       AND event.payload='{"runnable":true,"volume_mode":"named","dispatch_supported":false}'::jsonb)
    OR NOT EXISTS(SELECT 1 FROM outbox WHERE aggregate_id=child.instance_id AND aggregate_type='release'
       AND subject='hephaestus.agent_instance.created.v1' AND event_type='agent_instance.created.v1'
       AND payload=jsonb_build_object('instance_id',child.instance_id,'revision_id',child.revision_id,
         'release_agent_id',child.release_agent_id,'project_id',child.project_id,'runnable',true,'volume_mode','named','dispatch_supported',false,
         'schema_version',1,'message_id',id,'idempotency_key',id,'request_id',NULL,'trace_id',NULL))
    OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
       WHERE event.aggregate_type='agent_instance' AND event.aggregate_id=child.instance_id
         AND event.scope_kind='agent_instance' AND event.scope_id=child.instance_id
         AND event.event_type='agent_instance.changed' AND event.actor_type='user'
         AND event.actor_id=header.actor_id AND event.request_id=actual_request) THEN
   RAISE EXCEPTION 'recipe instance birth must commit exact prepared107 input, command, creation and outboxes' USING ERRCODE='23000';
 END IF;
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_recipe_instance_birth_commit() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER recipe_instance_birth_commit AFTER INSERT ON agent_instances
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_recipe_instance_birth_commit();

-- Prevent an unrelated generic command from poisoning/adopting a reserved
-- prepared key after preparation. Exact committed107 replay performs no INSERT.
CREATE FUNCTION guard_recipe_instance_command() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
BEGIN
 IF pg_try_advisory_xact_lock(recipe_import107_command_mutex(NEW.command_key)) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'instance command reservation is held' USING ERRCODE='55P03';
 END IF;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE import_command_key=NEW.command_key;
 IF child.instance_id IS NULL THEN RETURN NEW; END IF;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id;
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text) IS DISTINCT FROM 'hephaestus_app'
    OR NEW.operation IS DISTINCT FROM 'import_agent_with_volumes'
    OR NEW.aggregate_id IS DISTINCT FROM child.instance_id OR NEW.secondary_id IS DISTINCT FROM child.revision_id
    OR NEW.actor_id IS DISTINCT FROM header.actor_id OR NEW.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
    OR NEW.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
    OR NEW.input_hash IS DISTINCT FROM recipe_import107_input_hash(child,header.actor_id)
    OR NOT EXISTS(SELECT 1 FROM agent_instances WHERE id=child.instance_id AND active_revision_id=child.revision_id
       AND created_by=header.actor_id AND project_id=child.project_id AND volume_mode='named') THEN
   RAISE EXCEPTION 'prepared107 key must record its exact actual instance birth' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_recipe_instance_command() FROM PUBLIC;
CREATE TRIGGER recipe_instance_command_guard BEFORE INSERT ON release_command_inbox
 FOR EACH ROW EXECUTE FUNCTION guard_recipe_instance_command();
