-- source-model-sha256: ac00dabe86b9c473af56ea615b011445fa0597e3c1ea9802d83f0532fa82c148
-- Explicit typed authority is separate from immutable binding evidence.
-- No legacy binding or migration backfill creates a workload grant.
CREATE TABLE agent_instance_volume_mount_grants (
    id uuid PRIMARY KEY CHECK (id <> '00000000-0000-0000-0000-000000000000'::uuid),
    instance_revision_id uuid NOT NULL,
    instance_id uuid NOT NULL REFERENCES agent_instances(id),
    release_agent_id uuid NOT NULL REFERENCES release_agents(id),
    slot_key text NOT NULL,
    volume_id uuid NOT NULL REFERENCES agent_instance_state_volumes(id),
    access_mode text NOT NULL CHECK (access_mode IN ('read_only','read_write')),
    release_contract_hash bytea NOT NULL CHECK (octet_length(release_contract_hash)=32),
    created_by uuid NOT NULL REFERENCES users(id),
    request_id uuid NOT NULL,
    authorization_model_version text NOT NULL CHECK (length(authorization_model_version) BETWEEN 1 AND 128),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (instance_revision_id,slot_key)
        REFERENCES agent_instance_revision_volume_bindings(instance_revision_id,slot_key),
    UNIQUE(instance_revision_id,slot_key)
);
CREATE TABLE agent_instance_volume_mount_revocations (
    id uuid PRIMARY KEY CHECK (id <> '00000000-0000-0000-0000-000000000000'::uuid),
    grant_id uuid NOT NULL UNIQUE REFERENCES agent_instance_volume_mount_grants(id),
    created_by uuid NOT NULL REFERENCES users(id),
    request_id uuid NOT NULL,
    authorization_model_version text NOT NULL CHECK (length(authorization_model_version) BETWEEN 1 AND 128),
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Exact equivalence to frozen0102: project.can_manage and can_write are
-- maintainer; instance.manage/execute inherit them.0101 proves origin and
-- owning projects equal. Thus volume(Grant ANDAttach) equals this exact row.
-- Query authoritative tables rather than recursively reading melange_tuples.
-- Every future model transition must update/reverify this pinned parity.
CREATE FUNCTION private_volume_mount_source_is_live(p_actor uuid,p_volume uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public AS $$
    SELECT EXISTS(SELECT 1 FROM agent_instance_state_volumes volume
        JOIN project_maintainers manager ON manager.project_id=volume.project_id
        WHERE volume.id=p_volume AND manager.user_id=p_actor)
$$;
ALTER FUNCTION private_volume_mount_source_is_live(uuid,uuid) OWNER TO hephaestus_authz_owner;
REVOKE ALL ON FUNCTION private_volume_mount_source_is_live(uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION private_volume_mount_source_is_live(uuid,uuid) TO hephaestus_app,hephaestus_worker;
GRANT SELECT ON agent_instance_state_volumes,project_maintainers TO hephaestus_authz_owner;

CREATE FUNCTION private_volume_mount_binding_is_declared(p_revision uuid,p_slot text) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public AS $$
    SELECT EXISTS(SELECT 1 FROM agent_instance_revision_volume_bindings binding
        JOIN agent_instance_revisions revision ON revision.id=binding.instance_revision_id
        JOIN release_agents agent ON agent.id=revision.release_agent_id
        JOIN agent_instances instance ON instance.id=binding.instance_id
        JOIN agent_instance_state_volumes volume ON volume.id=binding.volume_id
        WHERE binding.instance_revision_id=p_revision AND binding.slot_key=p_slot
          AND binding.release_agent_id=agent.id AND binding.instance_id=revision.instance_id
          AND binding.project_id=instance.project_id AND binding.project_id=volume.project_id
          AND volume.capacity_bytes>=binding.minimum_capacity_bytes
          AND NOT EXISTS(SELECT 1 FROM release_capability_requirements requirement
              WHERE requirement.release_agent_id=agent.id AND requirement.slot_key=binding.slot_key)
          AND (
            (binding.provenance='explicit' AND
              (SELECT count(*) FROM jsonb_array_elements(CASE WHEN jsonb_typeof(agent.runtime_contract->'volume_slots')='array'
                  THEN agent.runtime_contract->'volume_slots' ELSE '[]'::jsonb END) declaration
               WHERE declaration->>'slot'=binding.slot_key)=1 AND
              EXISTS(SELECT 1 FROM jsonb_array_elements(CASE WHEN jsonb_typeof(agent.runtime_contract->'volume_slots')='array'
                  THEN agent.runtime_contract->'volume_slots' ELSE '[]'::jsonb END) declaration
               WHERE declaration->>'slot'=binding.slot_key
                 AND declaration->>'guest_path'=binding.guest_path
                 AND declaration->>'access_mode'=binding.access_mode
                 AND declaration->'required'=to_jsonb(binding.slot_required)
                 AND declaration->'minimum_capacity_bytes'=to_jsonb(binding.minimum_capacity_bytes)))
            OR (binding.provenance='legacy_state' AND agent.requires_state
                AND binding.slot_key='state' AND binding.guest_path='/var/lib/hephaestus'
                AND binding.access_mode='read_write' AND binding.slot_required
                AND binding.minimum_capacity_bytes=1 AND volume.instance_id=instance.id
                AND instance.state_volume_id=volume.id
                AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(CASE WHEN jsonb_typeof(agent.runtime_contract->'volume_slots')='array'
                    THEN agent.runtime_contract->'volume_slots' ELSE '[]'::jsonb END) declaration WHERE declaration->>'slot'='state'))))
$$;
ALTER FUNCTION private_volume_mount_binding_is_declared(uuid,text) OWNER TO hephaestus_authz_owner;
REVOKE ALL ON FUNCTION private_volume_mount_binding_is_declared(uuid,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION private_volume_mount_binding_is_declared(uuid,text) TO hephaestus_app,hephaestus_worker;
GRANT SELECT ON agent_instance_revision_volume_bindings,agent_instance_revisions,release_agents,
    agent_instances,release_capability_requirements TO hephaestus_authz_owner;

CREATE FUNCTION enforce_volume_mount_grant() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE binding agent_instance_revision_volume_bindings%ROWTYPE;
BEGIN
    IF NEW.created_by IS DISTINCT FROM NULLIF(current_setting('hephaestus.actor_id',true),'')::uuid
       OR NEW.request_id IS DISTINCT FROM NULLIF(current_setting('hephaestus.request_id',true),'')::uuid THEN
        RAISE EXCEPTION 'explicit mount grant requires actor request context' USING ERRCODE='insufficient_privilege';
    END IF;
    SELECT * INTO binding FROM agent_instance_revision_volume_bindings
        WHERE instance_revision_id=NEW.instance_revision_id AND slot_key=NEW.slot_key;
    IF binding.instance_id IS DISTINCT FROM NEW.instance_id
       OR binding.release_agent_id IS DISTINCT FROM NEW.release_agent_id
       OR binding.volume_id IS DISTINCT FROM NEW.volume_id OR binding.access_mode IS DISTINCT FROM NEW.access_mode
       OR NOT private_volume_mount_binding_is_declared(NEW.instance_revision_id,NEW.slot_key)
       OR NOT EXISTS(SELECT 1 FROM release_agents agent JOIN releases release ON release.id=agent.release_id
            WHERE agent.id=NEW.release_agent_id AND agent.runtime_contract_hash=NEW.release_contract_hash AND release.state='published')
       OR EXISTS(SELECT 1 FROM agent_updates WHERE candidate_revision_id=NEW.instance_revision_id) THEN
        RAISE EXCEPTION 'mount grant differs from exact frozen declaration or uses unsupported update candidate'
            USING ERRCODE='integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_volume_mount_grant() FROM PUBLIC;
CREATE TRIGGER volume_mount_grant_integrity BEFORE INSERT ON agent_instance_volume_mount_grants
    FOR EACH ROW EXECUTE FUNCTION enforce_volume_mount_grant();
CREATE TRIGGER volume_mount_grant_immutable BEFORE UPDATE OR DELETE ON agent_instance_volume_mount_grants
    FOR EACH ROW EXECUTE FUNCTION reject_capability_record_mutation();
CREATE TRIGGER volume_mount_revocation_immutable BEFORE UPDATE OR DELETE ON agent_instance_volume_mount_revocations
    FOR EACH ROW EXECUTE FUNCTION reject_capability_record_mutation();
CREATE FUNCTION enforce_volume_mount_revocation_context() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog,public AS $$
BEGIN
    IF NEW.created_by IS DISTINCT FROM NULLIF(current_setting('hephaestus.actor_id',true),'')::uuid
       OR NEW.request_id IS DISTINCT FROM NULLIF(current_setting('hephaestus.request_id',true),'')::uuid THEN
        RAISE EXCEPTION 'mount revocation requires actor request context' USING ERRCODE='insufficient_privilege';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_volume_mount_revocation_context() FROM PUBLIC;
CREATE TRIGGER volume_mount_revocation_context BEFORE INSERT ON agent_instance_volume_mount_revocations
    FOR EACH ROW EXECUTE FUNCTION enforce_volume_mount_revocation_context();

ALTER TABLE agent_instance_volume_mount_grants ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_volume_mount_grants FORCE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_volume_mount_revocations ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_volume_mount_revocations FORCE ROW LEVEL SECURITY;
GRANT SELECT,INSERT ON agent_instance_volume_mount_grants,agent_instance_volume_mount_revocations TO hephaestus_app;
GRANT SELECT ON agent_instance_volume_mount_grants,agent_instance_volume_mount_revocations TO hephaestus_worker,hephaestus_authz_owner;
CREATE POLICY volume_mount_grant_read ON agent_instance_volume_mount_grants FOR SELECT TO hephaestus_app
    USING(check_permission('user',hephaestus_actor_id(),'can_read','agent_instance',instance_id::text)=1);
CREATE POLICY volume_mount_grant_create ON agent_instance_volume_mount_grants FOR INSERT TO hephaestus_app
    WITH CHECK(created_by=hephaestus_actor_id()::uuid
      AND check_permission('user',hephaestus_actor_id(),'can_manage','agent_instance',instance_id::text)=1
      AND private_volume_mount_source_is_live(created_by,volume_id));
CREATE POLICY volume_mount_grant_worker_read ON agent_instance_volume_mount_grants FOR SELECT TO hephaestus_worker USING(true);
CREATE POLICY volume_mount_revocation_read ON agent_instance_volume_mount_revocations FOR SELECT TO hephaestus_app
    USING(EXISTS(SELECT 1 FROM agent_instance_volume_mount_grants grant_row WHERE grant_row.id=grant_id));
CREATE POLICY volume_mount_revocation_create ON agent_instance_volume_mount_revocations FOR INSERT TO hephaestus_app
    WITH CHECK(created_by=hephaestus_actor_id()::uuid AND EXISTS(SELECT 1 FROM agent_instance_volume_mount_grants grant_row
        WHERE grant_row.id=grant_id AND
          (check_permission('user',hephaestus_actor_id(),'can_manage','agent_instance',grant_row.instance_id::text)=1
           OR check_permission('user',hephaestus_actor_id(),'can_manage','state_volume',grant_row.volume_id::text)=1)));
CREATE POLICY volume_mount_revocation_worker_read ON agent_instance_volume_mount_revocations FOR SELECT TO hephaestus_worker USING(true);
CREATE TRIGGER volume_mount_grant_application_event AFTER INSERT ON agent_instance_volume_mount_grants
    FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
        'agent_instance','instance_id','agent_instance','instance_id','agent_instance.changed','');
CREATE FUNCTION capture_volume_mount_revocation_event() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE consumer uuid;
BEGIN
    SELECT instance_id INTO consumer FROM agent_instance_volume_mount_grants WHERE id=NEW.grant_id;
    PERFORM append_application_event(COALESCE(NULLIF(current_setting('hephaestus.occurrence_id',true),'')::uuid,gen_random_uuid()),'agent_instance',consumer,
        'agent_instance',consumer,'agent_instance.changed','updated');
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION capture_volume_mount_revocation_event() FROM PUBLIC;
CREATE TRIGGER volume_mount_revocation_application_event AFTER INSERT ON agent_instance_volume_mount_revocations
    FOR EACH ROW EXECUTE FUNCTION capture_volume_mount_revocation_event();

-- BEGIN GENERATED VOLUME MOUNT PROJECTION
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
-- END GENERATED VOLUME MOUNT PROJECTION
