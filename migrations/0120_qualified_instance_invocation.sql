-- PRIVATE115: qualified Invocation admission only; not runtime activation.
-- Apply after the reviewed113 and114. Normal/Update retain their old semantics.
-- No Git aliases, overrides, historical identity/session reconstruction or IO.
ALTER TABLE runs DROP CONSTRAINT runs_run_kind_check;
ALTER TABLE runs ADD CONSTRAINT runs_run_kind_check CHECK(run_kind IN ('normal','update','invocation'));
DO $$ DECLARE attachment_check text;
BEGIN
 -- Replace only the exact old two-column kind/attachment shape, not any fence.
 SELECT conname INTO STRICT attachment_check FROM pg_constraint
   WHERE conrelid='runs'::regclass AND contype='c' AND cardinality(conkey)=2
     AND conkey @> ARRAY[(SELECT attnum FROM pg_attribute WHERE attrelid='runs'::regclass AND attname='run_kind'),
                        (SELECT attnum FROM pg_attribute WHERE attrelid='runs'::regclass AND attname='attachment_id')]::smallint[];
 EXECUTE format('ALTER TABLE runs DROP CONSTRAINT %I',attachment_check);
END $$;
ALTER TABLE runs ADD CONSTRAINT run_kind_attachment_shape CHECK(
 (run_kind='normal' AND attachment_id IS NOT NULL)
 OR (run_kind IN ('update','invocation') AND attachment_id IS NULL));

CREATE TABLE agent_instance_execution_invocations (
 id uuid PRIMARY KEY CHECK(id<>'00000000-0000-0000-0000-000000000000'),
 command_key bytea NOT NULL UNIQUE CHECK(octet_length(command_key)=32),
 actor_id uuid NOT NULL REFERENCES users(id),
 request_id uuid NOT NULL CHECK(request_id<>'00000000-0000-0000-0000-000000000000'),
 instance_id uuid NOT NULL REFERENCES agent_instances(id),
 revision_id uuid NOT NULL REFERENCES agent_instance_revisions(id),
 activation_id uuid NOT NULL REFERENCES agent_instance_execution_activations(id),
 deployment_id uuid NOT NULL, resource_name text NOT NULL,
 release_id uuid NOT NULL REFERENCES releases(id),
 release_agent_id uuid NOT NULL REFERENCES release_agents(id),
 project_id uuid NOT NULL REFERENCES projects(id),
 run_id uuid NOT NULL UNIQUE CHECK(run_id<>'00000000-0000-0000-0000-000000000000') REFERENCES runs(id) DEFERRABLE INITIALLY DEFERRED,
 start_command_id uuid NOT NULL UNIQUE CHECK(start_command_id<>'00000000-0000-0000-0000-000000000000') REFERENCES command_inbox(command_id) DEFERRABLE INITIALLY DEFERRED,
 instance_version bigint NOT NULL CHECK(instance_version=2),
 admission_version integer NOT NULL CHECK(admission_version=115),
 parameters jsonb NOT NULL CHECK(jsonb_typeof(parameters)='object'),
 parameter_hash bytea NOT NULL CHECK(octet_length(parameter_hash)=32),
 effective_policy jsonb NOT NULL CHECK(jsonb_typeof(effective_policy)='object'),
 effective_policy_hash bytea NOT NULL CHECK(octet_length(effective_policy_hash)=32),
 platform_policy_version text NOT NULL CHECK(octet_length(platform_policy_version) BETWEEN 1 AND 128),
 requires_state boolean NOT NULL,
 prepared_hash bytea NOT NULL CHECK(octet_length(prepared_hash)=32),
 config_json jsonb NOT NULL CHECK(jsonb_typeof(config_json)='object'),
 config_bytes bytea NOT NULL CHECK(octet_length(config_bytes) BETWEEN 1 AND 65536),
 config_hash bytea NOT NULL CHECK(config_hash=sha256(config_bytes)),
 input_hash bytea NOT NULL CHECK(octet_length(input_hash)=32),
 authorization_model_version text NOT NULL CHECK(octet_length(authorization_model_version) BETWEEN 1 AND 128),
 event_id uuid NOT NULL UNIQUE REFERENCES application_events(id),
 event_cursor bigint NOT NULL CHECK(event_cursor>0),
 event_aggregate_version bigint NOT NULL CHECK(event_aggregate_version>0),
 start_outbox_id uuid NOT NULL UNIQUE REFERENCES outbox(id) DEFERRABLE INITIALLY DEFERRED,
 FOREIGN KEY(deployment_id,resource_name) REFERENCES recipe_prepared_instance_imports(deployment_id,resource_name)
);
CREATE TABLE agent_instance_execution_invocation_audits (
 invocation_id uuid NOT NULL REFERENCES agent_instance_execution_invocations(id),
 permission text NOT NULL CHECK(permission IN ('can_execute','can_use')),
 object_type text NOT NULL CHECK(object_type IN ('agent_instance','release_agent')),
 object_id uuid NOT NULL,
 audit_id uuid NOT NULL UNIQUE REFERENCES authorization_audit_events(id),
 PRIMARY KEY(invocation_id,permission,object_type,object_id)
);
CREATE FUNCTION reject_instance_invocation_rewrite() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN RAISE EXCEPTION 'Invocation provenance is immutable' USING ERRCODE='23000'; END $$;
REVOKE ALL ON FUNCTION reject_instance_invocation_rewrite() FROM PUBLIC;
CREATE TRIGGER instance_invocation_immutable BEFORE UPDATE OR DELETE ON agent_instance_execution_invocations
 FOR EACH ROW EXECUTE FUNCTION reject_instance_invocation_rewrite();
CREATE TRIGGER instance_invocation_audit_immutable BEFORE UPDATE OR DELETE ON agent_instance_execution_invocation_audits
 FOR EACH ROW EXECUTE FUNCTION reject_instance_invocation_rewrite();
ALTER TABLE agent_instance_execution_invocations ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_execution_invocations FORCE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_execution_invocation_audits ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_execution_invocation_audits FORCE ROW LEVEL SECURITY;
GRANT SELECT ON agent_instance_execution_invocations,agent_instance_execution_invocation_audits TO hephaestus_app,hephaestus_worker;
CREATE POLICY instance_invocation_read ON agent_instance_execution_invocations FOR SELECT TO hephaestus_app
 USING(check_permission('user',hephaestus_actor_id(),'can_read','agent_instance',instance_id::text)=1);
CREATE POLICY instance_invocation_audit_read ON agent_instance_execution_invocation_audits FOR SELECT TO hephaestus_app
 USING(EXISTS(SELECT 1 FROM agent_instance_execution_invocations receipt WHERE receipt.id=invocation_id));
