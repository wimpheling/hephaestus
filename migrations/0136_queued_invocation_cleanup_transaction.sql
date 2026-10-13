-- PRIVATE119 queued cleanup owning transaction boundary successor.
-- Load after frozen14e83 queued cleanup; existing data readers stay unchanged.
-- Role validation precedes this new guard; all original locks/proofs/ACLs remain.
CREATE OR REPLACE FUNCTION lock_invocation119_queued_cleanup(p_run uuid,p_instance uuid,p_original jsonb,p_namespace text,p_host text,p_config bytea,p_preparing boolean,p_completing boolean) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE early record; child recipe_prepared_instance_imports%ROWTYPE; extra bytea;
BEGIN
 PERFORM require_run_cleanup_wake118();
 -- BEGIN queued cleanup transaction boundary.
 IF current_setting('transaction_isolation') IS DISTINCT FROM 'read committed'
  OR current_setting('transaction_read_only') IS DISTINCT FROM 'off' THEN
  RAISE EXCEPTION 'queued cleanup requires writable read-committed transaction' USING ERRCODE='P1180'; END IF;
 -- END queued cleanup transaction boundary.
 FOR early IN SELECT * FROM invocation119_queued_cleanup_reservations(p_run,p_instance,p_original,p_namespace,p_host,p_config,p_preparing,p_completing)
   ORDER BY lock_class,command_mutex,first_mutex,second_mutex LOOP
  IF early.lock_class=0 THEN
   IF pg_try_advisory_xact_lock(early.command_mutex) IS DISTINCT FROM true THEN
    RAISE EXCEPTION 'queued original command reservation held' USING ERRCODE='55P03'; END IF;
  ELSE
   IF pg_try_advisory_xact_lock(early.first_mutex,early.second_mutex) IS DISTINCT FROM true THEN
    RAISE EXCEPTION 'queued original instance reservation held' USING ERRCODE='55P03'; END IF;
  END IF;
 END LOOP;
 SELECT * INTO child FROM recipe_prepared_instance_imports WHERE instance_id=p_instance;
 SELECT command_key INTO extra FROM release_command_inbox WHERE secondary_id=(p_original->>'removal_id')::uuid AND operation='request_instance_removal';
 -- Frozen helpers repeat TRY reservations then parent Project/Deployment/ALL
 -- Resources and Instance/ALL Runs/sorted Volumes/grants/remaining late NOWAIT.
 PERFORM lock_prepared_closure_parents118(p_instance,extra);
 PERFORM lock_prepared_closure_participants118(p_instance,child.project_id);
 PERFORM run_id FROM run_cleanup_targets WHERE run_id=p_run FOR UPDATE NOWAIT;
 PERFORM lease_id FROM run_cleanup_target_leases WHERE run_id=p_run ORDER BY volume_id,lease_id FOR UPDATE NOWAIT;
 PERFORM run_id FROM run_cleanup_receipts WHERE run_id=p_run ORDER BY generation FOR UPDATE NOWAIT;
 PERFORM run_id FROM run_cleanup_completions WHERE run_id=p_run ORDER BY generation FOR UPDATE NOWAIT;
 PERFORM command_id FROM command_inbox WHERE payload->>'run_id'=p_run::text ORDER BY command_id FOR UPDATE NOWAIT;
 PERFORM id FROM outbox WHERE aggregate_id=p_run ORDER BY id FOR UPDATE NOWAIT;
 PERFORM qualify_invocation119_queued_cleanup(p_run,p_instance,p_original,p_namespace,p_host,p_config,p_preparing,p_completing);
 -- Closure and target freezing stay in the existing owning transaction tail.
END $$;
REVOKE ALL ON FUNCTION lock_invocation119_queued_cleanup(uuid,uuid,jsonb,text,text,bytea,boolean,boolean) FROM PUBLIC,hephaestus_app,hephaestus_worker;
