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
CROSS JOIN LATERAL unnest(binding.granted_operations) operation(name);
