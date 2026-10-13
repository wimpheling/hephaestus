-- Private qualified activation only. No Invocation, Run, lease, VM or provider IO.
CREATE TABLE agent_instance_execution_activations (
 id uuid PRIMARY KEY CHECK(id<>'00000000-0000-0000-0000-000000000000'),
 instance_id uuid NOT NULL UNIQUE REFERENCES agent_instances(id),
 revision_id uuid NOT NULL REFERENCES agent_instance_revisions(id),
 deployment_id uuid NOT NULL,
 resource_name text NOT NULL,
 import_command_key bytea NOT NULL REFERENCES release_command_inbox(command_key),
 command_key bytea NOT NULL UNIQUE CHECK(octet_length(command_key)=32),
 creation_version bigint NOT NULL CHECK(creation_version=1),
 activated_version bigint NOT NULL CHECK(activated_version=2),
 actor_id uuid NOT NULL REFERENCES users(id),
 request_id uuid NOT NULL CHECK(request_id<>'00000000-0000-0000-0000-000000000000'),
 config_json jsonb NOT NULL CHECK(jsonb_typeof(config_json)='object'),
 config_bytes bytea NOT NULL CHECK(octet_length(config_bytes) BETWEEN 1 AND 65536),
 config_hash bytea NOT NULL CHECK(config_hash=sha256(config_bytes)),
 input_hash bytea NOT NULL CHECK(octet_length(input_hash)=32),
 authorization_model_version text NOT NULL CHECK(octet_length(authorization_model_version) BETWEEN 1 AND 128),
 event_id uuid NOT NULL UNIQUE REFERENCES application_events(id),
 event_cursor bigint NOT NULL CHECK(event_cursor>0),
 event_aggregate_version bigint NOT NULL CHECK(event_aggregate_version>0),
 instance_event_id uuid NOT NULL UNIQUE REFERENCES agent_instance_events(id) DEFERRABLE INITIALLY DEFERRED,
 outbox_id uuid NOT NULL UNIQUE REFERENCES outbox(id) DEFERRABLE INITIALLY DEFERRED,
 FOREIGN KEY(deployment_id,resource_name) REFERENCES recipe_prepared_instance_imports(deployment_id,resource_name)
);
CREATE TABLE agent_instance_execution_activation_audits (
 activation_id uuid NOT NULL REFERENCES agent_instance_execution_activations(id),
 permission text NOT NULL CHECK(permission IN ('can_manage','can_use')),
 object_type text NOT NULL CHECK(object_type IN ('agent_instance','release_agent')),
 object_id uuid NOT NULL,
 audit_id uuid NOT NULL UNIQUE REFERENCES authorization_audit_events(id),
 PRIMARY KEY(activation_id,permission,object_type,object_id)
);
CREATE FUNCTION reject_instance_activation_rewrite() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN RAISE EXCEPTION 'activation lineage is immutable' USING ERRCODE='23000'; END $$;
CREATE TRIGGER instance_activation_immutable BEFORE UPDATE OR DELETE ON agent_instance_execution_activations
 FOR EACH ROW EXECUTE FUNCTION reject_instance_activation_rewrite();
CREATE TRIGGER instance_activation_audit_immutable BEFORE UPDATE OR DELETE ON agent_instance_execution_activation_audits
 FOR EACH ROW EXECUTE FUNCTION reject_instance_activation_rewrite();
ALTER TABLE agent_instance_execution_activations ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_execution_activations FORCE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_execution_activation_audits ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_execution_activation_audits FORCE ROW LEVEL SECURITY;
GRANT SELECT ON agent_instance_execution_activations,agent_instance_execution_activation_audits TO hephaestus_app,hephaestus_worker;
CREATE POLICY instance_activation_read ON agent_instance_execution_activations FOR SELECT TO hephaestus_app
 USING(check_permission('user',hephaestus_actor_id(),'can_read','agent_instance',instance_id::text)=1);
CREATE POLICY instance_activation_audit_read ON agent_instance_execution_activation_audits FOR SELECT TO hephaestus_app
 USING(EXISTS(SELECT 1 FROM agent_instance_execution_activations activation WHERE activation.id=activation_id));
CREATE POLICY instance_activation_worker_read ON agent_instance_execution_activations FOR SELECT TO hephaestus_worker USING(true);
CREATE POLICY instance_activation_audit_worker_read ON agent_instance_execution_activation_audits FOR SELECT TO hephaestus_worker USING(true);
-- No actor/worker INSERT/UPDATE grants: only the narrow protected operation writes.

