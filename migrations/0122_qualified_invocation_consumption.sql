-- PRIVATE117: protected worker consumption of a preexisting qualified115 Run.
-- Requires reviewed113/114/115/116; enables no lease/progress/session/mount/VM IO.
CREATE TABLE run_invocation_consumptions (
 run_id uuid PRIMARY KEY REFERENCES runs(id),
 invocation_id uuid NOT NULL UNIQUE REFERENCES agent_instance_execution_invocations(id),
 start_command_id uuid NOT NULL UNIQUE REFERENCES command_inbox(command_id),
 start_payload jsonb NOT NULL CHECK(jsonb_typeof(start_payload)='object'),
 admission_input_hash bytea NOT NULL CHECK(octet_length(admission_input_hash)=32),
 config_bytes bytea NOT NULL CHECK(octet_length(config_bytes) BETWEEN 1 AND 65536),
 config_hash bytea NOT NULL CHECK(config_hash=sha256(config_bytes)),
 vm_provider_namespace text NOT NULL CHECK(run_cleanup_identifier_is_valid(vm_provider_namespace)),
 vm_host_id text NOT NULL CHECK(run_cleanup_identifier_is_valid(vm_host_id)),
 vm_id text NOT NULL CHECK(vm_id=run_id::text),
 event_id uuid NOT NULL UNIQUE REFERENCES application_events(id),
 event_cursor bigint NOT NULL CHECK(event_cursor>0),
 event_aggregate_version bigint NOT NULL CHECK(event_aggregate_version>0),
 consumed_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE run_invocation_consumptions ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_invocation_consumptions FORCE ROW LEVEL SECURITY;
REVOKE ALL ON run_invocation_consumptions FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT SELECT ON run_invocation_consumptions TO hephaestus_worker;
CREATE POLICY invocation_consumption_worker_read ON run_invocation_consumptions FOR SELECT TO hephaestus_worker USING(true);

CREATE FUNCTION require_invocation117_worker() RETURNS void
LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF (CASE WHEN current_setting('role',true)='none' THEN session_user::text ELSE current_setting('role',true) END)
      IS DISTINCT FROM 'hephaestus_worker' THEN
   RAISE EXCEPTION 'qualified Invocation consumption requires worker role' USING ERRCODE='42501';
 END IF;
END $$;
REVOKE ALL ON FUNCTION require_invocation117_worker() FROM PUBLIC;

-- Resolve immutable early reservations only. Labels are not an IO permit.
CREATE FUNCTION invocation117_reservation_context(p_run uuid)
RETURNS TABLE(command_mutex bigint,first_mutex integer,second_mutex integer)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE receipt agent_instance_execution_invocations%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
BEGIN
 PERFORM require_invocation117_worker();
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=receipt.instance_id;
 IF receipt.id IS NULL OR child.instance_id IS NULL OR qualified_invocation_graph_matches(receipt) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'exact immutable Invocation producer required' USING ERRCODE='P1170';
 END IF;
 RETURN QUERY SELECT keys.key,mutex.first_key,mutex.second_key
  FROM (SELECT DISTINCT key FROM unnest(ARRAY[recipe_import107_command_mutex(receipt.command_key),
    recipe_import107_command_mutex(child.import_command_key)]) key) keys
  CROSS JOIN recipe_import113_instance_mutex(receipt.instance_id) mutex ORDER BY keys.key;
END $$;
REVOKE ALL ON FUNCTION invocation117_reservation_context(uuid) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION invocation117_reservation_context(uuid) TO hephaestus_worker;

-- Raw callers may hold Project; TRY avoids reversing the trusted early order.
-- Trusted Rust blocks on actual signed keys BEFORE Project in its own new TX.
CREATE FUNCTION lock_invocation117(p_run uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE early record; receipt agent_instance_execution_invocations%ROWTYPE; selected record;
BEGIN
 PERFORM require_invocation117_worker();
 FOR early IN SELECT * FROM invocation117_reservation_context(p_run) ORDER BY command_mutex LOOP
   IF pg_try_advisory_xact_lock(early.command_mutex) IS DISTINCT FROM true THEN
     RAISE EXCEPTION 'Invocation command reservation held' USING ERRCODE='55P03';
   END IF;
 END LOOP;
 SELECT * INTO early FROM invocation117_reservation_context(p_run) LIMIT 1;
 IF pg_try_advisory_xact_lock(early.first_mutex,early.second_mutex) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'Invocation instance reservation held' USING ERRCODE='55P03';
 END IF;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 PERFORM 1 FROM projects WHERE id=receipt.project_id FOR NO KEY UPDATE NOWAIT;
 PERFORM 1 FROM recipe_deployments WHERE id=receipt.deployment_id FOR UPDATE NOWAIT;
 PERFORM 1 FROM recipe_deployment_resources WHERE deployment_id=receipt.deployment_id AND resource_name=receipt.resource_name FOR UPDATE NOWAIT;
 PERFORM 1 FROM agent_instances WHERE id=receipt.instance_id FOR UPDATE NOWAIT;
 PERFORM 1 FROM runs WHERE id=p_run FOR UPDATE NOWAIT;
 FOR selected IN SELECT DISTINCT volume_id FROM agent_instance_revision_volume_bindings WHERE instance_revision_id=receipt.revision_id ORDER BY volume_id LOOP
   PERFORM 1 FROM agent_instance_state_volumes WHERE id=selected.volume_id FOR UPDATE NOWAIT;
 END LOOP;
 FOR selected IN SELECT id FROM agent_instance_volume_mount_grants WHERE instance_revision_id=receipt.revision_id ORDER BY id LOOP
   PERFORM 1 FROM agent_instance_volume_mount_grants WHERE id=selected.id FOR UPDATE NOWAIT;
 END LOOP;
END $$;
REVOKE ALL ON FUNCTION lock_invocation117(uuid) FROM PUBLIC,hephaestus_app;

-- Immutable lineage inspection stays valid after Remove/permission withdrawal.
-- It requires the actual117 receipt, never just a provider tuple/config label.
CREATE FUNCTION invocation117_consumption_matches(p_run uuid) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE receipt agent_instance_execution_invocations%ROWTYPE; consumed run_invocation_consumptions%ROWTYPE;
BEGIN
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 SELECT * INTO consumed FROM run_invocation_consumptions WHERE run_id=p_run;
 RETURN receipt.id IS NOT NULL AND consumed.run_id IS NOT NULL
   AND qualified_invocation_graph_matches(receipt) IS NOT DISTINCT FROM true
   AND ROW(consumed.invocation_id,consumed.start_command_id,consumed.start_payload,consumed.admission_input_hash,
     consumed.config_bytes,consumed.config_hash,consumed.vm_provider_namespace,consumed.vm_host_id,consumed.vm_id)
    IS NOT DISTINCT FROM ROW(receipt.id,receipt.start_command_id,qualified_invocation_start_json(receipt),receipt.input_hash,
     receipt.config_bytes,receipt.config_hash,receipt.config_json->'vm'->>'namespace',receipt.config_json->'vm'->>'host_id',receipt.run_id::text)
   AND EXISTS(SELECT 1 FROM command_inbox inbox WHERE inbox.command_id=consumed.start_command_id
      AND inbox.command_type='start_run' AND inbox.payload=consumed.start_payload AND inbox.processed_at IS NOT NULL)
   AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox product ON product.event_id=event.id
     WHERE event.id=consumed.event_id AND event.cursor=consumed.event_cursor AND event.aggregate_version=consumed.event_aggregate_version
       AND event.occurrence_id=consumed.run_id AND event.scope_kind='run' AND event.scope_id=consumed.run_id
       AND event.aggregate_type='run' AND event.aggregate_id=consumed.run_id AND event.event_type='run.changed'
       AND event.change_kind='updated' AND event.safe_state='queued' AND event.related_id_one=receipt.project_id
       AND event.related_id_two IS NULL AND event.actor_type='system' AND event.actor_id IS NULL AND event.request_id IS NULL);
END $$;
REVOKE ALL ON FUNCTION invocation117_consumption_matches(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION inspect_qualified_invocation117(p_run uuid) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_invocation117_worker();
 RETURN invocation117_consumption_matches(p_run);
END $$;
REVOKE ALL ON FUNCTION inspect_qualified_invocation117(uuid) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION inspect_qualified_invocation117(uuid) TO hephaestus_worker;

-- Live execution is distinct from immutable inspection. Own held leases remain
-- explicitly unsupported until a qualified full frozen-set predicate is added.
-- Only queued/version0 may start/replay; no inferred LeasingVolume/progress.
CREATE FUNCTION invocation117_current_eligible(p_run uuid,p_start jsonb,p_config bytea) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE receipt agent_instance_execution_invocations%ROWTYPE; execution runs%ROWTYPE;
BEGIN
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF receipt.id IS NULL OR execution.id IS NULL OR p_start IS DISTINCT FROM qualified_invocation_start_json(receipt)
    OR p_config IS DISTINCT FROM receipt.config_bytes OR qualified_invocation_graph_matches(receipt) IS DISTINCT FROM true
    OR NOT EXISTS(SELECT 1 FROM users WHERE id=receipt.actor_id AND status='active')
    OR check_permission('user',receipt.actor_id::text,'can_execute','agent_instance',receipt.instance_id::text) IS DISTINCT FROM 1
    OR check_permission('user',receipt.actor_id::text,'can_use','release_agent',receipt.release_agent_id::text) IS DISTINCT FROM 1
    OR qualified_invocation_current_eligible(receipt.instance_id,receipt.revision_id,receipt.config_json) IS DISTINCT FROM true
    OR execution.cancel_requested_at IS NOT NULL OR execution.attachment_id IS NOT NULL
    OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=p_run AND released_at IS NULL)
    OR EXISTS(SELECT 1 FROM agent_instance_revision_volume_bindings binding JOIN agent_instance_state_volumes volume ON volume.id=binding.volume_id
      WHERE binding.instance_revision_id=receipt.revision_id AND volume.ready_retention_closure_id IS NOT NULL
        AND (volume.ready_retention_reopen_id IS NULL OR ready_retention_lease_reopening_matches(volume.ready_retention_reopen_id,receipt.instance_id,volume.id) IS DISTINCT FROM true)) THEN
   RETURN false;
 END IF;
 IF execution.state='cleaned_up' THEN
   RETURN invocation117_consumption_matches(p_run) AND EXISTS(
     SELECT 1 FROM run_cleanup_targets target JOIN run_cleanup_receipts observation
      ON observation.run_id=target.run_id AND observation.generation=target.generation AND observation.target_digest=target.target_digest
     JOIN run_cleanup_completions done ON done.run_id=observation.run_id AND done.generation=observation.generation AND done.target_digest=observation.target_digest
     WHERE target.run_id=p_run AND target.vm_provider_namespace=receipt.config_json->'vm'->>'namespace'
       AND target.vm_host_id=receipt.config_json->'vm'->>'host_id' AND target.vm_id=p_run::text
       AND run_cleanup_set_matches(target.run_id,target.generation,true));
 END IF;
 RETURN execution.state='queued' AND execution.state_version=0 AND execution.volume_acquisition_closed_at IS NULL
   AND execution.volume_selection_snapshot IS NULL AND num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)=0
   AND execution.outcome IS NULL AND execution.exit_code IS NULL AND execution.exit_signal IS NULL AND execution.failure IS NULL
   AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=p_run);
