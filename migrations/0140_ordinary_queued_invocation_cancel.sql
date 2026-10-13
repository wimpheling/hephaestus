-- Ordinary user cancellation of genuine115 Invocation before117 consumption.
-- User Cancel does not remove an instance or revoke grants. A later genuine
-- recipe Drain is independently qualified; no Start, target, VM observation or
-- origin is invented. Historic actor/request fields remain comparison data.

-- Each queued Drain consumer proves its own no-IO history and all actual grants
-- revoked. Siblings need not already be CleanedUp to prepare this Run; final
-- instance removal still requires every consumer completed in its owning API.
CREATE OR REPLACE FUNCTION invocation119_queued_cleanup_zero_history(execution runs) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 RETURN queued_drain_zero140(execution);
END $$;
REVOKE ALL ON FUNCTION invocation119_queued_cleanup_zero_history(runs) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION ordinary_queued_cancel_selected140(p_run uuid) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_invocation117_worker();
 -- Selection only: malformed/pending Control rows must enter positive denial,
 -- never disappear behind a filtered lookup or fall back to a Drain claim.
 RETURN EXISTS(SELECT 1 FROM control_requests WHERE run_id=p_run AND kind='cancel_run');
END $$;
REVOKE ALL ON FUNCTION ordinary_queued_cancel_selected140(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION ordinary_queued_cancel_selected140(uuid) TO hephaestus_worker;

-- Exact instance-only removal origin. This reads original comparison data; it
-- does not assert that this Run received a removal Cancel or was processed.
CREATE FUNCTION ordinary_queued_cancel_removal_origin140(p_instance uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 RETURN queued_removal_origin140(p_instance);
END $$;
REVOKE ALL ON FUNCTION ordinary_queued_cancel_removal_origin140(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION ordinary_queued_cancel_original140(p_run uuid,p_cancel jsonb) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_invocation117_worker();
 RETURN queued_ordinary_original140(p_run,p_cancel);
END $$;
REVOKE ALL ON FUNCTION ordinary_queued_cancel_original140(uuid,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION ordinary_queued_cancel_zero_history140(execution runs) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_invocation117_worker();
 RETURN queued_ordinary_zero140(execution);
END $$;
REVOKE ALL ON FUNCTION ordinary_queued_cancel_zero_history140(runs) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION ordinary_queued_cancel_events140(execution runs,p_original jsonb,p_prepared boolean) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_invocation117_worker();
 RETURN queued_ordinary_events140(execution,p_original,p_prepared);
END $$;
REVOKE ALL ON FUNCTION ordinary_queued_cancel_events140(runs,jsonb,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION ordinary_queued_cancel_qualify140(p_run uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea,p_mode text) RETURNS runs
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_invocation117_worker();
 RETURN queued_ordinary_qualify140(p_run,p_original,p_namespace,p_host,p_config,p_mode);
END $$;
REVOKE ALL ON FUNCTION ordinary_queued_cancel_qualify140(uuid,jsonb,text,text,bytea,text) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION ordinary_queued_cancel_reservations140(p_run uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea,p_mode text)
RETURNS TABLE(lock_class integer,command_mutex bigint,first_mutex integer,second_mutex integer)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_invocation117_worker();
 IF current_setting('transaction_isolation') IS DISTINCT FROM 'read committed'
  OR current_setting('transaction_read_only') IS DISTINCT FROM 'off' THEN
  RAISE EXCEPTION 'ordinary queued Cancel requires writable read-committed transaction' USING ERRCODE='P1400'; END IF;
 PERFORM ordinary_queued_cancel_qualify140(p_run,p_original,p_namespace,p_host,p_config,p_mode);
 -- Reuse the complete genuine import/Invoke/instance reservations without117
 -- consumption. Add all admitted Control identities, in the SAME sorted class.
 RETURN QUERY SELECT early.lock_class,early.command_mutex,early.first_mutex,early.second_mutex
  FROM invocation119_reservation_context(p_run) early
  UNION SELECT 0,recipe_import107_command_mutex(sha256(uuid_send((item->>'id')::uuid))),NULL::integer,NULL::integer
  FROM jsonb_array_elements(p_original->'controls') item
  UNION SELECT 0,recipe_import107_command_mutex(removal_command.command_key),NULL::integer,NULL::integer
  FROM release_command_inbox removal_command
  WHERE p_original ? 'removal' AND removal_command.operation='request_instance_removal'
   AND removal_command.secondary_id=(p_original->'removal'->>'removal_id')::uuid
  ORDER BY 1,2,3,4;
END $$;
REVOKE ALL ON FUNCTION ordinary_queued_cancel_reservations140(uuid,jsonb,text,text,bytea,text) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION ordinary_queued_cancel_reservations140(uuid,jsonb,text,text,bytea,text) TO hephaestus_worker;

CREATE FUNCTION lock_ordinary_queued_cancel140(p_run uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea,p_mode text) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE early record; execution runs%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE; extra bytea;
BEGIN
 PERFORM require_invocation117_worker();
 IF current_setting('lock_timeout')::interval=interval '0'
  OR current_setting('lock_timeout')::interval>interval '2 seconds' THEN
  PERFORM set_config('lock_timeout','2s',true);
 END IF;
 FOR early IN SELECT * FROM ordinary_queued_cancel_reservations140(p_run,p_original,p_namespace,p_host,p_config,p_mode)
   ORDER BY lock_class,command_mutex,first_mutex,second_mutex LOOP
  IF early.lock_class=0 THEN
   IF pg_try_advisory_xact_lock(early.command_mutex) IS DISTINCT FROM true THEN
    RAISE EXCEPTION 'ordinary original command reservation held' USING ERRCODE='55P03'; END IF;
  ELSE
   IF pg_try_advisory_xact_lock(early.first_mutex,early.second_mutex) IS DISTINCT FROM true THEN
    RAISE EXCEPTION 'ordinary original instance reservation held' USING ERRCODE='55P03'; END IF;
  END IF;
 END LOOP;
 SELECT * INTO execution FROM runs WHERE id=p_run;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=execution.instance_id;
 IF p_original ? 'removal' THEN
  SELECT command_key INTO extra FROM release_command_inbox
   WHERE operation='request_instance_removal' AND secondary_id=(p_original->'removal'->>'removal_id')::uuid;
 END IF;
 -- Existing helpers lock Project/Deployment/all resources, then actual Instance/
 -- all Runs/sorted Volumes/grants and remaining participants. They only lock;
 -- ordinary Cancel does NOT revoke grants, close the instance or other Runs.
 PERFORM lock_prepared_closure_parents118(execution.instance_id,extra);
 PERFORM lock_prepared_closure_participants118(execution.instance_id,child.project_id);
 PERFORM id FROM control_requests WHERE run_id=p_run ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM authorization_audit_events WHERE object_type='run' AND object_id=p_run ORDER BY id FOR UPDATE NOWAIT;
 PERFORM run_id FROM run_cleanup_targets WHERE run_id=p_run FOR UPDATE NOWAIT;
 PERFORM lease_id FROM run_cleanup_target_leases WHERE run_id=p_run ORDER BY volume_id,lease_id FOR UPDATE NOWAIT;
 PERFORM run_id FROM run_cleanup_receipts WHERE run_id=p_run ORDER BY generation FOR UPDATE NOWAIT;
 PERFORM run_id FROM run_cleanup_completions WHERE run_id=p_run ORDER BY generation FOR UPDATE NOWAIT;
 PERFORM command_id FROM command_inbox WHERE payload->>'run_id'=p_run::text
    OR command_id IN (SELECT id FROM control_requests WHERE run_id=p_run) ORDER BY command_id FOR UPDATE NOWAIT;
 PERFORM id FROM outbox WHERE aggregate_id=p_run OR aggregate_id IN (SELECT id FROM control_requests WHERE run_id=p_run)
   ORDER BY id FOR UPDATE NOWAIT;
 PERFORM id FROM run_events WHERE run_id=p_run ORDER BY sequence FOR UPDATE NOWAIT;
 PERFORM ordinary_queued_cancel_qualify140(p_run,p_original,p_namespace,p_host,p_config,p_mode);
END $$;
REVOKE ALL ON FUNCTION lock_ordinary_queued_cancel140(uuid,jsonb,text,text,bytea,text) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION lock_ordinary_queued_cancel140(uuid,jsonb,text,text,bytea,text) TO hephaestus_worker;

CREATE FUNCTION ordinary_cancel_delivery_reservations140(p_run uuid,p_cancel jsonb,p_namespace text,p_host text,p_config bytea)
RETURNS TABLE(lock_class integer,command_mutex bigint,first_mutex integer,second_mutex integer)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 RETURN QUERY SELECT * FROM ordinary_queued_cancel_reservations140(p_run,
  ordinary_queued_cancel_original140(p_run,p_cancel),p_namespace,p_host,p_config,'delivery');
END $$;
REVOKE ALL ON FUNCTION ordinary_cancel_delivery_reservations140(uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION ordinary_cancel_delivery_reservations140(uuid,jsonb,text,text,bytea) TO hephaestus_worker;

CREATE FUNCTION lock_ordinary_cancel_delivery140(p_run uuid,p_cancel jsonb,p_namespace text,p_host text,p_config bytea) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM lock_ordinary_queued_cancel140(p_run,ordinary_queued_cancel_original140(p_run,p_cancel),p_namespace,p_host,p_config,'delivery');
END $$;
REVOKE ALL ON FUNCTION lock_ordinary_cancel_delivery140(uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION lock_ordinary_cancel_delivery140(uuid,jsonb,text,text,bytea) TO hephaestus_worker;

CREATE FUNCTION verify_ordinary_cancel_delivery140(p_run uuid,p_cancel jsonb,p_namespace text,p_host text,p_config bytea) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 IF current_setting('transaction_isolation') IS DISTINCT FROM 'read committed'
  OR current_setting('transaction_read_only') IS DISTINCT FROM 'off' THEN
  RAISE EXCEPTION 'ordinary Cancel verification requires writable read-committed transaction' USING ERRCODE='P1400'; END IF;
 PERFORM ordinary_queued_cancel_qualify140(p_run,ordinary_queued_cancel_original140(p_run,p_cancel),p_namespace,p_host,p_config,'preparing');
 IF NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(p_cancel->>'command_id')::uuid
   AND command_type='cancel_run' AND payload=p_cancel AND processed_at IS NOT NULL) THEN
  RAISE EXCEPTION 'exact owning-processed ordinary Cancel required' USING ERRCODE='P1400'; END IF;
END $$;
REVOKE ALL ON FUNCTION verify_ordinary_cancel_delivery140(uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION verify_ordinary_cancel_delivery140(uuid,jsonb,text,text,bytea) TO hephaestus_worker;

CREATE FUNCTION discover_ordinary_queued_cancel_context140(p_run uuid,p_namespace text,p_host text,p_config bytea)
RETURNS TABLE(instance_id uuid,original jsonb)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE pins jsonb; execution runs%ROWTYPE;
BEGIN
 pins:=ordinary_queued_cancel_original140(p_run,NULL);
 execution:=ordinary_queued_cancel_qualify140(p_run,pins,p_namespace,p_host,p_config,'preparing');
 RETURN QUERY SELECT execution.instance_id,pins;
END $$;
REVOKE ALL ON FUNCTION discover_ordinary_queued_cancel_context140(uuid,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION discover_ordinary_queued_cancel_context140(uuid,text,text,bytea) TO hephaestus_worker;

CREATE FUNCTION load_ordinary_queued_cancel_prepared140(p_run uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea) RETURNS SETOF runs
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 RETURN NEXT ordinary_queued_cancel_qualify140(p_run,p_original,p_namespace,p_host,p_config,'prepared');
END $$;
REVOKE ALL ON FUNCTION load_ordinary_queued_cancel_prepared140(uuid,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION load_ordinary_queued_cancel_prepared140(uuid,jsonb,text,text,bytea) TO hephaestus_worker;

-- Raw calls must use the same owner/early TRY/parent/late NOWAIT boundary as
-- the adapter. BEFORE INSERT sees no new row; the existing typed owning tail
-- must atomically stamp processing and cancellation before deferred validation.
CREATE FUNCTION guard_queued_invocation_cancel_inbox140() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE; p_run uuid;
BEGIN
 IF jsonb_typeof(NEW.payload->'run_id') IS DISTINCT FROM 'string' THEN RETURN NEW; END IF;
 BEGIN p_run:=(NEW.payload->>'run_id')::uuid;
 EXCEPTION WHEN invalid_text_representation THEN RETURN NEW; END;
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.run_kind IS DISTINCT FROM 'invocation'
  OR EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=p_run) THEN RETURN NEW; END IF;
 PERFORM require_invocation117_worker();
 IF NEW.payload->>'command_id' IS DISTINCT FROM NEW.command_id::text THEN
  RAISE EXCEPTION 'queued Cancel command identity differs' USING ERRCODE='P1400'; END IF;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 IF ordinary_queued_cancel_selected140(p_run) THEN
  PERFORM lock_ordinary_cancel_delivery140(p_run,NEW.payload,execution.vm_provider_namespace,execution.vm_host_id,receipt.config_bytes);
 ELSE
  PERFORM lock_queued_cancel_delivery119(p_run,NEW.payload,execution.vm_provider_namespace,execution.vm_host_id,receipt.config_bytes);
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_queued_invocation_cancel_inbox140() FROM PUBLIC,hephaestus_app,hephaestus_worker;
CREATE TRIGGER b_queued_invocation_cancel_inbox140 BEFORE INSERT ON command_inbox FOR EACH ROW
 WHEN (NEW.command_type='cancel_run') EXECUTE FUNCTION guard_queued_invocation_cancel_inbox140();

CREATE FUNCTION validate_ordinary_queued_cancel_commit140() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE; p_run uuid;
BEGIN
 IF TG_TABLE_NAME='runs' THEN p_run:=NEW.id;
 ELSE
  IF jsonb_typeof(NEW.payload->'run_id') IS DISTINCT FROM 'string' THEN RETURN NULL; END IF;
  BEGIN p_run:=(NEW.payload->>'run_id')::uuid;
  EXCEPTION WHEN invalid_text_representation THEN RETURN NULL; END;
 END IF;
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.run_kind IS DISTINCT FROM 'invocation'
  OR EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=p_run) THEN RETURN NULL; END IF;
 PERFORM require_invocation117_worker();
 IF NOT ordinary_queued_cancel_selected140(p_run) THEN RETURN NULL; END IF;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 PERFORM ordinary_queued_cancel_qualify140(p_run,ordinary_queued_cancel_original140(p_run,NULL),
  execution.vm_provider_namespace,execution.vm_host_id,receipt.config_bytes,'preparing');
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION validate_ordinary_queued_cancel_commit140() FROM PUBLIC,hephaestus_app,hephaestus_worker;
CREATE CONSTRAINT TRIGGER ordinary_queued_cancel_inbox_commit140 AFTER INSERT OR UPDATE ON command_inbox
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (NEW.command_type='cancel_run')
 EXECUTE FUNCTION validate_ordinary_queued_cancel_commit140();
CREATE CONSTRAINT TRIGGER ordinary_queued_cancel_run_commit140 AFTER UPDATE ON runs
 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (NEW.run_kind='invocation' AND NEW.cancel_requested_at IS NOT NULL)
 EXECUTE FUNCTION validate_ordinary_queued_cancel_commit140();

-- Only the owning lock dispatch is extended; original metadata/phase guards remain.
CREATE OR REPLACE FUNCTION assert_invocation119_queued_cleanup_transition(p_run uuid,p_next text) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE; original jsonb; completing boolean;
BEGIN
 PERFORM require_run_cleanup_wake118();
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.id IS NULL OR execution.state NOT IN ('queued','cancelled','cleaning_up','cleaned_up')
  OR p_next IS NULL OR NOT (p_next=execution.state OR (execution.state='queued' AND p_next='cancelled')
    OR (execution.state='cancelled' AND p_next='cleaning_up') OR (execution.state='cleaning_up' AND p_next='cleaned_up')) THEN
  RAISE EXCEPTION 'only closed queued cancellation and cleanup progress permitted' USING ERRCODE='23000'; END IF;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 completing:=execution.state='cleaning_up' AND p_next='cleaned_up';
 IF ordinary_queued_cancel_selected140(p_run) THEN
  original:=ordinary_queued_cancel_original140(p_run,NULL);
  PERFORM lock_ordinary_queued_cancel140(p_run,original,execution.vm_provider_namespace,execution.vm_host_id,
   receipt.config_bytes,CASE WHEN completing THEN 'completing' ELSE 'prepared' END);
 ELSE
  original:=invocation119_queued_cleanup_original(p_run);
  PERFORM lock_invocation119_queued_cleanup(p_run,execution.instance_id,original,execution.vm_provider_namespace,
   execution.vm_host_id,receipt.config_bytes,false,completing);
 END IF;
 IF p_next='cleaning_up' AND NOT EXISTS(SELECT 1 FROM run_cleanup_receipts WHERE run_id=p_run AND generation=1) THEN
  RAISE EXCEPTION 'cleanup progress requires actual scoped observation' USING ERRCODE='23000'; END IF;
END $$;
REVOKE ALL ON FUNCTION assert_invocation119_queued_cleanup_transition(uuid,text) FROM PUBLIC,hephaestus_app,hephaestus_worker;

-- Only the owning lock dispatch is extended; original metadata/phase guards remain.
CREATE OR REPLACE FUNCTION validate_invocation119_queued_cleanup(p_old runs,p_new runs) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE; original jsonb;
BEGIN
 PERFORM require_run_cleanup_wake118();
 SELECT * INTO execution FROM runs WHERE id=p_new.id;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_new.id;
 IF ordinary_queued_cancel_selected140(p_new.id) THEN
  original:=ordinary_queued_cancel_original140(p_new.id,NULL);
  PERFORM lock_ordinary_queued_cancel140(p_new.id,original,execution.vm_provider_namespace,execution.vm_host_id,receipt.config_bytes,'prepared');
 ELSE
  original:=invocation119_queued_cleanup_original(p_new.id);
  PERFORM lock_invocation119_queued_cleanup(p_new.id,execution.instance_id,original,execution.vm_provider_namespace,
   execution.vm_host_id,receipt.config_bytes,false,false);
 END IF;
 IF p_old.run_kind IS DISTINCT FROM 'invocation' OR p_new.run_kind IS DISTINCT FROM 'invocation'
  OR (to_jsonb(p_new)-ARRAY['state','outcome','state_version','updated_at','volume_acquisition_closed_at'])
    IS DISTINCT FROM (to_jsonb(p_old)-ARRAY['state','outcome','state_version','updated_at','volume_acquisition_closed_at'])
  OR p_new.state NOT IN ('queued','cancelled','cleaning_up','cleaned_up')
  OR (p_new.state='queued' AND p_new.outcome IS NOT NULL)
  OR (p_new.state<>'queued' AND p_new.outcome IS DISTINCT FROM 'cancelled') THEN
  RAISE EXCEPTION 'queued cleanup metadata cannot change producer or outcome authority' USING ERRCODE='23000'; END IF;
 IF p_new.state IS DISTINCT FROM p_old.state THEN
  IF NOT ((p_old.state='queued' AND p_new.state='cancelled') OR (p_old.state='cancelled' AND p_new.state='cleaning_up')
    OR (p_old.state='cleaning_up' AND p_new.state='cleaned_up'))
   OR p_new.state_version IS DISTINCT FROM p_old.state_version+1
   OR p_new.volume_acquisition_closed_at IS DISTINCT FROM p_old.volume_acquisition_closed_at THEN
   RAISE EXCEPTION 'queued cleanup requires exact normal transition/version and permanent closure' USING ERRCODE='23000'; END IF;
 ELSE
  IF p_new.state_version IS DISTINCT FROM p_old.state_version OR p_new.outcome IS DISTINCT FROM p_old.outcome
    OR (p_new.volume_acquisition_closed_at IS DISTINCT FROM p_old.volume_acquisition_closed_at
      AND NOT (p_old.state='queued' AND p_old.volume_acquisition_closed_at IS NULL
        AND p_new.volume_acquisition_closed_at IS NOT NULL)) THEN
   RAISE EXCEPTION 'queued preparation cannot stamp state and replay cannot change history' USING ERRCODE='23000'; END IF;
 END IF;
END $$;
REVOKE ALL ON FUNCTION validate_invocation119_queued_cleanup(runs,runs) FROM PUBLIC,hephaestus_app,hephaestus_worker;


-- Owning Drain may inspect actual deliveries before any writable Cancel tail.
-- This worker RRRO facade returns DATA only: no delivery, reservation, cleanup
-- target, receipt or physical absence is admitted by observing the whole set.
CREATE FUNCTION observe_prepared_instance_drain140(p_instance uuid,p_drain uuid,p_drain_claim jsonb,p_create uuid,p_create_claim jsonb,p_prepared jsonb,p_namespace text,p_host text,p_config bytea)
RETURNS TABLE(run_id uuid,cancel_json jsonb,original jsonb)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE birth record; closure jsonb; claim recipe_effect_attempts%ROWTYPE;
 execution runs%ROWTYPE; qualified runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE;
 target run_cleanup_targets%ROWTYPE;
 context jsonb; expected jsonb; observed jsonb; commands jsonb; item jsonb; completed boolean;
 selected boolean; has_delivery boolean; processed integer; result jsonb:='[]'::jsonb;
BEGIN
 PERFORM require_run_cleanup_wake118();
 IF current_setting('transaction_isolation') IS DISTINCT FROM 'repeatable read'
  OR current_setting('transaction_read_only') IS DISTINCT FROM 'on' THEN
  RAISE EXCEPTION 'instance Drain observation requires worker RRRO' USING ERRCODE='P1400'; END IF;
 SELECT * INTO claim FROM recipe_effect_attempts WHERE id=p_drain;
 IF claim.id IS NULL OR claim.action IS DISTINCT FROM 'drain' OR claim.claim_json IS DISTINCT FROM p_drain_claim
  OR p_config IS NULL OR NOT run_cleanup_identifier_is_valid(p_namespace) OR NOT run_cleanup_identifier_is_valid(p_host)
  OR convert_from(p_config,'UTF8')::jsonb->>'version' IS DISTINCT FROM '1'
  OR convert_from(p_config,'UTF8')::jsonb->>'profile' IS DISTINCT FROM 'runtime_named_v1'
  OR convert_from(p_config,'UTF8')::jsonb->'vm' IS DISTINCT FROM jsonb_build_object('namespace',p_namespace,'host_id',p_host)
  OR convert_from(p_config,'UTF8')::jsonb->'volume'->>'host_id' IS DISTINCT FROM p_host THEN
  RAISE EXCEPTION 'exact configured original Drain claim required' USING ERRCODE='P1400'; END IF;
 -- Empty inventory still positively qualifies the original Create and closure.
 SELECT * INTO birth FROM observe_prepared_instance_safety118(p_create,p_create_claim,p_prepared,p_namespace,p_host);
 closure:=ordinary_queued_cancel_removal_origin140(p_instance);
 IF birth.birth_proven IS DISTINCT FROM true OR birth.removal_id IS DISTINCT FROM (closure->>'removal_id')::uuid
  OR prepared_instance_target118(p_create,p_create_claim,p_prepared) IS DISTINCT FROM p_instance
  OR run_cleanup_wake_original118(p_instance) IS DISTINCT FROM p_create
  OR (closure->>'drain_attempt')::uuid IS DISTINCT FROM p_drain
  OR (closure->>'drain_command')::uuid IS DISTINCT FROM claim.command_id
  OR EXISTS(SELECT 1 FROM agent_instance_volume_mount_grants grant_row WHERE grant_row.instance_id=p_instance
    AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_revocations WHERE grant_id=grant_row.id)) THEN
  RAISE EXCEPTION 'same original Create, Drain and revoked permanent closure required' USING ERRCODE='P1400'; END IF;
 -- Both association directions count. A foreign/null/dangling association must
 -- not disappear through a Run-owned filter, regardless of state/host/release.
 IF EXISTS(SELECT 1 FROM agent_instance_volume_leases lease LEFT JOIN runs associated ON associated.id=lease.run_id
   WHERE (lease.instance_id=p_instance OR associated.instance_id=p_instance)
    AND (associated.id IS NULL OR associated.instance_id IS DISTINCT FROM p_instance OR lease.instance_id IS DISTINCT FROM p_instance)) THEN
  RAISE EXCEPTION 'complete instance Drain lease associations contradict' USING ERRCODE='P1400'; END IF;
 IF EXISTS(SELECT 1 FROM agent_instance_execution_invocations invocation LEFT JOIN runs associated ON associated.id=invocation.run_id
   WHERE (invocation.instance_id=p_instance OR associated.instance_id=p_instance)
    AND (associated.id IS NULL OR associated.instance_id IS DISTINCT FROM p_instance OR invocation.instance_id IS DISTINCT FROM p_instance))
  OR EXISTS(SELECT 1 FROM run_invocation_consumptions consumption
   LEFT JOIN agent_instance_execution_invocations invocation ON invocation.id=consumption.invocation_id
   LEFT JOIN runs associated ON associated.id=consumption.run_id
   WHERE (invocation.instance_id=p_instance OR associated.instance_id=p_instance)
    AND (associated.id IS NULL OR invocation.id IS NULL OR invocation.run_id IS DISTINCT FROM consumption.run_id
      OR associated.instance_id IS DISTINCT FROM p_instance OR invocation.instance_id IS DISTINCT FROM p_instance)) THEN
  RAISE EXCEPTION 'complete Invocation original/consumption associations contradict' USING ERRCODE='P1400'; END IF;
 FOR execution IN SELECT * FROM runs WHERE instance_id=p_instance ORDER BY id LOOP
  SELECT * INTO receipt FROM agent_instance_execution_invocations invocation WHERE invocation.run_id=execution.id;
  IF execution.execution_profile IS DISTINCT FROM 'owned_canonical' OR execution.run_kind IS DISTINCT FROM 'invocation'
   OR execution.vm_binding_kind IS DISTINCT FROM 'planned' OR execution.vm_plan_admission_required IS DISTINCT FROM false
   OR execution.vm_id IS DISTINCT FROM execution.id::text
   OR ROW(execution.vm_provider_namespace,execution.vm_host_id) IS DISTINCT FROM ROW(p_namespace,p_host)
   OR receipt.id IS NULL OR receipt.config_bytes IS DISTINCT FROM p_config
   OR receipt.config_hash IS DISTINCT FROM sha256(receipt.config_bytes)
   OR receipt.config_json IS DISTINCT FROM convert_from(receipt.config_bytes,'UTF8')::jsonb
   OR qualified_invocation_graph_matches(receipt) IS DISTINCT FROM true THEN
   RAISE EXCEPTION 'every associated Run must have exact configured Invocation original' USING ERRCODE='P1400'; END IF;
  expected:=jsonb_build_object('command_id',run_cleanup_wake_cancel118(birth.removal_id,execution.id),
   'run_id',execution.id,'reason','instance removal requested');
  -- Global exact-ID observation catches malformed rows even under another owner
  -- or event class. The private qualifier checks the full committed envelope.
  has_delivery:=EXISTS(SELECT 1 FROM outbox WHERE payload->>'command_id'=expected->>'command_id');
  selected:=ordinary_queued_cancel_selected140(execution.id);
  observed:=NULL;
  IF has_delivery THEN
   IF (SELECT count(*) FROM outbox WHERE payload->>'command_id'=expected->>'command_id')<>1 THEN
    RAISE EXCEPTION 'one exact original Drain Cancel outbox required' USING ERRCODE='P1400'; END IF;
   SELECT payload-ARRAY['schema_version','message_id','idempotency_key','request_id','trace_id'] INTO observed
    FROM outbox WHERE payload->>'command_id'=expected->>'command_id';
   IF selected THEN context:=ordinary_queued_cancel_original140(execution.id,observed);
   ELSE context:=invocation119_cancel_original(execution.id,observed); END IF;
   IF observed IS DISTINCT FROM expected
    OR (CASE WHEN selected THEN context->'drain' ELSE context END) IS DISTINCT FROM closure THEN
    RAISE EXCEPTION 'actual Run DTO and same global Drain origin required' USING ERRCODE='P1400'; END IF;
  ELSIF selected THEN
   -- Only positive completed ordinary replay explains missing own Drain DTO.
   context:=ordinary_queued_cancel_original140(execution.id,NULL);
   IF NOT EXISTS(SELECT 1 FROM run_invocation_consumptions consumption WHERE consumption.run_id=execution.id) THEN
    qualified:=ordinary_queued_cancel_qualify140(execution.id,context,p_namespace,p_host,p_config,'prepared');
   END IF;
   -- Consumed117 completion is qualified below through its canonical full
   -- target/receipt/completion and frozen lease history, never the115 no-IO rule.
   IF execution.state IS DISTINCT FROM 'cleaned_up' OR context->'removal' IS DISTINCT FROM closure
    OR context ? 'drain' THEN
    RAISE EXCEPTION 'exact completed ordinary replay without own Drain DTO required' USING ERRCODE='P1400'; END IF;
  ELSE
   context:=closure;
  END IF;
  IF EXISTS(SELECT 1 FROM run_invocation_consumptions consumption WHERE consumption.run_id=execution.id) THEN
   -- Immutable117 and complete frozen lease history, never current dispatch or
   -- live-grant eligibility after genuine withdrawal. Old completed replay does
   -- not compare a newer consumer's current volume generation/holder/readiness.
   completed:=execution.state='cleaned_up';
   IF run_cleanup_wake_producer118(execution,p_config) IS DISTINCT FROM true
    OR run_cleanup_wake_set118(execution,completed) IS DISTINCT FROM true
    OR (completed AND ready_retention_run_terminal(execution,p_namespace,p_host) IS DISTINCT FROM true)
    OR (NOT has_delivery AND (NOT completed OR EXISTS(SELECT 1 FROM command_inbox
      WHERE command_id=(expected->>'command_id')::uuid))) THEN
    RAISE EXCEPTION 'exact consumed117 original and full cleanup/history qualification required' USING ERRCODE='P1400'; END IF;
   IF EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(expected->>'command_id')::uuid) THEN
    qualified:=qualify_run_cleanup_wake118(execution.id,p_instance,closure,p_namespace,p_host,p_config,true);
   ELSIF NOT completed THEN
    -- The processed118 facade requires its actual Cancel inbox. Pending DTO
    -- observation reuses only its pure original target/set predicates, without
    -- claiming that processing occurred or treating a partial receipt as done.
    SELECT * INTO target FROM run_cleanup_targets original_target WHERE original_target.run_id=execution.id;
    IF EXISTS(SELECT 1 FROM run_cleanup_completions completion WHERE completion.run_id=execution.id)
     OR (target.run_id IS NULL AND EXISTS(SELECT 1 FROM run_cleanup_receipts actual_receipt WHERE actual_receipt.run_id=execution.id))
     OR (target.run_id IS NOT NULL AND (execution.volume_acquisition_closed_at IS NULL
      OR ROW(target.instance_id,target.instance_revision_id,target.vm_target_kind,target.vm_provider_namespace,target.vm_host_id,target.vm_id)
       IS DISTINCT FROM ROW(execution.instance_id,execution.instance_revision_id,execution.vm_binding_kind,execution.vm_provider_namespace,execution.vm_host_id,execution.vm_id)
      OR target.target_digest IS DISTINCT FROM run_cleanup_target_digest(execution.id,target.generation)
      OR target.lease_count IS DISTINCT FROM (SELECT count(*)::integer FROM run_cleanup_target_leases frozen WHERE frozen.run_id=execution.id)
      OR EXISTS(SELECT 1 FROM run_cleanup_target_leases frozen WHERE frozen.run_id=execution.id AND frozen.generation<>target.generation)
      OR run_cleanup_set_matches(execution.id,target.generation,false) IS DISTINCT FROM true)) THEN
     RAISE EXCEPTION 'pending consumed original target or held set contradicts' USING ERRCODE='P1400'; END IF;
   END IF;
  ELSIF has_delivery THEN
   IF selected THEN qualified:=ordinary_queued_cancel_qualify140(execution.id,context,p_namespace,p_host,p_config,'delivery');
   ELSE qualified:=invocation119_cancel_qualify(execution.id,observed,p_namespace,p_host,p_config); END IF;
  ELSIF NOT selected THEN
   RAISE EXCEPTION 'unconsumed original lacks positive completed ordinary replay' USING ERRCODE='P1400';
  END IF;
  commands:=CASE WHEN selected THEN context->'cancels'
    WHEN has_delivery THEN jsonb_build_array(observed) ELSE '[]'::jsonb END;
  processed:=0;
  FOR item IN SELECT value FROM jsonb_array_elements(commands) LOOP
   IF (SELECT count(*) FROM outbox WHERE payload->>'command_id'=item->>'command_id')
      <>(CASE WHEN selected AND EXISTS(SELECT 1 FROM jsonb_array_elements(context->'controls') control
         WHERE control->>'id'=item->>'command_id') THEN 2 ELSE 1 END) THEN
    RAISE EXCEPTION 'global same-ID Cancel transport inventory differs' USING ERRCODE='P1400'; END IF;
   IF EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(item->>'command_id')::uuid) THEN
    IF NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(item->>'command_id')::uuid
      AND command_type='cancel_run' AND payload=item AND processed_at IS NOT NULL) THEN
     RAISE EXCEPTION 'pending or conflicting associated Cancel inbox is held' USING ERRCODE='P1400'; END IF;
    processed:=processed+1;
   END IF;
  END LOOP;
  IF (SELECT count(*) FROM outbox WHERE payload->>'command_id'=receipt.start_command_id::text)<>1
   OR (SELECT count(*) FROM outbox WHERE aggregate_id=execution.id OR payload->>'run_id'=execution.id::text)
    <>1+jsonb_array_length(commands)+(CASE WHEN selected THEN jsonb_array_length(context->'controls') ELSE 0 END)
   OR (SELECT count(*) FROM command_inbox WHERE payload->>'run_id'=execution.id::text)<>1+processed
   OR NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=receipt.start_command_id
     AND command_type='start_run' AND payload=qualified_invocation_start_json(receipt)
     AND (processed_at IS NOT NULL)=EXISTS(SELECT 1 FROM run_invocation_consumptions consumption WHERE consumption.run_id=execution.id)) THEN
   RAISE EXCEPTION 'complete associated outbox/inbox inventory differs' USING ERRCODE='P1400'; END IF;
  result:=result||jsonb_build_array(jsonb_build_object('run',execution.id,'cancel',observed,'original',context));
 END LOOP;
 -- No DTO is emitted until every original and association has qualified.
 RETURN QUERY SELECT (returned.value->>'run')::uuid,NULLIF(returned.value->'cancel','null'::jsonb),returned.value->'original' FROM jsonb_array_elements(result) AS returned(value);
END $$;
REVOKE ALL ON FUNCTION observe_prepared_instance_drain140(uuid,uuid,jsonb,uuid,jsonb,jsonb,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION observe_prepared_instance_drain140(uuid,uuid,jsonb,uuid,jsonb,jsonb,text,text,bytea) TO hephaestus_worker;


-- Private pure comparison predicates shared by the existing guarded worker
-- fronts and fresh actor Remove fronts. No role switch, lock, write, receipt or
-- authority is granted by these owner-only helpers. Historical1..139 unchanged.
CREATE FUNCTION queued_create_original140(p_instance uuid) RETURNS uuid
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 candidates uuid[]; original recipe_effect_attempts%ROWTYPE; document jsonb;
BEGIN
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=child.deployment_id;
 IF child.instance_id IS NULL OR header.deployment_id IS NULL OR header.profile IS DISTINCT FROM 'runtime_named_v1'
  OR header.input_hash IS DISTINCT FROM sha256(header.canonical_input)
  OR child.input_hash IS DISTINCT FROM sha256(child.canonical_import)
  OR child.command_id IS DISTINCT FROM header.command_id OR child.project_id IS DISTINCT FROM header.project_id
  OR header.instance_count NOT BETWEEN 1 AND 64
  OR header.instance_count IS DISTINCT FROM (SELECT count(*)::integer FROM recipe_prepared_instance_imports WHERE deployment_id=child.deployment_id)
  OR qualified_invocation_import_lineage_matches(p_instance) IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'complete immutable named113 import required' USING ERRCODE='P1180'; END IF;
 document:=convert_from(header.canonical_input,'UTF8')::jsonb;
 IF document->>'version' IS DISTINCT FROM '1' OR document->>'profile' IS DISTINCT FROM header.profile
  OR document->>'deployment_id' IS DISTINCT FROM header.deployment_id::text
  OR document->>'project_id' IS DISTINCT FROM header.project_id::text
  OR document->'intent_hash' IS DISTINCT FROM recipe_effect_hash_json(header.intent_hash)
  OR document->'platform' IS DISTINCT FROM header.platform_json
  OR document->'command' IS DISTINCT FROM (SELECT jsonb_build_object('id',id,'actor_id',actor_id,
    'idempotency_id',idempotency_id,'operation',operation) FROM recipe_deployment_commands WHERE id=header.command_id)
  OR jsonb_typeof(document->'instances') IS DISTINCT FROM 'array'
  OR jsonb_array_length(document->'instances') IS DISTINCT FROM header.instance_count
  OR header.instance_count IS DISTINCT FROM (SELECT count(*)::integer FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_kind='instance')
  OR EXISTS(SELECT 1 FROM recipe_prepared_instance_imports sibling WHERE sibling.deployment_id=child.deployment_id AND (
    sibling.ordinal>=header.instance_count OR sibling.project_id IS DISTINCT FROM header.project_id
    OR sibling.command_id IS DISTINCT FROM header.command_id OR sibling.input_hash IS DISTINCT FROM sha256(sibling.canonical_import)
    OR convert_from(sibling.canonical_import,'UTF8')::jsonb IS DISTINCT FROM document->'instances'->sibling.ordinal
    OR document->'instances'->sibling.ordinal->>'resource' IS DISTINCT FROM sibling.resource_name
    OR document->'instances'->sibling.ordinal->>'operation_id' IS DISTINCT FROM sibling.operation_id::text
    OR document->'instances'->sibling.ordinal->'command_key' IS DISTINCT FROM recipe_effect_hash_json(sibling.import_command_key)
    OR document->'instances'->sibling.ordinal->>'instance_id' IS DISTINCT FROM sibling.instance_id::text
    OR document->'instances'->sibling.ordinal->>'revision_id' IS DISTINCT FROM sibling.revision_id::text
    OR document->'instances'->sibling.ordinal->>'project_id' IS DISTINCT FROM sibling.project_id::text
    OR document->'instances'->sibling.ordinal->'source'->'pin' IS DISTINCT FROM jsonb_build_object('release_id',sibling.release_id,'release_agent_id',sibling.release_agent_id)
    OR document->'instances'->sibling.ordinal->'source'->'platform' IS DISTINCT FROM header.platform_json))
  OR NOT EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox product ON product.event_id=event.id
    WHERE event.id=header.event_id AND event.cursor=header.event_cursor AND event.aggregate_version=header.event_aggregate_version
     AND event.scope_kind='project' AND event.scope_id=header.project_id AND event.aggregate_type='project' AND event.aggregate_id=header.project_id
     AND event.event_type='project.changed' AND event.change_kind='updated' AND event.actor_type='user'
     AND event.actor_id=header.actor_id AND event.request_id=header.request_id) THEN
  RAISE EXCEPTION 'complete original113 bytes and committed preparation required' USING ERRCODE='P1180'; END IF;
 SELECT array_agg(DISTINCT attempt.id ORDER BY attempt.id) INTO candidates
  FROM recipe_effect_attempts attempt JOIN recipe_effect_verifications proof ON proof.attempt_id=attempt.id
  JOIN recipe_execution_transitions transition ON transition.attempt_id=attempt.id AND transition.verification_id=proof.id
  JOIN recipe_deployment_commands current_command ON current_command.id=transition.command_id
  WHERE attempt.deployment_id=child.deployment_id AND attempt.resource_name=child.resource_name
   AND attempt.command_id=header.command_id AND attempt.action='create' AND attempt.actor_id=header.actor_id
   AND attempt.project_id=child.project_id
   AND proof.recorded_by='hephaestus_worker' AND proof.action='create' AND proof.outcome='applied' AND proof.diagnostic IS NULL
   AND proof.deployment_id=attempt.deployment_id AND proof.project_id=attempt.project_id
   AND proof.resource_name=attempt.resource_name AND proof.generation=attempt.generation
   AND proof.input_hash=attempt.input_hash AND proof.identity_json=attempt.claim_json->'identity'
   AND transition.kind IN ('completed','reconciled') AND transition.operation_id=proof.verification_attempt_id
   AND transition.project_id=attempt.project_id AND transition.resource_name=attempt.resource_name
   AND transition.after_state->>'install'='ready' AND transition.after_state->>'active_attempt' IS NULL
   AND ((current_command.id=attempt.command_id AND current_command.operation='install')
    OR (transition.kind='reconciled' AND current_command.operation='remove'
      AND current_command.deployment_id=attempt.deployment_id AND current_command.project_id=attempt.project_id));
 IF COALESCE(cardinality(candidates),0)<>1 THEN
  RAISE EXCEPTION 'one committed original Create proof required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO original FROM recipe_effect_attempts WHERE id=candidates[1];
 IF NOT EXISTS(SELECT 1 FROM recipe_execution_transitions claimed JOIN application_events event ON event.id=claimed.event_id
    JOIN product_event_outbox product ON product.event_id=event.id
    WHERE claimed.attempt_id=original.id AND claimed.kind='claimed' AND claimed.command_id=original.command_id
     AND claimed.project_id=original.project_id AND claimed.resource_name=original.resource_name
     AND claimed.operation_id=original.id AND claimed.fingerprint=original.request_fingerprint AND claimed.before_state=original.before_state
     AND claimed.after_state->>'active_attempt'=original.id::text AND claimed.after_state->>'generation'=original.generation::text
     AND event.actor_id=original.actor_id AND event.request_id=original.request_id
     AND event.cursor=claimed.event_cursor AND event.aggregate_version=claimed.event_aggregate_version) THEN
  RAISE EXCEPTION 'committed immutable original Create claim required' USING ERRCODE='P1180'; END IF;
 IF prepared_instance_target118(original.id,original.claim_json,convert_from(child.canonical_import,'UTF8')::jsonb) IS DISTINCT FROM p_instance THEN
  RAISE EXCEPTION 'exact original prepared target required' USING ERRCODE='P1180'; END IF;
 RETURN original.id;
END $$;
REVOKE ALL ON FUNCTION queued_create_original140(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION queued_processed_closure140(p_run uuid,p_instance uuid,p_original jsonb) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
 drain recipe_effect_attempts%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 removal agent_instance_removal_requests%ROWTYPE; inbox release_command_inbox%ROWTYPE;
 expected jsonb; cancel jsonb; admission recipe_remove_resume_admissions%ROWTYPE; recorded jsonb;
BEGIN
 -- Global Run selection precedes ownership checks; no filtered absence inference.
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.id IS NULL OR execution.instance_id IS DISTINCT FROM p_instance OR execution.cancel_requested_at IS NULL THEN
  RAISE EXCEPTION 'exact original cancelled Run required' USING ERRCODE='P1180'; END IF;
 PERFORM queued_create_original140(p_instance);
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=(p_original->>'drain_attempt')::uuid;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=drain.command_id;
 SELECT * INTO removal FROM agent_instance_removal_requests WHERE id=(p_original->>'removal_id')::uuid;
 IF (SELECT count(*) FROM agent_instance_removal_requests WHERE instance_id=p_instance)<>1
  OR (SELECT count(*) FROM release_command_inbox WHERE secondary_id=removal.id AND operation='request_instance_removal')<>1 THEN
  RAISE EXCEPTION 'unique original permanent closure required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO inbox FROM release_command_inbox WHERE secondary_id=removal.id AND operation='request_instance_removal';
 expected:=jsonb_build_object('drain_attempt',drain.id,'drain_command',drain.command_id,'removal_id',removal.id,
  'command_key',recipe_effect_hash_json(inbox.command_key),'input_hash',recipe_effect_hash_json(removal.input_hash),
  'expected_version',removal.expected_version,'creator',removal.created_by,'request',removal.request_id);
 IF p_original IS DISTINCT FROM expected OR drain.id IS NULL OR removal.id IS NULL
  OR drain.action IS DISTINCT FROM 'drain' OR command.operation IS DISTINCT FROM 'remove'
  OR command.deployment_id IS DISTINCT FROM child.deployment_id OR command.project_id IS DISTINCT FROM child.project_id
  OR command.actor_id IS DISTINCT FROM drain.actor_id OR drain.actor_id IS DISTINCT FROM removal.created_by
  OR drain.deployment_id IS DISTINCT FROM child.deployment_id OR drain.resource_name IS DISTINCT FROM child.resource_name
  OR drain.project_id IS DISTINCT FROM child.project_id
  OR drain.input_hash IS DISTINCT FROM (SELECT input_hash FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name)
  OR drain.claim_json->'identity' IS DISTINCT FROM (SELECT plan_json->'identity' FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name)
  OR removal.instance_id IS DISTINCT FROM p_instance OR removal.expected_version<1
  OR original_drain_removal118(drain.id,p_instance) IS DISTINCT FROM removal.id
  OR original_drain_command_key118(drain.id,p_instance) IS DISTINCT FROM inbox.command_key
  OR inbox.aggregate_id IS DISTINCT FROM p_instance OR inbox.actor_id IS DISTINCT FROM removal.created_by
  OR inbox.request_id IS DISTINCT FROM removal.request_id OR inbox.input_hash IS DISTINCT FROM removal.input_hash
  OR removal.input_hash IS DISTINCT FROM sha256(convert_to(recipe_import107_json_bytes(jsonb_build_object('version',1,'operation','request_instance_removal',
    'actor',drain.actor_id,'instance',p_instance,'removal',removal.id,'expected_version',removal.expected_version),0),'UTF8'))
  OR prepared_instance_removal118(p_instance) IS DISTINCT FROM removal.id
  OR NOT EXISTS(SELECT 1 FROM recipe_execution_transitions claimed JOIN application_events event ON event.id=claimed.event_id
   JOIN product_event_outbox product ON product.event_id=event.id
   WHERE claimed.attempt_id=drain.id AND claimed.kind='claimed' AND claimed.command_id=drain.command_id
    AND claimed.project_id=drain.project_id AND claimed.resource_name=drain.resource_name
    AND claimed.operation_id=drain.id AND claimed.fingerprint=drain.request_fingerprint AND claimed.before_state=drain.before_state
    AND claimed.after_state->>'active_attempt'=drain.id::text AND claimed.after_state->>'generation'=drain.generation::text
    AND event.actor_id=drain.actor_id AND event.request_id=drain.request_id
    AND event.cursor=claimed.event_cursor AND event.aggregate_version=claimed.event_aggregate_version) THEN
  RAISE EXCEPTION 'immutable original Drain and closure correlation required' USING ERRCODE='P1180'; END IF;
 -- A fresh B continuation may preserve A's closure but cannot replace its pins.
 FOR admission IN SELECT * FROM recipe_remove_resume_admissions WHERE deployment_id=child.deployment_id LOOP
  IF (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=child.resource_name)>1 THEN
   RAISE EXCEPTION 'ambiguous admitted original closure' USING ERRCODE='P1180'; END IF;
  SELECT item INTO recorded FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=child.resource_name;
  IF recorded IS NOT NULL AND recorded IS DISTINCT FROM jsonb_build_object('resource',child.resource_name,'drain_claim',drain.claim_json,
    'actor',command.actor_id,'idempotency',command.idempotency_id,'removal',to_jsonb(removal),'inbox',to_jsonb(inbox)) THEN
   RAISE EXCEPTION 'admitted original closure changed' USING ERRCODE='P1180'; END IF;
 END LOOP;
 cancel:=jsonb_build_object('command_id',run_cleanup_wake_cancel118(removal.id,p_run),'run_id',p_run,'reason','instance removal requested');
 -- Existing release outbox uses aggregate_type=release with this exact Run ID;
 -- transport enrichment is compared separately from the canonical Cancel DTO.
 IF (SELECT count(*) FROM outbox WHERE aggregate_type='release' AND aggregate_id=p_run
    AND subject='heph.run.command.cancel.v1' AND event_type='run.cancel_requested.v1'
    AND payload->'command_id'=cancel->'command_id' AND payload->'run_id'=cancel->'run_id' AND payload->'reason'=cancel->'reason')<>1
  OR NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(cancel->>'command_id')::uuid
    AND command_type='cancel_run' AND payload=cancel AND processed_at IS NOT NULL) THEN
  RAISE EXCEPTION 'exact committed and owning-processed original Cancel required' USING ERRCODE='P1180'; END IF;
END $$;
REVOKE ALL ON FUNCTION queued_processed_closure140(uuid,uuid,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION queued_cancel_closure140(p_run uuid,p_instance uuid,p_original jsonb,p_cancel jsonb) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
 drain recipe_effect_attempts%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 removal agent_instance_removal_requests%ROWTYPE; inbox release_command_inbox%ROWTYPE;
 expected jsonb; cancel jsonb; admission recipe_remove_resume_admissions%ROWTYPE; recorded jsonb;
BEGIN
 -- Global Run selection precedes ownership checks; no filtered absence inference.
 SELECT * INTO execution FROM runs WHERE id=p_run;
 IF execution.id IS NULL OR execution.instance_id IS DISTINCT FROM p_instance OR execution.cancel_requested_at IS NULL THEN
  RAISE EXCEPTION 'exact original cancelled Run required' USING ERRCODE='P1180'; END IF;
 PERFORM queued_create_original140(p_instance);
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=(p_original->>'drain_attempt')::uuid;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=drain.command_id;
 SELECT * INTO removal FROM agent_instance_removal_requests WHERE id=(p_original->>'removal_id')::uuid;
 IF (SELECT count(*) FROM agent_instance_removal_requests WHERE instance_id=p_instance)<>1
  OR (SELECT count(*) FROM release_command_inbox WHERE secondary_id=removal.id AND operation='request_instance_removal')<>1 THEN
  RAISE EXCEPTION 'unique original permanent closure required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO inbox FROM release_command_inbox WHERE secondary_id=removal.id AND operation='request_instance_removal';
 expected:=jsonb_build_object('drain_attempt',drain.id,'drain_command',drain.command_id,'removal_id',removal.id,
  'command_key',recipe_effect_hash_json(inbox.command_key),'input_hash',recipe_effect_hash_json(removal.input_hash),
  'expected_version',removal.expected_version,'creator',removal.created_by,'request',removal.request_id);
 IF p_original IS DISTINCT FROM expected OR drain.id IS NULL OR removal.id IS NULL
  OR drain.action IS DISTINCT FROM 'drain' OR command.operation IS DISTINCT FROM 'remove'
  OR command.deployment_id IS DISTINCT FROM child.deployment_id OR command.project_id IS DISTINCT FROM child.project_id
  OR command.actor_id IS DISTINCT FROM drain.actor_id OR drain.actor_id IS DISTINCT FROM removal.created_by
  OR drain.deployment_id IS DISTINCT FROM child.deployment_id OR drain.resource_name IS DISTINCT FROM child.resource_name
  OR drain.project_id IS DISTINCT FROM child.project_id
  OR drain.input_hash IS DISTINCT FROM (SELECT input_hash FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name)
  OR drain.claim_json->'identity' IS DISTINCT FROM (SELECT plan_json->'identity' FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name)
  OR removal.instance_id IS DISTINCT FROM p_instance OR removal.expected_version<1
  OR original_drain_removal118(drain.id,p_instance) IS DISTINCT FROM removal.id
  OR original_drain_command_key118(drain.id,p_instance) IS DISTINCT FROM inbox.command_key
  OR inbox.aggregate_id IS DISTINCT FROM p_instance OR inbox.actor_id IS DISTINCT FROM removal.created_by
  OR inbox.request_id IS DISTINCT FROM removal.request_id OR inbox.input_hash IS DISTINCT FROM removal.input_hash
  OR removal.input_hash IS DISTINCT FROM sha256(convert_to(recipe_import107_json_bytes(jsonb_build_object('version',1,'operation','request_instance_removal',
    'actor',drain.actor_id,'instance',p_instance,'removal',removal.id,'expected_version',removal.expected_version),0),'UTF8'))
  OR prepared_instance_removal118(p_instance) IS DISTINCT FROM removal.id
  OR NOT EXISTS(SELECT 1 FROM recipe_execution_transitions claimed JOIN application_events event ON event.id=claimed.event_id
   JOIN product_event_outbox product ON product.event_id=event.id
   WHERE claimed.attempt_id=drain.id AND claimed.kind='claimed' AND claimed.command_id=drain.command_id
    AND claimed.project_id=drain.project_id AND claimed.resource_name=drain.resource_name
    AND claimed.operation_id=drain.id AND claimed.fingerprint=drain.request_fingerprint AND claimed.before_state=drain.before_state
    AND claimed.after_state->>'active_attempt'=drain.id::text AND claimed.after_state->>'generation'=drain.generation::text
    AND event.actor_id=drain.actor_id AND event.request_id=drain.request_id
    AND event.cursor=claimed.event_cursor AND event.aggregate_version=claimed.event_aggregate_version) THEN
  RAISE EXCEPTION 'immutable original Drain and closure correlation required' USING ERRCODE='P1180'; END IF;
 -- A fresh B continuation may preserve A's closure but cannot replace its pins.
 FOR admission IN SELECT * FROM recipe_remove_resume_admissions WHERE deployment_id=child.deployment_id LOOP
  IF (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=child.resource_name)>1 THEN
   RAISE EXCEPTION 'ambiguous admitted original closure' USING ERRCODE='P1180'; END IF;
  SELECT item INTO recorded FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=child.resource_name;
  IF recorded IS NOT NULL AND recorded IS DISTINCT FROM jsonb_build_object('resource',child.resource_name,'drain_claim',drain.claim_json,
    'actor',command.actor_id,'idempotency',command.idempotency_id,'removal',to_jsonb(removal),'inbox',to_jsonb(inbox)) THEN
   RAISE EXCEPTION 'admitted original closure changed' USING ERRCODE='P1180'; END IF;
 END LOOP;
 cancel:=jsonb_build_object('command_id',run_cleanup_wake_cancel118(removal.id,p_run),'run_id',p_run,'reason','instance removal requested');
 -- Exact typed DTO and committed transport correlation; publisher ACK is not authority.
 IF p_cancel IS DISTINCT FROM cancel THEN
  RAISE EXCEPTION 'exact original typed Cancel required' USING ERRCODE='P1180'; END IF;
 IF (SELECT count(*) FROM outbox WHERE aggregate_type='release' AND aggregate_id=p_run
    AND subject='heph.run.command.cancel.v1' AND event_type='run.cancel_requested.v1'
    AND payload=cancel||jsonb_build_object('schema_version',1,'message_id',id,'idempotency_key',id,'request_id',NULL,'trace_id',NULL))<>1 THEN
  RAISE EXCEPTION 'one exact committed original Cancel outbox required' USING ERRCODE='P1180'; END IF;
 -- First delivery is absent; only an exact already-processed row is a duplicate.
 -- An unprocessed conflict cannot be completed by the frozen ON CONFLICT tail.
 IF EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(cancel->>'command_id')::uuid)
  AND NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(cancel->>'command_id')::uuid
    AND command_type='cancel_run' AND payload=cancel AND processed_at IS NOT NULL) THEN
  RAISE EXCEPTION 'original Cancel inbox conflict or pending delivery is held' USING ERRCODE='P1180'; END IF;
END $$;
REVOKE ALL ON FUNCTION queued_cancel_closure140(uuid,uuid,jsonb,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION queued_cancel_original140(p_run uuid,p_cancel jsonb) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; child recipe_prepared_instance_imports%ROWTYPE;
 removal agent_instance_removal_requests%ROWTYPE; command release_command_inbox%ROWTYPE;
 candidates uuid[]; drain recipe_effect_attempts%ROWTYPE; original jsonb;
BEGIN
 SELECT * INTO execution FROM runs WHERE id=p_run;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=execution.instance_id;
 IF execution.id IS NULL OR child.instance_id IS NULL
  OR (SELECT count(*) FROM agent_instance_removal_requests WHERE instance_id=execution.instance_id)<>1 THEN
  RAISE EXCEPTION 'one original queued cleanup closure required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO removal FROM agent_instance_removal_requests WHERE instance_id=execution.instance_id;
 IF (SELECT count(*) FROM release_command_inbox WHERE operation='request_instance_removal' AND secondary_id=removal.id)<>1 THEN
  RAISE EXCEPTION 'one original removal command required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO command FROM release_command_inbox WHERE operation='request_instance_removal' AND secondary_id=removal.id;
 SELECT array_agg(id ORDER BY id) INTO candidates FROM recipe_effect_attempts
  WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name AND action='drain'
   AND original_drain_removal118(id,execution.instance_id)=removal.id;
 IF COALESCE(cardinality(candidates),0)<>1 THEN
  RAISE EXCEPTION 'one immutable original Drain required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=candidates[1];
 original:=jsonb_build_object('drain_attempt',drain.id,'drain_command',drain.command_id,'removal_id',removal.id,
  'command_key',recipe_effect_hash_json(command.command_key),'input_hash',recipe_effect_hash_json(removal.input_hash),
  'expected_version',removal.expected_version,'creator',removal.created_by,'request',removal.request_id);
 -- This is exact original DATA discovery for trigger comparison, not an actor.
 PERFORM queued_cancel_closure140(p_run,execution.instance_id,original,p_cancel);
 RETURN original;
END $$;
REVOKE ALL ON FUNCTION queued_cancel_original140(uuid,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION queued_removal_origin140(p_instance uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE child recipe_prepared_instance_imports%ROWTYPE; removal agent_instance_removal_requests%ROWTYPE;
 inbox release_command_inbox%ROWTYPE; drain recipe_effect_attempts%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 candidates uuid[]; original jsonb; expected jsonb; admission recipe_remove_resume_admissions%ROWTYPE; recorded jsonb;
BEGIN
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 IF child.instance_id IS NULL OR (SELECT count(*) FROM agent_instance_removal_requests WHERE instance_id=p_instance)<>1 THEN
  RAISE EXCEPTION 'one genuine instance removal origin required' USING ERRCODE='P1400'; END IF;
 SELECT * INTO removal FROM agent_instance_removal_requests WHERE instance_id=p_instance;
 IF (SELECT count(*) FROM release_command_inbox WHERE operation='request_instance_removal' AND secondary_id=removal.id)<>1 THEN
  RAISE EXCEPTION 'one original removal command required' USING ERRCODE='P1400'; END IF;
 SELECT * INTO inbox FROM release_command_inbox WHERE operation='request_instance_removal' AND secondary_id=removal.id;
 SELECT array_agg(id ORDER BY id) INTO candidates FROM recipe_effect_attempts
  WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name AND action='drain'
   AND original_drain_removal118(id,p_instance)=removal.id;
 IF COALESCE(cardinality(candidates),0)<>1 THEN
  RAISE EXCEPTION 'one immutable original Drain required' USING ERRCODE='P1400'; END IF;
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=candidates[1];
 original:=jsonb_build_object('drain_attempt',drain.id,'drain_command',drain.command_id,'removal_id',removal.id,
  'command_key',recipe_effect_hash_json(inbox.command_key),'input_hash',recipe_effect_hash_json(removal.input_hash),
  'expected_version',removal.expected_version,'creator',removal.created_by,'request',removal.request_id);
 PERFORM queued_create_original140(p_instance);
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 SELECT * INTO drain FROM recipe_effect_attempts WHERE id=(original->>'drain_attempt')::uuid;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=drain.command_id;
 SELECT * INTO removal FROM agent_instance_removal_requests WHERE id=(original->>'removal_id')::uuid;
 IF (SELECT count(*) FROM agent_instance_removal_requests WHERE instance_id=p_instance)<>1
  OR (SELECT count(*) FROM release_command_inbox WHERE secondary_id=removal.id AND operation='request_instance_removal')<>1 THEN
  RAISE EXCEPTION 'unique original permanent closure required' USING ERRCODE='P1180'; END IF;
 SELECT * INTO inbox FROM release_command_inbox WHERE secondary_id=removal.id AND operation='request_instance_removal';
 expected:=jsonb_build_object('drain_attempt',drain.id,'drain_command',drain.command_id,'removal_id',removal.id,
  'command_key',recipe_effect_hash_json(inbox.command_key),'input_hash',recipe_effect_hash_json(removal.input_hash),
  'expected_version',removal.expected_version,'creator',removal.created_by,'request',removal.request_id);
 IF original IS DISTINCT FROM expected OR drain.id IS NULL OR removal.id IS NULL
  OR drain.action IS DISTINCT FROM 'drain' OR command.operation IS DISTINCT FROM 'remove'
  OR command.deployment_id IS DISTINCT FROM child.deployment_id OR command.project_id IS DISTINCT FROM child.project_id
  OR command.actor_id IS DISTINCT FROM drain.actor_id OR drain.actor_id IS DISTINCT FROM removal.created_by
  OR drain.deployment_id IS DISTINCT FROM child.deployment_id OR drain.resource_name IS DISTINCT FROM child.resource_name
  OR drain.project_id IS DISTINCT FROM child.project_id
  OR drain.input_hash IS DISTINCT FROM (SELECT input_hash FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name)
  OR drain.claim_json->'identity' IS DISTINCT FROM (SELECT plan_json->'identity' FROM recipe_deployment_resources WHERE deployment_id=child.deployment_id AND resource_name=child.resource_name)
  OR removal.instance_id IS DISTINCT FROM p_instance OR removal.expected_version<1
  OR original_drain_removal118(drain.id,p_instance) IS DISTINCT FROM removal.id
  OR original_drain_command_key118(drain.id,p_instance) IS DISTINCT FROM inbox.command_key
  OR inbox.aggregate_id IS DISTINCT FROM p_instance OR inbox.actor_id IS DISTINCT FROM removal.created_by
  OR inbox.request_id IS DISTINCT FROM removal.request_id OR inbox.input_hash IS DISTINCT FROM removal.input_hash
  OR removal.input_hash IS DISTINCT FROM sha256(convert_to(recipe_import107_json_bytes(jsonb_build_object('version',1,'operation','request_instance_removal',
    'actor',drain.actor_id,'instance',p_instance,'removal',removal.id,'expected_version',removal.expected_version),0),'UTF8'))
  OR prepared_instance_removal118(p_instance) IS DISTINCT FROM removal.id
  OR NOT EXISTS(SELECT 1 FROM recipe_execution_transitions claimed JOIN application_events event ON event.id=claimed.event_id
   JOIN product_event_outbox product ON product.event_id=event.id
   WHERE claimed.attempt_id=drain.id AND claimed.kind='claimed' AND claimed.command_id=drain.command_id
    AND claimed.project_id=drain.project_id AND claimed.resource_name=drain.resource_name
    AND claimed.operation_id=drain.id AND claimed.fingerprint=drain.request_fingerprint AND claimed.before_state=drain.before_state
    AND claimed.after_state->>'active_attempt'=drain.id::text AND claimed.after_state->>'generation'=drain.generation::text
    AND event.actor_id=drain.actor_id AND event.request_id=drain.request_id
    AND event.cursor=claimed.event_cursor AND event.aggregate_version=claimed.event_aggregate_version) THEN
  RAISE EXCEPTION 'immutable original Drain and closure correlation required' USING ERRCODE='P1180'; END IF;
 -- A fresh B continuation may preserve A's closure but cannot replace its pins.
 FOR admission IN SELECT * FROM recipe_remove_resume_admissions WHERE deployment_id=child.deployment_id LOOP
  IF (SELECT count(*) FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=child.resource_name)>1 THEN
   RAISE EXCEPTION 'ambiguous admitted original closure' USING ERRCODE='P1180'; END IF;
  SELECT item INTO recorded FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=child.resource_name;
  IF recorded IS NOT NULL AND recorded IS DISTINCT FROM jsonb_build_object('resource',child.resource_name,'drain_claim',drain.claim_json,
    'actor',command.actor_id,'idempotency',command.idempotency_id,'removal',to_jsonb(removal),'inbox',to_jsonb(inbox)) THEN
   RAISE EXCEPTION 'admitted original closure changed' USING ERRCODE='P1180'; END IF;
 END LOOP;
 RETURN original;
END $$;
REVOKE ALL ON FUNCTION queued_removal_origin140(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION queued_ordinary_original140(p_run uuid,p_cancel jsonb) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE control control_requests%ROWTYPE; cancel jsonb; command jsonb;
 controls jsonb:='[]'::jsonb; cancels jsonb:='[]'::jsonb; supplied boolean:=p_cancel IS NULL;
 p_instance uuid; removal uuid; drain jsonb; drain_cancel jsonb; original jsonb; removal_origin jsonb;
BEGIN
 SELECT instance_id INTO p_instance FROM runs WHERE id=p_run;
 IF EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=p_instance) THEN
  -- Observed removal selects the mixed branch, never a supplied origin flag.
  -- The134 closure requires unique genuine recipe Drain/removal/event/outbox
  -- correlation and exact absent-or-processed owning inbox. Pending outbox does
  -- not assert Cancel processing, a VM receipt or cleanup completion.
  SELECT id INTO removal FROM agent_instance_removal_requests WHERE instance_id=p_instance;
  drain_cancel:=jsonb_build_object('command_id',run_cleanup_wake_cancel118(removal,p_run),
   'run_id',p_run,'reason','instance removal requested');
  removal_origin:=queued_removal_origin140(p_instance);
  -- Observe the exact derived command globally: wrong aggregate/body/transport
  -- must enter134 positive denial, never disappear into completed replay.
  IF EXISTS(SELECT 1 FROM outbox WHERE payload->>'command_id'=drain_cancel->>'command_id') THEN
   drain:=queued_cancel_original140(p_run,drain_cancel);
   IF drain IS DISTINCT FROM removal_origin THEN
    RAISE EXCEPTION 'Run and instance removal origins differ' USING ERRCODE='P1400'; END IF;
   supplied:=supplied OR p_cancel=drain_cancel;
   IF EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(drain_cancel->>'command_id')::uuid) THEN
    PERFORM queued_processed_closure140(p_run,p_instance,drain);
   END IF;
  ELSE
   --107 excludes already-CleanedUp Runs from removal Cancel emission. The
   -- qualifier below requires positive completed ordinary replay; absence is
   -- only checked inventory, never ordering, processing or cleanup authority.
   IF NOT EXISTS(SELECT 1 FROM runs WHERE id=p_run AND state='cleaned_up') THEN
    RAISE EXCEPTION 'missing removal Cancel for unfinished Run is held' USING ERRCODE='P1400'; END IF;
   drain_cancel:=NULL;
  END IF;
 END IF;
 -- Pending/denied Control requests are not admission. Every completed ordinary
 -- origin, every observed Review event and every Run Cancel outbox is checked.
 FOR control IN SELECT * FROM control_requests WHERE run_id=p_run AND kind='cancel_run'
   AND state='completed' ORDER BY id LOOP
  IF control.proposal_id IS NOT NULL OR control.processed_at IS NULL
   OR control.diagnostics IS DISTINCT FROM '[]'::jsonb THEN
   RAISE EXCEPTION 'exact completed ordinary Control required' USING ERRCODE='P1400'; END IF;
  cancel:=jsonb_build_object('command_id',control.id,'run_id',p_run,'reason',control.reason);
  supplied:=supplied OR p_cancel=cancel;
  command:=jsonb_build_object('command_id',control.id,'kind','cancel_run','actor_id',control.actor_id,
   'request_id',control.request_id,'repository_id',control.repository_id,'run_id',p_run,'proposal_id',NULL,'reason',control.reason);
  IF (SELECT count(*) FROM outbox WHERE aggregate_type='control_request' AND aggregate_id=control.id
     AND subject='hephaestus.control.execute' AND event_type='control.requested' AND payload=command)<>1
   OR (SELECT count(*) FROM outbox WHERE aggregate_id=control.id)<>1
   OR (SELECT count(*) FROM outbox WHERE aggregate_type='run' AND aggregate_id=p_run
     AND subject='heph.run.command.cancel.v1' AND event_type='run.cancel_requested' AND payload=cancel)<>1
   OR (SELECT count(*) FROM authorization_audit_events WHERE actor_id=control.actor_id
     AND request_id=control.request_id AND permission='can_cancel' AND object_type='run' AND object_id=p_run
     AND decision='allow' AND authorization_model_version=
      'melange-0.8.5:ac00dabe86b9c473af56ea615b011445fa0597e3c1ea9802d83f0532fa82c148')<>1 THEN
   RAISE EXCEPTION 'genuine authorized Control delivery required' USING ERRCODE='P1400'; END IF;
  controls:=controls||jsonb_build_array(to_jsonb(control));
  cancels:=cancels||jsonb_build_array(cancel);
 END LOOP;
 IF jsonb_array_length(controls)=0 OR supplied IS DISTINCT FROM true THEN
  RAISE EXCEPTION 'exact admitted ordinary typed Cancel required' USING ERRCODE='P1400'; END IF;
 -- Full multiset comparison permits distinct controls with the same actor/reason,
 -- while denying extra, missing or contradictory Review events. The production
 -- event has no command-ID field; no synthetic field or invented ordering is used.
 IF (SELECT count(*) FROM run_events WHERE run_id=p_run AND event_type='review.cancel_requested')<>jsonb_array_length(controls)
  OR EXISTS(SELECT 1 FROM (
   SELECT jsonb_build_object('actor_id',item->'actor_id','reason',item->'reason') payload,count(*) quantity
    FROM jsonb_array_elements(controls) item GROUP BY 1) expected
   WHERE expected.quantity<>(SELECT count(*) FROM run_events event WHERE event.run_id=p_run
     AND event.event_type='review.cancel_requested' AND event.payload=expected.payload)) THEN
  RAISE EXCEPTION 'all observed Review events must have actual admitted origins' USING ERRCODE='P1400'; END IF;
 original:=jsonb_build_object('origin','ordinary_cancel140','controls',controls,'cancels',cancels);
 IF removal_origin IS NOT NULL THEN
  original:=original||jsonb_build_object('removal',removal_origin);
 END IF;
 IF drain IS NOT NULL THEN
  -- Keep every real Review Control and the independently qualified Drain DTO.
  -- No target, receipt or prior ordinary origin is reset on later removal.
  original:=original||jsonb_build_object('drain',drain,'drain_cancel',drain_cancel,
   'cancels',cancels||jsonb_build_array(drain_cancel));
 END IF;
 RETURN original;
END $$;
REVOKE ALL ON FUNCTION queued_ordinary_original140(uuid,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION queued_ordinary_zero140(execution runs) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 -- The exact per-Run zero-history checks are shared in meaning with133. The
 -- instance-wide grant/other-Run closure conditions do not apply to user Cancel.
 RETURN num_nonnulls(execution.volume_id,execution.lease_id,execution.lease_fencing_token)=0
  AND execution.volume_selection_snapshot IS NULL
  -- Global for this Run: no host/state/expiry/release filtering or old receipt adoption.
  AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_vm_plan_admissions WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_legacy_vm_placements WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_requests WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM mailbox_delivery_attempts WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM agent_updates WHERE hook_run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_instance_provenance WHERE run_id=execution.id)
  -- Canonical119 authority IDs equal this Run UUID. Foreign owners may still
  -- collide with those primary/FK IDs; all statuses and linked history count.
  AND NOT EXISTS(SELECT 1 FROM run_authorization_snapshots WHERE run_id=execution.id OR id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_authorization_snapshot_bindings binding JOIN run_authorization_snapshots snapshot
    ON snapshot.id=binding.snapshot_id WHERE snapshot.run_id=execution.id OR binding.snapshot_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM runtime_authority_sessions WHERE run_id=execution.id OR id=execution.id OR snapshot_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM capability_audit_events WHERE run_id=execution.id
    OR runtime_session_id=execution.id OR snapshot_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_git_authority_snapshots git JOIN run_authorization_snapshots snapshot
    ON snapshot.id=git.snapshot_id WHERE snapshot.run_id=execution.id OR git.snapshot_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM runtime_git_credentials credential
    LEFT JOIN runtime_authority_sessions session ON session.id=credential.runtime_session_id
    LEFT JOIN run_authorization_snapshots snapshot ON snapshot.id=credential.authorization_snapshot_id
    WHERE session.run_id=execution.id OR snapshot.run_id=execution.id
      OR session.snapshot_id=execution.id OR credential.runtime_session_id=execution.id
      OR credential.authorization_snapshot_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM git_receives receive JOIN runtime_authority_sessions session
    ON session.id=receive.runtime_session_id WHERE session.run_id=execution.id
      OR receive.runtime_session_id=execution.id OR session.snapshot_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM secret_runtime_sessions WHERE run_id=execution.id OR id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM secret_runtime_mounts WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_secret_provenance WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM secret_leases WHERE run_id=execution.id OR session_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM secret_audit_events audit LEFT JOIN secret_leases lease ON lease.id=audit.lease_id
    WHERE audit.runtime_run_id=execution.id OR lease.run_id=execution.id OR lease.session_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM brokered_secret_lease_snapshots WHERE run_id=execution.id OR runtime_session_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM brokered_secret_audit_events WHERE run_id=execution.id OR runtime_session_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_workspaces WHERE run_id=execution.id)
  AND NOT EXISTS(SELECT 1 FROM run_results WHERE run_id=execution.id);
END $$;
REVOKE ALL ON FUNCTION queued_ordinary_zero140(runs) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION queued_drain_zero140(execution runs) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 RETURN queued_ordinary_zero140(execution)  AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_grants grant_row
    WHERE grant_row.instance_id=execution.instance_id AND NOT EXISTS(
      SELECT 1 FROM agent_instance_volume_mount_revocations WHERE grant_id=grant_row.id));
END $$;
REVOKE ALL ON FUNCTION queued_drain_zero140(runs) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION queued_ordinary_events140(execution runs,p_original jsonb,p_prepared boolean) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE phases integer; expected jsonb; total bigint;
BEGIN
 phases:=CASE execution.state WHEN 'queued' THEN 0 WHEN 'cancelled' THEN 1
   WHEN 'cleaning_up' THEN 2 WHEN 'cleaned_up' THEN 3 ELSE -1 END;
 IF phases<0 OR execution.state_version IS DISTINCT FROM phases::bigint THEN RETURN false; END IF;
 SELECT count(*) INTO total FROM run_events WHERE run_id=execution.id;
 IF NOT EXISTS(SELECT 1 FROM run_events WHERE run_id=execution.id AND sequence=1
    AND event_type='review.cancel_requested')
  OR EXISTS(SELECT 1 FROM run_events WHERE run_id=execution.id AND sequence NOT BETWEEN 1 AND total) THEN RETURN false; END IF;
 IF NOT p_prepared THEN
  RETURN phases=0 AND total=jsonb_array_length(p_original->'controls');
 END IF;
 expected:=jsonb_build_array(jsonb_build_object('type','run.cleanup_started','payload',
  jsonb_build_object('run_id',execution.id,'generation',1,'lease_count',0)));
 IF phases>=1 THEN expected:=expected||jsonb_build_array(jsonb_build_object('type','run.cancelled','payload',
  jsonb_build_object('run_id',execution.id,'state','cancelled','outcome','cancelled','exit',NULL,'failure',NULL))); END IF;
 IF phases>=2 THEN expected:=expected||jsonb_build_array(jsonb_build_object('type','run.cleaning_up','payload',
  jsonb_build_object('run_id',execution.id,'state','cleaning_up','outcome',NULL,'exit',NULL,'failure',NULL))); END IF;
 IF phases>=3 THEN expected:=expected||jsonb_build_array(jsonb_build_object('type','run.cleaned_up','payload',
  jsonb_build_object('run_id',execution.id,'state','cleaned_up','cleanup_generation',1))); END IF;
 -- Every Review row is independently checked against all actual origins above.
 -- Compare the complete ordered cleanup subsequence; legitimate later user
 -- controls may interleave without resetting the immutable target or Run phase.
 RETURN total=jsonb_array_length(p_original->'controls')+jsonb_array_length(expected)
  AND (SELECT jsonb_agg(jsonb_build_object('type',event_type,'payload',payload) ORDER BY sequence)
    FROM run_events WHERE run_id=execution.id AND event_type<>'review.cancel_requested')=expected;
END $$;
REVOKE ALL ON FUNCTION queued_ordinary_events140(runs,jsonb,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION queued_drain_events140(execution runs,p_prepared boolean) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE phases integer; expected jsonb;
BEGIN
 phases:=CASE execution.state WHEN 'queued' THEN 0 WHEN 'cancelled' THEN 1
   WHEN 'cleaning_up' THEN 2 WHEN 'cleaned_up' THEN 3 ELSE -1 END;
 IF phases<0 OR execution.state_version IS DISTINCT FROM phases::bigint THEN RETURN false; END IF;
 IF NOT p_prepared THEN
  RETURN phases=0 AND NOT EXISTS(SELECT 1 FROM run_events WHERE run_id=execution.id);
 END IF;
 expected:=jsonb_build_array(jsonb_build_object('type','run.cleanup_started','payload',
    jsonb_build_object('run_id',execution.id,'generation',1,'lease_count',0)));
 IF phases>=1 THEN expected:=expected||jsonb_build_array(jsonb_build_object('type','run.cancelled','payload',
    jsonb_build_object('run_id',execution.id,'state','cancelled','outcome','cancelled','exit',NULL,'failure',NULL))); END IF;
 IF phases>=2 THEN expected:=expected||jsonb_build_array(jsonb_build_object('type','run.cleaning_up','payload',
    jsonb_build_object('run_id',execution.id,'state','cleaning_up','outcome',NULL,'exit',NULL,'failure',NULL))); END IF;
 IF phases>=3 THEN expected:=expected||jsonb_build_array(jsonb_build_object('type','run.cleaned_up','payload',
    jsonb_build_object('run_id',execution.id,'state','cleaned_up','cleanup_generation',1))); END IF;
 RETURN (SELECT count(*) FROM run_events WHERE run_id=execution.id)=jsonb_array_length(expected)
  AND NOT EXISTS(SELECT 1 FROM run_events event WHERE event.run_id=execution.id
    AND (event.sequence NOT BETWEEN 1 AND jsonb_array_length(expected)
      OR event.event_type IS DISTINCT FROM (expected->(CASE WHEN event.sequence BETWEEN 1 AND 4 THEN event.sequence::integer-1 ELSE -1 END))->>'type'
      OR event.payload IS DISTINCT FROM (expected->(CASE WHEN event.sequence BETWEEN 1 AND 4 THEN event.sequence::integer-1 ELSE -1 END))->'payload'));
END $$;
REVOKE ALL ON FUNCTION queued_drain_events140(runs,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION queued_ordinary_qualify140(p_run uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea,p_mode text) RETURNS runs
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE;
 target run_cleanup_targets%ROWTYPE; observed run_cleanup_receipts%ROWTYPE; done run_cleanup_completions%ROWTYPE;
 processed integer; item jsonb;
BEGIN
 IF p_mode IS NULL OR p_mode NOT IN ('delivery','preparing','prepared','completing')
  OR p_original IS DISTINCT FROM queued_ordinary_original140(p_run,NULL) THEN
  RAISE EXCEPTION 'exact ordinary cancellation context and phase required' USING ERRCODE='P1400'; END IF;
 SELECT * INTO execution FROM runs WHERE id=p_run;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 IF execution.id IS NULL OR receipt.id IS NULL OR execution.instance_id IS DISTINCT FROM receipt.instance_id
  OR execution.state NOT IN ('queued','cancelled','cleaning_up','cleaned_up')
  OR execution.execution_profile IS DISTINCT FROM 'owned_canonical' OR execution.run_kind IS DISTINCT FROM 'invocation'
  OR execution.vm_binding_kind IS DISTINCT FROM 'planned' OR execution.vm_plan_admission_required IS DISTINCT FROM false
  OR ROW(execution.vm_provider_namespace,execution.vm_host_id) IS DISTINCT FROM ROW(p_namespace,p_host)
  OR execution.vm_id IS DISTINCT FROM execution.id::text
  OR NOT run_cleanup_identifier_is_valid(p_namespace) OR NOT run_cleanup_identifier_is_valid(p_host)
  OR p_config IS NULL OR receipt.config_bytes IS DISTINCT FROM p_config
  OR receipt.config_hash IS DISTINCT FROM sha256(receipt.config_bytes)
  OR receipt.config_json IS DISTINCT FROM convert_from(receipt.config_bytes,'UTF8')::jsonb
  OR receipt.config_json->>'version' IS DISTINCT FROM '1' OR receipt.config_json->>'profile' IS DISTINCT FROM 'runtime_named_v1'
  OR receipt.config_json->'vm' IS DISTINCT FROM jsonb_build_object('namespace',p_namespace,'host_id',p_host)
  OR receipt.config_json->'volume'->>'host_id' IS DISTINCT FROM p_host
  OR qualified_invocation_graph_matches(receipt) IS DISTINCT FROM true
  OR queued_ordinary_zero140(execution) IS DISTINCT FROM true
  OR (p_original ? 'removal' AND queued_drain_zero140(execution) IS DISTINCT FROM true)
  OR num_nonnulls(execution.exit_code,execution.exit_signal,execution.failure)<>0
  OR (execution.state='queued' AND execution.outcome IS NOT NULL)
  OR (execution.state<>'queued' AND execution.outcome IS DISTINCT FROM 'cancelled')
  OR NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=receipt.start_command_id
    AND command_type='start_run' AND payload=qualified_invocation_start_json(receipt) AND processed_at IS NULL)
  OR (SELECT count(*) FROM outbox WHERE aggregate_id=p_run OR payload->>'run_id'=p_run::text)
    <>1+jsonb_array_length(p_original->'cancels')+jsonb_array_length(p_original->'controls') THEN
  RAISE EXCEPTION 'genuine115 configured per-Run zero-history cancellation required' USING ERRCODE='P1400'; END IF;
 processed:=0;
 FOR item IN SELECT value FROM jsonb_array_elements(p_original->'cancels') LOOP
  IF EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(item->>'command_id')::uuid) THEN
   IF NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(item->>'command_id')::uuid
     AND command_type='cancel_run' AND payload=item AND processed_at IS NOT NULL) THEN
    RAISE EXCEPTION 'pending or conflicting ordinary Cancel inbox is held' USING ERRCODE='P1400'; END IF;
   processed:=processed+1;
  END IF;
 END LOOP;
 IF (p_mode<>'delivery' AND processed=0)
  OR (processed>0 AND execution.cancel_requested_at IS NULL)
  OR (processed=0 AND execution.cancel_requested_at IS NOT NULL)
  OR (SELECT count(*) FROM command_inbox WHERE payload->>'run_id'=p_run::text)<>processed+1
  OR EXISTS(SELECT 1 FROM command_inbox inbox JOIN control_requests control ON control.id=inbox.command_id
    WHERE control.run_id=p_run AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(p_original->'cancels') admitted
      WHERE admitted->>'command_id'=inbox.command_id::text)) THEN
  RAISE EXCEPTION 'exact owning-processed cancellation history required' USING ERRCODE='P1400'; END IF;
 SELECT * INTO target FROM run_cleanup_targets WHERE run_id=p_run;
 IF target.run_id IS NULL AND p_mode IN ('delivery','preparing') THEN
  IF execution.state IS DISTINCT FROM 'queued' OR execution.volume_acquisition_closed_at IS NOT NULL
   OR queued_ordinary_events140(execution,p_original,false) IS DISTINCT FROM true
   OR EXISTS(SELECT 1 FROM run_cleanup_target_leases WHERE run_id=p_run)
   OR EXISTS(SELECT 1 FROM run_cleanup_receipts WHERE run_id=p_run)
   OR EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=p_run) THEN
   RAISE EXCEPTION 'ordinary queued cancellation candidate has unexpected history' USING ERRCODE='P1400'; END IF;
  RETURN execution;
 END IF;
 SELECT * INTO observed FROM run_cleanup_receipts WHERE run_id=p_run AND generation=1;
 SELECT * INTO done FROM run_cleanup_completions WHERE run_id=p_run AND generation=1;
 IF processed=0 THEN
  RAISE EXCEPTION 'exact owning-processed cancellation history required' USING ERRCODE='P1400'; END IF;
 PERFORM queued_cleanup_target140(execution,p_mode='completing',queued_ordinary_events140(execution,p_original,true),true);
 IF p_original ? 'removal' AND NOT (p_original ? 'drain') THEN
  -- Positive already-completed ordinary replay, independently of removal order.
  -- Every original target/receipt/completion/outcome/event pin above is required
  -- before this branch can return. No new target, observation or processing is
  -- admitted; native/comparison timestamps are not ordering authority.
  IF execution.state IS DISTINCT FROM 'cleaned_up' OR observed.run_id IS NULL OR done.run_id IS NULL
   OR EXISTS(SELECT 1 FROM command_inbox WHERE command_id=run_cleanup_wake_cancel118(
     (p_original->'removal'->>'removal_id')::uuid,p_run)) THEN
   RAISE EXCEPTION 'positive completed ordinary replay without per-Run Drain lineage required' USING ERRCODE='P1400'; END IF;
 END IF;
 RETURN execution;