-- Immutable historical lineage only: original creator authority is not borrowed.
CREATE FUNCTION qualified_recipe_instance_birth_matches(p_instance uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 instance agent_instances%ROWTYPE; revision agent_instance_revisions%ROWTYPE;
 command release_command_inbox%ROWTYPE; input jsonb; policy jsonb; expected_policy_hash bytea; actual_request uuid;
BEGIN
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 IF child.instance_id IS NULL THEN RETURN false; END IF;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id;
 SELECT * INTO instance FROM agent_instances WHERE id=child.instance_id;
 SELECT * INTO revision FROM agent_instance_revisions WHERE id=child.revision_id;
 SELECT * INTO command FROM release_command_inbox WHERE command_key=child.import_command_key;
 input:=convert_from(child.canonical_import,'UTF8')::jsonb; policy:=input->'source'->'selected_policy';
 actual_request:=command.request_id;
 -- insert_revision hashes RuntimePolicy as a struct (vcpus,memory_mib,network),
 -- unlike import_hash's json! Value. Do not substitute C-sorted object bytes.
 expected_policy_hash:=sha256(convert_to('{"vcpus":'||recipe_import107_json_bytes(policy->'vcpus',0)||
   ',"memory_mib":'||recipe_import107_json_bytes(policy->'memory_mib',0)||
   ',"network":'||recipe_import107_json_bytes(policy->'network',0)||'}','UTF8'));
 IF NOT EXISTS(SELECT 1 FROM recipe_deployment_commands original JOIN recipe_deployments deployment
       ON deployment.id=original.deployment_id WHERE original.id=header.command_id
       AND original.operation='install' AND original.actor_id=header.actor_id
       AND deployment.created_by=original.actor_id AND deployment.first_request_id=original.first_request_id
       AND original.receipt_version=0 AND original.input_hash=deployment.input_hash
       AND header.intent_hash=deployment.input_hash AND header.profile='runtime_named_v1'
       AND (SELECT count(*) FROM recipe_deployment_commands WHERE deployment_id=deployment.id AND operation='install')=1)
    OR command.command_key IS NULL OR command.operation IS DISTINCT FROM 'import_agent_with_volumes'
    OR command.aggregate_id IS DISTINCT FROM child.instance_id OR command.secondary_id IS DISTINCT FROM child.revision_id
    OR command.actor_id IS DISTINCT FROM header.actor_id OR command.request_id IS DISTINCT FROM actual_request
    OR command.input_hash IS DISTINCT FROM recipe_import107_input_hash(child,header.actor_id)
    OR instance.active_revision_id IS DISTINCT FROM child.revision_id OR instance.created_by IS DISTINCT FROM header.actor_id
    OR instance.project_id IS DISTINCT FROM child.project_id OR instance.name IS DISTINCT FROM input->>'name'
    OR instance.volume_mode IS DISTINCT FROM 'named'
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
   RETURN false;
 END IF;
 RETURN true;
END $$;
REVOKE ALL ON FUNCTION qualified_recipe_instance_birth_matches(uuid) FROM PUBLIC;

-- Protected106 completion is required in addition to actual107 SQL birth.
-- Immutable original claim/proof/transition is data, not the creator's authority.
CREATE FUNCTION qualified_recipe_instance_creation_completed(p_instance uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT EXISTS(SELECT 1 FROM recipe_prepared_instance_imports child
   JOIN recipe_deployment_execution_preparations header ON header.deployment_id=child.deployment_id
   JOIN recipe_deployment_resources resource ON resource.deployment_id=child.deployment_id AND resource.resource_name=child.resource_name
   JOIN recipe_effect_attempts attempt ON attempt.deployment_id=resource.deployment_id
     AND attempt.resource_name=resource.resource_name AND attempt.command_id=header.command_id
   JOIN recipe_effect_verifications proof ON proof.attempt_id=attempt.id
   JOIN recipe_execution_transitions transition ON transition.attempt_id=attempt.id AND transition.verification_id=proof.id
   WHERE child.instance_id=p_instance AND resource.resource_kind='instance' AND resource.resource_id=child.instance_id
     AND resource.install_progress='ready' AND resource.active_attempt_id IS NULL AND resource.last_verified_action='create'
     AND attempt.action='create' AND attempt.actor_id=header.actor_id AND attempt.project_id=child.project_id
     AND attempt.generation=resource.execution_generation AND attempt.input_hash=resource.input_hash
     AND attempt.claim_json=jsonb_build_object('command_id',attempt.command_id,'deployment_id',attempt.deployment_id,
       'resource',attempt.resource_name,'identity',resource.plan_json->'identity','input_hash',recipe_effect_hash_json(attempt.input_hash),
       'resource_version',attempt.claim_json->'resource_version','generation',attempt.generation,'action','create',
       'attempt_id',attempt.id,'actor_id',attempt.actor_id,'request_id',attempt.request_id)
     AND proof.recorded_by='hephaestus_worker' AND proof.outcome='applied' AND proof.diagnostic IS NULL
     AND proof.deployment_id=child.deployment_id AND proof.project_id=child.project_id AND proof.resource_name=child.resource_name
     AND proof.generation=attempt.generation AND proof.action='create' AND proof.identity_json=resource.plan_json->'identity'
     AND proof.input_hash=resource.input_hash AND transition.kind IN ('completed','reconciled')
     AND transition.operation_id=proof.verification_attempt_id
     AND transition.command_id=header.command_id AND transition.deployment_id=child.deployment_id
     AND transition.project_id=child.project_id AND transition.resource_name=child.resource_name
     AND transition.actor_id=header.actor_id AND transition.after_state=recipe_effect_resource_state(resource))
$$;
REVOKE ALL ON FUNCTION qualified_recipe_instance_creation_completed(uuid) FROM PUBLIC;

CREATE FUNCTION qualified_instance_activation_context(p_instance uuid)
RETURNS TABLE(original_import_key bytea,release_agent uuid,first_mutex integer,second_mutex integer)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; actor uuid;
BEGIN
 actor:=NULLIF(hephaestus_actor_id(),'')::uuid;
 IF current_setting('role',true) IS DISTINCT FROM 'hephaestus_app'
    OR actor IS NULL OR NOT EXISTS(SELECT 1 FROM users WHERE id=actor AND status='active')
    OR check_permission('user',actor::text,'can_manage','agent_instance',p_instance::text) IS DISTINCT FROM 1 THEN
   RAISE EXCEPTION 'current active instance manager required' USING ERRCODE='42501';
 END IF;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 IF child.instance_id IS NULL THEN RAISE EXCEPTION 'positive113 birth required' USING ERRCODE='P1140'; END IF;
 IF check_permission('user',actor::text,'can_use','release_agent',child.release_agent_id::text) IS DISTINCT FROM 1 THEN
   RAISE EXCEPTION 'current source use required' USING ERRCODE='42501';
 END IF;
 RETURN QUERY SELECT child.import_command_key,child.release_agent_id,mutex.first_key,mutex.second_key
   FROM recipe_import113_instance_mutex(p_instance) mutex;
END $$;
REVOKE ALL ON FUNCTION qualified_instance_activation_context(uuid) FROM PUBLIC,hephaestus_worker;
GRANT EXECUTE ON FUNCTION qualified_instance_activation_context(uuid) TO hephaestus_app;

CREATE FUNCTION qualified_activation_readiness(p_instance uuid,p_revision uuid,cfg jsonb) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE binding agent_instance_revision_volume_bindings%ROWTYPE; volume agent_instance_state_volumes%ROWTYPE;
 grant_row agent_instance_volume_mount_grants%ROWTYPE; purpose volume_owned_backing_purposes%ROWTYPE;
BEGIN
 FOR binding IN SELECT * FROM agent_instance_revision_volume_bindings WHERE instance_revision_id=p_revision LOOP
   SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=binding.volume_id;
   SELECT * INTO grant_row FROM agent_instance_volume_mount_grants
     WHERE instance_revision_id=p_revision AND slot_key=binding.slot_key;
   IF volume.id IS NULL OR grant_row.id IS NULL OR binding.instance_id IS DISTINCT FROM p_instance
      OR volume.project_id IS DISTINCT FROM binding.project_id OR volume.state IS DISTINCT FROM 'ready'
      OR volume.provisioning_state IS DISTINCT FROM 'ready' OR volume.filesystem_uuid IS NULL
      OR volume.host_id IS DISTINCT FROM cfg->'volume'->>'host_id'
      OR volume.host_path IS NULL OR left(volume.host_path,length(cfg->'volume'->>'root')+1) IS DISTINCT FROM (cfg->'volume'->>'root')||'/'
      OR volume.capacity_bytes<binding.minimum_capacity_bytes OR NOT owned_birth_is_live(volume.id)
      OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE volume_id=volume.id AND released_at IS NULL)
      OR NOT private_volume_mount_binding_is_declared(p_revision,binding.slot_key)
      OR grant_row.instance_id IS DISTINCT FROM p_instance OR grant_row.release_agent_id IS DISTINCT FROM binding.release_agent_id
      OR grant_row.volume_id IS DISTINCT FROM binding.volume_id OR grant_row.access_mode IS DISTINCT FROM binding.access_mode
      OR EXISTS(SELECT 1 FROM agent_instance_volume_mount_revocations WHERE grant_id=grant_row.id)
      OR NOT EXISTS(SELECT 1 FROM users WHERE id=grant_row.created_by AND status='active')
      OR private_volume_mount_source_is_live(grant_row.created_by,volume.id) IS DISTINCT FROM true
      OR check_permission('agent_instance',p_instance::text,'agent_attach','state_volume',volume.id::text) IS DISTINCT FROM 1 THEN
     RETURN false;
   END IF;
   IF EXISTS(SELECT 1 FROM volume_creation_seals WHERE volume_id=volume.id) THEN
     SELECT * INTO purpose FROM volume_owned_backing_purposes WHERE volume_id=volume.id;
     IF purpose.volume_id IS NULL OR purpose.host_id IS DISTINCT FROM cfg->'volume'->>'host_id'
        OR purpose.root_path IS DISTINCT FROM cfg->'volume'->>'root'
        OR purpose.owner_namespace::text IS DISTINCT FROM cfg->'volume'->>'namespace'
        OR purpose.canonical_path IS DISTINCT FROM volume.host_path
        OR NOT EXISTS(SELECT 1 FROM volume_owned_provisioning_operations operation
           JOIN volume_owned_backing_observations observation ON observation.operation_id=operation.id
           WHERE operation.volume_id=volume.id AND operation.generation=volume.provisioning_generation
             AND observation.volume_id=volume.id AND observation.generation=operation.generation AND observation.phase='ready') THEN
       RETURN false;
     END IF;
   END IF;
 END LOOP;
 RETURN true;
END $$;
REVOKE ALL ON FUNCTION qualified_activation_readiness(uuid,uuid,jsonb) FROM PUBLIC;

CREATE FUNCTION write_qualified_instance_activation(p_id uuid,p_key bytea,p_instance uuid,p_revision uuid,
 p_creation_version bigint,p_config bytea,p_model text)
RETURNS TABLE(activation uuid,instance uuid,revision uuid,version bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 consumer agent_instances%ROWTYPE; stored agent_instance_execution_activations%ROWTYPE;
 agent release_agents%ROWTYPE; cfg jsonb; actor uuid; request uuid; mutex record; lock_key bigint;
 input_hash bytea; event_id uuid; event_cursor bigint; event_version bigint; instance_event uuid; domain_event uuid;
 audit_id uuid; required record; organization uuid; selected record;
BEGIN
 PERFORM * FROM qualified_instance_activation_context(p_instance);
 actor:=hephaestus_actor_id()::uuid; request:=NULLIF(current_setting('hephaestus.request_id',true),'')::uuid;
 IF request IS NULL OR request='00000000-0000-0000-0000-000000000000'
    OR p_id IS NULL OR p_id='00000000-0000-0000-0000-000000000000' OR octet_length(p_key) IS DISTINCT FROM 32
    OR p_creation_version IS DISTINCT FROM 1 OR p_config IS NULL OR octet_length(p_config) NOT BETWEEN 1 AND 65536
    OR p_model IS NULL OR octet_length(p_model) NOT BETWEEN 1 AND 128 OR p_model<>btrim(p_model) OR p_model ~ '[[:cntrl:]]' THEN
   RAISE EXCEPTION 'invalid activation input' USING ERRCODE='P1143';
 END IF;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 -- Raw callers may already hold Project: TRY locks cannot introduce a cycle.
 -- The trusted adapter's fresh TX blocks on these same ordered keys before rows.
 FOR lock_key IN SELECT DISTINCT key FROM (VALUES(recipe_import107_command_mutex(p_key)),
      (recipe_import107_command_mutex(child.import_command_key))) keys(key) ORDER BY key LOOP
   IF pg_try_advisory_xact_lock(lock_key) IS DISTINCT FROM true THEN
     RAISE EXCEPTION 'activation command reservation held' USING ERRCODE='55P03';
   END IF;
 END LOOP;
 SELECT * INTO mutex FROM recipe_import113_instance_mutex(p_instance);
 IF pg_try_advisory_xact_lock(mutex.first_key,mutex.second_key) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'activation instance reservation held' USING ERRCODE='55P03';
 END IF;
 PERFORM 1 FROM projects WHERE id=child.project_id FOR NO KEY UPDATE;
 PERFORM 1 FROM recipe_deployments WHERE id=child.deployment_id FOR UPDATE;
 PERFORM 1 FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name FOR UPDATE;
 SELECT * INTO consumer FROM agent_instances WHERE id=p_instance FOR UPDATE NOWAIT;
 -- Late locks are ordered globally and nonblocking: existing lease/revocation
 -- writers may hold these resources before appending their Project event.
 FOR selected IN SELECT DISTINCT volume_id FROM agent_instance_revision_volume_bindings WHERE instance_revision_id=child.revision_id ORDER BY volume_id LOOP
   PERFORM 1 FROM agent_instance_state_volumes WHERE id=selected.volume_id FOR UPDATE NOWAIT;
 END LOOP;
 FOR selected IN SELECT id FROM agent_instance_volume_mount_grants WHERE instance_revision_id=child.revision_id ORDER BY id LOOP
   PERFORM 1 FROM agent_instance_volume_mount_grants WHERE id=selected.id FOR UPDATE NOWAIT;
 END LOOP;
 PERFORM * FROM qualified_instance_activation_context(p_instance);
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id;
 SELECT * INTO agent FROM release_agents WHERE id=child.release_agent_id;
 cfg:=convert_from(p_config,'UTF8')::jsonb;
 IF jsonb_typeof(cfg->'version') IS DISTINCT FROM 'number' OR cfg->>'version' IS DISTINCT FROM '1' OR cfg->>'profile' IS DISTINCT FROM 'runtime_named_v1'
    OR cfg->'platform' IS DISTINCT FROM header.platform_json
    OR jsonb_typeof(cfg->'vm'->'namespace') IS DISTINCT FROM 'string'
    OR cfg->'vm'->>'namespace' IN ('.','..')
    OR octet_length(cfg->'vm'->>'namespace') NOT BETWEEN 1 AND 128 OR (cfg->'vm'->>'namespace') !~ '^[A-Za-z0-9_.:-]+$'
    OR jsonb_typeof(cfg->'vm'->'host_id') IS DISTINCT FROM 'string'
    OR cfg->'vm'->>'host_id' IN ('.','..')
    OR octet_length(cfg->'vm'->>'host_id') NOT BETWEEN 1 AND 128 OR (cfg->'vm'->>'host_id') !~ '^[A-Za-z0-9_.:-]+$'
    OR cfg->'volume'->>'host_id' IS DISTINCT FROM cfg->'vm'->>'host_id'
    OR jsonb_typeof(cfg->'volume'->'namespace') IS DISTINCT FROM 'string'
    OR (cfg->'volume'->>'namespace')::uuid='00000000-0000-0000-0000-000000000000'
    OR jsonb_typeof(cfg->'volume'->'root') IS DISTINCT FROM 'string'
    OR octet_length(cfg->'volume'->>'root') NOT BETWEEN 2 AND 4096 OR left(cfg->'volume'->>'root',1)<>'/'
    OR (cfg->'volume'->>'root') ~ '[[:cntrl:]]'
    OR right(cfg->'volume'->>'root',1)='/' OR (cfg->'volume'->>'root') ~ '(^|/)\.{1,2}(/|$)|//' THEN
   RAISE EXCEPTION 'exact configured profile/platform/root required' USING ERRCODE='P1144';
 END IF;
 IF child.revision_id IS DISTINCT FROM p_revision OR consumer.active_revision_id IS DISTINCT FROM child.revision_id THEN
   RAISE EXCEPTION 'activation revision changed' USING ERRCODE='P1142';
 END IF;
 IF consumer.state IS DISTINCT FROM 'active' OR consumer.volume_mode IS DISTINCT FROM 'named'
    OR recipe_cleanup_started(child.deployment_id)
    OR EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=p_instance)
    OR EXISTS(SELECT 1 FROM agent_updates WHERE instance_id=p_instance AND state NOT IN ('activated','rejected','cancelled'))
    OR NOT qualified_recipe_instance_birth_matches(p_instance)
    OR NOT qualified_recipe_instance_creation_completed(p_instance)
    OR NOT EXISTS(SELECT 1 FROM releases WHERE id=child.release_id AND state='published')
    OR agent.runtime_contract_hash IS DISTINCT FROM sha256(convert_to(recipe_import107_json_bytes(agent.runtime_contract,0),'UTF8'))
    OR recipe_effect_hash_json(agent.runtime_contract_hash) IS DISTINCT FROM (convert_from(child.canonical_import,'UTF8')::jsonb)->'source'->'runtime_contract_hash'
    OR agent.runtime_contract->>'image_reference' IS DISTINCT FROM (convert_from(child.canonical_import,'UTF8')::jsonb)->'source'->>'image'
    OR NOT qualified_activation_readiness(p_instance,p_revision,cfg) THEN
   RAISE EXCEPTION 'instance birth/readiness/closure is not eligible' USING ERRCODE='P1141';
 END IF;
 input_hash:=sha256(convert_to(jsonb_build_object('version',1,'operation','activate_instance','actor',actor,
   'activation',p_id,'instance',p_instance,'revision',p_revision,'creation_version',1,'activated_version',2,
   'config_hash',encode(sha256(p_config),'hex'),'prepared_hash',encode(child.input_hash,'hex'))::text,'UTF8'));
 SELECT * INTO stored FROM agent_instance_execution_activations WHERE instance_id=p_instance;
 IF stored.id IS NOT NULL THEN
   IF stored.config_bytes IS DISTINCT FROM p_config THEN
     RAISE EXCEPTION 'recorded activation configuration changed' USING ERRCODE='P1144';
   END IF;
   IF stored.id IS DISTINCT FROM p_id OR stored.command_key IS DISTINCT FROM p_key OR stored.actor_id IS DISTINCT FROM actor
      OR stored.input_hash IS DISTINCT FROM input_hash THEN
     RAISE EXCEPTION 'activation identity/configuration conflicts' USING ERRCODE='P1143';
   END IF;
   IF consumer.version IS DISTINCT FROM 2 OR consumer.run_gate_open IS DISTINCT FROM true
      OR NOT qualified_instance_activation_record_matches(stored) THEN
     RAISE EXCEPTION 'activation replay cannot reopen or mutate instance' USING ERRCODE='P1141';
   END IF;
 ELSE
   IF consumer.version IS DISTINCT FROM 1 OR consumer.run_gate_open IS DISTINCT FROM false THEN
     RAISE EXCEPTION 'activation needs exact birth version1 closed gate' USING ERRCODE='P1142';
   END IF;
   IF EXISTS(SELECT 1 FROM release_command_inbox WHERE command_key=p_key) THEN
     RAISE EXCEPTION 'activation command key is already used' USING ERRCODE='P1143';
   END IF;
   SELECT organization_id INTO organization FROM projects WHERE id=child.project_id;
   SELECT event.event_id,event.cursor,event.aggregate_version INTO event_id,event_cursor,event_version
     FROM append_application_event(gen_random_uuid(),'agent_instance',p_instance,'agent_instance',p_instance,
       'agent_instance.changed','updated',NULL,organization,child.project_id) event;
   instance_event:=gen_random_uuid(); domain_event:=gen_random_uuid();
   INSERT INTO agent_instance_execution_activations(id,instance_id,revision_id,deployment_id,resource_name,import_command_key,
     command_key,creation_version,activated_version,actor_id,request_id,config_json,config_bytes,config_hash,input_hash,
     authorization_model_version,event_id,event_cursor,event_aggregate_version,instance_event_id,outbox_id)
   VALUES(p_id,p_instance,p_revision,child.deployment_id,child.resource_name,child.import_command_key,p_key,1,2,actor,request,
     cfg,p_config,sha256(p_config),input_hash,p_model,event_id,event_cursor,event_version,instance_event,domain_event);
   INSERT INTO release_command_inbox(command_key,operation,aggregate_id,secondary_id,actor_id,request_id,input_hash)
     VALUES(p_key,'activate_instance',p_instance,p_id,actor,request,input_hash);
   INSERT INTO agent_instance_events(id,instance_id,revision_id,event_type,actor_id,request_id,payload)
     VALUES(instance_event,p_instance,p_revision,'instance.activated',actor,request,
       jsonb_build_object('activation_id',p_id,'revision_id',p_revision,'activated_version',2,'gate_open',true,'invocation_supported',false));
   INSERT INTO outbox(id,aggregate_type,aggregate_id,subject,event_type,payload,occurred_at)
     VALUES(domain_event,'release',p_instance,'hephaestus.agent_instance.activated.v1','agent_instance.activated.v1',
       jsonb_build_object('activation_id',p_id,'instance_id',p_instance,'revision_id',p_revision,'activated_version',2,
         'gate_open',true,'invocation_supported',false,'schema_version',1,'message_id',domain_event,
         'idempotency_key',domain_event,'request_id',NULL,'trace_id',NULL),now());
   UPDATE agent_instances AS target SET run_gate_open=true,version=target.version+1,updated_at=now()
     WHERE target.id=p_instance AND target.version=1 AND target.run_gate_open=false;
 END IF;
 -- Replays audit REAL current middleware request; original receipt/event stay immutable.
 FOR required IN SELECT 'can_manage'::text permission,'agent_instance'::text object_type,p_instance object_id
    UNION ALL SELECT 'can_use','release_agent',child.release_agent_id LOOP
   IF check_permission('user',actor::text,required.permission,required.object_type,required.object_id::text) IS DISTINCT FROM 1
      OR NOT EXISTS(SELECT 1 FROM users WHERE id=actor AND status='active') THEN
     RAISE EXCEPTION 'activation authority changed before allow audit' USING ERRCODE='42501';
   END IF;
   INSERT INTO authorization_audit_events(id,actor_id,permission,object_type,object_id,decision,request_id,authorization_model_version)
     VALUES(gen_random_uuid(),actor,required.permission,required.object_type,required.object_id,'allow',request,p_model) RETURNING id INTO audit_id;
   IF stored.id IS NULL THEN
     INSERT INTO agent_instance_execution_activation_audits(activation_id,permission,object_type,object_id,audit_id)
       VALUES(p_id,required.permission,required.object_type,required.object_id,audit_id);
   END IF;
 END LOOP;
 RETURN QUERY SELECT p_id,p_instance,p_revision,2::bigint;
END $$;
REVOKE ALL ON FUNCTION write_qualified_instance_activation(uuid,bytea,uuid,uuid,bigint,bytea,text) FROM PUBLIC,hephaestus_worker;
GRANT EXECUTE ON FUNCTION write_qualified_instance_activation(uuid,bytea,uuid,uuid,bigint,bytea,text) TO hephaestus_app;

-- Add only the one-time qualified gate exception. Every ordinary work-admission
-- helper still consults the unchanged globally-false named dispatch predicate.
CREATE OR REPLACE FUNCTION enforce_instance_volume_lifecycle() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE admission agent_instance_execution_activations%ROWTYPE;
BEGIN
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
    AND (NEW.run_gate_open OR (TG_OP='UPDATE' AND NEW.active_revision_id IS DISTINCT FROM OLD.active_revision_id) OR NEW.state='removed') THEN
   RAISE EXCEPTION 'permanent closure forbids reopening or unproven tombstone' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_instance_volume_lifecycle() FROM PUBLIC;

CREATE FUNCTION qualified_instance_activation_record_matches(receipt agent_instance_execution_activations) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE consumer agent_instances%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
BEGIN
 SELECT * INTO consumer FROM agent_instances WHERE id=receipt.instance_id;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE deployment_id=receipt.deployment_id AND resource_name=receipt.resource_name;
 IF consumer.version IS DISTINCT FROM 2 OR consumer.run_gate_open IS DISTINCT FROM true
    OR consumer.active_revision_id IS DISTINCT FROM receipt.revision_id OR consumer.state IS DISTINCT FROM 'active'
    OR child.instance_id IS DISTINCT FROM receipt.instance_id OR child.revision_id IS DISTINCT FROM receipt.revision_id
    OR child.import_command_key IS DISTINCT FROM receipt.import_command_key
    OR receipt.config_json IS DISTINCT FROM convert_from(receipt.config_bytes,'UTF8')::jsonb
    OR recipe_cleanup_started(receipt.deployment_id) OR NOT qualified_recipe_instance_birth_matches(receipt.instance_id)
    OR NOT qualified_recipe_instance_creation_completed(receipt.instance_id)
    OR NOT qualified_activation_readiness(receipt.instance_id,receipt.revision_id,receipt.config_json)
    OR NOT EXISTS(SELECT 1 FROM release_agents source JOIN releases published ON published.id=source.release_id
       WHERE source.id=child.release_agent_id AND published.id=child.release_id AND published.state='published'
         AND source.runtime_contract_hash=sha256(convert_to(recipe_import107_json_bytes(source.runtime_contract,0),'UTF8'))
         AND recipe_effect_hash_json(source.runtime_contract_hash)=(convert_from(child.canonical_import,'UTF8')::jsonb)->'source'->'runtime_contract_hash'
         AND source.runtime_contract->>'image_reference'=(convert_from(child.canonical_import,'UTF8')::jsonb)->'source'->>'image')
    OR NOT EXISTS(SELECT 1 FROM users WHERE id=receipt.actor_id AND status='active')
    OR check_permission('user',receipt.actor_id::text,'can_manage','agent_instance',receipt.instance_id::text) IS DISTINCT FROM 1
    OR check_permission('user',receipt.actor_id::text,'can_use','release_agent',child.release_agent_id::text) IS DISTINCT FROM 1
    OR NOT EXISTS(SELECT 1 FROM release_command_inbox command WHERE command.command_key=receipt.command_key
       AND command.operation='activate_instance' AND command.aggregate_id=receipt.instance_id AND command.secondary_id=receipt.id
       AND command.actor_id=receipt.actor_id AND command.request_id=receipt.request_id AND command.input_hash=receipt.input_hash)
    OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox product ON product.event_id=event.id
       WHERE event.id=receipt.event_id AND event.cursor=receipt.event_cursor AND event.aggregate_version=receipt.event_aggregate_version
       AND event.aggregate_type='agent_instance' AND event.aggregate_id=receipt.instance_id
       AND event.scope_kind='agent_instance' AND event.scope_id=receipt.instance_id
       AND event.event_type='agent_instance.changed' AND event.actor_type='user'
       AND event.actor_id=receipt.actor_id AND event.request_id=receipt.request_id)
    OR (SELECT count(*) FROM agent_instance_execution_activation_audits WHERE activation_id=receipt.id)<>2
    OR EXISTS(SELECT 1 FROM agent_instance_execution_activation_audits link
       LEFT JOIN authorization_audit_events audit ON audit.id=link.audit_id
       WHERE link.activation_id=receipt.id AND (audit.id IS NULL OR audit.actor_id IS DISTINCT FROM receipt.actor_id
          OR audit.permission IS DISTINCT FROM link.permission OR audit.object_type IS DISTINCT FROM link.object_type
          OR audit.object_id IS DISTINCT FROM link.object_id OR audit.decision IS DISTINCT FROM 'allow'
          OR audit.request_id IS DISTINCT FROM receipt.request_id OR audit.authorization_model_version IS DISTINCT FROM receipt.authorization_model_version
          OR NOT ((link.permission='can_manage' AND link.object_type='agent_instance' AND link.object_id=receipt.instance_id)
               OR (link.permission='can_use' AND link.object_type='release_agent' AND link.object_id=child.release_agent_id))))
    OR NOT EXISTS(SELECT 1 FROM agent_instance_events event WHERE event.id=receipt.instance_event_id
       AND event.instance_id=receipt.instance_id AND event.revision_id=receipt.revision_id AND event.update_id IS NULL
       AND event.actor_id=receipt.actor_id AND event.request_id=receipt.request_id AND event.event_type='instance.activated'
       AND event.payload=jsonb_build_object('activation_id',receipt.id,'revision_id',receipt.revision_id,'activated_version',2,'gate_open',true,'invocation_supported',false))
    OR NOT EXISTS(SELECT 1 FROM outbox event WHERE event.id=receipt.outbox_id AND event.aggregate_type='release'
       AND event.aggregate_id=receipt.instance_id AND event.subject='hephaestus.agent_instance.activated.v1'
       AND event.event_type='agent_instance.activated.v1'
       AND event.payload=jsonb_build_object('activation_id',receipt.id,'instance_id',receipt.instance_id,'revision_id',receipt.revision_id,'activated_version',2,
         'gate_open',true,'invocation_supported',false,'schema_version',1,'message_id',receipt.outbox_id,
         'idempotency_key',receipt.outbox_id,'request_id',NULL,'trace_id',NULL)) THEN
   RETURN false;
 END IF;
 RETURN true;
