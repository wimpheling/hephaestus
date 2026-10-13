-- PRIVATE119-B1, AFTER immutable reviewed119-A. Worker metadata only.
-- Requires Core's reviewed 64-bit length-framed hashes. SQL timestamps are
-- microsecond-exact BEFORE Core identity creation; original nanosecond decimal
-- inputs are checked, never silently rounded. No historical rows are rewritten.
-- Qualified issuance has a fixed maximum of3600 seconds AND every held lease
-- expiry. The ordinary runtime session policy and global named helper stay unchanged.
-- Saved invoker identity is recorded DATA checked against current Execute/Use;
-- it is never reconstructed as an AuthenticatedIdentity or attachment principal.

CREATE FUNCTION invocation119_hash_frame(value bytea) RETURNS bytea
LANGUAGE sql IMMUTABLE STRICT SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT int8send(octet_length(value)::bigint)||value
$$;
REVOKE ALL ON FUNCTION invocation119_hash_frame(bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_i128(value numeric) RETURNS bytea
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE remaining numeric; result bytea:=decode(repeat('00',16),'hex'); byte_index integer;
BEGIN
 IF value<>trunc(value) OR value < -170141183460469231731687303715884105728
   OR value>170141183460469231731687303715884105727 THEN
  RAISE EXCEPTION 'qualified119 signed128 timestamp invalid' USING ERRCODE='22003'; END IF;
 remaining:=CASE WHEN value<0 THEN value+340282366920938463463374607431768211456 ELSE value END;
 FOR byte_index IN REVERSE 15..0 LOOP
  result:=set_byte(result,byte_index,mod(remaining,256)::integer);
  remaining:=div(remaining,256);
 END LOOP;
 RETURN result;
END $$;
REVOKE ALL ON FUNCTION invocation119_i128(numeric) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_snapshot_hash(p_run uuid,p_model text) RETURNS bytea
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; binding agent_capability_bindings%ROWTYPE; value bytea; number bigint;
BEGIN
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.id IS NULL OR p_model IS NULL OR length(p_model) NOT BETWEEN 1 AND 128 THEN
  RAISE EXCEPTION 'exact qualified snapshot identity required' USING ERRCODE='23000'; END IF;
 SELECT count(*) INTO number FROM agent_capability_bindings WHERE instance_revision_id=execution.instance_revision_id;
 value:=invocation119_hash_frame(convert_to('hephaestus.authorization-snapshot.v1','UTF8'))
   ||invocation119_hash_frame(convert_to('agent_instance','UTF8'))
   ||invocation119_hash_frame(uuid_send(execution.instance_id))
   ||invocation119_hash_frame(uuid_send(execution.instance_revision_id))
   ||invocation119_hash_frame(convert_to(p_model,'UTF8'))
   ||invocation119_hash_frame(int8send(number));
 FOR binding IN SELECT * FROM agent_capability_bindings WHERE instance_revision_id=execution.instance_revision_id
   ORDER BY slot_key COLLATE "C" LOOP
  value:=value||invocation119_hash_frame(binding.normalized_hash);
 END LOOP;
 RETURN sha256(value);
END $$;
REVOKE ALL ON FUNCTION invocation119_snapshot_hash(uuid,text) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION invocation119_snapshot_hash(uuid,text) TO hephaestus_worker;

CREATE FUNCTION invocation119_identity_hash(p_run uuid,p_snapshot bytea,p_issued numeric,p_expires numeric) RETURNS bytea
LANGUAGE sql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT sha256(invocation119_hash_frame(convert_to('hephaestus.runtime-session-identity.v1','UTF8'))
  ||invocation119_hash_frame(uuid_send(execution.id))
  ||invocation119_hash_frame(convert_to('agent_instance','UTF8'))
  ||invocation119_hash_frame(uuid_send(execution.instance_id))
  ||invocation119_hash_frame(uuid_send(execution.instance_revision_id))
  ||invocation119_hash_frame(convert_to('run','UTF8'))
  ||invocation119_hash_frame(uuid_send(execution.id))
  ||invocation119_hash_frame(uuid_send(execution.id))
  ||invocation119_hash_frame(p_snapshot)
  ||invocation119_hash_frame(invocation119_i128(p_issued))
  ||invocation119_hash_frame(invocation119_i128(p_expires)))
 FROM runs execution WHERE execution.id=p_run
$$;
REVOKE ALL ON FUNCTION invocation119_identity_hash(uuid,bytea,numeric,numeric) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION invocation119_identity_hash(uuid,bytea,numeric,numeric) TO hephaestus_worker;

CREATE FUNCTION invocation119_no_git(p_run uuid) RETURNS boolean
LANGUAGE sql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT EXISTS(SELECT 1 FROM runs execution WHERE execution.id=p_run AND execution.attachment_id IS NULL
  AND NOT EXISTS(SELECT 1 FROM release_capability_requirements requirement WHERE requirement.release_agent_id=execution.release_agent_id
    AND (requirement.required_operations||requirement.optional_operations)&&ARRAY['git_read','create_ref','update_ref','force_update_ref','delete_ref','create_tag','delete_tag','trigger_run','manage_attachments']::text[])
  AND NOT EXISTS(SELECT 1 FROM agent_git_capability_bindings binding WHERE binding.instance_revision_id=execution.instance_revision_id)
  AND NOT EXISTS(SELECT 1 FROM run_git_authority_snapshots WHERE snapshot_id=p_run))
$$;
REVOKE ALL ON FUNCTION invocation119_no_git(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_operations_frame(operations text[]) RETURNS bytea
LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE value bytea:=invocation119_hash_frame(int8send(cardinality(operations)::bigint)); operation text;
BEGIN
 FOR operation IN SELECT item FROM unnest(operations) item ORDER BY item COLLATE "C" LOOP
  value:=value||invocation119_hash_frame(convert_to(operation,'UTF8'));
 END LOOP;
 RETURN value;
END $$;
REVOKE ALL ON FUNCTION invocation119_operations_frame(text[]) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_binding_hash_matches(binding agent_capability_bindings) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE requirement release_capability_requirements%ROWTYPE; requirement_hash bytea; binding_hash bytea;
BEGIN
 SELECT * INTO requirement FROM release_capability_requirements WHERE id=binding.requirement_id;
 IF requirement.id IS NULL OR requirement.release_agent_id IS DISTINCT FROM binding.release_agent_id
  OR requirement.slot_key IS DISTINCT FROM binding.slot_key OR requirement.resource_kind IS DISTINCT FROM binding.resource_kind
  OR NOT requirement.required_operations<@binding.granted_operations
  OR NOT binding.granted_operations<@(requirement.required_operations||requirement.optional_operations) THEN RETURN false; END IF;
 requirement_hash:=sha256(invocation119_hash_frame(convert_to('hephaestus.capability-requirement.v1','UTF8'))
  ||invocation119_hash_frame(uuid_send(requirement.id))||invocation119_hash_frame(convert_to(requirement.slot_key,'UTF8'))
  ||invocation119_hash_frame(convert_to(requirement.resource_kind,'UTF8'))
  ||invocation119_operations_frame(requirement.required_operations)||invocation119_operations_frame(requirement.optional_operations)
  ||invocation119_hash_frame(CASE WHEN requirement.slot_required THEN decode('01','hex') ELSE decode('00','hex') END));
 binding_hash:=sha256(invocation119_hash_frame(convert_to('hephaestus.capability-binding.v1','UTF8'))
  ||invocation119_hash_frame(uuid_send(binding.id))||invocation119_hash_frame(uuid_send(requirement.id))
  ||invocation119_hash_frame(requirement_hash)||invocation119_hash_frame(convert_to(binding.slot_key,'UTF8'))
  ||invocation119_hash_frame(convert_to(binding.resource_kind,'UTF8'))||invocation119_hash_frame(uuid_send(binding.resource_id))
  ||invocation119_operations_frame(binding.granted_operations));
 RETURN requirement.normalized_hash IS NOT DISTINCT FROM requirement_hash
  AND binding.requirement_hash IS NOT DISTINCT FROM requirement_hash
  AND binding.normalized_hash IS NOT DISTINCT FROM binding_hash;
END $$;
REVOKE ALL ON FUNCTION invocation119_binding_hash_matches(agent_capability_bindings) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_capabilities_live(p_run uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; binding agent_capability_bindings%ROWTYPE; operation text;
BEGIN
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.id IS NULL OR invocation119_no_git(p_run) IS DISTINCT FROM true
  OR EXISTS(SELECT 1 FROM release_capability_requirements requirement WHERE requirement.release_agent_id=execution.release_agent_id
   AND requirement.slot_required AND NOT EXISTS(SELECT 1 FROM agent_capability_bindings candidate
    WHERE candidate.instance_revision_id=execution.instance_revision_id AND candidate.requirement_id=requirement.id)) THEN RETURN false; END IF;
 FOR binding IN SELECT * FROM agent_capability_bindings WHERE instance_revision_id=execution.instance_revision_id LOOP
  IF invocation119_binding_hash_matches(binding) IS DISTINCT FROM true
   OR NOT EXISTS(SELECT 1 FROM users WHERE id=binding.created_by AND status='active')
   OR can_grant_agent_capability_operations(binding.created_by,binding.resource_kind,binding.resource_id,binding.granted_operations) IS DISTINCT FROM true THEN RETURN false; END IF;
  FOREACH operation IN ARRAY binding.granted_operations LOOP
   IF check_permission('agent_instance',execution.instance_id::text,'agent_'||operation,binding.resource_kind,binding.resource_id::text) IS DISTINCT FROM 1 THEN RETURN false; END IF;
  END LOOP;
 END LOOP;
 RETURN true;
END $$;
REVOKE ALL ON FUNCTION invocation119_capabilities_live(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- Entire original immutable ceiling, not a subset of currently surviving rows.
CREATE FUNCTION invocation119_session_graph(p_run uuid,p_live boolean) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE session runtime_authority_sessions%ROWTYPE; snapshot run_authorization_snapshots%ROWTYPE; execution runs%ROWTYPE;
BEGIN
 SELECT * INTO execution FROM runs WHERE id=p_run;
 SELECT * INTO session FROM runtime_authority_sessions WHERE run_id=p_run;
 SELECT * INTO snapshot FROM run_authorization_snapshots WHERE run_id=p_run;
 IF execution.id IS NULL OR session.id IS NULL OR snapshot.id IS NULL THEN RETURN false; END IF;
 RETURN session.id=p_run AND snapshot.id=p_run AND session.snapshot_id=p_run
  AND ROW(session.instance_id,session.instance_revision_id,session.attachment_id)
   IS NOT DISTINCT FROM ROW(execution.instance_id,execution.instance_revision_id,execution.attachment_id)
  AND ROW(snapshot.instance_id,snapshot.instance_revision_id)
   IS NOT DISTINCT FROM ROW(execution.instance_id,execution.instance_revision_id)
  AND session.attachment_id IS NULL AND session.issuance_generation=1
  AND snapshot.normalized_hash IS NOT DISTINCT FROM invocation119_snapshot_hash(p_run,snapshot.authorization_model_version)
  AND NOT EXISTS(SELECT 1 FROM agent_capability_bindings binding WHERE binding.instance_revision_id=execution.instance_revision_id
   AND binding.authorization_model_version IS DISTINCT FROM snapshot.authorization_model_version)
  AND session.snapshot_hash IS NOT DISTINCT FROM snapshot.normalized_hash
  AND session.identity_hash IS NOT DISTINCT FROM invocation119_identity_hash(p_run,snapshot.normalized_hash,
      extract(epoch FROM session.issued_at)*1000000000,extract(epoch FROM session.expires_at)*1000000000)
  AND session.expires_at>session.issued_at AND session.expires_at<=session.issued_at+interval '3600 seconds'
  AND (SELECT count(*) FROM run_authorization_snapshot_bindings WHERE snapshot_id=p_run)
   =(SELECT count(*) FROM agent_capability_bindings WHERE instance_revision_id=execution.instance_revision_id)
  AND NOT EXISTS(SELECT 1 FROM (
    SELECT binding.*,row_number() OVER(ORDER BY slot_key COLLATE "C")-1 ordinal FROM agent_capability_bindings binding
    WHERE instance_revision_id=execution.instance_revision_id) original
   LEFT JOIN run_authorization_snapshot_bindings copied ON copied.snapshot_id=p_run AND copied.binding_id=original.id
   WHERE copied.binding_id IS NULL OR ROW(copied.instance_revision_id,copied.ordinal,copied.binding_hash,copied.slot_key,copied.resource_kind,copied.resource_id,copied.granted_operations)
    IS DISTINCT FROM ROW(original.instance_revision_id,original.ordinal::integer,original.normalized_hash,original.slot_key,original.resource_kind,original.resource_id,original.granted_operations))
  AND invocation119_no_git(p_run) IS NOT DISTINCT FROM true
  AND (NOT p_live OR (session.status IN ('pending_handoff','active') AND session.revoked_at IS NULL
      AND session.issued_at<=clock_timestamp() AND session.expires_at>clock_timestamp()
      AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease WHERE lease.run_id=p_run AND session.expires_at>lease.expires_at)));
END $$;
REVOKE ALL ON FUNCTION invocation119_session_graph(uuid,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION invocation119_acknowledged_set(p_run uuid) RETURNS boolean
LANGUAGE sql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT invocation119_session_graph(p_run,true) IS NOT DISTINCT FROM true
  AND EXISTS(SELECT 1 FROM runtime_authority_sessions WHERE run_id=p_run AND status='active' AND acknowledged_at IS NOT NULL)
  AND invocation119_frozen_set(p_run) IS NOT DISTINCT FROM true
  AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease JOIN agent_instance_state_volumes volume ON volume.id=lease.volume_id
    WHERE lease.run_id=p_run AND (lease.attached_at IS NULL OR volume.state IS DISTINCT FROM 'attached'))
$$;
REVOKE ALL ON FUNCTION invocation119_acknowledged_set(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- The late phases require an already acknowledged exact session and attached full set.
CREATE OR REPLACE FUNCTION invocation119_live(p_run uuid) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE receipt agent_instance_execution_invocations%ROWTYPE; execution runs%ROWTYPE;
BEGIN
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 SELECT * INTO execution FROM runs WHERE id=p_run;
 RETURN invocation119_immutable(p_run) IS NOT DISTINCT FROM true
  AND execution.state IN ('queued','leasing_volume','provisioning','starting','running')
  AND (execution.state NOT IN ('starting','running') OR invocation119_acknowledged_set(p_run) IS NOT DISTINCT FROM true)
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

CREATE FUNCTION invocation119_authority_eligible(p_run uuid,p_config bytea,p_monitor boolean) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE;
BEGIN
 SELECT * INTO execution FROM runs WHERE id=p_run;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 RETURN receipt.run_id=p_run AND receipt.config_bytes IS NOT DISTINCT FROM p_config
  AND invocation119_live(p_run) IS NOT DISTINCT FROM true
  AND invocation119_frozen_set(p_run) IS NOT DISTINCT FROM true
  AND invocation119_capabilities_live(p_run) IS NOT DISTINCT FROM true
  AND ((NOT p_monitor AND execution.state IN ('leasing_volume','provisioning'))
    OR (p_monitor AND execution.state IN ('provisioning','starting','running')
      AND invocation119_session_graph(p_run,true) IS NOT DISTINCT FROM true));
END $$;
REVOKE ALL ON FUNCTION invocation119_authority_eligible(uuid,bytea,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION lock_invocation119_authority(p_run uuid,p_config bytea) RETURNS runs
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; candidate record;
BEGIN
 execution:=lock_qualified_invocation119(p_run,p_config);
 -- Additional ceiling/issuer rows follow the existing late volume/grant order.
 -- Raw paths never block after acquiring a child. Trusted adapters reserve
 -- the early tagged113 keys in their own fresh transaction before all rows.
 FOR candidate IN SELECT actor_id id FROM agent_instance_execution_invocations WHERE run_id=p_run
   UNION SELECT created_by FROM agent_capability_bindings WHERE instance_revision_id=execution.instance_revision_id
   UNION SELECT created_by FROM agent_instance_volume_mount_grants WHERE instance_revision_id=execution.instance_revision_id
   ORDER BY id LOOP
  PERFORM 1 FROM users WHERE id=candidate.id FOR UPDATE NOWAIT;
 END LOOP;
 PERFORM 1 FROM agent_capability_bindings WHERE instance_revision_id=execution.instance_revision_id ORDER BY id FOR UPDATE NOWAIT;
 PERFORM 1 FROM agent_git_capability_bindings WHERE instance_revision_id=execution.instance_revision_id ORDER BY binding_id FOR UPDATE NOWAIT;
 PERFORM 1 FROM run_authorization_snapshots WHERE run_id=p_run FOR UPDATE NOWAIT;
 PERFORM 1 FROM runtime_authority_sessions WHERE run_id=p_run FOR UPDATE NOWAIT;
 RETURN execution;
END $$;
REVOKE ALL ON FUNCTION lock_invocation119_authority(uuid,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION read_invocation119_authority_context(p_run uuid,p_config bytea,p_monitor boolean)
RETURNS TABLE(actor_id uuid,request_id uuid,instance_id uuid,revision_id uuid,release_id uuid,release_agent_id uuid,command_id uuid)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_invocation117_worker();
 IF invocation119_authority_eligible(p_run,p_config,p_monitor) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'Invocation source/phase/config/ceiling is no longer eligible' USING ERRCODE='P1192'; END IF;
 RETURN QUERY SELECT receipt.actor_id,receipt.request_id,receipt.instance_id,receipt.revision_id,
  receipt.release_id,receipt.release_agent_id,receipt.start_command_id
  FROM agent_instance_execution_invocations receipt WHERE receipt.run_id=p_run;
END $$;
REVOKE ALL ON FUNCTION read_invocation119_authority_context(uuid,bytea,boolean) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION read_invocation119_authority_context(uuid,bytea,boolean) TO hephaestus_worker;

CREATE FUNCTION invocation119_valid_interval(p_run uuid,p_issued timestamptz,p_expires timestamptz) RETURNS boolean
LANGUAGE sql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT p_issued IS NOT NULL AND p_expires IS NOT NULL AND isfinite(p_issued) AND isfinite(p_expires)
  AND p_expires>p_issued AND p_expires<=p_issued+interval '3600 seconds'
  AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease WHERE lease.run_id=p_run AND p_expires>lease.expires_at)
$$;
REVOKE ALL ON FUNCTION invocation119_valid_interval(uuid,timestamptz,timestamptz) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION write_invocation119_session(p_run uuid,p_config bytea,p_model text,p_snapshot bytea,p_identity bytea,
 p_generation bigint,p_credential bytea,p_issued timestamptz,p_expires timestamptz,p_issue_ns text,p_expire_ns text)
RETURNS SETOF runtime_authority_sessions
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; binding record; snapshot_hash bytea; issue_ns numeric; expire_ns numeric;
BEGIN
 PERFORM require_invocation117_worker();
 execution:=lock_invocation119_authority(p_run,p_config);
 IF p_issue_ns IS NULL OR p_expire_ns IS NULL OR p_issue_ns !~ '^-?[0-9]{1,21}$' OR p_expire_ns !~ '^-?[0-9]{1,21}$' THEN
  RAISE EXCEPTION 'exact bounded original119 nanoseconds required' USING ERRCODE='22003'; END IF;
 issue_ns:=p_issue_ns::numeric;expire_ns:=p_expire_ns::numeric;
 IF issue_ns NOT BETWEEN -377705116800000000000 AND 253402300799999999000
  OR expire_ns NOT BETWEEN -377705116800000000000 AND 253402300799999999000
  OR mod(issue_ns,1000)<>0 OR mod(expire_ns,1000)<>0
  OR issue_ns IS DISTINCT FROM extract(epoch FROM p_issued)*1000000000
  OR expire_ns IS DISTINCT FROM extract(epoch FROM p_expires)*1000000000
  OR invocation119_valid_interval(p_run,p_issued,p_expires) IS DISTINCT FROM true
  OR p_issued>clock_timestamp() OR p_expires<=clock_timestamp() THEN
  RAISE EXCEPTION 'qualified119 issue/expiry must already be PG-precision and bounded' USING ERRCODE='22003'; END IF;
 IF execution.state IS DISTINCT FROM 'provisioning' OR invocation119_authority_eligible(p_run,p_config,false) IS DISTINCT FROM true
  OR p_generation IS DISTINCT FROM 1 OR octet_length(p_credential) IS DISTINCT FROM 32
  OR p_model IS NULL OR length(p_model) NOT BETWEEN 1 AND 128
  OR EXISTS(SELECT 1 FROM agent_capability_bindings WHERE instance_revision_id=execution.instance_revision_id AND authorization_model_version IS DISTINCT FROM p_model) THEN
  RAISE EXCEPTION 'qualified119 new issuance requires Provisioning current full ceiling' USING ERRCODE='P1192'; END IF;
 snapshot_hash:=invocation119_snapshot_hash(p_run,p_model);
 IF p_snapshot IS DISTINCT FROM snapshot_hash
  OR p_identity IS DISTINCT FROM invocation119_identity_hash(p_run,snapshot_hash,issue_ns,expire_ns) THEN
  RAISE EXCEPTION 'qualified119 original Core hash does not match exact graph/times' USING ERRCODE='23000'; END IF;
 IF EXISTS(SELECT 1 FROM runtime_authority_sessions WHERE id=p_run OR run_id=p_run)
   OR EXISTS(SELECT 1 FROM run_authorization_snapshots WHERE id=p_run OR run_id=p_run) THEN
  RAISE EXCEPTION 'qualified119 issuance already has immutable session history' USING ERRCODE='23000'; END IF;
 INSERT INTO run_authorization_snapshots(id,run_id,instance_id,instance_revision_id,authorization_model_version,normalized_hash)
  VALUES(p_run,p_run,execution.instance_id,execution.instance_revision_id,p_model,snapshot_hash);
 FOR binding IN SELECT original.*,row_number() OVER(ORDER BY slot_key COLLATE "C")-1 ordinal
  FROM agent_capability_bindings original WHERE instance_revision_id=execution.instance_revision_id ORDER BY slot_key COLLATE "C" LOOP
  INSERT INTO run_authorization_snapshot_bindings(snapshot_id,instance_revision_id,ordinal,binding_id,binding_hash,slot_key,resource_kind,resource_id,granted_operations)
   VALUES(p_run,execution.instance_revision_id,binding.ordinal::integer,binding.id,binding.normalized_hash,binding.slot_key,binding.resource_kind,binding.resource_id,binding.granted_operations);
 END LOOP;
 INSERT INTO runtime_authority_sessions(id,snapshot_id,run_id,instance_id,instance_revision_id,attachment_id,
   identity_hash,snapshot_hash,issuance_generation,credential_hash,status,issued_at,expires_at)
  VALUES(p_run,p_run,p_run,execution.instance_id,execution.instance_revision_id,NULL,p_identity,snapshot_hash,1,p_credential,'pending_handoff',p_issued,p_expires);
 IF invocation119_session_graph(p_run,true) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'qualified119 writer did not produce complete durable ceiling' USING ERRCODE='23000'; END IF;
 RETURN QUERY SELECT * FROM runtime_authority_sessions WHERE run_id=p_run;
END $$;
REVOKE ALL ON FUNCTION write_invocation119_session(uuid,bytea,text,bytea,bytea,bigint,bytea,timestamptz,timestamptz,text,text) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION write_invocation119_session(uuid,bytea,text,bytea,bytea,bigint,bytea,timestamptz,timestamptz,text,text) TO hephaestus_worker;

CREATE FUNCTION find_invocation119_session(p_session uuid,p_config bytea) RETURNS SETOF runtime_authority_sessions
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; session runtime_authority_sessions%ROWTYPE;
BEGIN
 PERFORM require_invocation117_worker();
 SELECT * INTO execution FROM runs WHERE id=p_session;
 IF invocation119_immutable(p_session) IS DISTINCT FROM true OR execution.id IS NULL
  OR p_config IS DISTINCT FROM (SELECT config_bytes FROM run_invocation_consumptions WHERE run_id=p_session) THEN
  RAISE EXCEPTION 'exact qualified119 session identity/config required' USING ERRCODE='P1192'; END IF;
 SELECT * INTO session FROM runtime_authority_sessions WHERE id=p_session AND run_id=p_session;
 IF session.id IS NULL THEN
  IF EXISTS(SELECT 1 FROM runtime_authority_sessions WHERE id=p_session OR run_id=p_session)
    OR EXISTS(SELECT 1 FROM run_authorization_snapshots WHERE id=p_session OR run_id=p_session)
    OR invocation119_authority_eligible(p_session,p_config,false) IS DISTINCT FROM true
    OR execution.state IS DISTINCT FROM 'provisioning' THEN
   RAISE EXCEPTION 'withdrawn or orphaned119 session is not absent/new issuance' USING ERRCODE='P1192'; END IF;
  RETURN;
 END IF;
 IF invocation119_session_graph(p_session,false) IS DISTINCT FROM true THEN
  RAISE EXCEPTION '119 session history is inconsistent' USING ERRCODE='23000'; END IF;
 IF session.status='pending_handoff' AND (execution.state IS DISTINCT FROM 'provisioning'
    OR invocation119_authority_eligible(p_session,p_config,true) IS DISTINCT FROM true) THEN
  RAISE EXCEPTION 'pending119 handoff no longer authorized; cannot reissue' USING ERRCODE='P1192'; END IF;
 -- Active/revoked/expired data remains visible to lifecycle cleanup. Issuers
 -- do not issue from these states; live execution uses the separately checked ceiling.
 RETURN NEXT session;
END $$;
REVOKE ALL ON FUNCTION find_invocation119_session(uuid,bytea) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION find_invocation119_session(uuid,bytea) TO hephaestus_worker;

CREATE FUNCTION acknowledge_invocation119_session(p_session uuid,p_config bytea,p_generation bigint,p_at timestamptz)
RETURNS SETOF runtime_authority_sessions
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; session runtime_authority_sessions%ROWTYPE;
BEGIN
 PERFORM require_invocation117_worker();
 execution:=lock_invocation119_authority(p_session,p_config);
 SELECT * INTO session FROM runtime_authority_sessions WHERE id=p_session AND run_id=p_session FOR UPDATE NOWAIT;
 IF execution.state IS DISTINCT FROM 'provisioning' OR invocation119_authority_eligible(p_session,p_config,true) IS DISTINCT FROM true
  OR p_generation IS DISTINCT FROM session.issuance_generation
  OR session.status NOT IN ('pending_handoff','active') OR p_at IS NULL OR p_at<session.issued_at OR p_at>=session.expires_at THEN
  RAISE EXCEPTION '119 acknowledgement requires current Provisioning exact generation/ceiling' USING ERRCODE='P1192'; END IF;
 IF session.status='pending_handoff' THEN
  UPDATE runtime_authority_sessions AS target SET status='active',acknowledged_at=p_at,updated_at=p_at WHERE target.id=p_session;
 END IF;
 RETURN QUERY SELECT * FROM runtime_authority_sessions WHERE id=p_session;
END $$;
REVOKE ALL ON FUNCTION acknowledge_invocation119_session(uuid,bytea,bigint,timestamptz) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION acknowledge_invocation119_session(uuid,bytea,bigint,timestamptz) TO hephaestus_worker;

-- Current rights checks happen before returning a ceiling or copying any row.
-- Revocation/expiry metadata and old cleanup remain independent of live rights.
CREATE FUNCTION guard_invocation119_authority_birth() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; config bytea; p_run uuid;
BEGIN
 IF TG_TABLE_NAME='run_authorization_snapshot_bindings' THEN
  SELECT run_id INTO p_run FROM run_authorization_snapshots WHERE id=NEW.snapshot_id;
 ELSE p_run:=NEW.run_id; END IF;
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.run_kind IS DISTINCT FROM 'invocation' THEN RETURN NEW; END IF;
 PERFORM require_invocation117_worker();
 SELECT config_bytes INTO config FROM run_invocation_consumptions WHERE run_id=p_run;
 execution:=lock_invocation119_authority(p_run,config);
 IF execution.state IS DISTINCT FROM 'provisioning' OR invocation119_authority_eligible(p_run,config,false) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'raw119 birth requires current Provisioning/fullheld/config/delegation' USING ERRCODE='P1192'; END IF;
 IF TG_TABLE_NAME='run_authorization_snapshots' THEN
  IF NEW.id IS DISTINCT FROM p_run OR NEW.instance_id IS DISTINCT FROM execution.instance_id
   OR NEW.instance_revision_id IS DISTINCT FROM execution.instance_revision_id
   OR NEW.normalized_hash IS DISTINCT FROM invocation119_snapshot_hash(p_run,NEW.authorization_model_version)
   OR EXISTS(SELECT 1 FROM agent_capability_bindings WHERE instance_revision_id=execution.instance_revision_id AND authorization_model_version IS DISTINCT FROM NEW.authorization_model_version) THEN
   RAISE EXCEPTION 'raw119 snapshot differs from complete immutable Core ceiling' USING ERRCODE='23000'; END IF;
 ELSIF TG_TABLE_NAME='runtime_authority_sessions' THEN
  IF NEW.id IS DISTINCT FROM p_run OR NEW.snapshot_id IS DISTINCT FROM p_run OR NEW.attachment_id IS NOT NULL
    OR NEW.instance_id IS DISTINCT FROM execution.instance_id OR NEW.instance_revision_id IS DISTINCT FROM execution.instance_revision_id
    OR NEW.status IS DISTINCT FROM 'pending_handoff' OR NEW.issuance_generation IS DISTINCT FROM 1
    OR invocation119_valid_interval(p_run,NEW.issued_at,NEW.expires_at) IS DISTINCT FROM true
    OR extract(epoch FROM NEW.issued_at)*1000000000 < -377705116800000000000
    OR extract(epoch FROM NEW.expires_at)*1000000000 > 253402300799999999000
    OR NEW.issued_at>clock_timestamp() OR NEW.expires_at<=clock_timestamp()
    OR NEW.identity_hash IS DISTINCT FROM invocation119_identity_hash(p_run,NEW.snapshot_hash,
       extract(epoch FROM NEW.issued_at)*1000000000,extract(epoch FROM NEW.expires_at)*1000000000) THEN
   RAISE EXCEPTION 'raw119 identity/status/times differ from exact Core claims' USING ERRCODE='23000'; END IF;
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_invocation119_authority_birth() FROM PUBLIC,hephaestus_app,hephaestus_worker;
CREATE TRIGGER invocation119_snapshot_birth BEFORE INSERT ON run_authorization_snapshots
 FOR EACH ROW EXECUTE FUNCTION guard_invocation119_authority_birth();
CREATE TRIGGER invocation119_binding_birth BEFORE INSERT ON run_authorization_snapshot_bindings
 FOR EACH ROW EXECUTE FUNCTION guard_invocation119_authority_birth();
CREATE TRIGGER invocation119_session_birth BEFORE INSERT ON runtime_authority_sessions
 FOR EACH ROW EXECUTE FUNCTION guard_invocation119_authority_birth();

CREATE FUNCTION validate_invocation119_authority_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE p_run uuid; execution runs%ROWTYPE; config bytea;
BEGIN
 IF TG_TABLE_NAME='run_authorization_snapshot_bindings' THEN
  SELECT run_id INTO p_run FROM run_authorization_snapshots WHERE id=NEW.snapshot_id;
 ELSE p_run:=NEW.run_id; END IF;
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.run_kind IS DISTINCT FROM 'invocation' THEN RETURN NULL; END IF;
 PERFORM require_invocation117_worker();
 SELECT config_bytes INTO config FROM run_invocation_consumptions WHERE run_id=p_run;
 IF execution.state IS DISTINCT FROM 'provisioning'
  OR invocation119_session_graph(p_run,true) IS DISTINCT FROM true
  OR invocation119_authority_eligible(p_run,config,false) IS DISTINCT FROM true THEN
  RAISE EXCEPTION '119 authority birth must commit complete exact current ceiling/session' USING ERRCODE='23000'; END IF;
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_invocation119_authority_commit() FROM PUBLIC,hephaestus_app,hephaestus_worker;
CREATE CONSTRAINT TRIGGER invocation119_snapshot_complete AFTER INSERT ON run_authorization_snapshots
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_invocation119_authority_commit();
CREATE CONSTRAINT TRIGGER invocation119_binding_complete AFTER INSERT ON run_authorization_snapshot_bindings
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_invocation119_authority_commit();
CREATE CONSTRAINT TRIGGER invocation119_session_complete AFTER INSERT ON runtime_authority_sessions
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION validate_invocation119_authority_commit();

CREATE FUNCTION guard_invocation119_session_update() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; config bytea;
BEGIN
 SELECT * INTO execution FROM runs WHERE id=OLD.run_id;
 IF execution.run_kind IS DISTINCT FROM 'invocation' THEN RETURN NEW; END IF;
 PERFORM require_invocation117_worker();
 IF OLD.status='pending_handoff' AND NEW.status='active' THEN
  SELECT config_bytes INTO config FROM run_invocation_consumptions WHERE run_id=OLD.run_id;
  execution:=lock_invocation119_authority(OLD.run_id,config);
  IF execution.state IS DISTINCT FROM 'provisioning'
    OR invocation119_authority_eligible(OLD.run_id,config,true) IS DISTINCT FROM true THEN
   RAISE EXCEPTION '119 acknowledgement no longer has current issuance authority' USING ERRCODE='P1192'; END IF;
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_invocation119_session_update() FROM PUBLIC,hephaestus_app,hephaestus_worker;
CREATE TRIGGER invocation119_session_live_ack BEFORE UPDATE ON runtime_authority_sessions
 FOR EACH ROW EXECUTE FUNCTION guard_invocation119_session_update();

CREATE FUNCTION deny_invocation119_git_copy() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE copied_snapshot uuid;
BEGIN
 IF TG_TABLE_NAME='run_git_authority_snapshots' THEN copied_snapshot:=NEW.snapshot_id;
 ELSE
  copied_snapshot:=NEW.authorization_snapshot_id;
  IF EXISTS(SELECT 1 FROM runtime_authority_sessions session JOIN runs execution ON execution.id=session.run_id
    WHERE session.id=NEW.runtime_session_id AND execution.run_kind='invocation') THEN
   RAISE EXCEPTION 'qualified Invocation has no Git credential under any substituted snapshot' USING ERRCODE='0A000'; END IF;
 END IF;
 IF EXISTS(SELECT 1 FROM run_authorization_snapshots snapshot JOIN runs execution ON execution.id=snapshot.run_id
  WHERE snapshot.id=copied_snapshot AND execution.run_kind='invocation') THEN
  RAISE EXCEPTION 'qualified Invocation has no Git session binding or credential' USING ERRCODE='0A000'; END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION deny_invocation119_git_copy() FROM PUBLIC,hephaestus_app,hephaestus_worker;
CREATE TRIGGER invocation119_no_git_snapshot BEFORE INSERT ON run_git_authority_snapshots
 FOR EACH ROW EXECUTE FUNCTION deny_invocation119_git_copy();
CREATE TRIGGER invocation119_no_git_credential BEFORE INSERT ON runtime_git_credentials
 FOR EACH ROW EXECUTE FUNCTION deny_invocation119_git_copy();

CREATE FUNCTION invocation119_typed_mount_authorized(p_run uuid,p_config bytea,p_instance uuid,p_revision uuid,p_agent uuid,
 p_slot text,p_volume uuid,p_mode text,p_contract bytea) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
 PERFORM require_invocation117_worker();
 SELECT * INTO execution FROM runs WHERE id=p_run;
 RETURN invocation119_authority_eligible(p_run,p_config,execution.state IN ('starting','running')) IS NOT DISTINCT FROM true
  AND execution.state IN ('leasing_volume','provisioning','starting','running')
  AND ROW(execution.instance_id,execution.instance_revision_id,execution.release_agent_id)
   IS NOT DISTINCT FROM ROW(p_instance,p_revision,p_agent)
  AND EXISTS(SELECT 1 FROM agent_instance_revision_volume_bindings binding JOIN agent_instance_volume_mount_grants grant_row
   ON grant_row.instance_revision_id=binding.instance_revision_id AND grant_row.slot_key=binding.slot_key
   WHERE binding.instance_revision_id=p_revision AND binding.slot_key=p_slot AND binding.volume_id=p_volume
    AND binding.access_mode=p_mode AND grant_row.volume_id=p_volume AND grant_row.access_mode=p_mode
    AND grant_row.release_contract_hash=p_contract
    AND EXISTS(SELECT 1 FROM agent_instance_volume_leases lease WHERE lease.run_id=p_run AND lease.volume_id=p_volume
      AND lease.selected_revision_id=p_revision AND lease.slot_key=p_slot AND lease.access_mode=p_mode));
END $$;
REVOKE ALL ON FUNCTION invocation119_typed_mount_authorized(uuid,bytea,uuid,uuid,uuid,text,uuid,text,bytea) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION invocation119_typed_mount_authorized(uuid,bytea,uuid,uuid,uuid,text,uuid,text,bytea) TO hephaestus_worker;

CREATE OR REPLACE FUNCTION assert_invocation119_transition(p_run uuid,p_next text) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
 PERFORM require_invocation117_worker();
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF invocation119_immutable(p_run) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'Invocation transition needs immutable consumed producer' USING ERRCODE='23000';
 END IF;
 IF p_next IN ('starting','running') THEN
  IF (p_next='starting' AND execution.state<>'provisioning')
   OR (p_next='running' AND execution.state<>'starting')
   OR invocation119_live(p_run) IS DISTINCT FROM true
   OR invocation119_acknowledged_set(p_run) IS DISTINCT FROM true
   OR invocation119_capabilities_live(p_run) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'Invocation progress needs current acknowledged exact attached ceiling' USING ERRCODE='23000'; END IF;
 ELSIF p_next='succeeded' THEN
  RAISE EXCEPTION 'Invocation actual exit/result integration is not provided by119-B1' USING ERRCODE='0A000';
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

CREATE OR REPLACE FUNCTION validate_invocation119_commit() RETURNS trigger
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
 IF (execution.state IN ('starting','running') AND (invocation119_acknowledged_set(p_run) IS DISTINCT FROM true
      OR invocation119_capabilities_live(p_run) IS DISTINCT FROM true))
   OR invocation119_live(p_run) IS DISTINCT FROM true
   OR (execution.volume_selection_snapshot IS NOT NULL AND invocation119_frozen_set(p_run) IS DISTINCT FROM true)
   OR (execution.state='provisioning' AND invocation119_frozen_set(p_run) IS DISTINCT FROM true) THEN
  RAISE EXCEPTION 'Invocation transaction lacks complete current119 acquisition/progress evidence' USING ERRCODE='23000';
 END IF;
 RETURN NULL;
END $$;

-- Existing application-role broker lookup keeps its wire and ACL. Invocation
-- must additionally match the acknowledged live agent_instance revision ceiling.
CREATE OR REPLACE FUNCTION authenticate_runtime_authority(p_credential_hash bytea)
RETURNS TABLE (
    session_id uuid,
    snapshot_id uuid,
    run_id uuid,
    instance_id uuid,
    instance_revision_id uuid,
    attachment_id uuid,
    issuance_generation bigint,
    expires_at timestamptz
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, public
STABLE
AS $$
    SELECT session.id, session.snapshot_id, session.run_id,
           session.instance_id, session.instance_revision_id,
           session.attachment_id, session.issuance_generation,
           session.expires_at
    FROM runtime_authority_sessions AS session
    WHERE session.credential_hash = p_credential_hash
      AND session.status = 'active'
      AND session.expires_at > now()
      AND (NOT EXISTS(SELECT 1 FROM runs execution WHERE execution.id=session.run_id AND execution.run_kind='invocation')
        OR invocation119_authority_eligible(session.run_id,
            (SELECT config_bytes FROM run_invocation_consumptions WHERE run_id=session.run_id),true) IS NOT DISTINCT FROM true)
$$;

-- B1 successor: preserve the frozen119-A invoker trigger/ACL and entire
-- ordinary body. Only a real worker calling a positive immutable Invocation
-- through a protected definer can use the existing refresh path.
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
            IF current_user::text='hephaestus_worker' THEN
                PERFORM require_run_cleanup_worker();
            ELSE
                -- This branch is inside TG_OP UPDATE: OLD is assigned. The
                -- actual caller check precedes any Run lookup under a definer.
                PERFORM require_invocation117_worker();
                IF invocation119_immutable(OLD.run_id) IS DISTINCT FROM true THEN
                    RAISE EXCEPTION 'definer lease mutation requires exact Invocation worker lineage' USING ERRCODE='42501';
                END IF;
            END IF;
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


-- B1 successor for the UPDATE-only scalar guard: preserve its invoker
-- security, existing ACL/trigger identity and every Legacy fence/recovery check.
-- Only the actual worker with positive immutable Invocation lineage may pass
-- the original lifecycle guard while the protected refresh function is definer.
CREATE OR REPLACE FUNCTION guard_legacy_scalar_lease_progress() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
    -- Check the real invoker before any Run lookup: RLS-hidden classification
    -- must never turn an actor lifecycle mutation into an unchecked branch.
    IF ROW(NEW.state,NEW.acquired_at,NEW.heartbeat_at,NEW.expires_at,NEW.attached_at,NEW.recovering_at,NEW.released_at)
        IS DISTINCT FROM ROW(OLD.state,OLD.acquired_at,OLD.heartbeat_at,OLD.expires_at,OLD.attached_at,OLD.recovering_at,OLD.released_at) THEN
        IF current_user::text='hephaestus_worker' THEN
            PERFORM require_run_cleanup_worker();
        ELSE
            -- This trigger is UPDATE-only: OLD is assigned. Check the actual
            -- caller before reading any Run under a protected definer.
            PERFORM require_invocation117_worker();
            IF invocation119_immutable(OLD.run_id) IS DISTINCT FROM true THEN
                RAISE EXCEPTION 'definer scalar guard mutation requires exact Invocation worker lineage' USING ERRCODE='42501';
            END IF;
        END IF;
    END IF;
    SELECT * INTO execution FROM runs WHERE id=OLD.run_id;
    IF execution.execution_profile IS DISTINCT FROM 'legacy_scalar' THEN RETURN NEW; END IF;
    IF OLD.released_at IS NOT NULL THEN
        IF ROW(NEW.acquired_at,NEW.heartbeat_at,NEW.expires_at,NEW.attached_at,NEW.recovering_at)
            IS DISTINCT FROM ROW(OLD.acquired_at,OLD.heartbeat_at,OLD.expires_at,OLD.attached_at,OLD.recovering_at) THEN
            RAISE EXCEPTION 'released scalar lifecycle evidence is immutable' USING ERRCODE='23000';
        END IF;
        RETURN NEW;
    END IF;
    PERFORM lock_legacy_scalar_context(OLD.run_id,OLD.volume_id,OLD.host_id,true);
    IF legacy_scalar_lease_matches(OLD) IS NOT TRUE THEN
        RAISE EXCEPTION 'scalar progress cannot adopt non-origin history' USING ERRCODE='23000';
    END IF;
    PERFORM assert_legacy_scalar_current_fence(OLD);
    IF NEW.acquired_at IS DISTINCT FROM OLD.acquired_at THEN
        RAISE EXCEPTION 'scalar birth time is immutable' USING ERRCODE='23000';
    END IF;
    IF NEW.state IS DISTINCT FROM OLD.state AND NEW.state NOT IN ('released','recovery_required') THEN
        RAISE EXCEPTION 'scalar recovery cannot reopen an active lease' USING ERRCODE='23000';
    END IF;
    IF NEW.heartbeat_at IS DISTINCT FROM OLD.heartbeat_at
       OR NEW.expires_at IS DISTINCT FROM OLD.expires_at
       OR NEW.attached_at IS DISTINCT FROM OLD.attached_at THEN
        IF execution.volume_acquisition_closed_at IS NOT NULL OR execution.cancel_requested_at IS NOT NULL
           OR execution.state IN ('cleaning_up','cleaned_up') OR OLD.state<>'active'
           OR OLD.recovering_at IS NOT NULL OR OLD.expires_at<=clock_timestamp()
           OR NEW.heartbeat_at<OLD.heartbeat_at OR NEW.heartbeat_at>=OLD.expires_at
           OR NEW.expires_at<=NEW.heartbeat_at OR NEW.expires_at<=clock_timestamp()
           OR (OLD.attached_at IS NOT NULL AND NEW.attached_at IS DISTINCT FROM OLD.attached_at)
           OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes
                WHERE id=OLD.volume_id AND state IN ('ready','attached')) THEN
            RAISE EXCEPTION 'expired or closed scalar progress is held' USING ERRCODE='23000';
        END IF;
    END IF;
    IF NEW.recovering_at IS DISTINCT FROM OLD.recovering_at OR NEW.state='recovery_required' THEN
        IF execution.volume_acquisition_closed_at IS NULL OR OLD.expires_at>clock_timestamp()
           OR NEW.state<>'recovery_required' OR NEW.recovering_at IS NULL
           OR (OLD.recovering_at IS NOT NULL AND NEW.recovering_at IS DISTINCT FROM OLD.recovering_at) THEN
            RAISE EXCEPTION 'scalar recovery needs permanent closure and exact expired fence' USING ERRCODE='23000';
        END IF;
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_legacy_scalar_lease_progress() FROM PUBLIC;
