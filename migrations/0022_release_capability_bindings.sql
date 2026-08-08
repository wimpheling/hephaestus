-- Immutable release capability requirements and exact instance-revision
-- bindings. Runtime snapshots and credentials are added at the dispatch seam;
-- this migration establishes the release/instance authority ceiling.

ALTER TABLE release_agents
    ADD COLUMN capability_requirements_hash bytea NOT NULL
        DEFAULT decode(
            'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
            'hex'
        )
        CHECK (octet_length(capability_requirements_hash) = 32);

CREATE FUNCTION capability_operations_are_unique(operations text[]) RETURNS boolean
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT cardinality(operations) = count(DISTINCT operation)
    FROM unnest(operations) AS operation
$$;

CREATE FUNCTION capability_operations_are_legal(
    resource_kind text,
    operations text[]
) RETURNS boolean
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT COALESCE(bool_and(
        CASE resource_kind
            WHEN 'repository' THEN operation = ANY(ARRAY[
                'inspect', 'git_read', 'create_ref', 'update_ref',
                'force_update_ref', 'delete_ref', 'create_tag', 'delete_tag',
                'trigger_run', 'manage_attachments'
            ]::text[])
            WHEN 'project' THEN operation = ANY(ARRAY[
                'inspect', 'configure', 'execute', 'update', 'pause', 'recover'
            ]::text[])
            WHEN 'agent_instance' THEN operation = ANY(ARRAY[
                'inspect', 'configure', 'execute', 'update', 'pause', 'recover'
            ]::text[])
            WHEN 'gateway' THEN operation = ANY(ARRAY[
                'inspect', 'configure', 'execute', 'update', 'pause', 'recover'
            ]::text[])
            WHEN 'run' THEN operation = ANY(ARRAY[
                'inspect', 'cancel', 'recover'
            ]::text[])
            WHEN 'state_volume' THEN operation = ANY(ARRAY[
                'inspect', 'attach', 'restore'
            ]::text[])
            ELSE false
        END
    ), true)
    FROM unnest(operations) AS operation
$$;

CREATE TABLE release_capability_requirements (
    id uuid PRIMARY KEY,
    release_agent_id uuid NOT NULL REFERENCES release_agents(id),
    slot_key text NOT NULL CHECK (
        slot_key ~ '^[a-z][a-z0-9_-]{0,63}$'
    ),
    purpose text NOT NULL CHECK (length(purpose) BETWEEN 1 AND 512),
    resource_kind text NOT NULL CHECK (
        resource_kind IN (
            'repository', 'project', 'agent_instance', 'gateway', 'run',
            'state_volume'
        )
    ),
    required_operations text[] NOT NULL,
    optional_operations text[] NOT NULL,
    slot_required boolean NOT NULL,
    normalized_hash bytea NOT NULL CHECK (octet_length(normalized_hash) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (release_agent_id, slot_key),
    UNIQUE (id, release_agent_id),
    CHECK (
        cardinality(required_operations) + cardinality(optional_operations)
            BETWEEN 1 AND 32
    ),
    CHECK (
        required_operations <@ ARRAY[
            'inspect', 'configure', 'execute', 'update', 'pause', 'recover',
            'cancel', 'attach', 'restore', 'git_read', 'create_ref',
            'update_ref', 'force_update_ref', 'delete_ref', 'create_tag',
            'delete_tag', 'trigger_run', 'manage_attachments'
        ]::text[]
        AND optional_operations <@ ARRAY[
            'inspect', 'configure', 'execute', 'update', 'pause', 'recover',
            'cancel', 'attach', 'restore', 'git_read', 'create_ref',
            'update_ref', 'force_update_ref', 'delete_ref', 'create_tag',
            'delete_tag', 'trigger_run', 'manage_attachments'
        ]::text[]
        AND NOT required_operations && optional_operations
    ),
    CHECK (capability_operations_are_unique(required_operations)),
    CHECK (capability_operations_are_unique(optional_operations)),
    CHECK (capability_operations_are_legal(resource_kind, required_operations)),
    CHECK (capability_operations_are_legal(resource_kind, optional_operations))
);
CREATE INDEX release_capability_requirements_by_agent
    ON release_capability_requirements (release_agent_id, slot_key, id);

-- Delegating a project resource to an agent is deliberately independent of
-- project membership and ordinary workload management. A project maintainer
-- may administer this role, but receives no grant authority until an explicit
-- row exists.
CREATE TABLE project_capability_granters (
    project_id uuid NOT NULL REFERENCES projects(id),
    user_id uuid NOT NULL REFERENCES users(id),
    created_by uuid NOT NULL REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, user_id)
);

