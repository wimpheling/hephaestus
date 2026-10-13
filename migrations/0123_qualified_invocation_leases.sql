-- PRIVATE119-A. Qualified Invocation lease/progress metadata only.
-- Requires exact reviewed113/114/115/116/117. No session issuance, Starting,
-- Running, mount, VM, or global named dispatch support is enabled here.
-- Recorded invoker IDs are data; no historic authentication is reconstructed.

CREATE FUNCTION invocation119_immutable(p_run uuid) RETURNS boolean
LANGUAGE sql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT invocation117_consumption_matches(p_run) IS NOT DISTINCT FROM true
   AND EXISTS(SELECT 1 FROM runs execution WHERE execution.id=p_run
      AND execution.run_kind='invocation' AND execution.execution_profile='owned_canonical'
      AND execution.attachment_id IS NULL
      AND num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)=0)
$$;
REVOKE ALL ON FUNCTION invocation119_immutable(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- Complete original113 reservation set, independent of current live rights.
-- Hash collisions serialize only: full producer identity is rechecked separately.
CREATE FUNCTION invocation119_reservation_context(p_run uuid)
RETURNS TABLE(lock_class integer,command_mutex bigint,first_mutex integer,second_mutex integer)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE receipt agent_instance_execution_invocations%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
BEGIN
 PERFORM require_invocation117_worker();
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=receipt.instance_id;
 IF receipt.id IS NULL OR child.instance_id IS NULL OR qualified_invocation_graph_matches(receipt) IS DISTINCT FROM true
   OR NOT EXISTS(SELECT 1 FROM recipe_deployment_execution_preparations header
      WHERE header.deployment_id=child.deployment_id AND header.instance_count BETWEEN 1 AND 64
       AND header.instance_count=(SELECT count(*) FROM recipe_prepared_instance_imports WHERE deployment_id=header.deployment_id)) THEN
  RAISE EXCEPTION 'complete immutable Invocation preparation required' USING ERRCODE='P1190';
 END IF;
 RETURN QUERY SELECT 0,keys.key,NULL::integer,NULL::integer FROM (
    SELECT recipe_import107_command_mutex(prepared.import_command_key) key FROM recipe_prepared_instance_imports prepared WHERE prepared.deployment_id=child.deployment_id
    UNION SELECT recipe_import107_command_mutex(receipt.command_key)) keys ORDER BY keys.key;
 RETURN QUERY SELECT DISTINCT 1,NULL::bigint,keys.first_key,keys.second_key
   FROM recipe_prepared_instance_imports prepared CROSS JOIN LATERAL recipe_import113_instance_mutex(prepared.instance_id) keys
   WHERE prepared.deployment_id=child.deployment_id ORDER BY keys.first_key,keys.second_key;
END $$;
REVOKE ALL ON FUNCTION invocation119_reservation_context(uuid) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION invocation119_reservation_context(uuid) TO hephaestus_worker;

CREATE FUNCTION reserve_invocation119(p_run uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE early record;
BEGIN
 PERFORM require_invocation117_worker();
 FOR early IN SELECT * FROM invocation119_reservation_context(p_run)
   ORDER BY lock_class,command_mutex,first_mutex,second_mutex LOOP
  IF early.lock_class=0 THEN
   IF pg_try_advisory_xact_lock(early.command_mutex) IS DISTINCT FROM true THEN
    RAISE EXCEPTION 'Invocation whole-preparation command reservation held' USING ERRCODE='55P03'; END IF;
  ELSE
   IF pg_try_advisory_xact_lock(early.first_mutex,early.second_mutex) IS DISTINCT FROM true THEN
    RAISE EXCEPTION 'Invocation whole-preparation instance reservation held' USING ERRCODE='55P03'; END IF;
  END IF;
 END LOOP;
END $$;
REVOKE ALL ON FUNCTION reserve_invocation119(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- Complete canonical selection JSON, reconstructed from actual frozen bindings.
-- This is data comparison, never a grant or actual-provider/backing proof.
CREATE FUNCTION invocation119_selections(p_run uuid) RETURNS jsonb
LANGUAGE sql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 WITH identity AS (
  SELECT receipt.*,jsonb_build_object('run',receipt.run_id,'instance',receipt.instance_id,
   'revision',receipt.revision_id,'release',receipt.release_id,'release_agent',receipt.release_agent_id,
   'project',receipt.project_id) AS value,agent.runtime_contract_hash AS contract_hash
  FROM agent_instance_execution_invocations receipt JOIN release_agents agent ON agent.id=receipt.release_agent_id
  WHERE receipt.run_id=p_run)
 SELECT jsonb_build_object('identity',identity.value,'selections',COALESCE((SELECT jsonb_agg(
  jsonb_build_object('identity',identity.value,
   'scope',jsonb_build_object('instance_id',binding.instance_id,'revision_id',binding.instance_revision_id,
    'release_agent_id',binding.release_agent_id,'slot',binding.slot_key,'volume_id',binding.volume_id,
    'access_mode',binding.access_mode,'release_contract_hash',
      (SELECT jsonb_agg(get_byte(identity.contract_hash,i) ORDER BY i) FROM generate_series(0,31) i)),
   'declaration',jsonb_build_object('slot',binding.slot_key,'guest_path',binding.guest_path,
    'access_mode',binding.access_mode,'required',binding.slot_required,'minimum_capacity_bytes',binding.minimum_capacity_bytes),
   'origin',binding.provenance) ORDER BY binding.slot_key)
  FROM agent_instance_revision_volume_bindings binding WHERE binding.instance_revision_id=identity.revision_id),'[]'::jsonb))
 FROM identity
$$;
REVOKE ALL ON FUNCTION invocation119_selections(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;
-- The SECURITY INVOKER lease guard calls these read-only functions as the
-- actual worker. The assertion still checks SET ROLE/session_user; the
-- selection projection supplies metadata, never execution or IO authority.
-- Actor and PUBLIC execute privileges remain revoked.
GRANT EXECUTE ON FUNCTION require_invocation117_worker(),invocation119_selections(uuid) TO hephaestus_worker;

-- Initial readiness accepts only Ready; after acquisition Attached is legitimate
-- only together with the exact full own-set/fence predicate below. Neither a
-- foreign holder nor an expired/recovering fence can be renewed into authority.
CREATE FUNCTION invocation119_bindings_live(p_run uuid,p_instance uuid,p_revision uuid,cfg jsonb) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE binding agent_instance_revision_volume_bindings%ROWTYPE; volume agent_instance_state_volumes%ROWTYPE;
 grant_row agent_instance_volume_mount_grants%ROWTYPE; purpose volume_owned_backing_purposes%ROWTYPE;
BEGIN
 FOR binding IN SELECT * FROM agent_instance_revision_volume_bindings WHERE instance_revision_id=p_revision LOOP
   SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=binding.volume_id;
   SELECT * INTO grant_row FROM agent_instance_volume_mount_grants
     WHERE instance_revision_id=p_revision AND slot_key=binding.slot_key;
   IF volume.id IS NULL OR grant_row.id IS NULL OR binding.instance_id IS DISTINCT FROM p_instance
      OR volume.project_id IS DISTINCT FROM binding.project_id OR volume.state NOT IN ('ready','attached')
      OR volume.provisioning_state IS DISTINCT FROM 'ready' OR volume.filesystem_uuid IS NULL
      OR volume.host_id IS DISTINCT FROM cfg->'volume'->>'host_id'
      OR volume.host_path IS NULL OR left(volume.host_path,length(cfg->'volume'->>'root')+1) IS DISTINCT FROM (cfg->'volume'->>'root')||'/'
      OR volume.capacity_bytes<binding.minimum_capacity_bytes OR NOT owned_birth_is_live(volume.id)
      OR (volume.ready_retention_closure_id IS NOT NULL AND (volume.ready_retention_reopen_id IS NULL
        OR ready_retention_lease_reopening_matches(volume.ready_retention_reopen_id,p_instance,volume.id) IS DISTINCT FROM true))
      OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE volume_id=volume.id AND released_at IS NULL AND run_id IS DISTINCT FROM p_run)
      OR NOT private_volume_mount_binding_is_declared(p_revision,binding.slot_key)
      OR grant_row.instance_id IS DISTINCT FROM p_instance OR grant_row.release_agent_id IS DISTINCT FROM binding.release_agent_id
      OR grant_row.volume_id IS DISTINCT FROM binding.volume_id OR grant_row.access_mode IS DISTINCT FROM binding.access_mode
      OR grant_row.release_contract_hash IS DISTINCT FROM (SELECT runtime_contract_hash FROM release_agents WHERE id=binding.release_agent_id)
      OR binding.provenance NOT IN ('explicit','legacy_declaration')
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
REVOKE ALL ON FUNCTION invocation119_bindings_live(uuid,uuid,uuid,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_live(p_run uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE receipt agent_instance_execution_invocations%ROWTYPE; execution runs%ROWTYPE;
BEGIN
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 SELECT * INTO execution FROM runs WHERE id=p_run;
 RETURN invocation119_immutable(p_run) IS NOT DISTINCT FROM true
  AND execution.state IN ('queued','leasing_volume','provisioning')
  AND execution.cancel_requested_at IS NULL AND execution.volume_acquisition_closed_at IS NULL
  AND execution.outcome IS NULL AND execution.exit_code IS NULL AND execution.exit_signal IS NULL AND execution.failure IS NULL
  AND EXISTS(SELECT 1 FROM users WHERE id=receipt.actor_id AND status='active')
  AND check_permission('user',receipt.actor_id::text,'can_execute','agent_instance',receipt.instance_id::text) IS NOT DISTINCT FROM 1
  AND check_permission('user',receipt.actor_id::text,'can_use','release_agent',receipt.release_agent_id::text) IS NOT DISTINCT FROM 1
  AND EXISTS(SELECT 1 FROM agent_instances consumer JOIN agent_instance_revisions revision ON revision.id=receipt.revision_id
   JOIN recipe_prepared_instance_imports child ON child.instance_id=consumer.id
   JOIN release_agents source ON source.id=child.release_agent_id AND source.release_id=child.release_id
   JOIN releases published ON published.id=source.release_id
   WHERE consumer.id=receipt.instance_id AND consumer.version=2 AND consumer.run_gate_open AND consumer.state='active'
    AND consumer.active_revision_id=receipt.revision_id AND consumer.volume_mode='named'
    AND revision.instance_id=consumer.id AND revision.runnable
    AND recipe_cleanup_started(child.deployment_id) IS DISTINCT FROM true
    AND NOT EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=consumer.id)
    AND qualified_invocation_creation_lineage_matches(consumer.id) IS NOT DISTINCT FROM true
    AND qualified_invocation_import_lineage_matches(consumer.id) IS NOT DISTINCT FROM true
    AND published.state='published'
    AND source.runtime_contract_hash=sha256(convert_to(recipe_import107_json_bytes(source.runtime_contract,0),'UTF8'))
    AND recipe_effect_hash_json(source.runtime_contract_hash)=(convert_from(child.canonical_import,'UTF8')::jsonb)->'source'->'runtime_contract_hash'
    AND source.runtime_contract->>'image_reference'=(convert_from(child.canonical_import,'UTF8')::jsonb)->'source'->>'image')
  AND invocation119_bindings_live(p_run,receipt.instance_id,receipt.revision_id,receipt.config_json) IS NOT DISTINCT FROM true;
END $$;
REVOKE ALL ON FUNCTION invocation119_live(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_frozen_set(p_run uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; selected_count integer;
BEGIN
 SELECT * INTO execution FROM runs WHERE id=p_run;
 selected_count:=jsonb_array_length(invocation119_selections(p_run)->'selections');
 RETURN invocation119_immutable(p_run) IS NOT DISTINCT FROM true
  AND selected_count BETWEEN 0 AND 32
  AND execution.volume_selection_snapshot IS NOT DISTINCT FROM invocation119_selections(p_run)
  AND profiled_run_snapshot_matches(execution) IS NOT DISTINCT FROM true
  AND (SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=p_run)=selected_count
  AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease
   LEFT JOIN agent_instance_state_volumes volume ON volume.id=lease.volume_id
   LEFT JOIN agent_instance_revision_volume_bindings binding ON binding.instance_revision_id=execution.instance_revision_id AND binding.slot_key=lease.slot_key
   WHERE lease.run_id=p_run AND (volume.id IS NULL OR binding.volume_id IS DISTINCT FROM lease.volume_id
    OR ROW(lease.instance_id,lease.selected_revision_id,lease.selected_release_id,lease.selected_release_agent_id,
      lease.selected_project_id,lease.selection_origin,lease.access_mode,lease.guest_path,lease.slot_required,lease.minimum_capacity_bytes)
     IS DISTINCT FROM ROW(execution.instance_id,execution.instance_revision_id,execution.release_id,execution.release_agent_id,
      volume.project_id,binding.provenance,binding.access_mode,binding.guest_path,binding.slot_required,binding.minimum_capacity_bytes)
    OR lease.release_contract_hash IS DISTINCT FROM (SELECT runtime_contract_hash FROM release_agents WHERE id=execution.release_agent_id)
    OR lease.update_id IS NOT NULL OR lease.state IS DISTINCT FROM 'active' OR lease.released_at IS NOT NULL OR lease.recovering_at IS NOT NULL
    OR lease.expires_at<=clock_timestamp() OR volume.lease_generation IS DISTINCT FROM lease.fencing_token
    OR lease.host_id IS DISTINCT FROM execution.vm_host_id OR volume.host_id IS DISTINCT FROM lease.host_id
    OR volume.state NOT IN ('ready','attached') OR volume.provisioning_state IS DISTINCT FROM 'ready'
    OR EXISTS(SELECT 1 FROM agent_instance_volume_leases other WHERE other.volume_id=lease.volume_id AND other.id<>lease.id AND other.released_at IS NULL)));
END $$;
REVOKE ALL ON FUNCTION invocation119_frozen_set(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- Legitimate pre-acquisition has no history for this Run. Lost/released own
-- history can never be converted into a new generation by resetting a snapshot.
CREATE FUNCTION invocation119_open_set(p_run uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF invocation119_live(p_run) IS DISTINCT FROM true THEN RETURN false; END IF;
 IF execution.volume_selection_snapshot IS NOT NULL THEN RETURN invocation119_frozen_set(p_run); END IF;
 RETURN NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM agent_instance_revision_volume_bindings binding JOIN agent_instance_state_volumes volume ON volume.id=binding.volume_id
    WHERE binding.instance_revision_id=execution.instance_revision_id AND volume.state IS DISTINCT FROM 'ready');
END $$;
REVOKE ALL ON FUNCTION invocation119_open_set(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- All raw row locks are NOWAIT: a worker may already hold a child. Trusted
-- adapters block only on the actual sorted early advisory keys in fresh TXs.
-- Contention is held55P03, never expiry/absence or permission to replace a Run.
CREATE FUNCTION lock_qualified_invocation119(p_run uuid,p_config bytea) RETURNS runs
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE receipt agent_instance_execution_invocations%ROWTYPE; selected record; execution runs%ROWTYPE;
BEGIN
 PERFORM require_invocation117_worker();
 PERFORM reserve_invocation119(p_run);
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 IF invocation119_immutable(p_run) IS DISTINCT FROM true OR p_config IS DISTINCT FROM receipt.config_bytes THEN
  RAISE EXCEPTION 'complete consumed Invocation/config required' USING ERRCODE='P1190';
 END IF;
 PERFORM 1 FROM projects WHERE id=receipt.project_id FOR NO KEY UPDATE NOWAIT;
 PERFORM 1 FROM recipe_deployments WHERE id=receipt.deployment_id FOR UPDATE NOWAIT;
 PERFORM 1 FROM recipe_deployment_resources WHERE deployment_id=receipt.deployment_id AND resource_name=receipt.resource_name FOR UPDATE NOWAIT;
 PERFORM 1 FROM agent_instances WHERE id=receipt.instance_id FOR UPDATE NOWAIT;
 SELECT * INTO execution FROM runs WHERE id=p_run FOR UPDATE NOWAIT;
 FOR selected IN SELECT volume_id FROM agent_instance_revision_volume_bindings WHERE instance_revision_id=receipt.revision_id
   UNION SELECT volume_id FROM agent_instance_volume_leases WHERE run_id=p_run ORDER BY volume_id LOOP
  PERFORM 1 FROM agent_instance_state_volumes WHERE id=selected.volume_id FOR UPDATE NOWAIT;
 END LOOP;
 FOR selected IN SELECT id FROM agent_instance_volume_mount_grants WHERE instance_revision_id=receipt.revision_id ORDER BY id LOOP
  PERFORM 1 FROM agent_instance_volume_mount_grants WHERE id=selected.id FOR UPDATE NOWAIT;
 END LOOP;
 PERFORM 1 FROM agent_instance_volume_leases WHERE run_id=p_run ORDER BY volume_id,id FOR UPDATE NOWAIT;
 IF invocation119_immutable(p_run) IS DISTINCT FROM true OR p_config IS DISTINCT FROM receipt.config_bytes THEN
  RAISE EXCEPTION 'Invocation lineage changed under locks' USING ERRCODE='P1190';
 END IF;
 RETURN execution;
END $$;
REVOKE ALL ON FUNCTION lock_qualified_invocation119(uuid,bytea) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION lock_qualified_invocation119(uuid,bytea) TO hephaestus_worker;

CREATE FUNCTION read_qualified_invocation119(p_run uuid,p_start jsonb,p_config bytea) RETURNS SETOF runs
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
 execution:=lock_qualified_invocation119(p_run,p_config);
 IF NOT EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=p_run AND start_payload=p_start)
   OR invocation119_open_set(p_run) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'current Invocation runtime metadata is held' USING ERRCODE='P1191';
 END IF;
 RETURN NEXT execution;
END $$;
REVOKE ALL ON FUNCTION read_qualified_invocation119(uuid,jsonb,bytea) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION read_qualified_invocation119(uuid,jsonb,bytea) TO hephaestus_worker;

-- First consumption remains the protected117 queued-only operation. A known
-- consumption may replay later only with full119 current/frozen evidence.
CREATE FUNCTION consume_qualified_invocation119(p_run uuid,p_start jsonb,p_config bytea,p_namespace text,p_host text,p_vm text)
RETURNS SETOF runs LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_invocation117_worker();
 PERFORM reserve_invocation119(p_run);
 IF NOT EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=p_run) THEN
  RETURN QUERY SELECT * FROM consume_qualified_invocation117(p_run,p_start,p_config,p_namespace,p_host,p_vm);
 ELSE
  IF NOT EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=p_run
    AND ROW(vm_provider_namespace,vm_host_id,vm_id) IS NOT DISTINCT FROM ROW(p_namespace,p_host,p_vm)) THEN
   RAISE EXCEPTION 'Invocation replay provider changed' USING ERRCODE='P1190';
  END IF;
  RETURN QUERY SELECT * FROM read_qualified_invocation119(p_run,p_start,p_config);
 END IF;
END $$;
REVOKE ALL ON FUNCTION consume_qualified_invocation119(uuid,jsonb,bytea,text,text,text) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION consume_qualified_invocation119(uuid,jsonb,bytea,text,text,text) TO hephaestus_worker;

CREATE FUNCTION load_qualified_invocation119_selections(p_run uuid,p_config bytea) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM lock_qualified_invocation119(p_run,p_config);
 IF invocation119_open_set(p_run) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'current Invocation selection admission held' USING ERRCODE='P1191';
 END IF;
 RETURN invocation119_selections(p_run);
END $$;
REVOKE ALL ON FUNCTION load_qualified_invocation119_selections(uuid,bytea) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION load_qualified_invocation119_selections(uuid,bytea) TO hephaestus_worker;

CREATE FUNCTION assert_invocation119_transition(p_run uuid,p_next text) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
 PERFORM require_invocation117_worker();
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF invocation119_immutable(p_run) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'Invocation transition needs immutable consumed producer' USING ERRCODE='23000';
 END IF;
 IF p_next IN ('starting','running','succeeded') THEN
  RAISE EXCEPTION 'Invocation session/progress is not implemented by119-A' USING ERRCODE='0A000';
 ELSIF p_next='leasing_volume' THEN
  IF execution.state<>'queued' OR execution.state_version<>0 OR invocation119_open_set(p_run) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'first Invocation progress needs current queued admission' USING ERRCODE='23000';
  END IF;
 ELSIF p_next='provisioning' THEN
  IF execution.state<>'leasing_volume' OR invocation119_live(p_run) IS DISTINCT FROM true OR invocation119_frozen_set(p_run) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'Invocation Provisioning needs the complete frozen live set' USING ERRCODE='23000';
  END IF;
 ELSIF p_next='cleaned_up' THEN
  IF execution.state<>'cleaning_up' OR execution.volume_acquisition_closed_at IS NULL
    OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=p_run AND released_at IS NULL)
    OR NOT EXISTS(SELECT 1 FROM run_cleanup_targets target JOIN run_cleanup_completions done
      ON done.run_id=target.run_id AND done.generation=target.generation AND done.target_digest=target.target_digest
      WHERE target.run_id=p_run AND ROW(target.vm_provider_namespace,target.vm_host_id,target.vm_id)
       IS NOT DISTINCT FROM ROW(execution.vm_provider_namespace,execution.vm_host_id,execution.vm_id)
       AND run_cleanup_set_matches(p_run,target.generation,true)) THEN
   RAISE EXCEPTION 'Invocation cleanup lacks protected whole-set completion' USING ERRCODE='23000';
  END IF;
 END IF;
END $$;
REVOKE ALL ON FUNCTION assert_invocation119_transition(uuid,text) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION assert_invocation119_transition(uuid,text) TO hephaestus_worker;

CREATE FUNCTION acquire_qualified_invocation119(p_run uuid,p_config bytea,p_snapshot jsonb,p_host text,p_now timestamptz,p_expires timestamptz)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; binding agent_instance_revision_volume_bindings%ROWTYPE;
 volume agent_instance_state_volumes%ROWTYPE; contract bytea;
BEGIN
 execution:=lock_qualified_invocation119(p_run,p_config);
 IF execution.state NOT IN ('leasing_volume','provisioning') OR p_host IS DISTINCT FROM execution.vm_host_id
   OR p_now IS NULL OR p_expires IS NULL OR p_expires<=p_now OR p_expires<=clock_timestamp()
   OR p_snapshot IS DISTINCT FROM invocation119_selections(p_run) OR invocation119_open_set(p_run) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'Invocation acquisition needs exact current whole-set admission' USING ERRCODE='P1191';
 END IF;
 IF execution.volume_selection_snapshot IS NOT NULL THEN RETURN; END IF;
 -- Validate all bindings/holders before the first INSERT. Global volume locks
 -- already protect against concurrent next-fence allocation on another Run.
 IF EXISTS(SELECT 1 FROM agent_instance_revision_volume_bindings selected JOIN agent_instance_state_volumes backing ON backing.id=selected.volume_id
   WHERE selected.instance_revision_id=execution.instance_revision_id
    AND (backing.state IS DISTINCT FROM 'ready' OR backing.provisioning_state IS DISTINCT FROM 'ready'
      OR backing.host_id IS DISTINCT FROM p_host OR backing.lease_generation=9223372036854775807
      OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE volume_id=backing.id AND released_at IS NULL))) THEN
  RAISE EXCEPTION 'selected Invocation backing is globally held or uncertain' USING ERRCODE='P1191';
 END IF;
 SELECT runtime_contract_hash INTO contract FROM release_agents WHERE id=execution.release_agent_id;
 UPDATE runs SET volume_selection_snapshot=p_snapshot WHERE id=p_run AND volume_selection_snapshot IS NULL;
 FOR binding IN SELECT * FROM agent_instance_revision_volume_bindings WHERE instance_revision_id=execution.instance_revision_id ORDER BY volume_id LOOP
  SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=binding.volume_id;
  INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,
   acquired_at,heartbeat_at,expires_at,selection_origin,selected_revision_id,selected_release_id,selected_release_agent_id,
   selected_project_id,slot_key,access_mode,guest_path,slot_required,minimum_capacity_bytes,release_contract_hash)
  VALUES(gen_random_uuid(),volume.id,execution.instance_id,p_run,p_host,volume.lease_generation+1,'active',
   p_now,p_now,p_expires,binding.provenance,execution.instance_revision_id,execution.release_id,execution.release_agent_id,
   binding.project_id,binding.slot_key,binding.access_mode,binding.guest_path,binding.slot_required,binding.minimum_capacity_bytes,contract);
 END LOOP;
 IF invocation119_live(p_run) IS DISTINCT FROM true OR invocation119_frozen_set(p_run) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'Invocation acquisition failed complete frozen-set verification' USING ERRCODE='23000';
 END IF;
END $$;
REVOKE ALL ON FUNCTION acquire_qualified_invocation119(uuid,bytea,jsonb,text,timestamptz,timestamptz) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION acquire_qualified_invocation119(uuid,bytea,jsonb,text,timestamptz,timestamptz) TO hephaestus_worker;

CREATE FUNCTION refresh_qualified_invocation119(p_run uuid,p_config bytea,p_snapshot jsonb,p_host text,p_now timestamptz,p_expires timestamptz,p_attached boolean)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; held agent_instance_volume_leases%ROWTYPE;
BEGIN
 execution:=lock_qualified_invocation119(p_run,p_config);
 IF p_host IS DISTINCT FROM execution.vm_host_id OR p_snapshot IS DISTINCT FROM execution.volume_selection_snapshot
   OR p_now IS NULL OR p_expires IS NULL OR p_attached IS NULL OR p_expires<=p_now OR p_expires<=clock_timestamp()
   OR invocation119_live(p_run) IS DISTINCT FROM true OR invocation119_frozen_set(p_run) IS DISTINCT FROM true
   OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=p_run AND (p_now<heartbeat_at OR p_now>=expires_at)) THEN
  RAISE EXCEPTION 'Invocation refresh cannot revive expired, withdrawn or substituted full-set evidence' USING ERRCODE='P1191';
 END IF;
 FOR held IN SELECT * FROM agent_instance_volume_leases WHERE run_id=p_run ORDER BY volume_id,id LOOP
  UPDATE agent_instance_volume_leases SET heartbeat_at=p_now,expires_at=p_expires,
   attached_at=CASE WHEN p_attached THEN COALESCE(attached_at,p_now) ELSE attached_at END
   WHERE id=held.id AND fencing_token=held.fencing_token AND released_at IS NULL AND recovering_at IS NULL;
  IF NOT FOUND THEN RAISE EXCEPTION 'Invocation lease refresh CAS lost' USING ERRCODE='23000'; END IF;
  IF p_attached THEN
   UPDATE agent_instance_state_volumes SET state='attached',updated_at=p_now
    WHERE id=held.volume_id AND lease_generation=held.fencing_token AND host_id=p_host
      AND state IN ('ready','attached') AND provisioning_state='ready';
   IF NOT FOUND THEN RAISE EXCEPTION 'Invocation backing attach CAS lost' USING ERRCODE='23000'; END IF;
  END IF;
 END LOOP;
 IF invocation119_live(p_run) IS DISTINCT FROM true OR invocation119_frozen_set(p_run) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'Invocation refresh must preserve the complete current full set' USING ERRCODE='23000';
 END IF;