END $$;
REVOKE ALL ON FUNCTION queued_ordinary_qualify140(uuid,jsonb,text,text,bytea,text) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION queued_cleanup_target140(execution runs,p_completing boolean,p_events boolean,p_ordinary boolean) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE p_run uuid:=execution.id; target run_cleanup_targets%ROWTYPE; receipt run_cleanup_receipts%ROWTYPE;
 done run_cleanup_completions%ROWTYPE;
BEGIN
 SELECT * INTO target FROM run_cleanup_targets WHERE run_id=p_run;
 SELECT * INTO receipt FROM run_cleanup_receipts WHERE run_id=p_run AND generation=1;
 SELECT * INTO done FROM run_cleanup_completions WHERE run_id=p_run AND generation=1;
 IF p_ordinary IS NULL OR p_completing IS NULL OR execution.volume_acquisition_closed_at IS NULL OR target.run_id IS NULL
  OR target.generation IS DISTINCT FROM 1::bigint OR target.lease_count IS DISTINCT FROM 0
  OR target.target_digest IS DISTINCT FROM run_cleanup_target_digest(p_run,target.generation)
  OR ROW(target.instance_id,target.instance_revision_id,target.vm_target_kind,target.vm_provider_namespace,target.vm_host_id,target.vm_id)
    IS DISTINCT FROM ROW(execution.instance_id,execution.instance_revision_id,execution.vm_binding_kind,
      execution.vm_provider_namespace,execution.vm_host_id,execution.vm_id)
  OR EXISTS(SELECT 1 FROM run_cleanup_target_leases WHERE run_id=p_run)
  OR p_events IS DISTINCT FROM true
  OR (SELECT count(*) FROM run_cleanup_receipts WHERE run_id=p_run)<>(CASE WHEN receipt.run_id IS NULL THEN 0 ELSE 1 END)
  OR (SELECT count(*) FROM run_cleanup_completions WHERE run_id=p_run)<>(CASE WHEN done.run_id IS NULL THEN 0 ELSE 1 END)
  OR (receipt.run_id IS NOT NULL AND (receipt.target_digest IS DISTINCT FROM target.target_digest
    OR ROW(receipt.vm_provider_namespace,receipt.vm_host_id,receipt.vm_id)
      IS DISTINCT FROM ROW(target.vm_provider_namespace,target.vm_host_id,target.vm_id)
    OR receipt.observation_kind NOT IN ('destroyed','authoritatively_absent')))
  OR (execution.state='queued' AND receipt.run_id IS NOT NULL)
  OR (execution.state IN ('cleaning_up','cleaned_up') AND receipt.run_id IS NULL)
  OR (p_completing AND execution.state IS DISTINCT FROM 'cleaning_up') THEN
  RAISE EXCEPTION '%',CASE WHEN p_ordinary THEN 'exact ordinary prepared zero-target cleanup proof required' ELSE 'exact closed queued zero-target cleanup proof required' END USING ERRCODE=CASE WHEN p_ordinary THEN 'P1400' ELSE 'P1180' END; END IF;
 IF execution.state='cleaned_up' OR p_completing THEN
  IF receipt.run_id IS NULL OR done.run_id IS NULL OR done.target_digest IS DISTINCT FROM target.target_digest
    OR run_cleanup_set_matches(p_run,target.generation,true) IS DISTINCT FROM true THEN
   RAISE EXCEPTION '%',CASE WHEN p_ordinary THEN 'original scoped receipt and complete zero-set required' ELSE 'original scoped receipt zero-set completion required' END USING ERRCODE=CASE WHEN p_ordinary THEN 'P1400' ELSE 'P1180' END; END IF;
 ELSE
  IF done.run_id IS NOT NULL OR run_cleanup_set_matches(p_run,target.generation,false) IS DISTINCT FROM true THEN
   RAISE EXCEPTION '%',CASE WHEN p_ordinary THEN 'unfinished ordinary zero-target cleanup proof required' ELSE 'unfinished closed zero-target cleanup proof required' END USING ERRCODE=CASE WHEN p_ordinary THEN 'P1400' ELSE 'P1180' END; END IF;
 END IF;
 RETURN;