CREATE POLICY instance_invocation_worker_read ON agent_instance_execution_invocations FOR SELECT TO hephaestus_worker USING(true);
CREATE POLICY instance_invocation_audit_worker_read ON agent_instance_execution_invocation_audits FOR SELECT TO hephaestus_worker USING(true);
-- Neither actor nor worker receives raw DML; no worker admission/minting port.

-- Immutable producer correlation is independent of later gate, issuer status,
-- lease presence, resource generation/progress, names and Source permissions.
-- The immutable protected106 after-state is rederived with its original inputs.
CREATE FUNCTION qualified_invocation_import_lineage_matches(p_instance uuid) RETURNS boolean
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
    OR instance.created_by IS DISTINCT FROM header.actor_id
    OR instance.project_id IS DISTINCT FROM child.project_id
    OR instance.volume_mode IS DISTINCT FROM 'named'
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
REVOKE ALL ON FUNCTION qualified_invocation_import_lineage_matches(uuid) FROM PUBLIC;

CREATE FUNCTION qualified_invocation_creation_lineage_matches(p_instance uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT EXISTS(SELECT 1 FROM recipe_prepared_instance_imports child
   JOIN recipe_deployment_execution_preparations header ON header.deployment_id=child.deployment_id
   JOIN recipe_deployment_resources resource ON resource.deployment_id=child.deployment_id AND resource.resource_name=child.resource_name
   JOIN recipe_effect_attempts attempt ON attempt.deployment_id=resource.deployment_id
     AND attempt.resource_name=resource.resource_name AND attempt.command_id=header.command_id
   JOIN recipe_effect_verifications proof ON proof.attempt_id=attempt.id
   JOIN recipe_execution_transitions transition ON transition.attempt_id=attempt.id AND transition.verification_id=proof.id
   WHERE child.instance_id=p_instance AND resource.resource_kind='instance' AND resource.resource_id=child.instance_id
     AND attempt.action='create' AND attempt.actor_id=header.actor_id AND attempt.project_id=child.project_id
     AND attempt.input_hash=resource.input_hash
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
     AND transition.actor_id=header.actor_id
     -- These are the exact106 projection keys, not current resource progress.
     AND transition.after_state @> jsonb_build_object('install','ready','active_attempt',NULL,
          'diagnostic',NULL,'last_action','create','generation',attempt.generation)
     AND transition.after_state=recipe_effect_expected_state(transition.before_state,attempt.before_state,
          'create',transition.kind,'applied',NULL,attempt.id))
$$;
REVOKE ALL ON FUNCTION qualified_invocation_creation_lineage_matches(uuid) FROM PUBLIC;

-- Original114 issuer and allow audits are historical DATA, not current grants.
CREATE FUNCTION qualified_invocation_activation_lineage_matches(receipt agent_instance_execution_activations) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE;
BEGIN
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE deployment_id=receipt.deployment_id AND resource_name=receipt.resource_name;
 IF child.instance_id IS DISTINCT FROM receipt.instance_id OR child.revision_id IS DISTINCT FROM receipt.revision_id
    OR child.import_command_key IS DISTINCT FROM receipt.import_command_key
    OR receipt.config_json IS DISTINCT FROM convert_from(receipt.config_bytes,'UTF8')::jsonb
    OR qualified_invocation_import_lineage_matches(receipt.instance_id) IS DISTINCT FROM true
    OR qualified_invocation_creation_lineage_matches(receipt.instance_id) IS DISTINCT FROM true
    OR receipt.creation_version IS DISTINCT FROM 1 OR receipt.activated_version IS DISTINCT FROM 2
    OR receipt.config_hash IS DISTINCT FROM sha256(receipt.config_bytes)
    OR receipt.input_hash IS DISTINCT FROM sha256(convert_to(jsonb_build_object('version',1,'operation','activate_instance',
       'actor',receipt.actor_id,'activation',receipt.id,'instance',receipt.instance_id,'revision',receipt.revision_id,
       'creation_version',1,'activated_version',2,'config_hash',encode(receipt.config_hash,'hex'),
       'prepared_hash',encode(child.input_hash,'hex'))::text,'UTF8'))
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
REVOKE ALL ON FUNCTION qualified_invocation_activation_lineage_matches(agent_instance_execution_activations) FROM PUBLIC;

CREATE FUNCTION qualified_instance_invocation_context(p_instance uuid)
RETURNS TABLE(original_import_key bytea,release_agent uuid,first_mutex integer,second_mutex integer)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; actor uuid;
BEGIN
 actor:=NULLIF(hephaestus_actor_id(),'')::uuid;
 IF current_setting('role',true) IS DISTINCT FROM 'hephaestus_app' OR actor IS NULL
    OR NOT EXISTS(SELECT 1 FROM users WHERE id=actor AND status='active')
    OR check_permission('user',actor::text,'can_execute','agent_instance',p_instance::text) IS DISTINCT FROM 1 THEN
   RAISE EXCEPTION 'fresh active instance executor required' USING ERRCODE='42501';
 END IF;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 IF child.instance_id IS NULL THEN RAISE EXCEPTION 'positive113 birth required' USING ERRCODE='P1150'; END IF;
 IF check_permission('user',actor::text,'can_use','release_agent',child.release_agent_id::text) IS DISTINCT FROM 1 THEN
   RAISE EXCEPTION 'current exact source use required' USING ERRCODE='42501';
 END IF;
 RETURN QUERY SELECT child.import_command_key,child.release_agent_id,mutex.first_key,mutex.second_key
   FROM recipe_import113_instance_mutex(p_instance) mutex;
END $$;
REVOKE ALL ON FUNCTION qualified_instance_invocation_context(uuid) FROM PUBLIC,hephaestus_worker;
GRANT EXECUTE ON FUNCTION qualified_instance_invocation_context(uuid) TO hephaestus_app;

CREATE FUNCTION qualified_invocation_current_eligible(p_instance uuid,p_revision uuid,p_config jsonb) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT EXISTS(SELECT 1 FROM agent_instances consumer
   JOIN recipe_prepared_instance_imports child ON child.instance_id=consumer.id
   JOIN release_agents source ON source.id=child.release_agent_id AND source.release_id=child.release_id
   JOIN releases published ON published.id=source.release_id
   WHERE consumer.id=p_instance AND child.revision_id=p_revision
     AND consumer.version=2 AND consumer.run_gate_open AND consumer.state='active'
     AND consumer.active_revision_id=p_revision AND consumer.volume_mode='named'
     AND recipe_cleanup_started(child.deployment_id) IS DISTINCT FROM true
     AND qualified_recipe_instance_birth_matches(p_instance)
     AND qualified_recipe_instance_creation_completed(p_instance)
     AND qualified_activation_readiness(p_instance,p_revision,p_config)
     AND published.state='published'
     AND source.runtime_contract_hash=sha256(convert_to(recipe_import107_json_bytes(source.runtime_contract,0),'UTF8'))
     AND recipe_effect_hash_json(source.runtime_contract_hash)=(convert_from(child.canonical_import,'UTF8')::jsonb)->'source'->'runtime_contract_hash'
     AND source.runtime_contract->>'image_reference'=(convert_from(child.canonical_import,'UTF8')::jsonb)->'source'->>'image')
$$;
REVOKE ALL ON FUNCTION qualified_invocation_current_eligible(uuid,uuid,jsonb) FROM PUBLIC;

CREATE FUNCTION qualified_invocation_start_json(receipt agent_instance_execution_invocations) RETURNS jsonb
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT jsonb_build_object('command_id',receipt.start_command_id,'run_id',receipt.run_id,
   'instance_id',receipt.instance_id,'instance_revision_id',receipt.revision_id,
   'release_id',receipt.release_id,'release_agent_id',receipt.release_agent_id,
   'attachment_id',NULL,'kind','invocation','requires_state',receipt.requires_state)
$$;
REVOKE ALL ON FUNCTION qualified_invocation_start_json(agent_instance_execution_invocations) FROM PUBLIC;

CREATE FUNCTION qualified_invocation_run_matches(receipt agent_instance_execution_invocations,execution runs) RETURNS boolean
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT receipt.id IS NOT NULL AND execution.id IS NOT NULL
   AND ROW(execution.id,execution.command_id,execution.instance_id,execution.instance_revision_id,
       execution.release_id,execution.release_agent_id,execution.attachment_id,execution.run_kind,execution.requires_state,
       execution.execution_profile,execution.vm_id,execution.vm_provider_namespace,execution.vm_host_id,
       execution.vm_binding_kind,execution.vm_plan_admission_required)
     IS NOT DISTINCT FROM ROW(receipt.run_id,receipt.start_command_id,receipt.instance_id,receipt.revision_id,
       receipt.release_id,receipt.release_agent_id,NULL::uuid,'invocation'::text,receipt.requires_state,
       'owned_canonical'::text,receipt.run_id::text,receipt.config_json->'vm'->>'namespace',receipt.config_json->'vm'->>'host_id',
       'planned'::text,false)
$$;
REVOKE ALL ON FUNCTION qualified_invocation_run_matches(agent_instance_execution_invocations,runs) FROM PUBLIC;

CREATE FUNCTION qualified_invocation_graph_matches(receipt agent_instance_execution_invocations) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; revision agent_instance_revisions%ROWTYPE;
 activation agent_instance_execution_activations%ROWTYPE; execution runs%ROWTYPE;
BEGIN
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE deployment_id=receipt.deployment_id AND resource_name=receipt.resource_name;
 SELECT * INTO revision FROM agent_instance_revisions WHERE id=receipt.revision_id;
 SELECT * INTO activation FROM agent_instance_execution_activations WHERE id=receipt.activation_id;
 SELECT * INTO execution FROM runs WHERE id=receipt.run_id;
 IF child.instance_id IS DISTINCT FROM receipt.instance_id OR child.revision_id IS DISTINCT FROM receipt.revision_id
    OR child.project_id IS DISTINCT FROM receipt.project_id OR child.release_id IS DISTINCT FROM receipt.release_id
    OR child.release_agent_id IS DISTINCT FROM receipt.release_agent_id OR child.input_hash IS DISTINCT FROM receipt.prepared_hash
    OR receipt.admission_version IS DISTINCT FROM 115 OR receipt.input_hash IS DISTINCT FROM qualified_invocation_input_hash(receipt)
    OR activation.instance_id IS DISTINCT FROM receipt.instance_id OR activation.revision_id IS DISTINCT FROM receipt.revision_id
    OR activation.config_bytes IS DISTINCT FROM receipt.config_bytes OR activation.config_json IS DISTINCT FROM receipt.config_json
    OR receipt.config_json IS DISTINCT FROM convert_from(receipt.config_bytes,'UTF8')::jsonb
    OR NOT qualified_invocation_activation_lineage_matches(activation)
    OR ROW(receipt.parameters,receipt.parameter_hash,receipt.effective_policy,receipt.effective_policy_hash,receipt.platform_policy_version)
       IS DISTINCT FROM ROW(revision.parameters,revision.parameter_hash,revision.effective_runtime_policy,revision.effective_policy_hash,revision.platform_policy_version)
    OR qualified_invocation_run_matches(receipt,execution) IS DISTINCT FROM true
    OR NOT EXISTS(SELECT 1 FROM release_agents agent WHERE agent.id=receipt.release_agent_id AND agent.requires_state=receipt.requires_state)
    OR EXISTS(SELECT 1 FROM run_requests WHERE run_id=receipt.run_id)
    OR EXISTS(SELECT 1 FROM mailbox_delivery_attempts WHERE run_id=receipt.run_id)
    OR EXISTS(SELECT 1 FROM agent_updates WHERE hook_run_id=receipt.run_id)
    OR EXISTS(SELECT 1 FROM run_instance_provenance WHERE run_id=receipt.run_id)
    OR NOT EXISTS(SELECT 1 FROM command_inbox inbox WHERE inbox.command_id=receipt.start_command_id
       AND inbox.command_type='start_run' AND inbox.payload=qualified_invocation_start_json(receipt))
    OR NOT EXISTS(SELECT 1 FROM release_command_inbox command WHERE command.command_key=receipt.command_key
       AND command.operation='invoke_instance' AND command.aggregate_id=receipt.instance_id AND command.secondary_id=receipt.run_id
       AND command.actor_id=receipt.actor_id AND command.request_id=receipt.request_id AND command.input_hash=receipt.input_hash)
    OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox product ON product.event_id=event.id
       WHERE event.id=receipt.event_id AND event.cursor=receipt.event_cursor AND event.aggregate_version=receipt.event_aggregate_version
       AND event.scope_kind='run' AND event.scope_id=receipt.run_id AND event.aggregate_type='run' AND event.aggregate_id=receipt.run_id
       AND event.event_type='run.changed' AND event.change_kind='created' AND event.safe_state='queued'
       AND event.related_id_one=receipt.project_id AND event.related_id_two IS NULL
       AND event.actor_type='user' AND event.actor_id=receipt.actor_id AND event.request_id=receipt.request_id)
    OR NOT EXISTS(SELECT 1 FROM outbox start WHERE start.id=receipt.start_outbox_id AND start.aggregate_type='release'
       AND start.aggregate_id=receipt.run_id AND start.subject='hephaestus.run.start' AND start.event_type='run.start.v1'
       AND start.payload=qualified_invocation_start_json(receipt)||jsonb_build_object('schema_version',1,'message_id',start.id,
          'idempotency_key',start.id,'request_id',NULL,'trace_id',NULL))
    OR (SELECT count(*) FROM agent_instance_execution_invocation_audits WHERE invocation_id=receipt.id)<>2
    OR EXISTS(SELECT 1 FROM agent_instance_execution_invocation_audits link
       LEFT JOIN authorization_audit_events audit ON audit.id=link.audit_id WHERE link.invocation_id=receipt.id
       AND (audit.id IS NULL OR audit.actor_id IS DISTINCT FROM receipt.actor_id OR audit.permission IS DISTINCT FROM link.permission
         OR audit.object_type IS DISTINCT FROM link.object_type OR audit.object_id IS DISTINCT FROM link.object_id
         OR audit.decision IS DISTINCT FROM 'allow' OR audit.request_id IS DISTINCT FROM receipt.request_id
         OR audit.authorization_model_version IS DISTINCT FROM receipt.authorization_model_version
         OR NOT((link.permission='can_execute' AND link.object_type='agent_instance' AND link.object_id=receipt.instance_id)
            OR (link.permission='can_use' AND link.object_type='release_agent' AND link.object_id=receipt.release_agent_id)))) THEN
   RETURN false;
 END IF;
 RETURN true;
