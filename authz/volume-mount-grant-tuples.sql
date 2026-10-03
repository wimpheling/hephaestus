-- Typed grants contribute only coarse agent_attach. Runtime still checks the
-- exact run, revision, resource, slot, mode and contract hash independently.
SELECT 'agent_instance', mount_grant.instance_id::text, 'agent_attach',
       'state_volume', mount_grant.volume_id::text
FROM agent_instance_volume_mount_grants mount_grant
JOIN agent_instance_revisions revision ON revision.id=mount_grant.instance_revision_id
    AND revision.instance_id=mount_grant.instance_id AND revision.release_agent_id=mount_grant.release_agent_id
JOIN agent_instances instance ON instance.id=revision.instance_id AND instance.active_revision_id=revision.id
JOIN release_agents agent ON agent.id=revision.release_agent_id
    AND agent.runtime_contract_hash=mount_grant.release_contract_hash
JOIN releases release ON release.id=agent.release_id AND release.state='published'
WHERE instance.state IN ('active','update_rejected') AND revision.runnable
  AND private_volume_mount_binding_is_declared(revision.id,mount_grant.slot_key)
  AND private_volume_mount_source_is_live(mount_grant.created_by,mount_grant.volume_id)
  AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_revocations revoked
      WHERE revoked.grant_id=mount_grant.id)
