-- PRIVATE119 pre-acquisition: AFTER frozen119-A/B1/B2; no existing successor.
-- First-acquisition eligibility only. No ceiling, credential, lease or IO grant.
CREATE FUNCTION verify_invocation119_pre_acquisition(p_run uuid,p_config bytea,
 p_instance uuid,p_revision uuid,p_release uuid,p_agent uuid,p_command uuid,
 p_requires_state boolean,p_vm text,p_state_version bigint,p_model text) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public,pg_temp AS $$
DECLARE execution runs%ROWTYPE; receipt agent_instance_execution_invocations%ROWTYPE;
 revision agent_instance_revisions%ROWTYPE;
BEGIN
 -- Actual caller first, before any classification or protected record lookup.
 PERFORM require_invocation117_worker();
 SELECT * INTO execution FROM runs WHERE id=p_run;
 SELECT * INTO receipt FROM agent_instance_execution_invocations WHERE run_id=p_run;
 SELECT * INTO revision FROM agent_instance_revisions WHERE id=execution.instance_revision_id;
 IF execution.id IS NULL OR receipt.id IS NULL OR revision.id IS NULL
  OR execution.run_kind IS DISTINCT FROM 'invocation'
  OR execution.execution_profile IS DISTINCT FROM 'owned_canonical'
  OR execution.state IS DISTINCT FROM 'leasing_volume'
  OR execution.vm_binding_kind IS DISTINCT FROM 'planned'
  OR execution.vm_plan_admission_required OR execution.legacy_vm_placement_required
  OR execution.vm_id IS DISTINCT FROM p_run::text
  OR execution.vm_provider_namespace IS DISTINCT FROM receipt.config_json->'vm'->>'namespace'
  OR execution.vm_host_id IS DISTINCT FROM receipt.config_json->'vm'->>'host_id'
  OR num_nonnulls(execution.attachment_id,execution.volume_id,execution.lease_id,execution.lease_fencing_token)<>0
  OR ROW(execution.instance_id,execution.instance_revision_id,execution.release_id,execution.release_agent_id,
         execution.command_id,execution.requires_state,execution.vm_id,execution.state_version)
    IS DISTINCT FROM ROW(p_instance,p_revision,p_release,p_agent,p_command,p_requires_state,p_vm,p_state_version)
  OR receipt.config_bytes IS DISTINCT FROM p_config
  OR p_model IS NULL OR octet_length(p_model) NOT BETWEEN 1 AND 128
  OR receipt.authorization_model_version IS DISTINCT FROM p_model
  OR revision.instance_id IS DISTINCT FROM execution.instance_id
  OR revision.secret_bindings IS DISTINCT FROM '[]'::jsonb
  OR execution.volume_selection_snapshot IS NOT NULL
  OR execution.volume_acquisition_closed_at IS NOT NULL
  OR invocation119_immutable(p_run) IS DISTINCT FROM true
  OR invocation119_open_set(p_run) IS DISTINCT FROM true
  OR invocation119_live(p_run) IS DISTINCT FROM true
  OR invocation119_capabilities_live(p_run) IS DISTINCT FROM true
  OR EXISTS(SELECT 1 FROM agent_capability_bindings WHERE instance_revision_id=revision.id
    AND authorization_model_version IS DISTINCT FROM p_model) THEN
  RETURN false;
 END IF;
 -- All statuses count. A released fence or foreign record using the exact
 -- authority IDs is history, never a fresh first acquisition or absent ceiling.
 RETURN NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM runtime_authority_sessions WHERE run_id=p_run OR id=p_run OR snapshot_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM run_authorization_snapshots WHERE run_id=p_run OR id=p_run)
  AND NOT EXISTS(SELECT 1 FROM run_authorization_snapshot_bindings WHERE snapshot_id=p_run
    OR snapshot_id IN (SELECT id FROM run_authorization_snapshots WHERE run_id=p_run))
  AND NOT EXISTS(SELECT 1 FROM run_git_authority_snapshots WHERE snapshot_id=p_run
    OR snapshot_id IN (SELECT id FROM run_authorization_snapshots WHERE run_id=p_run))
  AND NOT EXISTS(SELECT 1 FROM runtime_git_credentials WHERE runtime_session_id=p_run OR authorization_snapshot_id=p_run
    OR runtime_session_id IN (SELECT id FROM runtime_authority_sessions WHERE run_id=p_run)
    OR authorization_snapshot_id IN (SELECT id FROM run_authorization_snapshots WHERE run_id=p_run))
  AND NOT EXISTS(SELECT 1 FROM run_vm_plan_admissions WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM run_legacy_vm_placements WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM run_cleanup_targets WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM run_cleanup_target_leases WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM run_cleanup_receipts WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=p_run)
  -- Own secret history contradicts first acquisition. Full B2 checks all
  -- selected-revision bindings/rules at actual Provisioning before provider IO.
  AND NOT EXISTS(SELECT 1 FROM run_secret_provenance WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM run_instance_provenance WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM secret_runtime_sessions WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM secret_leases WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM secret_runtime_mounts WHERE run_id=p_run)
  AND NOT EXISTS(SELECT 1 FROM brokered_secret_lease_snapshots WHERE run_id=p_run);
END $$;
REVOKE ALL ON FUNCTION verify_invocation119_pre_acquisition(uuid,bytea,uuid,uuid,uuid,uuid,uuid,boolean,text,bigint,text) FROM PUBLIC,hephaestus_app;
GRANT EXECUTE ON FUNCTION verify_invocation119_pre_acquisition(uuid,bytea,uuid,uuid,uuid,uuid,uuid,boolean,text,bigint,text) TO hephaestus_worker;