CREATE FUNCTION enforce_release_capability_requirement_draft() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM release_agents AS agent
        JOIN releases AS release ON release.id = agent.release_id
        WHERE agent.id = NEW.release_agent_id AND release.state = 'draft'
    ) THEN
        RAISE EXCEPTION 'release capability requirements freeze at publication'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_release_capability_requirement_draft() FROM PUBLIC;
CREATE TRIGGER release_capability_requirements_require_draft
BEFORE INSERT ON release_capability_requirements
FOR EACH ROW EXECUTE FUNCTION enforce_release_capability_requirement_draft();

CREATE FUNCTION reject_release_agent_capability_hash_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'release capability requirement hash is immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
CREATE TRIGGER release_agents_capability_hash_immutable
BEFORE UPDATE OF capability_requirements_hash ON release_agents
FOR EACH ROW
WHEN (OLD.capability_requirements_hash IS DISTINCT FROM NEW.capability_requirements_hash)
EXECUTE FUNCTION reject_release_agent_capability_hash_mutation();

ALTER TABLE agent_instance_revisions
    ADD CONSTRAINT agent_instance_revisions_id_release_agent_unique
    UNIQUE (id, release_agent_id);

CREATE TABLE agent_capability_bindings (
    id uuid PRIMARY KEY,
    instance_revision_id uuid NOT NULL,
    release_agent_id uuid NOT NULL,
    requirement_id uuid NOT NULL,
    requirement_hash bytea NOT NULL
        CHECK (octet_length(requirement_hash) = 32),
    slot_key text NOT NULL CHECK (
        slot_key ~ '^[a-z][a-z0-9_-]{0,63}$'
    ),
    resource_kind text NOT NULL CHECK (
        resource_kind IN (
            'repository', 'project', 'agent_instance', 'gateway', 'run',
            'state_volume'
        )
    ),
    resource_id uuid NOT NULL,
    granted_operations text[] NOT NULL CHECK (
        cardinality(granted_operations) BETWEEN 1 AND 32
    ),
    normalized_hash bytea NOT NULL CHECK (octet_length(normalized_hash) = 32),
    authorization_model_version text NOT NULL
        CHECK (length(authorization_model_version) BETWEEN 1 AND 128),
    created_by uuid NOT NULL REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (instance_revision_id, release_agent_id)
        REFERENCES agent_instance_revisions(id, release_agent_id),
    FOREIGN KEY (requirement_id, release_agent_id)
        REFERENCES release_capability_requirements(id, release_agent_id),
    UNIQUE (instance_revision_id, slot_key),
    UNIQUE (id, instance_revision_id),
    CHECK (
        granted_operations <@ ARRAY[
            'inspect', 'configure', 'execute', 'update', 'pause', 'recover',
            'cancel', 'attach', 'restore', 'git_read', 'create_ref',
            'update_ref', 'force_update_ref', 'delete_ref', 'create_tag',
            'delete_tag', 'trigger_run', 'manage_attachments'
        ]::text[]
    ),
    CHECK (capability_operations_are_unique(granted_operations)),
    CHECK (capability_operations_are_legal(resource_kind, granted_operations))
);
CREATE INDEX agent_capability_bindings_by_resource
    ON agent_capability_bindings (resource_kind, resource_id, id);

-- Only bindings on the currently active immutable revision contribute live
-- agent-instance authority. Historical bindings remain auditable without
-- accumulating ambient permissions on the durable workload principal.
-- Gateway resources and principals remain reserved for MVP 03; there is no
-- synthetic gateway table or tuple source in this migration.
CREATE OR REPLACE VIEW melange_tuples (
    subject_type, subject_id, relation, object_type, object_id
) AS
SELECT subject_type, subject_id, relation, object_type, object_id
FROM melange_base_tuples
UNION ALL
SELECT
    'project', image.project_id::text, 'project',
    'repository_oci_image', image.id::text
FROM repository_oci_image_definitions AS image
UNION ALL
SELECT
    'user', capability_granter.user_id::text, 'capability_granter',
    'project', capability_granter.project_id::text
FROM project_capability_granters AS capability_granter
UNION ALL
SELECT
    'agent_instance', revision.instance_id::text,
    'agent_' || operation.name,
    binding.resource_kind, binding.resource_id::text
FROM agent_capability_bindings AS binding
JOIN agent_instance_revisions AS revision
  ON revision.id = binding.instance_revision_id
JOIN agent_instances AS instance
  ON instance.id = revision.instance_id
 AND instance.active_revision_id = revision.id
CROSS JOIN LATERAL unnest(binding.granted_operations) AS operation(name)
WHERE binding.resource_kind <> 'gateway';

CREATE FUNCTION enforce_capability_resource_integrity() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    consuming_project_id uuid;
    resource_project_id uuid;
    declared_slot_key text;
    declared_resource_kind text;
    declared_required_operations text[];
    declared_optional_operations text[];
    declared_hash bytea;