END $$;
REVOKE ALL ON FUNCTION qualified_invocation_graph_matches(agent_instance_execution_invocations) FROM PUBLIC;

CREATE FUNCTION qualified_invocation_queued_birth_matches(candidate runs) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE receipt agent_instance_execution_invocations%ROWTYPE; activation agent_instance_execution_activations%ROWTYPE;
BEGIN
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=candidate.id;
 SELECT * INTO activation FROM agent_instance_execution_activations WHERE id=receipt.activation_id;
 RETURN current_setting('role',true)='hephaestus_app' AND receipt.id IS NOT NULL
   AND receipt.actor_id::text IS NOT DISTINCT FROM hephaestus_actor_id()
   AND receipt.request_id IS NOT DISTINCT FROM NULLIF(current_setting('hephaestus.request_id',true),'')::uuid
   AND EXISTS(SELECT 1 FROM users WHERE id=receipt.actor_id AND status='active')
   AND check_permission('user',receipt.actor_id::text,'can_execute','agent_instance',receipt.instance_id::text)=1
   AND check_permission('user',receipt.actor_id::text,'can_use','release_agent',receipt.release_agent_id::text)=1
   AND qualified_invocation_activation_lineage_matches(activation)
   AND qualified_invocation_current_eligible(receipt.instance_id,receipt.revision_id,receipt.config_json)
   AND qualified_invocation_run_matches(receipt,candidate)
   AND candidate.state='queued' AND candidate.state_version=0 AND candidate.cancel_requested_at IS NULL
   AND candidate.volume_acquisition_closed_at IS NULL AND candidate.volume_selection_snapshot IS NULL
   AND num_nonnulls(candidate.volume_id,candidate.lease_id,candidate.lease_fencing_token)=0;