END $$;
REVOKE ALL ON FUNCTION queued_cleanup_target140(runs,boolean,boolean,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION queued_resume_associations140(p_instance uuid) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 RETURN NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease LEFT JOIN runs execution ON execution.id=lease.run_id
   WHERE (lease.instance_id=p_instance OR execution.instance_id=p_instance)
    AND (execution.id IS NULL OR execution.instance_id IS DISTINCT FROM p_instance OR lease.instance_id IS DISTINCT FROM p_instance))
  AND NOT EXISTS(SELECT 1 FROM agent_instance_execution_invocations receipt LEFT JOIN runs execution ON execution.id=receipt.run_id
   WHERE (receipt.instance_id=p_instance OR execution.instance_id=p_instance)
    AND (execution.id IS NULL OR execution.instance_id IS DISTINCT FROM p_instance OR receipt.instance_id IS DISTINCT FROM p_instance))
  AND NOT EXISTS(SELECT 1 FROM run_invocation_consumptions consumed
   LEFT JOIN agent_instance_execution_invocations receipt ON receipt.id=consumed.invocation_id
   LEFT JOIN runs execution ON execution.id=consumed.run_id
   WHERE (receipt.instance_id=p_instance OR execution.instance_id=p_instance)
    AND (execution.id IS NULL OR receipt.id IS NULL OR receipt.run_id IS DISTINCT FROM consumed.run_id
      OR execution.instance_id IS DISTINCT FROM p_instance OR receipt.instance_id IS DISTINCT FROM p_instance))
  AND NOT EXISTS(SELECT 1 FROM run_cleanup_targets target LEFT JOIN runs execution ON execution.id=target.run_id
   WHERE (target.instance_id=p_instance OR execution.instance_id=p_instance)
    AND (execution.id IS NULL OR execution.instance_id IS DISTINCT FROM p_instance OR target.instance_id IS DISTINCT FROM p_instance))
  AND NOT EXISTS(SELECT 1 FROM run_cleanup_target_leases frozen
   LEFT JOIN runs execution ON execution.id=frozen.run_id
   LEFT JOIN agent_instance_volume_leases lease ON lease.id=frozen.lease_id
   WHERE (execution.instance_id=p_instance OR lease.instance_id=p_instance)
    AND (execution.id IS NULL OR lease.id IS NULL OR execution.instance_id IS DISTINCT FROM p_instance
      OR lease.instance_id IS DISTINCT FROM p_instance OR lease.run_id IS DISTINCT FROM execution.id));