BEGIN
    SELECT slot_key, resource_kind, required_operations, optional_operations,
           normalized_hash
    INTO declared_slot_key, declared_resource_kind,
         declared_required_operations, declared_optional_operations,
         declared_hash
    FROM release_capability_requirements
    WHERE id = NEW.requirement_id
      AND release_agent_id = NEW.release_agent_id;

    IF declared_slot_key IS NULL
       OR NEW.slot_key <> declared_slot_key
       OR NEW.resource_kind <> declared_resource_kind
       OR NEW.requirement_hash <> declared_hash
       OR NOT (NEW.granted_operations @> declared_required_operations)
       OR NOT (
           NEW.granted_operations
               <@ (declared_required_operations || declared_optional_operations)
       )
    THEN
        RAISE EXCEPTION 'capability binding exceeds or mismatches release requirement'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;

    SELECT instance.project_id INTO consuming_project_id
    FROM agent_instance_revisions AS revision
    JOIN agent_instances AS instance ON instance.id = revision.instance_id
    WHERE revision.id = NEW.instance_revision_id;

    IF consuming_project_id IS NULL THEN
        RAISE EXCEPTION 'capability binding revision is unavailable'
            USING ERRCODE = 'foreign_key_violation';
    END IF;

    CASE NEW.resource_kind
        WHEN 'repository' THEN
            SELECT project_id INTO resource_project_id
            FROM repositories WHERE id = NEW.resource_id;
        WHEN 'project' THEN
            SELECT id INTO resource_project_id
            FROM projects WHERE id = NEW.resource_id;
        WHEN 'agent_instance' THEN
            SELECT project_id INTO resource_project_id
            FROM agent_instances WHERE id = NEW.resource_id;
        WHEN 'run' THEN
            SELECT instance.project_id INTO resource_project_id
            FROM runs AS run
            JOIN agent_instances AS instance ON instance.id = run.instance_id
            WHERE run.id = NEW.resource_id;
        WHEN 'state_volume' THEN
            SELECT instance.project_id INTO resource_project_id
            FROM agent_instance_state_volumes AS volume
            JOIN agent_instances AS instance ON instance.id = volume.instance_id
            WHERE volume.id = NEW.resource_id;
        WHEN 'gateway' THEN
            RAISE EXCEPTION 'gateway resources are unavailable before MVP 03'
                USING ERRCODE = 'foreign_key_violation';
        ELSE
            RAISE EXCEPTION 'unsupported capability resource kind'
                USING ERRCODE = 'foreign_key_violation';
    END CASE;

    IF resource_project_id IS NULL THEN
        RAISE EXCEPTION 'capability resource is unavailable'
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    IF resource_project_id <> consuming_project_id THEN
        RAISE EXCEPTION 'cross-project capability binding requires a sharing contract'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_capability_resource_integrity() FROM PUBLIC;
CREATE TRIGGER agent_capability_bindings_resource_integrity
BEFORE INSERT ON agent_capability_bindings
FOR EACH ROW EXECUTE FUNCTION enforce_capability_resource_integrity();

CREATE FUNCTION reject_capability_record_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'capability requirements and bindings are immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
CREATE TRIGGER release_capability_requirements_immutable
BEFORE UPDATE OR DELETE ON release_capability_requirements
FOR EACH ROW EXECUTE FUNCTION reject_capability_record_mutation();
CREATE TRIGGER agent_capability_bindings_immutable
BEFORE UPDATE OR DELETE ON agent_capability_bindings
FOR EACH ROW EXECUTE FUNCTION reject_capability_record_mutation();

-- Binding selection is two-sided: a user must be explicitly allowed to
-- delegate the exact resource and must personally hold every semantic
-- operation being delegated. Keep this function beside the binding schema so
-- direct app-role inserts receive the same deny-by-default check as the Rust
-- service boundary.
CREATE FUNCTION can_grant_agent_capability_operations(
    actor_id uuid,
    resource_kind text,
    resource_id uuid,
    operations text[]
) RETURNS boolean
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public
SET row_security = off
AS $$
DECLARE
    operation text;
    required_permission text;