END $$;
REVOKE ALL ON FUNCTION qualified_invocation_queued_birth_matches(runs) FROM PUBLIC;
CREATE FUNCTION qualified_invocation_input_hash(receipt agent_instance_execution_invocations) RETURNS bytea
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT sha256(convert_to(jsonb_build_object('version',1,'operation','invoke_instance','actor',receipt.actor_id,
   'invocation',receipt.id,'instance',receipt.instance_id,'revision',receipt.revision_id,'activation',receipt.activation_id,
   'run',receipt.run_id,'start_command',receipt.start_command_id,'instance_version',2,'admission_version',receipt.admission_version,
   'prepared_hash',encode(receipt.prepared_hash,'hex'),'config_hash',encode(receipt.config_hash,'hex'),
   'parameter_hash',encode(receipt.parameter_hash,'hex'),'effective_policy_hash',encode(receipt.effective_policy_hash,'hex'),
   'platform_policy_version',receipt.platform_policy_version)::text,'UTF8'))
$$;
REVOKE ALL ON FUNCTION qualified_invocation_input_hash(agent_instance_execution_invocations) FROM PUBLIC;

CREATE FUNCTION write_qualified_instance_invocation(p_id uuid,p_key bytea,p_run uuid,p_start uuid,
 p_instance uuid,p_revision uuid,p_config bytea,p_model text)
RETURNS TABLE(invocation uuid,instance uuid,revision uuid,run uuid,start_command uuid)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; saved_revision agent_instance_revisions%ROWTYPE;
 consumer agent_instances%ROWTYPE; activation agent_instance_execution_activations%ROWTYPE;
 saved agent_instance_execution_invocations%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE;
 actor uuid; request uuid; mutex record; lock_key bigint; selected record; required record; audit_id uuid;
