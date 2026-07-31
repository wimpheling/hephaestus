SELECT jsonb_build_object(
    'instance_id', instance.id,
    'project_id', instance.project_id,
    'state', instance.state,
    'run_gate_open', instance.run_gate_open,
    'active_revision_id', instance.active_revision_id,
    'state_volume_id', instance.state_volume_id,
    'revisions', (
        SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'id', revision.id,
            'release_agent_id', revision.release_agent_id,
            'runnable', revision.runnable,
            'diagnostics', revision.diagnostics,
            'platform_policy_version', revision.platform_policy_version
        ) ORDER BY revision.created_at), '[]'::jsonb)
        FROM agent_instance_revisions revision
        WHERE revision.instance_id = instance.id
    ),
    'attachments', (
        SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'id', attachment.id,
            'repository_id', attachment.repository_id,
            'ref_selector', attachment.ref_selector,
            'enabled', attachment.enabled,
            'removed_at', attachment.removed_at
        ) ORDER BY attachment.created_at), '[]'::jsonb)
        FROM agent_attachments attachment
        WHERE attachment.instance_id = instance.id
    ),
    'updates', (
        SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'id', update_record.id,
            'state', update_record.state,
            'candidate_revision_id', update_record.candidate_revision_id,
            'hook_run_id', update_record.hook_run_id,
            'final_decision', update_record.final_decision
        ) ORDER BY update_record.created_at), '[]'::jsonb)
        FROM agent_updates update_record
        WHERE update_record.instance_id = instance.id
    ),
    'leases', (
        SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'id', lease.id, 'run_id', lease.run_id,
            'state', lease.state, 'fencing_token', lease.fencing_token
        ) ORDER BY lease.acquired_at), '[]'::jsonb)
        FROM agent_instance_volume_leases lease
        WHERE lease.instance_id = instance.id
    )
) FROM agent_instances instance WHERE instance.id = $1