END $$;
REVOKE ALL ON FUNCTION refresh_qualified_invocation119(uuid,bytea,jsonb,text,timestamptz,timestamptz,boolean) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION refresh_qualified_invocation119(uuid,bytea,jsonb,text,timestamptz,timestamptz,boolean) TO hephaestus_worker;

CREATE OR REPLACE FUNCTION enforce_exact_run_volume_lease() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    execution runs%ROWTYPE;
    consumer agent_instances%ROWTYPE;
    agent release_agents%ROWTYPE;
    volume agent_instance_state_volumes%ROWTYPE;
    binding agent_instance_revision_volume_bindings%ROWTYPE;
    config bytea;
BEGIN
    IF TG_OP='UPDATE' THEN
        IF ROW(NEW.state,NEW.acquired_at,NEW.heartbeat_at,NEW.expires_at,NEW.attached_at,NEW.recovering_at,NEW.released_at)
          IS DISTINCT FROM ROW(OLD.state,OLD.acquired_at,OLD.heartbeat_at,OLD.expires_at,OLD.attached_at,OLD.recovering_at,OLD.released_at) THEN
            PERFORM require_run_cleanup_worker();
        END IF;
        SELECT * INTO execution FROM runs WHERE id=OLD.run_id;
        IF ROW(NEW.id,NEW.volume_id,NEW.instance_id,NEW.run_id,NEW.update_id,
            NEW.host_id,NEW.fencing_token,NEW.selection_origin,NEW.selected_revision_id,
            NEW.selected_release_id,NEW.selected_release_agent_id,NEW.selected_project_id,
            NEW.slot_key,NEW.access_mode,NEW.guest_path,NEW.slot_required,
            NEW.minimum_capacity_bytes,NEW.release_contract_hash)
            IS DISTINCT FROM ROW(OLD.id,OLD.volume_id,OLD.instance_id,OLD.run_id,OLD.update_id,
            OLD.host_id,OLD.fencing_token,OLD.selection_origin,OLD.selected_revision_id,
            OLD.selected_release_id,OLD.selected_release_agent_id,OLD.selected_project_id,
            OLD.slot_key,OLD.access_mode,OLD.guest_path,OLD.slot_required,
            OLD.minimum_capacity_bytes,OLD.release_contract_hash) THEN
            RAISE EXCEPTION 'lease selection identity is immutable' USING ERRCODE='23000';
        END IF;
        IF (OLD.selection_origin='historical_unproven'
            OR NOT EXISTS(SELECT 1 FROM runs WHERE id=OLD.run_id AND execution_profile IN ('legacy_scalar','owned_canonical'))) AND
            (NEW.heartbeat_at IS DISTINCT FROM OLD.heartbeat_at OR
             NEW.expires_at IS DISTINCT FROM OLD.expires_at OR
             NEW.attached_at IS DISTINCT FROM OLD.attached_at) THEN
            RAISE EXCEPTION 'historical unproven lease is recovery only' USING ERRCODE='23000';
        END IF;
        IF execution.run_kind='invocation' THEN
            IF NEW.acquired_at IS DISTINCT FROM OLD.acquired_at THEN
                RAISE EXCEPTION 'Invocation lease birth time is immutable' USING ERRCODE='23000';
            END IF;
            IF ROW(NEW.heartbeat_at,NEW.expires_at,NEW.attached_at)
                IS DISTINCT FROM ROW(OLD.heartbeat_at,OLD.expires_at,OLD.attached_at) THEN
                PERFORM require_invocation117_worker();
                SELECT config_bytes INTO config FROM run_invocation_consumptions WHERE run_id=OLD.run_id;
                PERFORM lock_qualified_invocation119(OLD.run_id,config);
                IF invocation119_live(OLD.run_id) IS DISTINCT FROM true OR invocation119_frozen_set(OLD.run_id) IS DISTINCT FROM true
                  OR NEW.heartbeat_at<OLD.heartbeat_at OR NEW.heartbeat_at>=OLD.expires_at OR NEW.expires_at<=NEW.heartbeat_at
                  OR NEW.expires_at<=clock_timestamp() OR (OLD.attached_at IS NOT NULL AND NEW.attached_at IS DISTINCT FROM OLD.attached_at) THEN
                    RAISE EXCEPTION 'Invocation cannot refresh withdrawn/expired/substituted whole-set evidence' USING ERRCODE='23000';
                END IF;
            END IF;
        END IF;
        RETURN NEW;
    END IF;
    IF NEW.selection_origin='historical_unproven' OR NEW.run_id IS NULL THEN
        RAISE EXCEPTION 'cannot manufacture historical run lease evidence' USING ERRCODE='23000';
    END IF;
    SELECT * INTO execution FROM runs WHERE id=NEW.run_id;
    IF execution.run_kind='invocation' THEN
 PERFORM require_invocation117_worker();
 SELECT config_bytes INTO config FROM run_invocation_consumptions WHERE run_id=NEW.run_id;
 execution:=lock_qualified_invocation119(NEW.run_id,config);
 SELECT * INTO binding FROM agent_instance_revision_volume_bindings WHERE instance_revision_id=execution.instance_revision_id AND slot_key=NEW.slot_key;
 SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=NEW.volume_id;
 IF execution.state NOT IN ('leasing_volume','provisioning') OR invocation119_live(NEW.run_id) IS DISTINCT FROM true
  OR execution.volume_selection_snapshot IS NULL
  OR execution.volume_selection_snapshot IS DISTINCT FROM invocation119_selections(NEW.run_id)
  OR NEW.id IS NULL OR NEW.update_id IS NOT NULL OR NEW.state IS DISTINCT FROM 'active'
  OR NEW.released_at IS NOT NULL OR NEW.recovering_at IS NOT NULL OR NEW.attached_at IS NOT NULL
  OR NEW.acquired_at IS NULL OR NEW.heartbeat_at IS DISTINCT FROM NEW.acquired_at
  OR NEW.expires_at<=NEW.acquired_at OR NEW.expires_at<=clock_timestamp()
  OR volume.id IS NULL OR binding.volume_id IS DISTINCT FROM volume.id OR NEW.selection_origin NOT IN ('explicit','legacy_declaration')
  OR ROW(NEW.instance_id,NEW.selected_revision_id,NEW.selected_release_id,NEW.selected_release_agent_id,
   NEW.selected_project_id,NEW.selection_origin,NEW.access_mode,NEW.guest_path,NEW.slot_required,
   NEW.minimum_capacity_bytes,NEW.release_contract_hash)
   IS DISTINCT FROM ROW(execution.instance_id,execution.instance_revision_id,execution.release_id,execution.release_agent_id,
    binding.project_id,binding.provenance,binding.access_mode,binding.guest_path,binding.slot_required,binding.minimum_capacity_bytes,
    (SELECT runtime_contract_hash FROM release_agents WHERE id=execution.release_agent_id))
  OR volume.state IS DISTINCT FROM 'ready' OR volume.provisioning_state IS DISTINCT FROM 'ready'
  OR NEW.host_id IS DISTINCT FROM execution.vm_host_id OR volume.host_id IS DISTINCT FROM NEW.host_id
  OR NEW.fencing_token IS DISTINCT FROM volume.lease_generation+1
  OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE volume_id=volume.id AND released_at IS NULL) THEN
  RAISE EXCEPTION 'Invocation lease is not an exact fresh admitted binding/fence' USING ERRCODE='23000';
 END IF;
 UPDATE agent_instance_state_volumes SET lease_generation=NEW.fencing_token,updated_at=now() WHERE id=volume.id;
      RETURN NEW;
    END IF;
    -- Same global order as permanent closure: consumer -> run -> resource.
    SELECT * INTO consumer FROM agent_instances WHERE id=NEW.instance_id FOR UPDATE;
    SELECT * INTO execution FROM runs WHERE id=NEW.run_id FOR UPDATE;
    IF execution.execution_profile NOT IN ('legacy_scalar','owned_canonical')
       OR (NEW.selection_origin IN ('explicit','legacy_declaration') AND execution.execution_profile<>'owned_canonical')
       OR execution.instance_id IS DISTINCT FROM consumer.id
       OR execution.volume_acquisition_closed_at IS NOT NULL
       OR (execution.vm_plan_admission_required AND NOT EXISTS(SELECT 1 FROM run_vm_plan_admissions WHERE run_id=execution.id AND consumed_at IS NOT NULL AND consumption_kind='provision'))
       OR execution.state NOT IN ('leasing_volume','provisioning')
       OR execution.cancel_requested_at IS NOT NULL THEN
        RAISE EXCEPTION 'run is closed to new volume acquisitions' USING ERRCODE='23000';
    END IF;
    PERFORM assert_instance_accepts_work(consumer.id);
    SELECT * INTO agent FROM release_agents WHERE id=execution.release_agent_id;
    SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=NEW.volume_id FOR UPDATE;
    IF agent.id IS NULL OR agent.release_id IS DISTINCT FROM execution.release_id
       OR NOT EXISTS(SELECT 1 FROM agent_instance_revisions revision
            WHERE revision.id=execution.instance_revision_id AND revision.instance_id=consumer.id
            AND revision.release_agent_id=agent.id)
       OR volume.project_id IS DISTINCT FROM consumer.project_id THEN
        RAISE EXCEPTION 'lease differs from exact run consumer' USING ERRCODE='23000';
    END IF;
    IF NOT EXISTS(SELECT 1 FROM releases WHERE id=agent.release_id AND state='published') THEN
        RAISE EXCEPTION 'selected release is not published' USING ERRCODE='23000';
    END IF;
    IF NEW.selection_origin='legacy_origin' AND execution.run_kind='update' THEN
        IF consumer.state<>'updating' OR consumer.run_gate_open OR
           NOT EXISTS(SELECT 1 FROM agent_updates candidate
                WHERE candidate.instance_id=consumer.id
                AND candidate.candidate_revision_id=execution.instance_revision_id
                AND candidate.hook_run_id=execution.id AND candidate.state='hook_running') THEN
            RAISE EXCEPTION 'legacy update hook is not the admitted candidate' USING ERRCODE='23000';
        END IF;
    ELSE
        IF execution.run_kind<>'normal' OR consumer.state NOT IN ('active','update_rejected')
           OR NOT consumer.run_gate_open
           OR consumer.active_revision_id IS DISTINCT FROM execution.instance_revision_id
           OR NOT EXISTS(SELECT 1 FROM agent_instance_revisions revision
                WHERE revision.id=execution.instance_revision_id AND revision.runnable) THEN
            RAISE EXCEPTION 'exact normal run revision is not active and admitted' USING ERRCODE='23000';
        END IF;
    END IF;
    IF NEW.selection_origin='legacy_origin' THEN
        IF consumer.volume_mode<>'legacy' OR NOT agent.requires_state
           OR consumer.state_volume_id IS DISTINCT FROM volume.id
           OR volume.instance_id IS DISTINCT FROM consumer.id
           OR jsonb_array_length(COALESCE(agent.runtime_contract->'volume_slots','[]'::jsonb))<>0 THEN
            RAISE EXCEPTION 'legacy origin is not proven' USING ERRCODE='23000';
        END IF;
        IF num_nonnulls(NEW.selected_revision_id,NEW.selected_release_id,
            NEW.selected_release_agent_id,NEW.selected_project_id,NEW.slot_key,
            NEW.access_mode,NEW.guest_path,NEW.slot_required,NEW.minimum_capacity_bytes,
            NEW.release_contract_hash)>0 AND
            ROW(NEW.selected_revision_id,NEW.selected_release_id,NEW.selected_release_agent_id,
                NEW.selected_project_id,NEW.slot_key,NEW.access_mode,NEW.guest_path,
                NEW.slot_required,NEW.minimum_capacity_bytes,NEW.release_contract_hash)
            IS DISTINCT FROM ROW(execution.instance_revision_id,execution.release_id,
                execution.release_agent_id,consumer.project_id,'state'::text,'read_write'::text,
                '/var/lib/hephaestus'::text,true,1::bigint,agent.runtime_contract_hash) THEN
            RAISE EXCEPTION 'supplied legacy selection is not exact' USING ERRCODE='23000';
        END IF;
        -- Also covers old scalar callers without a fabricated named binding.
        NEW.selected_revision_id:=execution.instance_revision_id;
        NEW.selected_release_id:=execution.release_id;
        NEW.selected_release_agent_id:=execution.release_agent_id;
        NEW.selected_project_id:=consumer.project_id;
        NEW.slot_key:='state';NEW.access_mode:='read_write';
        NEW.guest_path:='/var/lib/hephaestus';NEW.slot_required:=true;
        NEW.minimum_capacity_bytes:=1;NEW.release_contract_hash:=agent.runtime_contract_hash;
    ELSE
        SELECT * INTO binding FROM agent_instance_revision_volume_bindings
            WHERE instance_revision_id=execution.instance_revision_id AND slot_key=NEW.slot_key;
        IF consumer.volume_mode<>'named' OR execution.run_kind<>'normal'
           OR NOT named_volume_dispatch_supported(consumer.id)
           OR binding.volume_id IS DISTINCT FROM volume.id
           OR binding.provenance IS DISTINCT FROM NEW.selection_origin
           OR ROW(NEW.selected_revision_id,NEW.selected_release_id,NEW.selected_release_agent_id,
                  NEW.selected_project_id,NEW.access_mode,NEW.guest_path,NEW.slot_required,
                  NEW.minimum_capacity_bytes,NEW.release_contract_hash)
              IS DISTINCT FROM ROW(execution.instance_revision_id,execution.release_id,agent.id,
                  consumer.project_id,binding.access_mode,binding.guest_path,binding.slot_required,
                  binding.minimum_capacity_bytes,agent.runtime_contract_hash)
           OR NOT private_volume_mount_binding_is_declared(binding.instance_revision_id,binding.slot_key)
           OR NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_grants grant_record
                WHERE grant_record.instance_id=consumer.id
                AND grant_record.instance_revision_id=binding.instance_revision_id
                AND grant_record.release_agent_id=agent.id AND grant_record.slot_key=binding.slot_key
                AND grant_record.volume_id=volume.id AND grant_record.access_mode=binding.access_mode
                AND grant_record.release_contract_hash=agent.runtime_contract_hash
                AND private_volume_mount_source_is_live(grant_record.created_by,volume.id)
                AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_revocations revoked
                    WHERE revoked.grant_id=grant_record.id))
           OR check_permission('agent_instance',consumer.id::text,'agent_attach',
                'state_volume',volume.id::text)<>1 THEN
            RAISE EXCEPTION 'exact named mount authority is unavailable' USING ERRCODE='42501';
        END IF;
    END IF;
    IF volume.state<>'ready' OR volume.provisioning_state<>'ready'
       OR volume.host_id IS NULL OR NEW.host_id IS DISTINCT FROM volume.host_id
       OR NEW.fencing_token IS DISTINCT FROM volume.lease_generation+1 THEN
        RAISE EXCEPTION 'volume is not ready on the exact host and next fence' USING ERRCODE='23000';
    END IF;
    IF volume.capacity_bytes<NEW.minimum_capacity_bytes THEN
        RAISE EXCEPTION 'volume capacity is below the exact declaration' USING ERRCODE='23000';
    END IF;
    UPDATE agent_instance_state_volumes SET lease_generation=NEW.fencing_token,updated_at=now()
        WHERE id=NEW.volume_id;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_exact_run_volume_lease() FROM PUBLIC;