BEGIN
 PERFORM * FROM qualified_instance_invocation_context(p_instance);
 actor:=NULLIF(hephaestus_actor_id(),'')::uuid;
 request:=NULLIF(current_setting('hephaestus.request_id',true),'')::uuid;
 IF p_id IS NULL OR p_run IS NULL OR p_start IS NULL OR request IS NULL
    OR '00000000-0000-0000-0000-000000000000'::uuid IN (p_id,p_run,p_start,request)
    OR p_key IS NULL OR octet_length(p_key)<>32 OR p_config IS NULL OR octet_length(p_config) NOT BETWEEN 1 AND 65536
    OR p_model IS NULL OR octet_length(p_model) NOT BETWEEN 1 AND 128 THEN
   RAISE EXCEPTION 'bounded complete Invocation input required' USING ERRCODE='P1153';
 END IF;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 -- Raw callers may already hold Project: nonblocking reservations fail held,
 -- whereas trusted Rust takes these ordered keys in its fresh transaction first.
 FOR lock_key IN SELECT DISTINCT key FROM unnest(ARRAY[recipe_import107_command_mutex(p_key),
       recipe_import107_command_mutex(child.import_command_key)]) key ORDER BY key LOOP
   IF pg_try_advisory_xact_lock(lock_key) IS DISTINCT FROM true THEN
     RAISE EXCEPTION 'Invocation command reservation held' USING ERRCODE='55P03';
   END IF;
 END LOOP;
 SELECT * INTO mutex FROM recipe_import113_instance_mutex(p_instance);
 IF pg_try_advisory_xact_lock(mutex.first_key,mutex.second_key) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'Invocation instance reservation held' USING ERRCODE='55P03';
 END IF;
 PERFORM 1 FROM projects WHERE id=child.project_id FOR NO KEY UPDATE;
 PERFORM 1 FROM recipe_deployments WHERE id=child.deployment_id FOR UPDATE;
 PERFORM 1 FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name FOR UPDATE;
 SELECT * INTO consumer FROM agent_instances WHERE id=p_instance FOR UPDATE NOWAIT;
 FOR selected IN SELECT DISTINCT volume_id FROM agent_instance_revision_volume_bindings WHERE instance_revision_id=child.revision_id ORDER BY volume_id LOOP
   PERFORM 1 FROM agent_instance_state_volumes WHERE id=selected.volume_id FOR UPDATE NOWAIT;
 END LOOP;
 FOR selected IN SELECT id FROM agent_instance_volume_mount_grants WHERE instance_revision_id=child.revision_id ORDER BY id LOOP
   PERFORM 1 FROM agent_instance_volume_mount_grants WHERE id=selected.id FOR UPDATE NOWAIT;
 END LOOP;
 -- All live facts are rechecked after locks; no cached preview grants authority.
 PERFORM * FROM qualified_instance_invocation_context(p_instance);
 SELECT * INTO activation FROM agent_instance_execution_activations WHERE instance_id=p_instance;
 SELECT * INTO saved_revision FROM agent_instance_revisions WHERE id=child.revision_id;
 IF p_revision IS DISTINCT FROM child.revision_id OR consumer.active_revision_id IS DISTINCT FROM p_revision THEN
   RAISE EXCEPTION 'Invocation must use exact current immutable revision' USING ERRCODE='P1152';
 END IF;
 IF activation.id IS NULL OR consumer.version IS DISTINCT FROM 2 OR consumer.run_gate_open IS DISTINCT FROM true
    OR consumer.state IS DISTINCT FROM 'active' OR consumer.volume_mode IS DISTINCT FROM 'named'
    OR qualified_invocation_activation_lineage_matches(activation) IS DISTINCT FROM true
    OR qualified_invocation_current_eligible(p_instance,p_revision,activation.config_json) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'explicit completed qualified activation required; Invocation never opens a gate' USING ERRCODE='P1151';
 END IF;
 IF activation.config_bytes IS DISTINCT FROM p_config OR activation.config_json IS DISTINCT FROM convert_from(p_config,'UTF8')::jsonb THEN
   RAISE EXCEPTION 'configured provider/platform/root/profile drift' USING ERRCODE='P1154';
 END IF;
 receipt.id:=p_id; receipt.command_key:=p_key; receipt.actor_id:=actor; receipt.request_id:=request;
 receipt.instance_id:=p_instance; receipt.revision_id:=p_revision; receipt.activation_id:=activation.id;
 receipt.deployment_id:=child.deployment_id; receipt.resource_name:=child.resource_name;
 receipt.project_id:=child.project_id; receipt.release_id:=child.release_id; receipt.release_agent_id:=child.release_agent_id;
 receipt.run_id:=p_run; receipt.start_command_id:=p_start; receipt.instance_version:=2; receipt.admission_version:=115;
 receipt.parameters:=saved_revision.parameters; receipt.parameter_hash:=saved_revision.parameter_hash;
 receipt.effective_policy:=saved_revision.effective_runtime_policy; receipt.effective_policy_hash:=saved_revision.effective_policy_hash;
 receipt.platform_policy_version:=saved_revision.platform_policy_version; receipt.requires_state:=(SELECT requires_state FROM release_agents WHERE id=child.release_agent_id);
 receipt.prepared_hash:=child.input_hash; receipt.config_bytes:=p_config; receipt.config_json:=activation.config_json; receipt.config_hash:=sha256(p_config);
 receipt.input_hash:=qualified_invocation_input_hash(receipt); receipt.authorization_model_version:=p_model;
 IF (SELECT count(*) FROM agent_instance_execution_invocations WHERE id=p_id OR command_key=p_key OR run_id=p_run OR start_command_id=p_start)>1 THEN
   RAISE EXCEPTION 'contradictory Invocation history held' USING ERRCODE='P1153';
 END IF;
 SELECT * INTO saved FROM agent_instance_execution_invocations WHERE id=p_id OR command_key=p_key OR run_id=p_run OR start_command_id=p_start;
 IF saved.id IS NOT NULL THEN
   IF ROW(saved.id,saved.command_key,saved.actor_id,saved.instance_id,saved.revision_id,saved.run_id,saved.start_command_id,saved.input_hash)
      IS DISTINCT FROM ROW(p_id,p_key,actor,p_instance,p_revision,p_run,p_start,receipt.input_hash) THEN
     RAISE EXCEPTION 'Invocation actor/identity/input conflict' USING ERRCODE='P1153';
   END IF;
   IF qualified_invocation_graph_matches(saved) IS DISTINCT FROM true OR saved.input_hash IS DISTINCT FROM qualified_invocation_input_hash(saved) THEN
     RAISE EXCEPTION 'committed Invocation lineage is incomplete' USING ERRCODE='P1151';
   END IF;
 ELSE
   -- Predicted IDs/key alone cannot adopt an independently created Run/command.
   IF EXISTS(SELECT 1 FROM runs WHERE id=p_run OR command_id=p_start)
      OR EXISTS(SELECT 1 FROM command_inbox WHERE command_id=p_start)
      OR EXISTS(SELECT 1 FROM release_command_inbox WHERE command_key=p_key)
      OR EXISTS(SELECT 1 FROM application_events WHERE aggregate_type='run' AND aggregate_id=p_run)
      OR EXISTS(SELECT 1 FROM outbox WHERE aggregate_id=p_run) THEN
     RAISE EXCEPTION 'Invocation IDs or key have independent history' USING ERRCODE='P1153';
   END IF;
   -- Explicit admission event captures fresh returned IDs; the normal runs
   -- trigger subsequently appends its ordinary lifecycle projections unchanged.
   SELECT event.event_id,event.cursor,event.aggregate_version INTO receipt.event_id,receipt.event_cursor,receipt.event_aggregate_version
     FROM append_application_event(gen_random_uuid(),'run',p_run,'run',p_run,'run.changed','created','queued',child.project_id,NULL) event;
   receipt.start_outbox_id:=gen_random_uuid();
   INSERT INTO agent_instance_execution_invocations SELECT (receipt).*;
   INSERT INTO release_command_inbox(command_key,operation,aggregate_id,secondary_id,actor_id,request_id,input_hash)
     VALUES(p_key,'invoke_instance',p_instance,p_run,actor,request,receipt.input_hash);
   INSERT INTO command_inbox(command_id,command_type,payload) VALUES(p_start,'start_run',qualified_invocation_start_json(receipt));
   INSERT INTO runs(id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,run_kind,command_id,requires_state,
      state,state_version,created_at,updated_at,execution_profile,vm_id,vm_provider_namespace,vm_host_id,vm_binding_kind,vm_plan_admission_required)
     VALUES(p_run,p_instance,p_revision,child.release_id,child.release_agent_id,NULL,'invocation',p_start,receipt.requires_state,
       'queued',0,now(),now(),'owned_canonical',p_run::text,receipt.config_json->'vm'->>'namespace',receipt.config_json->'vm'->>'host_id','planned',false);
   INSERT INTO outbox(id,aggregate_type,aggregate_id,subject,event_type,payload,occurred_at)
     VALUES(receipt.start_outbox_id,'release',p_run,'hephaestus.run.start','run.start.v1',
       qualified_invocation_start_json(receipt)||jsonb_build_object('schema_version',1,'message_id',receipt.start_outbox_id,
         'idempotency_key',receipt.start_outbox_id,'request_id',NULL,'trace_id',NULL),now());
 END IF;
 FOR required IN SELECT 'can_execute'::text permission,'agent_instance'::text object_type,p_instance object_id
    UNION ALL SELECT 'can_use','release_agent',child.release_agent_id LOOP
   IF check_permission('user',actor::text,required.permission,required.object_type,required.object_id::text) IS DISTINCT FROM 1
      OR NOT EXISTS(SELECT 1 FROM users WHERE id=actor AND status='active') THEN
     RAISE EXCEPTION 'fresh Invocation authority changed' USING ERRCODE='42501';
   END IF;
   INSERT INTO authorization_audit_events(id,actor_id,permission,object_type,object_id,decision,request_id,authorization_model_version)
     VALUES(gen_random_uuid(),actor,required.permission,required.object_type,required.object_id,'allow',request,p_model) RETURNING id INTO audit_id;
   IF saved.id IS NULL THEN
     INSERT INTO agent_instance_execution_invocation_audits(invocation_id,permission,object_type,object_id,audit_id)
       VALUES(p_id,required.permission,required.object_type,required.object_id,audit_id);
   END IF;
 END LOOP;
 RETURN QUERY SELECT p_id,p_instance,p_revision,p_run,p_start;
