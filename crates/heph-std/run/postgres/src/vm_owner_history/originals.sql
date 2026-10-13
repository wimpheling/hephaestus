SELECT execution.execution_profile,
 CASE WHEN execution.execution_profile='legacy_scalar' THEN plan.provider_namespace::text
      ELSE execution.vm_provider_namespace END AS namespace,
 CASE WHEN execution.execution_profile='legacy_scalar' THEN plan.host_id
      ELSE execution.vm_host_id END AS host,
 COALESCE(CASE
 WHEN execution.execution_profile='legacy_scalar' THEN
   public.legacy_vm_placement_matches(execution,plan)
   AND NOT execution.vm_plan_admission_required
   AND num_nonnulls(execution.vm_provider_namespace,execution.vm_host_id,execution.vm_binding_kind)=0
   AND NOT EXISTS(SELECT 1 FROM public.run_vm_plan_admissions WHERE run_id=execution.id)
   AND NOT EXISTS(SELECT 1 FROM public.agent_instance_execution_invocations WHERE run_id=execution.id)
 WHEN execution.execution_profile='owned_canonical' THEN
   NOT execution.legacy_vm_placement_required AND plan.run_id IS NULL
   AND execution.vm_binding_kind='planned' AND execution.vm_id=execution.id::text
   AND CASE execution.run_kind
   WHEN 'normal' THEN NOT execution.vm_plan_admission_required
     AND NOT EXISTS(SELECT 1 FROM public.run_vm_plan_admissions WHERE run_id=execution.id)
     AND NOT EXISTS(SELECT 1 FROM public.mailbox_delivery_attempts WHERE run_id=execution.id)
     AND NOT EXISTS(SELECT 1 FROM public.agent_instance_execution_invocations WHERE run_id=execution.id)
     AND NOT EXISTS(SELECT 1 FROM public.release_command_inbox WHERE operation='begin_update_hook' AND secondary_id=execution.id)
     AND (SELECT count(*) FROM public.run_requests WHERE run_id=execution.id)=1
     AND EXISTS(SELECT 1 FROM public.run_requests request
       WHERE request.run_id=execution.id AND request.request_kind='instance_normal'
       AND ROW(request.command_id,request.instance_id,request.instance_revision_id,request.release_id,
               request.release_agent_id,request.attachment_id,request.requires_state)
         IS NOT DISTINCT FROM ROW(execution.command_id,execution.instance_id,execution.instance_revision_id,
               execution.release_id,execution.release_agent_id,execution.attachment_id,execution.requires_state))
     AND EXISTS(SELECT 1 FROM public.command_inbox inbox WHERE inbox.command_id=execution.command_id
       AND inbox.command_type='start_run' AND inbox.payload=jsonb_build_object(
         'command_id',execution.command_id,'run_id',execution.id,'instance_id',execution.instance_id,
         'instance_revision_id',execution.instance_revision_id,'release_id',execution.release_id,
         'release_agent_id',execution.release_agent_id,'attachment_id',execution.attachment_id,
         'kind',execution.run_kind,'requires_state',execution.requires_state))
   WHEN 'update' THEN execution.vm_plan_admission_required
     AND NOT EXISTS(SELECT 1 FROM public.run_requests WHERE run_id=execution.id)
     AND NOT EXISTS(SELECT 1 FROM public.mailbox_delivery_attempts WHERE run_id=execution.id)
     AND NOT EXISTS(SELECT 1 FROM public.agent_instance_execution_invocations WHERE run_id=execution.id)
     AND EXISTS(SELECT 1 FROM public.run_vm_plan_admissions admission
       JOIN public.release_command_inbox command ON command.operation='begin_update_hook'
         AND command.aggregate_id=admission.update_id AND command.secondary_id=admission.run_id
       WHERE admission.run_id=execution.id AND admission.consumed_at IS NOT NULL
       AND admission.consumption_kind IN ('provision','cleanup')
       AND ROW(admission.command_id,admission.instance_id,admission.instance_revision_id,
               admission.vm_id,admission.vm_provider_namespace,admission.vm_host_id)
         IS NOT DISTINCT FROM ROW(execution.command_id,execution.instance_id,execution.instance_revision_id,
               execution.vm_id,execution.vm_provider_namespace,execution.vm_host_id)
       AND ROW(command.command_key,command.actor_id,command.request_id)
         IS NOT DISTINCT FROM ROW(admission.producer_command_key,admission.actor_id,admission.request_id))
   WHEN 'invocation' THEN public.run_profile_producer_matches(execution)
   ELSE false END
 ELSE false END,false) AS qualified
FROM public.runs execution
LEFT JOIN public.run_legacy_vm_placements plan ON plan.run_id=execution.id
ORDER BY execution.id