END $$;
REVOKE ALL ON FUNCTION invocation117_current_eligible(uuid,jsonb,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION consume_qualified_invocation117(p_run uuid,p_start jsonb,p_config bytea,p_namespace text,p_host text,p_vm text)
RETURNS SETOF runs LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE receipt agent_instance_execution_invocations%ROWTYPE; consumed run_invocation_consumptions%ROWTYPE;
 event record; worker_actor text; worker_request text; worker_subject text;
BEGIN
 PERFORM require_invocation117_worker();
 IF p_start IS NULL OR octet_length(p_start::text)>65536 OR p_config IS NULL OR octet_length(p_config) NOT BETWEEN 1 AND 65536 THEN
   RAISE EXCEPTION 'complete bounded Invocation input required' USING ERRCODE='P1173';
 END IF;
 PERFORM lock_invocation117(p_run);
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 IF ROW(p_namespace,p_host,p_vm) IS DISTINCT FROM ROW(receipt.config_json->'vm'->>'namespace',receipt.config_json->'vm'->>'host_id',p_run::text)
    OR invocation117_current_eligible(p_run,p_start,p_config) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'Invocation current admission is closed, held or conflicting' USING ERRCODE='P1171';
 END IF;
 SELECT * INTO consumed FROM run_invocation_consumptions WHERE run_id=p_run;
 IF consumed.run_id IS NOT NULL THEN
   IF invocation117_consumption_matches(p_run) IS DISTINCT FROM true THEN
     RAISE EXCEPTION 'Invocation consumption lineage contradicts producer' USING ERRCODE='P1170';
   END IF;
 ELSE
   -- Event belongs to the worker operation, not archived actor/request authority.
   worker_actor:=current_setting('hephaestus.actor_id',true); worker_request:=current_setting('hephaestus.request_id',true);
   worker_subject:=current_setting('hephaestus.subject_type',true);
   PERFORM set_config('hephaestus.actor_id','',true); PERFORM set_config('hephaestus.request_id','',true);
   PERFORM set_config('hephaestus.subject_type','system',true);
   SELECT * INTO event FROM append_application_event(p_run,'run',p_run,'run',p_run,'run.changed','updated','queued',receipt.project_id,NULL);
   PERFORM set_config('hephaestus.actor_id',coalesce(worker_actor,''),true);
   PERFORM set_config('hephaestus.request_id',coalesce(worker_request,''),true);
   PERFORM set_config('hephaestus.subject_type',coalesce(worker_subject,''),true);
   INSERT INTO run_invocation_consumptions(run_id,invocation_id,start_command_id,start_payload,admission_input_hash,
     config_bytes,config_hash,vm_provider_namespace,vm_host_id,vm_id,event_id,event_cursor,event_aggregate_version)
   VALUES(p_run,receipt.id,receipt.start_command_id,p_start,receipt.input_hash,p_config,sha256(p_config),p_namespace,p_host,p_vm,
     event.event_id,event.cursor,event.aggregate_version);
   UPDATE command_inbox SET processed_at=now() WHERE command_id=receipt.start_command_id AND command_type='start_run'
     AND payload=p_start AND processed_at IS NULL;
   IF NOT FOUND THEN RAISE EXCEPTION 'exact unconsumed Start inbox required' USING ERRCODE='P1170'; END IF;
 END IF;
 RETURN QUERY SELECT stored.* FROM runs stored WHERE stored.id=p_run;
END $$;
REVOKE ALL ON FUNCTION consume_qualified_invocation117(uuid,jsonb,bytea,text,text,text) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION consume_qualified_invocation117(uuid,jsonb,bytea,text,text,text) TO hephaestus_worker;

CREATE FUNCTION read_qualified_invocation117(p_run uuid,p_start jsonb,p_config bytea) RETURNS SETOF runs
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_invocation117_worker();
 PERFORM lock_invocation117(p_run);
 IF invocation117_consumption_matches(p_run) IS DISTINCT FROM true
    OR invocation117_current_eligible(p_run,p_start,p_config) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'Invocation runtime catalogue requires consumed current eligibility' USING ERRCODE='P1171';
 END IF;
 RETURN QUERY SELECT stored.* FROM runs stored WHERE stored.id=p_run AND stored.state='queued' AND stored.state_version=0;
END $$;
REVOKE ALL ON FUNCTION read_qualified_invocation117(uuid,jsonb,bytea) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION read_qualified_invocation117(uuid,jsonb,bytea) TO hephaestus_worker;

CREATE FUNCTION reject_invocation117_rewrite() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN RAISE EXCEPTION 'Invocation worker consumption is immutable' USING ERRCODE='23000'; END $$;
REVOKE ALL ON FUNCTION reject_invocation117_rewrite() FROM PUBLIC;
CREATE TRIGGER invocation117_immutable BEFORE UPDATE OR DELETE ON run_invocation_consumptions FOR EACH ROW EXECUTE FUNCTION reject_invocation117_rewrite();
CREATE FUNCTION validate_invocation117_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF invocation117_consumption_matches(NEW.run_id) IS DISTINCT FROM true
    OR invocation117_current_eligible(NEW.run_id,NEW.start_payload,NEW.config_bytes) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'Invocation consumption must commit complete current-eligible producer, Start inbox and worker event' USING ERRCODE='23000';
 END IF;
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_invocation117_commit() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER invocation117_commit AFTER INSERT ON run_invocation_consumptions
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_invocation117_commit();
-- No overrides to global named dispatch, work admission, lease/progress/session
-- or VM plan guards.117 is metadata consumption/catalogue only, not execution.