END $$;
REVOKE ALL ON FUNCTION write_qualified_instance_invocation(uuid,bytea,uuid,uuid,uuid,uuid,bytea,text) FROM PUBLIC,hephaestus_worker;
GRANT EXECUTE ON FUNCTION write_qualified_instance_invocation(uuid,bytea,uuid,uuid,uuid,uuid,bytea,text) TO hephaestus_app;

CREATE FUNCTION validate_qualified_instance_invocation_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF qualified_invocation_graph_matches(NEW) IS DISTINCT FROM true OR NEW.input_hash IS DISTINCT FROM qualified_invocation_input_hash(NEW)
    OR NOT EXISTS(SELECT 1 FROM users WHERE id=NEW.actor_id AND status='active')
    OR check_permission('user',NEW.actor_id::text,'can_execute','agent_instance',NEW.instance_id::text) IS DISTINCT FROM 1
    OR check_permission('user',NEW.actor_id::text,'can_use','release_agent',NEW.release_agent_id::text) IS DISTINCT FROM 1
    OR qualified_invocation_current_eligible(NEW.instance_id,NEW.revision_id,NEW.config_json) IS DISTINCT FROM true
    OR NOT EXISTS(SELECT 1 FROM runs candidate WHERE candidate.id=NEW.run_id
       AND qualified_invocation_queued_birth_matches(candidate)) THEN
   RAISE EXCEPTION 'Invocation must commit complete exact current-authorized graph' USING ERRCODE='23000';
 END IF;
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_qualified_instance_invocation_commit() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER qualified_instance_invocation_commit AFTER INSERT ON agent_instance_execution_invocations
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_qualified_instance_invocation_commit();

-- Closed producer branches; Invocation never borrows the old Update ELSE.
CREATE OR REPLACE FUNCTION run_profile_producer_matches(execution runs) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public AS $$
    SELECT (execution.execution_profile<>'owned_canonical'
        OR NOT EXISTS(SELECT 1 FROM mailbox_delivery_attempts WHERE run_id=execution.id))
      AND CASE WHEN execution.run_kind='normal' THEN
        EXISTS(SELECT 1 FROM run_requests request
            WHERE request.run_id=execution.id AND request.command_id=execution.command_id
              AND ROW(request.instance_id,request.instance_revision_id,request.release_id,
                      request.release_agent_id,request.attachment_id,request.requires_state)
                IS NOT DISTINCT FROM ROW(execution.instance_id,execution.instance_revision_id,
                      execution.release_id,execution.release_agent_id,execution.attachment_id,execution.requires_state)
              AND request.request_kind='instance_normal'
              AND request.dispatch_state IN ('pending','dispatched')
              AND EXISTS(SELECT 1 FROM command_inbox inbox WHERE inbox.command_id=execution.command_id
                  AND inbox.command_type='start_run'
                  AND ROW(inbox.payload->>'command_id',inbox.payload->>'run_id',inbox.payload->>'instance_id',
                          inbox.payload->>'instance_revision_id',inbox.payload->>'release_id',inbox.payload->>'release_agent_id',
                          inbox.payload->>'attachment_id',inbox.payload->>'kind',inbox.payload->'requires_state')
                    IS NOT DISTINCT FROM ROW(execution.command_id::text,execution.id::text,execution.instance_id::text,
                          execution.instance_revision_id::text,execution.release_id::text,execution.release_agent_id::text,
                          execution.attachment_id::text,execution.run_kind,to_jsonb(execution.requires_state))))
        OR (execution.execution_profile='legacy_scalar' AND EXISTS(SELECT 1 FROM mailbox_delivery_attempts attempt
            JOIN mailbox_events event ON event.id=attempt.event_id AND event.mailbox_id=attempt.mailbox_id
            WHERE attempt.run_id=execution.id AND attempt.command_id=execution.command_id
              AND attempt.instance_id=execution.instance_id
              AND attempt.instance_revision_id=execution.instance_revision_id
              AND event.instance_id=execution.instance_id
              AND attempt.target_ref IS NOT NULL AND attempt.target_commit IS NOT NULL
              AND attempt.state='leased'))
    WHEN execution.run_kind='update' THEN EXISTS(SELECT 1 FROM agent_updates update
        JOIN agent_instance_revisions candidate ON candidate.id=update.candidate_revision_id
            AND candidate.instance_id=update.instance_id
        JOIN release_agents candidate_agent ON candidate_agent.id=candidate.release_agent_id
        JOIN release_command_inbox command ON command.operation='begin_update_hook'
            AND command.aggregate_id=update.id AND command.secondary_id=execution.id
        WHERE update.hook_run_id=execution.id AND update.instance_id=execution.instance_id
          AND update.candidate_revision_id=execution.instance_revision_id
          AND candidate_agent.release_id=execution.release_id AND candidate_agent.id=execution.release_agent_id
          AND candidate_agent.requires_state=execution.requires_state AND update.state='hook_running'
          AND command.actor_id IS NOT NULL AND command.request_id IS NOT NULL
          AND (execution.execution_profile<>'owned_canonical' OR EXISTS(SELECT 1 FROM run_vm_plan_admissions admission
              WHERE admission.run_id=execution.id AND admission.command_id=execution.command_id
                AND admission.update_id=update.id AND admission.producer_command_key=command.command_key
                AND admission.actor_id=command.actor_id AND admission.request_id=command.request_id)))
    WHEN execution.run_kind='invocation' THEN EXISTS(SELECT 1 FROM agent_instance_execution_invocations receipt
       WHERE receipt.run_id=execution.id AND qualified_invocation_run_matches(receipt,execution)
         AND qualified_invocation_graph_matches(receipt)
         AND receipt.input_hash=qualified_invocation_input_hash(receipt))
    ELSE false END