CREATE OR REPLACE FUNCTION profiled_run_locked(p_run uuid) RETURNS runs
LANGUAGE plpgsql AS $$
DECLARE owner uuid; execution runs%ROWTYPE;
BEGIN
    PERFORM require_run_cleanup_worker();
    SELECT * INTO execution FROM runs WHERE id=p_run;
    IF execution.run_kind='invocation' THEN
        RETURN lock_qualified_invocation119(p_run,(SELECT config_bytes FROM run_invocation_consumptions WHERE run_id=p_run));
    END IF;
    SELECT instance_id INTO owner FROM runs WHERE id=p_run;
    IF owner IS NULL THEN RAISE EXCEPTION 'Run not found' USING ERRCODE='23000'; END IF;
    PERFORM id FROM agent_instances WHERE id=owner FOR UPDATE;
    SELECT * INTO execution FROM runs WHERE id=p_run AND instance_id=owner FOR UPDATE;
    IF execution.execution_profile NOT IN ('legacy_scalar','owned_canonical') THEN
        RAISE EXCEPTION 'Run history has no positive execution profile' USING ERRCODE='23000';
    END IF;
    -- Same global resource order as acquisition and canonical cleanup. Include
    -- the released scalar tuple for read-only replay, never reset its resource.
    PERFORM id FROM agent_instance_state_volumes WHERE id=execution.volume_id OR id IN (
        SELECT volume_id FROM agent_instance_volume_leases WHERE run_id=p_run AND released_at IS NULL)
        ORDER BY id FOR UPDATE;
    PERFORM id FROM agent_instance_volume_leases WHERE run_id=p_run ORDER BY volume_id,id FOR UPDATE;
    RETURN execution;
