-- PRIVATE additive119 qualified original queued-cleanup comparison DATA.
-- Load after frozen14e83 queued cleanup and931f Cancel delivery successors.
-- No actor reconstruction, credentials, locks, writes or provider IO authority.
CREATE FUNCTION discover_queued_run_cleanup_context119(p_run uuid,p_namespace text,p_host text,p_config bytea)
RETURNS TABLE(instance_id uuid,original jsonb)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE owner uuid; pins jsonb; execution runs%ROWTYPE;
BEGIN
 PERFORM require_run_cleanup_wake118();
 SELECT run.instance_id INTO owner FROM runs run WHERE run.id=p_run;
 pins:=invocation119_queued_cleanup_original(p_run);
 -- Derivation alone is not proof: qualify current actual full configuration and
 -- genuine115/processed original Cancel/zero history/candidate or prepared work.
 execution:=qualify_invocation119_queued_cleanup(p_run,owner,pins,p_namespace,p_host,p_config,true,false);
 RETURN QUERY SELECT execution.instance_id,pins;
END $$;
REVOKE ALL ON FUNCTION discover_queued_run_cleanup_context119(uuid,text,text,bytea) FROM PUBLIC,hephaestus_app,hephaestus_worker;
GRANT EXECUTE ON FUNCTION discover_queued_run_cleanup_context119(uuid,text,text,bytea) TO hephaestus_worker;