$$;
REVOKE ALL ON FUNCTION run_profile_producer_matches(runs) FROM PUBLIC;

CREATE OR REPLACE FUNCTION enforce_run_execution_profile() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
    IF TG_OP='INSERT' AND NEW.run_kind='invocation' THEN
        IF qualified_invocation_queued_birth_matches(NEW) IS DISTINCT FROM true THEN
            RAISE EXCEPTION 'exact protected actor Invocation required' USING ERRCODE='42501';
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP='UPDATE' THEN
        IF NEW.execution_profile IS DISTINCT FROM OLD.execution_profile THEN
            RAISE EXCEPTION 'Run execution profile is immutable; history cannot be adopted' USING ERRCODE='23000';
        END IF;
        IF OLD.execution_profile<>'historical_unproven' AND
            ROW(NEW.id,NEW.instance_id,NEW.instance_revision_id,NEW.release_id,NEW.release_agent_id,
                NEW.attachment_id,NEW.run_kind,NEW.command_id,NEW.requires_state)
            IS DISTINCT FROM ROW(OLD.id,OLD.instance_id,OLD.instance_revision_id,OLD.release_id,
                OLD.release_agent_id,OLD.attachment_id,OLD.run_kind,OLD.command_id,OLD.requires_state) THEN
            RAISE EXCEPTION 'positively profiled Run pins and producer identity are immutable' USING ERRCODE='23000';
        END IF;
        IF OLD.execution_profile='historical_unproven' AND
            ((NEW.state IS DISTINCT FROM OLD.state AND NEW.state IN ('leasing_volume','provisioning','starting','running','cleaned_up'))
             OR ROW(NEW.vm_id,NEW.vm_provider_namespace,NEW.vm_host_id,NEW.vm_binding_kind,
                    NEW.volume_id,NEW.lease_id,NEW.lease_fencing_token)
                IS DISTINCT FROM ROW(OLD.vm_id,OLD.vm_provider_namespace,OLD.vm_host_id,OLD.vm_binding_kind,
                    OLD.volume_id,OLD.lease_id,OLD.lease_fencing_token)) THEN
            RAISE EXCEPTION 'unclassified Run history remains held, without new IO or fabricated cleanup' USING ERRCODE='23000';
        END IF;
    ELSIF NEW.execution_profile<>'historical_unproven' THEN
        IF (CASE WHEN current_setting('role',true)='none' THEN session_user::text ELSE current_setting('role',true) END)<>'hephaestus_worker'
           OR NEW.state<>'queued' OR NEW.volume_acquisition_closed_at IS NOT NULL
           OR num_nonnulls(NEW.volume_id,NEW.lease_id,NEW.lease_fencing_token)>0 THEN
            RAISE EXCEPTION 'positive Run profiles require explicit fresh worker admission' USING ERRCODE='42501';
        END IF;
    ELSIF NEW.state<>'queued' OR num_nonnulls(NEW.vm_id,NEW.vm_provider_namespace,NEW.vm_host_id,
        NEW.vm_binding_kind,NEW.volume_id,NEW.lease_id,NEW.lease_fencing_token)>0 THEN
        RAISE EXCEPTION 'unspecified Run profile cannot manufacture started or owned history' USING ERRCODE='23000';
    END IF;
    IF NEW.execution_profile='legacy_scalar' AND
        (NEW.vm_plan_admission_required OR num_nonnulls(NEW.vm_provider_namespace,NEW.vm_host_id,NEW.vm_binding_kind)>0
         OR NEW.volume_selection_snapshot IS NOT NULL) THEN
        RAISE EXCEPTION 'legacy scalar profile conveys no canonical scope or plural selection' USING ERRCODE='23000';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_run_execution_profile() FROM PUBLIC;

