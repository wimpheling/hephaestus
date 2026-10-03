-- Current tuple projection. Migration 0102 applies this only after 0101 adds
-- stable volume ownership. Historical base inputs remain in authz/history.
CREATE OR REPLACE VIEW melange_tuples (
    subject_type, subject_id, relation, object_type, object_id
) AS
SELECT subject_type, subject_id, relation, object_type, object_id
FROM melange_base_tuples WHERE subject_id IS NOT NULL
UNION ALL
SELECT 'project', volume.project_id::text, 'project', 'state_volume', volume.id::text
FROM agent_instance_state_volumes volume
UNION ALL
SELECT 'project', image.project_id::text, 'project', 'repository_oci_image', image.id::text
FROM repository_oci_image_definitions image
UNION ALL
SELECT 'user', granter.user_id::text, 'capability_granter', 'project', granter.project_id::text
FROM project_capability_granters granter
UNION ALL
SELECT 'project', gateway.project_id::text, 'project', 'gateway', gateway.id::text
FROM gateways gateway
UNION ALL
SELECT 'gateway', revision.gateway_id::text, 'gateway', 'gateway_revision', revision.id::text
FROM gateway_revisions revision
UNION ALL
SELECT 'agent_instance', revision.instance_id::text, 'agent_' || operation.name,
    binding.resource_kind, binding.resource_id::text
FROM agent_capability_bindings binding
JOIN agent_instance_revisions revision ON revision.id = binding.instance_revision_id
JOIN agent_instances instance ON instance.id = revision.instance_id AND instance.active_revision_id = revision.id
CROSS JOIN LATERAL unnest(binding.granted_operations) operation(name)

UNION ALL
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
;
