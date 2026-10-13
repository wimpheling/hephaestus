-- PRIVATE119-B2, AFTER frozen reviewed119-A and B1. Read-only empty family only.
-- Saved invoker IDs remain checked data, never AuthenticatedIdentity or bearer.
CREATE FUNCTION verify_invocation119_empty_secrets(p_run uuid,p_config bytea,
 p_instance uuid,p_revision uuid,p_release uuid,p_agent uuid,p_command uuid,
 p_requires_state boolean,p_vm text,p_phase text) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; revision agent_instance_revisions%ROWTYPE; ceiling boolean;
BEGIN
 -- Check the actual caller BEFORE any classification/Run lookup under definer.
 PERFORM require_invocation117_worker();
 SELECT * INTO execution FROM runs WHERE id=p_run;
 SELECT * INTO revision FROM agent_instance_revisions WHERE id=execution.instance_revision_id;
 IF execution.id IS NULL OR revision.id IS NULL
  OR execution.run_kind IS DISTINCT FROM 'invocation'
  OR execution.execution_profile IS DISTINCT FROM 'owned_canonical'
  OR execution.state NOT IN ('provisioning','starting','running')
  OR execution.vm_id IS NULL
  OR num_nonnulls(execution.attachment_id,execution.volume_id,execution.lease_id,execution.lease_fencing_token)<>0
  OR ROW(execution.instance_id,execution.instance_revision_id,execution.release_id,execution.release_agent_id,
         execution.command_id,execution.requires_state,execution.vm_id,execution.state)
    IS DISTINCT FROM ROW(p_instance,p_revision,p_release,p_agent,p_command,p_requires_state,p_vm,p_phase)
  OR revision.instance_id IS DISTINCT FROM execution.instance_id
  OR jsonb_typeof(revision.secret_bindings) IS DISTINCT FROM 'array'
  OR revision.secret_bindings IS DISTINCT FROM '[]'::jsonb THEN
  RETURN false;
 END IF;
 ceiling:=EXISTS(SELECT 1 FROM runtime_authority_sessions WHERE run_id=p_run OR id=p_run)
  OR EXISTS(SELECT 1 FROM run_authorization_snapshots WHERE run_id=p_run OR id=p_run)
  OR EXISTS(SELECT 1 FROM run_authorization_snapshot_bindings WHERE snapshot_id=p_run
   OR snapshot_id IN (SELECT id FROM run_authorization_snapshots WHERE run_id=p_run));
 -- Secrets prepare precedes runtime issuance. Any existing authority evidence
 -- is a ceiling, never an absence that can authorize replacement or fallback.
 IF invocation119_authority_eligible(p_run,p_config,execution.state IN ('starting','running')) IS DISTINCT FROM true
  OR (ceiling AND invocation119_session_graph(p_run,true) IS DISTINCT FROM true) THEN
  RETURN false;
 END IF;
 -- This first family has never carried secrets. All statuses contradict it;
 -- RLS-hidden rows and revoked/destroyed records must never appear as empty.
 RETURN NOT EXISTS(SELECT 1 FROM agent_secret_bindings WHERE instance_revision_id=revision.id)
  AND NOT EXISTS(SELECT 1 FROM run_secret_provenance WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM run_instance_provenance WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM secret_runtime_sessions WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM secret_leases WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM secret_runtime_mounts WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM brokered_secret_rules WHERE instance_revision_id=revision.id)
  AND NOT EXISTS(SELECT 1 FROM brokered_secret_lease_snapshots WHERE run_id=p_run);
END $$;
REVOKE ALL ON FUNCTION verify_invocation119_empty_secrets(uuid,bytea,uuid,uuid,uuid,uuid,uuid,boolean,text,text) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION verify_invocation119_empty_secrets(uuid,bytea,uuid,uuid,uuid,uuid,uuid,boolean,text,text) TO hephaestus_worker;