END $$;
REVOKE ALL ON FUNCTION queued_resume_associations140(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE FUNCTION queued_invocation_resume140(p_run uuid,p_namespace text,p_host text,p_platform jsonb) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE;
 header recipe_deployment_execution_preparations%ROWTYPE; context jsonb; cancel jsonb;
 qualified runs%ROWTYPE; processed boolean; has_target boolean;
BEGIN
 SELECT * INTO execution FROM runs WHERE id=p_run;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=receipt.deployment_id;
 -- The unchanged132 branch alone qualifies actual117 consumption. This branch
 -- requires positive115 originals and their entire zero-resource history.
 IF execution.id IS NULL OR receipt.id IS NULL OR header.deployment_id IS NULL
  OR EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=p_run) THEN RETURN false; END IF;
 IF queued_resume_associations140(execution.instance_id) IS DISTINCT FROM true
  OR qualified_invocation_graph_matches(receipt) IS DISTINCT FROM true
  OR queued_ordinary_zero140(execution) IS DISTINCT FROM true
  OR execution.state NOT IN ('queued','cancelled','cleaning_up','cleaned_up')
  OR execution.vm_id IS DISTINCT FROM p_run::text
  OR ROW(execution.vm_provider_namespace,execution.vm_host_id) IS DISTINCT FROM ROW(p_namespace,p_host)
  OR NOT run_cleanup_identifier_is_valid(p_namespace) OR NOT run_cleanup_identifier_is_valid(p_host)
  OR receipt.config_hash IS DISTINCT FROM sha256(receipt.config_bytes)
  OR receipt.config_json IS DISTINCT FROM convert_from(receipt.config_bytes,'UTF8')::jsonb
  OR receipt.config_json->>'version' IS DISTINCT FROM '1' OR receipt.config_json->>'profile' IS DISTINCT FROM 'runtime_named_v1'
  OR receipt.config_json->'vm' IS DISTINCT FROM jsonb_build_object('namespace',p_namespace,'host_id',p_host)
  OR receipt.config_json->'volume'->>'host_id' IS DISTINCT FROM p_host
  OR receipt.config_json->'platform' IS DISTINCT FROM p_platform OR header.platform_json IS DISTINCT FROM p_platform
  OR num_nonnulls(execution.exit_code,execution.exit_signal,execution.failure)<>0
  OR NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=receipt.start_command_id
    AND command_type='start_run' AND payload=qualified_invocation_start_json(receipt) AND processed_at IS NULL)
  OR (SELECT count(*) FROM outbox WHERE payload->>'command_id'=receipt.start_command_id::text)<>1 THEN RETURN false; END IF;
 PERFORM queued_create_original140(execution.instance_id);
 has_target:=EXISTS(SELECT 1 FROM run_cleanup_targets WHERE run_id=p_run);
 IF EXISTS(SELECT 1 FROM control_requests WHERE run_id=p_run AND kind='cancel_run') THEN
  context:=queued_ordinary_original140(p_run,NULL);
  qualified:=queued_ordinary_qualify140(p_run,context,p_namespace,p_host,receipt.config_bytes,
    CASE WHEN has_target THEN 'prepared' ELSE 'delivery' END);
  -- The complete original qualifier checks every associated envelope; add the
  -- global same-command count, including a malformed row under a foreign owner.
  IF EXISTS(SELECT 1 FROM jsonb_array_elements(context->'cancels') item
    WHERE (SELECT count(*) FROM outbox WHERE payload->>'command_id'=item->>'command_id')
      <>(CASE WHEN EXISTS(SELECT 1 FROM jsonb_array_elements(context->'controls') control
        WHERE control->>'id'=item->>'command_id') THEN 2 ELSE 1 END)) THEN RETURN false; END IF;
  RETURN qualified.id IS NOT DISTINCT FROM p_run;
 END IF;
 IF NOT EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=execution.instance_id) THEN
  -- Unclaimed genuine queued work may obtain its FIRST owning Drain later.
  -- This is comparison-only admission, never processing, preparation or IO.
  RETURN execution.state='queued' AND execution.state_version=0 AND execution.outcome IS NULL
   AND execution.cancel_requested_at IS NULL AND execution.volume_acquisition_closed_at IS NULL
   AND NOT has_target AND queued_drain_events140(execution,false)
   AND (SELECT count(*) FROM outbox WHERE aggregate_id=p_run OR payload->>'run_id'=p_run::text)=1
   AND (SELECT count(*) FROM command_inbox WHERE payload->>'run_id'=p_run::text)=1
   AND NOT EXISTS(SELECT 1 FROM run_cleanup_target_leases WHERE run_id=p_run)
   AND NOT EXISTS(SELECT 1 FROM run_cleanup_receipts WHERE run_id=p_run)
   AND NOT EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=p_run);
 END IF;
 context:=queued_removal_origin140(execution.instance_id);
 cancel:=jsonb_build_object('command_id',run_cleanup_wake_cancel118((context->>'removal_id')::uuid,p_run),
   'run_id',p_run,'reason','instance removal requested');
 PERFORM queued_cancel_closure140(p_run,execution.instance_id,context,cancel);
 processed:=EXISTS(SELECT 1 FROM command_inbox WHERE command_id=(cancel->>'command_id')::uuid);
 IF queued_drain_zero140(execution) IS DISTINCT FROM true
  OR (SELECT count(*) FROM outbox WHERE payload->>'command_id'=cancel->>'command_id')<>1
  OR (SELECT count(*) FROM outbox WHERE aggregate_id=p_run OR payload->>'run_id'=p_run::text)<>2
  OR (SELECT count(*) FROM command_inbox WHERE payload->>'run_id'=p_run::text)<>(CASE WHEN processed THEN 2 ELSE 1 END)
  OR (execution.state='queued' AND execution.outcome IS NOT NULL)
  OR (execution.state<>'queued' AND execution.outcome IS DISTINCT FROM 'cancelled') THEN RETURN false; END IF;
 IF NOT has_target THEN
  RETURN execution.state='queued' AND execution.volume_acquisition_closed_at IS NULL
   AND queued_drain_events140(execution,false)
   AND NOT EXISTS(SELECT 1 FROM run_cleanup_target_leases WHERE run_id=p_run)
   AND NOT EXISTS(SELECT 1 FROM run_cleanup_receipts WHERE run_id=p_run)
   AND NOT EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=p_run);
 END IF;
 IF NOT processed THEN RETURN false; END IF;
 PERFORM queued_processed_closure140(p_run,execution.instance_id,context);
 PERFORM queued_cleanup_target140(execution,false,queued_drain_events140(execution,true),false);
 RETURN true;