END $$;

CREATE OR REPLACE FUNCTION profiled_run_pins_match(execution runs,p_live boolean) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT CASE WHEN execution.run_kind='invocation' THEN
      invocation119_immutable(execution.id) AND (NOT p_live OR invocation119_live(execution.id))
      ELSE (SELECT execution.execution_profile IN ('legacy_scalar','owned_canonical') AND EXISTS(
        SELECT 1 FROM agent_instances consumer
        JOIN agent_instance_revisions revision ON revision.id=execution.instance_revision_id
          AND revision.instance_id=consumer.id AND revision.release_agent_id=execution.release_agent_id
        JOIN release_agents agent ON agent.id=revision.release_agent_id AND agent.release_id=execution.release_id
        JOIN releases release ON release.id=agent.release_id
        WHERE consumer.id=execution.instance_id AND agent.requires_state=execution.requires_state
          AND octet_length(agent.runtime_contract_hash)=32
          AND (execution.execution_profile<>'legacy_scalar' OR legacy_scalar_run_matches(execution))
          AND (NOT p_live OR (release.state='published' AND revision.runnable
            AND execution.cancel_requested_at IS NULL AND execution.volume_acquisition_closed_at IS NULL
            AND NOT EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=consumer.id)
            AND (consumer.volume_mode<>'named' OR named_volume_dispatch_supported(consumer.id))
            AND ((execution.run_kind='normal' AND consumer.active_revision_id=revision.id
                AND consumer.run_gate_open AND consumer.state IN ('active','update_rejected')
                AND EXISTS(SELECT 1 FROM agent_attachments attachment WHERE attachment.id=execution.attachment_id
                  AND attachment.instance_id=consumer.id AND attachment.enabled AND attachment.removed_at IS NULL))
              OR (execution.run_kind='update' AND consumer.state='updating' AND NOT consumer.run_gate_open
                AND EXISTS(SELECT 1 FROM agent_updates update WHERE update.hook_run_id=execution.id
                  AND update.instance_id=consumer.id AND update.candidate_revision_id=revision.id
                  AND update.state='hook_running'))))))) END
