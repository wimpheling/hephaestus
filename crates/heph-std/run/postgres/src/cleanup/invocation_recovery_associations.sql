-- Compare immutable association/whole frozen selections, not current grants or live expiry.
SELECT lease.run_id, execution.execution_profile,
 COALESCE(CASE execution.execution_profile
 WHEN 'owned_canonical' THEN
   execution.run_kind='invocation' AND lease.update_id IS NULL
   AND binding.volume_id=lease.volume_id
   AND execution.volume_selection_snapshot=public.invocation119_selections(execution.id)
   AND jsonb_array_length(execution.volume_selection_snapshot->'selections')=
       (SELECT count(*) FROM public.agent_instance_volume_leases original WHERE original.run_id=execution.id)
   AND ROW(lease.instance_id,lease.selected_revision_id,lease.selected_release_id,
       lease.selected_release_agent_id,lease.selected_project_id,lease.selection_origin,
       lease.access_mode,lease.guest_path,lease.slot_required,lease.minimum_capacity_bytes,lease.release_contract_hash)
       IS NOT DISTINCT FROM ROW(execution.instance_id,execution.instance_revision_id,execution.release_id,
           execution.release_agent_id,volume.project_id,binding.provenance,binding.access_mode,
           binding.guest_path,binding.slot_required,binding.minimum_capacity_bytes,agent.runtime_contract_hash)
   AND lease.selection_origin IN ('explicit','legacy_declaration')
   AND lease.host_id=execution.vm_host_id AND lease.host_id=volume.host_id
   AND lease.fencing_token>0 AND lease.fencing_token=volume.lease_generation
   AND lease.state IN ('active','recovering') AND volume.id IS NOT NULL
   AND NOT EXISTS(SELECT 1 FROM public.agent_instance_volume_leases other
       WHERE other.volume_id=lease.volume_id AND other.id<>lease.id AND other.released_at IS NULL)
 WHEN 'legacy_scalar' THEN public.legacy_scalar_lease_matches(lease)
 ELSE false END,false) AS exact,
 (lease.recovering_at IS NOT NULL OR lease.state='recovering'
  OR ($1::timestamptz IS NOT NULL AND lease.expires_at<=$1)) AS stale
FROM public.agent_instance_volume_leases lease
LEFT JOIN public.runs execution ON execution.id=lease.run_id
LEFT JOIN public.agent_instance_state_volumes volume ON volume.id=lease.volume_id
LEFT JOIN public.agent_instance_revision_volume_bindings binding
    ON binding.instance_revision_id=execution.instance_revision_id AND binding.slot_key=lease.slot_key
LEFT JOIN public.release_agents agent ON agent.id=execution.release_agent_id
WHERE lease.released_at IS NULL
ORDER BY lease.run_id,lease.volume_id,lease.id