END $$;
REVOKE ALL ON FUNCTION qualified_instance_activation_record_matches(agent_instance_execution_activations) FROM PUBLIC;

CREATE FUNCTION validate_qualified_instance_activation_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF NOT qualified_instance_activation_record_matches(NEW) THEN
   RAISE EXCEPTION 'activation must commit exact command, audits, gate and both outboxes' USING ERRCODE='23000';
 END IF;
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_qualified_instance_activation_commit() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER qualified_instance_activation_commit AFTER INSERT ON agent_instance_execution_activations
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_qualified_instance_activation_commit();

CREATE POLICY qualified_activation_command_read ON release_command_inbox FOR SELECT TO hephaestus_app
 USING(operation='activate_instance' AND actor_id=hephaestus_actor_id()::uuid
   AND check_permission('user',hephaestus_actor_id(),'can_read','agent_instance',aggregate_id::text)=1);
CREATE POLICY qualified_activation_command_create ON release_command_inbox FOR INSERT TO hephaestus_app
 WITH CHECK(operation='activate_instance' AND actor_id=hephaestus_actor_id()::uuid
   AND request_id=NULLIF(current_setting('hephaestus.request_id',true),'')::uuid
   AND EXISTS(SELECT 1 FROM agent_instance_execution_activations activation
      WHERE activation.command_key=release_command_inbox.command_key AND activation.id=secondary_id
        AND activation.instance_id=aggregate_id AND activation.actor_id=actor_id
        AND activation.request_id=release_command_inbox.request_id AND activation.input_hash=release_command_inbox.input_hash));
CREATE POLICY qualified_activation_event_create ON agent_instance_events FOR INSERT TO hephaestus_app
 WITH CHECK(event_type='instance.activated' AND actor_id=hephaestus_actor_id()::uuid
   AND request_id=NULLIF(current_setting('hephaestus.request_id',true),'')::uuid
   AND EXISTS(SELECT 1 FROM agent_instance_execution_activations activation
      WHERE activation.instance_event_id=agent_instance_events.id AND activation.instance_id=instance_id
        AND activation.revision_id=agent_instance_events.revision_id AND activation.actor_id=agent_instance_events.actor_id
        AND activation.request_id=agent_instance_events.request_id));