$$;

CREATE OR REPLACE FUNCTION profiled_run_binding_matches(execution runs) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE held agent_instance_volume_leases%ROWTYPE; count_held bigint;
BEGIN
    IF execution.run_kind='invocation' THEN
        RETURN num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)=0
          AND execution.vm_id=execution.id::text AND execution.vm_binding_kind='planned'
          AND invocation119_live(execution.id) AND invocation119_frozen_set(execution.id);
    END IF;
    IF execution.vm_id IS NULL OR NOT run_cleanup_identifier_is_valid(execution.vm_id)
       OR num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token) NOT IN (0,3) THEN RETURN false; END IF;
    SELECT count(*) INTO count_held FROM agent_instance_volume_leases WHERE run_id=execution.id AND released_at IS NULL;
    IF execution.execution_profile='legacy_scalar' THEN
        IF NOT execution.requires_state THEN
            RETURN execution.volume_id IS NULL AND NOT EXISTS(
                SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id);
        END IF;
        IF count_held<>1 THEN RETURN false; END IF;
        SELECT * INTO held FROM agent_instance_volume_leases WHERE id=execution.lease_id AND run_id=execution.id;
        IF held.id IS NULL OR legacy_scalar_lease_matches(held) IS NOT TRUE
           OR ROW(held.volume_id,held.fencing_token) IS DISTINCT FROM ROW(execution.volume_id,execution.lease_fencing_token) THEN RETURN false; END IF;
    ELSE
        IF execution.vm_binding_kind IS DISTINCT FROM 'planned' OR execution.vm_id<>execution.id::text
           OR execution.vm_provider_namespace IS NULL OR execution.vm_host_id IS NULL
           OR execution.volume_selection_snapshot IS NULL
           OR jsonb_array_length(execution.volume_selection_snapshot->'selections')<>count_held
           OR profiled_run_snapshot_matches(execution) IS NOT TRUE
           OR EXISTS(SELECT 1 FROM run_vm_plan_admissions WHERE run_id=execution.id
               AND (consumed_at IS NULL OR consumption_kind IS DISTINCT FROM 'provision')) THEN RETURN false; END IF;
        -- The existing checked complete-set acquisition freezes the snapshot.
        -- Only its singleton proven LegacyOrigin can have a scalar projection.
        IF execution.volume_id IS NOT NULL THEN
            SELECT * INTO held FROM agent_instance_volume_leases WHERE id=execution.lease_id AND run_id=execution.id;
            IF count_held<>1 OR held.selection_origin IS DISTINCT FROM 'legacy_origin'
              OR ROW(held.volume_id,held.fencing_token) IS DISTINCT FROM ROW(execution.volume_id,execution.lease_fencing_token) THEN RETURN false; END IF;
        ELSIF EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id
            AND released_at IS NULL AND selection_origin='legacy_origin') THEN RETURN false; END IF;
    END IF;
    RETURN NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease
        JOIN agent_instance_state_volumes volume ON volume.id=lease.volume_id
        WHERE lease.run_id=execution.id AND lease.released_at IS NULL
          AND (lease.state<>'active' OR lease.recovering_at IS NOT NULL OR lease.expires_at<=clock_timestamp()
            OR volume.provisioning_state<>'ready' OR volume.state NOT IN ('ready','attached')
            OR volume.host_id IS DISTINCT FROM lease.host_id OR volume.lease_generation<>lease.fencing_token
            OR lease.selected_revision_id IS DISTINCT FROM execution.instance_revision_id
            OR lease.selected_release_id IS DISTINCT FROM execution.release_id
            OR lease.selected_release_agent_id IS DISTINCT FROM execution.release_agent_id
            OR lease.selected_project_id IS DISTINCT FROM volume.project_id
            OR lease.release_contract_hash IS DISTINCT FROM (SELECT runtime_contract_hash FROM release_agents WHERE id=execution.release_agent_id)
            OR lease.instance_id IS DISTINCT FROM execution.instance_id
            OR (execution.execution_profile='owned_canonical' AND lease.host_id IS DISTINCT FROM execution.vm_host_id)));