CREATE OR REPLACE FUNCTION enforce_run_vm_owner_scope() RETURNS trigger
LANGUAGE plpgsql AS $$ BEGIN
    IF TG_OP='INSERT' AND NEW.run_kind='invocation' THEN
        IF qualified_invocation_queued_birth_matches(NEW) IS DISTINCT FROM true THEN
            RAISE EXCEPTION 'exact protected actor Invocation required' USING ERRCODE='42501';
        END IF;
        RETURN NEW;
    END IF;
    IF NEW.execution_profile='legacy_scalar' THEN
        IF num_nonnulls(NEW.vm_provider_namespace,NEW.vm_host_id,NEW.vm_binding_kind)>0
           OR NEW.vm_plan_admission_required
           OR (NEW.vm_id IS NOT NULL AND NOT run_cleanup_identifier_is_valid(NEW.vm_id)) THEN
            RAISE EXCEPTION 'legacy scalar VM identity is not an owned absence target' USING ERRCODE='23000';
        END IF;
        IF TG_OP='UPDATE' THEN
            IF OLD.vm_id IS NOT NULL AND NEW.vm_id IS DISTINCT FROM OLD.vm_id THEN
                RAISE EXCEPTION 'legacy VM identity cannot be replaced' USING ERRCODE='23000';
            END IF;
            IF NEW.vm_id IS DISTINCT FROM OLD.vm_id THEN
                PERFORM require_run_cleanup_worker();
                IF NEW.volume_acquisition_closed_at IS NOT NULL
                   OR NEW.state NOT IN ('leasing_volume','provisioning') THEN
                    RAISE EXCEPTION 'legacy scalar VM must bind only during preparation' USING ERRCODE='23000';
                END IF;
            END IF;
        ELSIF NEW.vm_id IS NOT NULL THEN
            RAISE EXCEPTION 'legacy scalar insert must be pre-IO' USING ERRCODE='23000';
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP='UPDATE' THEN
        IF OLD.vm_binding_kind IS NOT NULL AND
           ROW(NEW.id,NEW.instance_id,NEW.instance_revision_id,NEW.release_id,NEW.release_agent_id,NEW.attachment_id,NEW.run_kind,NEW.command_id,NEW.requires_state)
            IS DISTINCT FROM ROW(OLD.id,OLD.instance_id,OLD.instance_revision_id,OLD.release_id,OLD.release_agent_id,OLD.attachment_id,OLD.run_kind,OLD.command_id,OLD.requires_state) THEN
            RAISE EXCEPTION 'planned run inputs are immutable' USING ERRCODE='23000';
        END IF;
        IF ROW(NEW.vm_id,NEW.vm_provider_namespace,NEW.vm_host_id,NEW.vm_binding_kind)
            IS NOT DISTINCT FROM ROW(OLD.vm_id,OLD.vm_provider_namespace,OLD.vm_host_id,OLD.vm_binding_kind) THEN
            RETURN NEW;
        END IF;
        IF OLD.vm_id IS NOT NULL OR OLD.vm_binding_kind IS NOT NULL THEN
            RAISE EXCEPTION 'VM identity and owner are immutable or historical unresolved' USING ERRCODE='23000';
        END IF;
        IF OLD.execution_profile<>'owned_canonical' OR NOT OLD.vm_plan_admission_required OR NOT EXISTS(SELECT 1 FROM run_vm_plan_admissions admission
            WHERE admission.run_id=NEW.id AND admission.command_id=NEW.command_id
              AND admission.instance_id=NEW.instance_id AND admission.instance_revision_id=NEW.instance_revision_id
              AND admission.consumed_at IS NOT NULL AND admission.vm_id=NEW.vm_id
              AND admission.vm_provider_namespace=NEW.vm_provider_namespace AND admission.vm_host_id=NEW.vm_host_id) THEN
            RAISE EXCEPTION 'historical NULL scope is not VM planning admission' USING ERRCODE='23000';
        END IF;
    ELSIF NEW.vm_id IS NULL AND NEW.vm_binding_kind IS NULL THEN
        RETURN NEW;
    END IF;
    IF NEW.execution_profile<>'owned_canonical' OR NEW.vm_id IS DISTINCT FROM NEW.id::text THEN
        RAISE EXCEPTION 'fresh planned VM identity must equal exact run UUID' USING ERRCODE='23000';
    END IF;
    PERFORM require_run_cleanup_worker();
    IF NEW.vm_binding_kind IS DISTINCT FROM 'planned' OR NEW.vm_id IS NULL
       OR NEW.vm_provider_namespace IS NULL OR NEW.vm_host_id IS NULL
       OR ((NEW.volume_acquisition_closed_at IS NOT NULL OR NEW.state NOT IN ('queued','leasing_volume','provisioning'))
           AND NOT EXISTS(SELECT 1 FROM run_vm_plan_admissions WHERE run_id=NEW.id AND consumed_at IS NOT NULL AND consumption_kind='cleanup'
               AND vm_id=NEW.vm_id AND vm_provider_namespace=NEW.vm_provider_namespace AND vm_host_id=NEW.vm_host_id)) THEN
        RAISE EXCEPTION 'VM scope must be planned before provisioning IO' USING ERRCODE='23000';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_run_vm_owner_scope() FROM PUBLIC;

CREATE OR REPLACE FUNCTION validate_fresh_run_execution_profile() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
    -- Evaluate final same-transaction rows, not the INSERT trigger's stale NEW.
    SELECT * INTO execution FROM runs WHERE id=NEW.id;
    IF execution.run_kind='invocation' THEN
        IF execution.execution_profile IS DISTINCT FROM 'owned_canonical'
           OR run_profile_producer_matches(execution) IS DISTINCT FROM true THEN
            RAISE EXCEPTION 'Invocation needs exact protected complete producer graph' USING ERRCODE='23000';
        END IF;
        RETURN NULL;
    END IF;
    IF execution.execution_profile='historical_unproven' THEN RETURN NULL; END IF;
    IF NOT EXISTS(SELECT 1 FROM agent_instances consumer
        JOIN agent_instance_revisions revision ON revision.id=execution.instance_revision_id AND revision.instance_id=consumer.id
        JOIN release_agents agent ON agent.id=revision.release_agent_id AND agent.id=execution.release_agent_id
        JOIN releases release ON release.id=agent.release_id AND release.id=execution.release_id
        WHERE consumer.id=execution.instance_id AND execution.requires_state=agent.requires_state
          AND release.state='published' AND octet_length(agent.runtime_contract_hash)=32
          AND NOT EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=consumer.id)
          AND ((execution.run_kind='normal' AND consumer.active_revision_id=revision.id
                AND revision.runnable AND consumer.run_gate_open AND consumer.state IN ('active','update_rejected')
                AND EXISTS(SELECT 1 FROM agent_attachments attachment WHERE attachment.id=execution.attachment_id
                    AND attachment.instance_id=consumer.id AND attachment.enabled AND attachment.removed_at IS NULL))
            OR (execution.run_kind='update' AND consumer.state='updating' AND NOT consumer.run_gate_open))
          AND (execution.execution_profile<>'legacy_scalar'
            OR (consumer.volume_mode='legacy'
              AND jsonb_typeof(COALESCE(agent.runtime_contract->'volume_slots','[]'::jsonb))='array'
              AND jsonb_array_length(COALESCE(agent.runtime_contract->'volume_slots','[]'::jsonb))=0)))
       OR NOT run_profile_producer_matches(execution) THEN
        RAISE EXCEPTION 'fresh execution profile needs exact released revision and producer provenance' USING ERRCODE='23000';
    END IF;
    IF execution.execution_profile='owned_canonical' AND
        ((execution.vm_binding_kind='planned' AND execution.vm_id=execution.id::text
            AND run_cleanup_identifier_is_valid(execution.vm_provider_namespace)
            AND run_cleanup_identifier_is_valid(execution.vm_host_id))
          OR (execution.vm_plan_admission_required AND execution.vm_id IS NULL
            AND execution.vm_binding_kind IS NULL AND EXISTS(SELECT 1 FROM run_vm_plan_admissions admission
              WHERE admission.run_id=execution.id AND admission.command_id=execution.command_id
                AND admission.instance_id=execution.instance_id AND admission.instance_revision_id=execution.instance_revision_id
                AND admission.consumed_at IS NULL))) IS NOT TRUE THEN
        RAISE EXCEPTION 'owned profile needs exact planned scope or committed fresh no-IO admission, including zero slots' USING ERRCODE='23000';
    END IF;
    RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_fresh_run_execution_profile() FROM PUBLIC;

CREATE OR REPLACE FUNCTION enforce_instance_work_admission() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 -- Do not access a runs-only field on mailbox/lease/update trigger records.
 IF TG_TABLE_NAME='runs' THEN
   IF NEW.run_kind='invocation' THEN
     IF qualified_invocation_queued_birth_matches(NEW) IS DISTINCT FROM true THEN
       RAISE EXCEPTION 'unproven Invocation is not named work admission' USING ERRCODE='23000';
     END IF;
     RETURN NEW;
   END IF;
 END IF;
 PERFORM assert_instance_accepts_work(NEW.instance_id);
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_instance_work_admission() FROM PUBLIC;
-- named_volume_dispatch_supported, enforce_instance_run_progress, mounts,
-- live session sources and VM consumers are unchanged and still fail closed.