END $$;
REVOKE ALL ON FUNCTION queued_invocation_resume140(uuid,text,text,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE OR REPLACE FUNCTION run_cleanup_wake_original118(p_instance uuid) RETURNS uuid
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 RETURN queued_create_original140(p_instance);
END $$;
REVOKE ALL ON FUNCTION run_cleanup_wake_original118(uuid) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE OR REPLACE FUNCTION run_cleanup_wake_closure118(p_run uuid,p_instance uuid,p_original jsonb) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 PERFORM queued_processed_closure140(p_run,p_instance,p_original);
END $$;
REVOKE ALL ON FUNCTION run_cleanup_wake_closure118(uuid,uuid,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE OR REPLACE FUNCTION invocation119_cancel_closure(p_run uuid,p_instance uuid,p_original jsonb,p_cancel jsonb) RETURNS void
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 PERFORM queued_cancel_closure140(p_run,p_instance,p_original,p_cancel);
END $$;
REVOKE ALL ON FUNCTION invocation119_cancel_closure(uuid,uuid,jsonb,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE OR REPLACE FUNCTION invocation119_cancel_original(p_run uuid,p_cancel jsonb) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 RETURN queued_cancel_original140(p_run,p_cancel);
END $$;
REVOKE ALL ON FUNCTION invocation119_cancel_original(uuid,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE OR REPLACE FUNCTION invocation119_queued_cleanup_events(execution runs,p_prepared boolean) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
BEGIN
 PERFORM require_run_cleanup_wake118();
 RETURN queued_drain_events140(execution,p_prepared);
END $$;
REVOKE ALL ON FUNCTION invocation119_queued_cleanup_events(runs,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE OR REPLACE FUNCTION invocation119_cleanup_only_matches(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea,p_completing boolean) RETURNS runs
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE;
BEGIN
 PERFORM require_run_cleanup_wake118();
 execution:=invocation119_queued_cleanup_base(p_run,p_instance,p_original,p_namespace,p_host,p_config);
 PERFORM queued_cleanup_target140(execution,p_completing,invocation119_queued_cleanup_events(execution,true),false);
 RETURN execution;
END $$;
REVOKE ALL ON FUNCTION invocation119_cleanup_only_matches(uuid,uuid,jsonb,text,text,bytea,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE OR REPLACE FUNCTION invocation119_remove_resume_matches(p_run uuid,p_namespace text,p_host text,p_platform jsonb)
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER
SET search_path=pg_catalog,public,pg_temp AS $$
 SELECT invocation119_immutable(p_run) IS NOT DISTINCT FROM true
  AND EXISTS(SELECT 1 FROM runs execution
   JOIN agent_instance_execution_invocations receipt ON receipt.run_id=execution.id
   JOIN run_invocation_consumptions consumed ON consumed.run_id=execution.id
   JOIN recipe_prepared_instance_imports child ON child.instance_id=execution.instance_id
   JOIN recipe_deployment_execution_preparations header ON header.deployment_id=child.deployment_id
   WHERE execution.id=p_run AND execution.run_kind='invocation'
    AND execution.execution_profile='owned_canonical' AND execution.vm_binding_kind='planned'
    AND execution.vm_plan_admission_required IS DISTINCT FROM true
    AND ROW(execution.vm_provider_namespace,execution.vm_host_id,execution.vm_id)
      IS NOT DISTINCT FROM ROW(p_namespace,p_host,consumed.vm_id)
    AND consumed.vm_id=receipt.run_id::text
    AND consumed.config_bytes IS NOT DISTINCT FROM receipt.config_bytes
    AND consumed.config_hash IS NOT DISTINCT FROM receipt.config_hash
    AND receipt.config_hash=sha256(receipt.config_bytes)
    AND receipt.config_json IS NOT DISTINCT FROM convert_from(receipt.config_bytes,'UTF8')::jsonb
    AND receipt.config_json->>'version'='1'
    AND receipt.config_json->>'profile'='runtime_named_v1'
    AND receipt.config_json->'vm' IS NOT DISTINCT FROM jsonb_build_object('namespace',p_namespace,'host_id',p_host)
    AND receipt.config_json->'platform' IS NOT DISTINCT FROM p_platform
    AND header.platform_json IS NOT DISTINCT FROM p_platform
    -- Exact115 graph and117 consumption already compare every original113
    -- prepared volume, binding, grant, project and complete configuration.
    AND qualified_invocation_graph_matches(receipt) IS NOT DISTINCT FROM true)
 OR queued_invocation_resume140(p_run,p_namespace,p_host,p_platform) IS NOT DISTINCT FROM true
$$;
REVOKE ALL ON FUNCTION invocation119_remove_resume_matches(uuid,text,text,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE OR REPLACE FUNCTION guard_remove_resume_admission118() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE deployment recipe_deployments%ROWTYPE; command recipe_deployment_commands%ROWTYPE;
 header recipe_deployment_execution_preparations%ROWTYPE; resource record; kind text;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'Remove resume admission is immutable' USING ERRCODE='23000'; END IF;
 PERFORM lock_remove_resume118(NEW.deployment_id);
 PERFORM remove_resume_actor118(NEW.deployment_id);
 SELECT * INTO deployment FROM recipe_deployments WHERE id=NEW.deployment_id;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=NEW.command_id;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=NEW.deployment_id;
 IF deployment.id IS NULL OR deployment.lifecycle<>'removing' OR deployment.version IS DISTINCT FROM NEW.expected_version
  OR command.operation IS DISTINCT FROM 'remove' OR command.deployment_id IS DISTINCT FROM NEW.deployment_id OR command.project_id IS DISTINCT FROM NEW.project_id
  OR deployment.project_id IS DISTINCT FROM NEW.project_id OR deployment.input_hash IS DISTINCT FROM NEW.intent_hash
  OR header.input_hash IS DISTINCT FROM NEW.preparation_hash OR header.platform_json IS DISTINCT FROM NEW.platform_json
  OR NEW.actor_id::text IS DISTINCT FROM hephaestus_actor_id() OR NEW.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
  OR command.actor_id IS DISTINCT FROM NEW.actor_id OR command.first_request_id IS DISTINCT FROM NEW.request_id OR command.event_id IS DISTINCT FROM NEW.event_id
  OR command.idempotency_id::text IS DISTINCT FROM current_setting('hephaestus.occurrence_id',true)
  OR command.receipt_version IS DISTINCT FROM NEW.expected_version OR command.receipt_lifecycle<>'removing'
  OR command.input_hash IS DISTINCT FROM sha256(convert_to(format('["hephaestus-recipe-remove-v1","%s",%s]',deployment.id,NEW.expected_version),'UTF8'))
  OR EXISTS(SELECT 1 FROM recipe_prepared_instance_imports associated WHERE associated.deployment_id=NEW.deployment_id
    AND queued_resume_associations140(associated.instance_id) IS DISTINCT FROM true)
  OR EXISTS(SELECT 1 FROM runs execution JOIN recipe_prepared_instance_imports child ON child.instance_id=execution.instance_id
    WHERE child.deployment_id=NEW.deployment_id AND (
      (execution.execution_profile='owned_canonical' AND execution.vm_provider_namespace=NEW.vm_provider_namespace
       AND execution.vm_host_id=NEW.vm_host_id AND EXISTS(SELECT 1 FROM run_vm_plan_admissions plan
         WHERE plan.run_id=execution.id AND plan.consumed_at IS NOT NULL AND plan.vm_id=execution.vm_id
           AND plan.vm_provider_namespace=execution.vm_provider_namespace AND plan.vm_host_id=execution.vm_host_id))
      OR invocation119_remove_resume_matches(execution.id,NEW.vm_provider_namespace,NEW.vm_host_id,NEW.platform_json) IS NOT DISTINCT FROM true
      OR (execution.execution_profile='legacy_scalar' AND EXISTS(SELECT 1 FROM run_legacy_vm_placements plan
         WHERE plan.run_id=execution.id AND plan.consumed_at IS NOT NULL
           AND plan.provider_namespace::text=NEW.vm_provider_namespace AND plan.host_id=NEW.vm_host_id
           AND legacy_vm_placement_matches(execution,plan)))) IS DISTINCT FROM true)
  OR NEW.context_json IS DISTINCT FROM remove_resume_context118(NEW.deployment_id)
  OR NOT EXISTS(SELECT 1 FROM application_events e JOIN product_event_outbox o ON o.event_id=e.id WHERE e.id=NEW.event_id
    AND e.actor_id=NEW.actor_id AND e.request_id=NEW.request_id AND e.scope_kind='project' AND e.scope_id=NEW.project_id
    AND e.occurrence_id=command.idempotency_id AND e.cursor=command.event_cursor AND e.aggregate_version=command.event_aggregate_version)
  OR NOT EXISTS(SELECT 1 FROM authorization_audit_events a WHERE a.actor_id=NEW.actor_id AND a.request_id=NEW.request_id
    AND a.object_type='project' AND a.object_id=NEW.project_id AND a.permission='can_manage' AND a.decision='allow') THEN
  RAISE EXCEPTION 'fresh exact current manager command/CAS/preparation required' USING ERRCODE='23000'; END IF;
 FOR resource IN SELECT * FROM recipe_deployment_resources WHERE deployment_id=NEW.deployment_id AND ownership='owned' LOOP
  kind:=CASE resource.resource_kind WHEN 'volume' THEN 'state_volume' ELSE 'agent_instance' END;
  IF recipe_owned_resource_project(kind,resource.resource_id) IS NOT NULL AND NOT EXISTS(SELECT 1 FROM authorization_audit_events a
   WHERE a.actor_id=NEW.actor_id AND a.request_id=NEW.request_id AND a.object_type=kind AND a.object_id=resource.resource_id
    AND a.permission='can_manage' AND a.decision='allow') THEN RAISE EXCEPTION 'all owned decision audits required' USING ERRCODE='42501'; END IF;
 END LOOP;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_remove_resume_admission118() FROM PUBLIC,hephaestus_app,hephaestus_worker;

CREATE OR REPLACE FUNCTION load_existing_instance_closure118(p_command uuid,p_deployment uuid,p_project uuid,p_intent_hash bytea,
 p_attempt uuid,p_claim jsonb,p_prepared jsonb,p_namespace text,p_host text,p_platform jsonb) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE admission recipe_remove_resume_admissions%ROWTYPE; header recipe_deployment_execution_preparations%ROWTYPE;
 command recipe_deployment_commands%ROWTYPE; consumer uuid; fresh jsonb; selected jsonb; recorded jsonb;
 original recipe_effect_attempts%ROWTYPE; original_command recipe_deployment_commands%ROWTYPE; count integer;
BEGIN
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app' THEN
  RAISE EXCEPTION 'current manager actor pool required' USING ERRCODE='42501'; END IF;
 PERFORM remove_resume_actor118(p_deployment);
 PERFORM prepared_instance_actor118(p_command,p_attempt);
 consumer:=prepared_instance_target118(p_attempt,p_claim,p_prepared);
 SELECT * INTO admission FROM recipe_remove_resume_admissions WHERE command_id=p_command;
 SELECT * INTO header FROM recipe_deployment_execution_preparations WHERE deployment_id=p_deployment;
 SELECT * INTO command FROM recipe_deployment_commands WHERE id=p_command;
 IF admission.command_id IS NULL OR admission.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
  OR NULLIF(current_setting('hephaestus.request_id',true),'') IS NULL
  OR command.operation IS DISTINCT FROM 'remove' OR command.actor_id IS DISTINCT FROM admission.actor_id
  OR command.first_request_id IS DISTINCT FROM admission.request_id OR command.event_id IS DISTINCT FROM admission.event_id
  OR command.idempotency_id::text IS DISTINCT FROM current_setting('hephaestus.occurrence_id',true)
  OR command.receipt_version IS DISTINCT FROM admission.expected_version OR command.receipt_lifecycle IS DISTINCT FROM 'removing'
  OR command.input_hash IS DISTINCT FROM sha256(convert_to(format('["hephaestus-recipe-remove-v1","%s",%s]',p_deployment,admission.expected_version),'UTF8'))
  OR admission.deployment_id IS DISTINCT FROM p_deployment OR admission.project_id IS DISTINCT FROM p_project
  OR admission.intent_hash IS DISTINCT FROM p_intent_hash OR header.intent_hash IS DISTINCT FROM p_intent_hash
  OR header.project_id IS DISTINCT FROM p_project OR header.profile IS DISTINCT FROM 'runtime_named_v1'
  OR admission.preparation_hash IS DISTINCT FROM header.input_hash OR header.platform_json IS DISTINCT FROM p_platform
  OR admission.vm_provider_namespace IS DISTINCT FROM p_namespace OR admission.vm_host_id IS DISTINCT FROM p_host
  OR admission.platform_json IS DISTINCT FROM p_platform
  OR NOT EXISTS(SELECT 1 FROM recipe_prepared_instance_imports child WHERE child.deployment_id=p_deployment
    AND child.instance_id=consumer AND child.resource_name=p_claim->>'resource') THEN
  RAISE EXCEPTION 'exact current continuation and original preparation required' USING ERRCODE='P1180'; END IF;
 -- Current request is fresh provenance; the immutable first admission and original
 -- closure request remain history and are deliberately not substituted for it.
 IF EXISTS(SELECT 1 FROM recipe_prepared_instance_imports associated WHERE associated.deployment_id=p_deployment
    AND queued_resume_associations140(associated.instance_id) IS DISTINCT FROM true)
  OR EXISTS(SELECT 1 FROM runs execution JOIN recipe_prepared_instance_imports child ON child.instance_id=execution.instance_id
    WHERE child.deployment_id=p_deployment AND (
      (execution.execution_profile='owned_canonical' AND execution.vm_provider_namespace=admission.vm_provider_namespace
       AND execution.vm_host_id=admission.vm_host_id AND EXISTS(SELECT 1 FROM run_vm_plan_admissions plan
         WHERE plan.run_id=execution.id AND plan.consumed_at IS NOT NULL AND plan.vm_id=execution.vm_id
           AND plan.vm_provider_namespace=execution.vm_provider_namespace AND plan.vm_host_id=execution.vm_host_id))
      OR invocation119_remove_resume_matches(execution.id,admission.vm_provider_namespace,admission.vm_host_id,admission.platform_json) IS NOT DISTINCT FROM true
      OR (execution.execution_profile='legacy_scalar' AND EXISTS(SELECT 1 FROM run_legacy_vm_placements plan
         WHERE plan.run_id=execution.id AND plan.consumed_at IS NOT NULL
           AND plan.provider_namespace::text=admission.vm_provider_namespace AND plan.host_id=admission.vm_host_id
           AND legacy_vm_placement_matches(execution,plan)))) IS DISTINCT FROM true) THEN
  RAISE EXCEPTION 'unknown or foreign current runtime history held' USING ERRCODE='P1180'; END IF;
 fresh:=remove_resume_context118(p_deployment);
 SELECT count(*) INTO count FROM jsonb_array_elements(fresh->'closures') item WHERE item->>'resource'=p_claim->>'resource';
 IF count<>1 THEN RAISE EXCEPTION 'one exact original closure required' USING ERRCODE='P1180'; END IF;
 SELECT item INTO selected FROM jsonb_array_elements(fresh->'closures') item WHERE item->>'resource'=p_claim->>'resource';
 SELECT count(*) INTO count FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=p_claim->>'resource';
 IF count<>1 THEN RAISE EXCEPTION 'one admitted original closure required' USING ERRCODE='P1180'; END IF;
 SELECT item INTO recorded FROM jsonb_array_elements(admission.context_json->'closures') item WHERE item->>'resource'=p_claim->>'resource';
 IF selected IS DISTINCT FROM recorded THEN RAISE EXCEPTION 'original closure changed after admission' USING ERRCODE='P1180'; END IF;
 SELECT * INTO original FROM recipe_effect_attempts WHERE id=(selected->'drain_claim'->>'attempt_id')::uuid;
 SELECT * INTO original_command FROM recipe_deployment_commands WHERE id=original.command_id;
 IF original.id IS NULL OR original.action IS DISTINCT FROM 'drain' OR original.claim_json IS DISTINCT FROM selected->'drain_claim'
  OR original.deployment_id IS DISTINCT FROM p_deployment OR original.project_id IS DISTINCT FROM p_project
  OR original.resource_name IS DISTINCT FROM p_claim->>'resource' OR original.input_hash IS DISTINCT FROM
    (SELECT input_hash FROM recipe_deployment_resources WHERE deployment_id=p_deployment AND resource_name=p_claim->>'resource')
  OR original.claim_json->'identity' IS DISTINCT FROM p_claim->'identity'
  OR original_command.operation IS DISTINCT FROM 'remove' OR original_command.actor_id IS DISTINCT FROM original.actor_id
  OR original_command.actor_id IS DISTINCT FROM (selected->>'actor')::uuid
  OR original_command.idempotency_id IS DISTINCT FROM (selected->>'idempotency')::uuid
  OR NOT EXISTS(SELECT 1 FROM recipe_execution_transitions claimed JOIN application_events event ON event.id=claimed.event_id
    JOIN product_event_outbox outbox ON outbox.event_id=event.id
    WHERE claimed.attempt_id=original.id AND claimed.kind='claimed' AND claimed.command_id=original.command_id
      AND claimed.project_id=original.project_id AND claimed.resource_name=original.resource_name
      AND claimed.operation_id=original.id AND claimed.fingerprint=original.request_fingerprint AND claimed.before_state=original.before_state
      AND claimed.after_state->>'active_attempt'=original.id::text AND claimed.after_state->>'generation'=original.generation::text
      AND event.actor_id=original.actor_id AND event.request_id=original.request_id
      AND event.cursor=claimed.event_cursor AND event.aggregate_version=claimed.event_aggregate_version) THEN
  RAISE EXCEPTION 'immutable original Drain lineage required' USING ERRCODE='P1180'; END IF;
 -- Return only declared original comparison fields, never saved parameters or full
 -- admission/placement rows. No status/empty history is physical cleanup evidence.
 RETURN jsonb_build_object('drain',selected->'drain_claim','actor',original_command.actor_id,
  'idempotency',original_command.idempotency_id,'removal_id',selected->'removal'->'id',
  'command_key',(SELECT jsonb_agg(get_byte(decode(substr(selected->'inbox'->>'command_key',3),'hex'),n) ORDER BY n) FROM generate_series(0,31) n),
  'expected_version',selected->'removal'->'expected_version','creator',selected->'removal'->'created_by',
  'request',selected->'removal'->'request_id',
  'input_hash',(SELECT jsonb_agg(get_byte(decode(substr(selected->'removal'->>'input_hash',3),'hex'),n) ORDER BY n) FROM generate_series(0,31) n));
END $$;
REVOKE ALL ON FUNCTION load_existing_instance_closure118(uuid,uuid,uuid,bytea,uuid,jsonb,jsonb,text,text,jsonb) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION load_existing_instance_closure118(uuid,uuid,uuid,bytea,uuid,jsonb,jsonb,text,text,jsonb) TO hephaestus_app;