END $$;

CREATE OR REPLACE FUNCTION assert_profiled_run_transition(p_run uuid,p_next text) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
    PERFORM require_run_cleanup_worker();
    SELECT * INTO execution FROM runs WHERE id=p_run;
    IF execution.run_kind='invocation' THEN
        PERFORM assert_invocation119_transition(p_run,p_next);
        RETURN;
    END IF;
    IF profiled_run_pins_match(execution,false) IS NOT TRUE THEN
        RAISE EXCEPTION 'transition requires exact positive released Run pins' USING ERRCODE='23000';
    END IF;
    IF p_next IN ('leasing_volume','provisioning','starting','running') THEN
        IF profiled_run_pins_match(execution,true) IS NOT TRUE
           OR (p_next<>'leasing_volume' AND profiled_run_binding_matches(execution) IS NOT TRUE) THEN
            RAISE EXCEPTION 'new IO needs open exact profile, pins and binding' USING ERRCODE='23000';
        END IF;
    ELSIF p_next='cleaned_up' THEN
        IF execution.state<>'cleaning_up' OR execution.volume_acquisition_closed_at IS NULL
           OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id AND released_at IS NULL)
           OR (execution.execution_profile='legacy_scalar' AND legacy_scalar_terminal_matches(execution) IS NOT TRUE)
           OR (execution.execution_profile='owned_canonical' AND NOT EXISTS(
               SELECT 1 FROM run_cleanup_completions completion WHERE completion.run_id=execution.id
                 AND run_cleanup_set_matches(execution.id,completion.generation,true))) THEN
            RAISE EXCEPTION 'terminal profile transition lacks exact closed completion evidence' USING ERRCODE='23000';
        END IF;
    END IF;