BEGIN
    IF check_permission(
        'user', actor_id::text, 'can_grant_agent_capability',
        resource_kind, resource_id::text
    ) <> 1 THEN
        RETURN false;
    END IF;

    FOREACH operation IN ARRAY operations LOOP
        required_permission := CASE resource_kind
            WHEN 'repository' THEN CASE
                WHEN operation IN ('inspect', 'git_read') THEN 'can_read'
                WHEN operation IN (
                    'create_ref', 'update_ref', 'force_update_ref',
                    'delete_ref', 'create_tag', 'delete_tag', 'trigger_run',
                    'manage_attachments'
                ) THEN 'can_write'
            END
            WHEN 'project' THEN CASE
                WHEN operation = 'inspect' THEN 'can_read'
                WHEN operation = 'execute' THEN 'can_write'
                WHEN operation IN (
                    'configure', 'update', 'pause', 'recover'
                ) THEN 'can_manage'
            END
            WHEN 'agent_instance' THEN CASE
                WHEN operation = 'inspect' THEN 'can_read'
                WHEN operation IN ('configure', 'pause') THEN 'can_manage'
                WHEN operation = 'execute' THEN 'can_execute'
                WHEN operation = 'update' THEN 'can_update'
                WHEN operation = 'recover' THEN 'can_recover'
            END
            WHEN 'run' THEN CASE
                WHEN operation = 'inspect' THEN 'can_read'
                WHEN operation = 'cancel' THEN 'can_cancel'
                WHEN operation = 'recover' THEN 'can_recover'
            END
            WHEN 'state_volume' THEN CASE
                WHEN operation = 'inspect' THEN 'can_read'
                WHEN operation = 'attach' THEN 'can_attach'
                WHEN operation = 'restore' THEN 'can_restore'
            END
        END;
        IF required_permission IS NULL OR check_permission(
            'user', actor_id::text, required_permission,
            resource_kind, resource_id::text
        ) <> 1 THEN
            RETURN false;
        END IF;
    END LOOP;
    RETURN true;
END
$$;
REVOKE ALL ON FUNCTION can_grant_agent_capability_operations(
    uuid, text, uuid, text[]
) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION can_grant_agent_capability_operations(
    uuid, text, uuid, text[]
) TO hephaestus_app, hephaestus_worker;

GRANT SELECT ON release_capability_requirements, agent_capability_bindings,
    project_capability_granters
    TO hephaestus_app, hephaestus_worker;
GRANT INSERT ON release_capability_requirements
    TO hephaestus_worker;
GRANT INSERT ON agent_capability_bindings
    TO hephaestus_app, hephaestus_worker;
GRANT INSERT, DELETE ON project_capability_granters
    TO hephaestus_app, hephaestus_worker;

ALTER TABLE project_capability_granters ENABLE ROW LEVEL SECURITY;
ALTER TABLE project_capability_granters FORCE ROW LEVEL SECURITY;
CREATE POLICY project_capability_granters_user_select
    ON project_capability_granters FOR SELECT TO hephaestus_app
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'project', project_id::text
    ) = 1);
CREATE POLICY project_capability_granters_user_insert
    ON project_capability_granters FOR INSERT TO hephaestus_app
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'project', project_id::text
    ) = 1 AND created_by::text = hephaestus_actor_id());
CREATE POLICY project_capability_granters_user_delete
    ON project_capability_granters FOR DELETE TO hephaestus_app
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_manage',
        'project', project_id::text
    ) = 1);
CREATE POLICY project_capability_granters_worker
    ON project_capability_granters TO hephaestus_worker
    USING (true) WITH CHECK (true);

ALTER TABLE release_capability_requirements ENABLE ROW LEVEL SECURITY;
ALTER TABLE release_capability_requirements FORCE ROW LEVEL SECURITY;
CREATE POLICY release_capability_requirements_user_select
    ON release_capability_requirements FOR SELECT TO hephaestus_app
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'release_agent', release_agent_id::text
    ) = 1);
CREATE POLICY release_capability_requirements_worker
    ON release_capability_requirements TO hephaestus_worker
    USING (true) WITH CHECK (true);

ALTER TABLE agent_capability_bindings ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_capability_bindings FORCE ROW LEVEL SECURITY;
CREATE POLICY agent_capability_bindings_user_select
    ON agent_capability_bindings FOR SELECT TO hephaestus_app
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read', 'agent_instance', (
            SELECT revision.instance_id::text
            FROM agent_instance_revisions AS revision
            WHERE revision.id = instance_revision_id
        )
    ) = 1);
CREATE POLICY agent_capability_bindings_user_insert
    ON agent_capability_bindings FOR INSERT TO hephaestus_app
    WITH CHECK (
        check_permission(
            'user', hephaestus_actor_id(), 'can_manage', 'agent_instance', (
                SELECT revision.instance_id::text
                FROM agent_instance_revisions AS revision
                WHERE revision.id = instance_revision_id
            )
        ) = 1
        AND can_grant_agent_capability_operations(
            hephaestus_actor_id()::uuid,
            resource_kind,
            resource_id,
            granted_operations
        )
    );
CREATE POLICY agent_capability_bindings_worker
    ON agent_capability_bindings TO hephaestus_worker
    USING (true) WITH CHECK (true);