END $$;

CREATE OR REPLACE FUNCTION enforce_instance_work_admission() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE config bytea;
BEGIN
 IF TG_TABLE_NAME='agent_instance_volume_leases' THEN
  IF EXISTS(SELECT 1 FROM runs WHERE id=NEW.run_id AND run_kind='invocation') THEN
   PERFORM require_invocation117_worker();
   SELECT config_bytes INTO config FROM run_invocation_consumptions WHERE run_id=NEW.run_id;
   PERFORM lock_qualified_invocation119(NEW.run_id,config);
   IF invocation119_live(NEW.run_id) IS DISTINCT FROM true THEN
    RAISE EXCEPTION 'Invocation lease admission needs current qualified producer' USING ERRCODE='23000';
   END IF;
   RETURN NEW;
  END IF;
 END IF;
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

CREATE OR REPLACE FUNCTION enforce_instance_run_progress() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE config bytea;
BEGIN
    IF OLD.run_kind='invocation' AND NEW.state IS DISTINCT FROM OLD.state THEN
        PERFORM require_invocation117_worker();
        SELECT config_bytes INTO config FROM run_invocation_consumptions WHERE run_id=OLD.id;
        PERFORM lock_qualified_invocation119(OLD.id,config);
        PERFORM assert_invocation119_transition(OLD.id,NEW.state);
        RETURN NEW;
    END IF;
    IF NEW.state IS DISTINCT FROM OLD.state AND NEW.state IN ('leasing_volume','provisioning','starting','running') THEN
        PERFORM assert_instance_accepts_work(NEW.instance_id);
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_instance_run_progress() FROM PUBLIC;

-- Existing invoker-rights profile/lease guards call these narrow worker-only
-- readers. They grant metadata predicates, never caller or physical authority.
GRANT EXECUTE ON FUNCTION invocation119_immutable(uuid),invocation119_live(uuid),invocation119_frozen_set(uuid),invocation119_open_set(uuid) TO hephaestus_worker;

CREATE FUNCTION guard_invocation119_snapshot() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE config bytea;
BEGIN
 IF OLD.run_kind<>'invocation' THEN RETURN NEW; END IF;
 IF NEW.volume_selection_snapshot IS DISTINCT FROM OLD.volume_selection_snapshot THEN
  PERFORM require_invocation117_worker();
  SELECT config_bytes INTO config FROM run_invocation_consumptions WHERE run_id=OLD.id;
  PERFORM lock_qualified_invocation119(OLD.id,config);
  IF OLD.volume_selection_snapshot IS NOT NULL OR OLD.state NOT IN ('leasing_volume','provisioning')
    OR invocation119_live(OLD.id) IS DISTINCT FROM true
    OR NEW.volume_selection_snapshot IS DISTINCT FROM invocation119_selections(OLD.id)
    OR EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=OLD.id) THEN
   RAISE EXCEPTION 'Invocation snapshot must freeze exact first acquisition only' USING ERRCODE='23000';
  END IF;
 END IF;
 IF NEW.state IS DISTINCT FROM OLD.state AND NEW.state_version IS DISTINCT FROM OLD.state_version+1 THEN
  RAISE EXCEPTION 'Invocation state progress requires exact version advance' USING ERRCODE='23000';
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_invocation119_snapshot() FROM PUBLIC;
CREATE TRIGGER invocation119_snapshot_guard BEFORE UPDATE ON runs FOR EACH ROW EXECUTE FUNCTION guard_invocation119_snapshot();

CREATE FUNCTION validate_invocation119_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE p_run uuid; execution runs%ROWTYPE;
BEGIN
 IF TG_TABLE_NAME='runs' THEN
  IF NEW.run_kind<>'invocation' THEN RETURN NULL; END IF;
  p_run:=NEW.id;
  IF NEW.state IS NOT DISTINCT FROM OLD.state AND NEW.volume_selection_snapshot IS NOT DISTINCT FROM OLD.volume_selection_snapshot THEN RETURN NULL; END IF;
 ELSE
  p_run:=NEW.run_id;
  IF TG_OP='UPDATE' THEN
   IF ROW(NEW.heartbeat_at,NEW.expires_at,NEW.attached_at) IS NOT DISTINCT FROM ROW(OLD.heartbeat_at,OLD.expires_at,OLD.attached_at) THEN RETURN NULL; END IF;
  END IF;
 END IF;
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.run_kind IS DISTINCT FROM 'invocation' THEN RETURN NULL; END IF;
 PERFORM require_invocation117_worker();
 -- Cleanup has independent existing receipt/full-set proof. It must remain
 -- possible after Execute/Source/gate loss and cannot be mistaken for admission.
 -- Failed/Cancelled outcomes do not clear a fence or prove physical quiescence;
 -- canonical receipts remain mandatory before any reuse or CleanedUp.
 -- A lease birth must finish the complete frozen set even if the same TX
 -- later records a failure/cleanup outcome. Those outcomes cannot adopt a
 -- partial acquisition or bypass the birth fence invariant.
 IF TG_TABLE_NAME='agent_instance_volume_leases' AND TG_OP='INSERT' THEN
  IF invocation119_frozen_set(p_run) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'Invocation lease birth lacks complete frozen acquisition' USING ERRCODE='23000'; END IF;
 END IF;
 IF execution.state IN ('failed','cancelled','cleaning_up','cleaned_up') THEN RETURN NULL; END IF;
 IF execution.state IN ('starting','running') OR invocation119_live(p_run) IS DISTINCT FROM true
   OR (execution.volume_selection_snapshot IS NOT NULL AND invocation119_frozen_set(p_run) IS DISTINCT FROM true)
   OR (execution.state='provisioning' AND invocation119_frozen_set(p_run) IS DISTINCT FROM true) THEN
  RAISE EXCEPTION 'Invocation transaction lacks complete current119 acquisition/progress evidence' USING ERRCODE='23000';
 END IF;
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_invocation119_commit() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER invocation119_run_complete AFTER UPDATE ON runs
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_invocation119_commit();
CREATE CONSTRAINT TRIGGER invocation119_lease_complete AFTER INSERT OR UPDATE ON agent_instance_volume_leases
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_invocation119_commit();
-- Retained/reopened116 and original113 first-birth lease exclusion triggers stay
-- installed unchanged. Canonical cleanup target/receipt/completion and release
-- guards stay unchanged. No lease/session/progress exception is global.

-- Global volume fencing cannot be advanced independently of a held Invocation
-- lease. This immutable consistency check does not depend on execution rights,
-- expiry, or the live gate, and does not prevent proven canonical release.
CREATE FUNCTION validate_invocation119_volume_fence() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF EXISTS(SELECT 1 FROM agent_instance_volume_leases lease JOIN runs execution ON execution.id=lease.run_id
   JOIN agent_instance_state_volumes volume ON volume.id=lease.volume_id
   WHERE volume.id=NEW.id AND execution.run_kind='invocation' AND lease.released_at IS NULL
    AND (invocation119_immutable(execution.id) IS DISTINCT FROM true
      OR volume.lease_generation IS DISTINCT FROM lease.fencing_token OR volume.host_id IS DISTINCT FROM lease.host_id)) THEN
  RAISE EXCEPTION 'held Invocation requires exact global volume fence' USING ERRCODE='23000';
 END IF;
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_invocation119_volume_fence() FROM PUBLIC,hephaestus_app,hephaestus_worker;
CREATE CONSTRAINT TRIGGER invocation119_volume_fence AFTER UPDATE ON agent_instance_state_volumes
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_invocation119_volume_fence();
