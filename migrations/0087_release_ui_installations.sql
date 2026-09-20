-- Durable project/repository UI installation identities.  Global installation
-- ownership is intentionally outside this migration.

-- These unique tuples are added here so every later composite FK has an exact
-- declared target. Migration 0084 remains unchanged.
ALTER TABLE release_ui_descriptors
    ADD CONSTRAINT release_ui_descriptors_release_key_scope_unique
    UNIQUE (release_id, ui_key, scope);
ALTER TABLE gateway_revisions
    ADD CONSTRAINT gateway_revisions_installation_exact_unique
    UNIQUE (id, gateway_id, release_id, release_agent_id, exposure);

CREATE TABLE ui_installations (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id),
    repository_id uuid,
    scope text NOT NULL CHECK (scope IN ('project', 'repository')),
    ui_key text NOT NULL CHECK (ui_key ~ '^[a-z][a-z0-9-]{0,63}$'),
    lifecycle text NOT NULL CHECK (
        lifecycle IN ('enabled', 'disabled', 'removed')
    ),
    current_generation_id uuid NOT NULL,
    created_by uuid NOT NULL REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    removed_at timestamptz,
    FOREIGN KEY (repository_id, project_id)
        REFERENCES repositories(id, project_id),
    CHECK ((scope = 'project') = (repository_id IS NULL)),
    CHECK ((lifecycle = 'removed') = (removed_at IS NOT NULL)),
    UNIQUE (id, ui_key, scope)
);

-- Only one live installation may own a project/repository UI key. A removed
-- installation remains immutable history and therefore releases the key.
CREATE UNIQUE INDEX ui_installations_active_owner_key
    ON ui_installations (project_id, repository_id, ui_key)
    NULLS NOT DISTINCT
    WHERE lifecycle <> 'removed';
CREATE INDEX ui_installations_project_lifecycle
    ON ui_installations (project_id, lifecycle, created_at DESC, id);
CREATE INDEX ui_installations_repository_lifecycle
    ON ui_installations (repository_id, lifecycle, created_at DESC, id)
    WHERE repository_id IS NOT NULL;

CREATE TABLE ui_installation_generations (
    id uuid PRIMARY KEY,
    installation_id uuid NOT NULL,
    generation_no bigint NOT NULL CHECK (generation_no > 0),
    release_id uuid NOT NULL,
    ui_key text NOT NULL,
    ui_scope text NOT NULL CHECK (ui_scope IN ('project', 'repository')),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (installation_id, generation_no),
    UNIQUE (id, installation_id),
    UNIQUE (id, release_id, ui_key),
    FOREIGN KEY (installation_id, ui_key, ui_scope)
        REFERENCES ui_installations(id, ui_key, scope),
    FOREIGN KEY (release_id, ui_key, ui_scope)
        REFERENCES release_ui_descriptors(release_id, ui_key, scope)
);

ALTER TABLE ui_installations
    ADD CONSTRAINT ui_installations_current_generation_fk
    FOREIGN KEY (current_generation_id, id)
    REFERENCES ui_installation_generations(id, installation_id)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE ui_installation_bindings (
    installation_id uuid NOT NULL,
    generation_id uuid NOT NULL,
    binding_kind text NOT NULL CHECK (
        binding_kind IN ('managed_service', 'api')
    ),
    binding_key text NOT NULL CHECK (binding_key ~ '^[a-z][a-z0-9-]{0,63}$'),
    release_id uuid NOT NULL,
    ui_key text NOT NULL,
    gateway_id uuid NOT NULL REFERENCES gateways(id),
    gateway_revision_id uuid NOT NULL,
    release_agent_id uuid NOT NULL,
    gateway_name text NOT NULL CHECK (
        gateway_name ~ '^[a-z][a-z0-9_-]{0,63}$'
    ),
    method text NOT NULL CHECK (
        method IN ('GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS')
    ),
    route text NOT NULL CHECK (octet_length(route) BETWEEN 2 AND 512),
    exposure text NOT NULL CHECK (exposure = 'heph_authenticated'),
    CHECK (
        (binding_kind = 'managed_service' AND binding_key = 'service' AND method = 'GET')
        OR binding_kind = 'api'
    ),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (generation_id, binding_kind, binding_key),
    FOREIGN KEY (generation_id, installation_id)
        REFERENCES ui_installation_generations(id, installation_id),
    FOREIGN KEY (generation_id, release_id, ui_key)
        REFERENCES ui_installation_generations(id, release_id, ui_key),
    FOREIGN KEY (release_id, ui_key)
        REFERENCES release_ui_descriptors(release_id, ui_key),
    FOREIGN KEY (
        gateway_revision_id, gateway_id, release_id, release_agent_id, exposure
    ) REFERENCES gateway_revisions(
        id, gateway_id, release_id, release_agent_id, exposure
    ),
    FOREIGN KEY (release_agent_id, release_id)
        REFERENCES release_agents(id, release_id)
);
CREATE INDEX ui_installation_bindings_installation
    ON ui_installation_bindings (installation_id, generation_id);

CREATE TABLE ui_installation_commands (
    command_key bytea PRIMARY KEY CHECK (octet_length(command_key) = 32),
    caller_idempotency_key text NOT NULL CHECK (
        octet_length(caller_idempotency_key) BETWEEN 1 AND 256
    ),
    operation text NOT NULL CHECK (
        operation IN ('install', 'activate', 'rollback', 'disable', 'remove')
    ),
    installation_id uuid NOT NULL REFERENCES ui_installations(id),
    actor_id uuid NOT NULL REFERENCES users(id),
    request_id uuid NOT NULL,
    input_hash bytea NOT NULL CHECK (octet_length(input_hash) = 32),
    expected_generation_id uuid,
    result_generation_id uuid NOT NULL,
    result_lifecycle text NOT NULL CHECK (
        result_lifecycle IN ('enabled', 'disabled', 'removed')
    ),
    completed_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (expected_generation_id, installation_id)
        REFERENCES ui_installation_generations(id, installation_id),
    FOREIGN KEY (result_generation_id, installation_id)
        REFERENCES ui_installation_generations(id, installation_id),
    CHECK (
        (operation IN ('install', 'activate', 'rollback')
            AND result_lifecycle = 'enabled')
        OR (operation = 'disable' AND result_lifecycle = 'disabled')
        OR (operation = 'remove' AND result_lifecycle = 'removed')
    ),
    UNIQUE (actor_id, operation, caller_idempotency_key)
);
CREATE INDEX ui_installation_commands_installation
    ON ui_installation_commands (installation_id, completed_at DESC, command_key);

CREATE FUNCTION reject_ui_installation_immutable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'UI installation evidence is immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
REVOKE ALL ON FUNCTION reject_ui_installation_immutable() FROM PUBLIC;

CREATE FUNCTION protect_ui_installation_identity() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.lifecycle = 'removed' THEN
        RAISE EXCEPTION 'removed UI installation is terminal'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF ROW(OLD.id, OLD.project_id, OLD.repository_id, OLD.scope, OLD.ui_key,
           OLD.created_by, OLD.created_at)
       IS DISTINCT FROM ROW(NEW.id, NEW.project_id, NEW.repository_id, NEW.scope, NEW.ui_key,
           NEW.created_by, NEW.created_at)
    THEN
        RAISE EXCEPTION 'UI installation owner and key are immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION protect_ui_installation_identity() FROM PUBLIC;

CREATE TRIGGER ui_installation_identity_immutable
BEFORE UPDATE ON ui_installations
FOR EACH ROW EXECUTE FUNCTION protect_ui_installation_identity();
CREATE TRIGGER ui_installation_delete_forbidden
BEFORE DELETE ON ui_installations
FOR EACH ROW EXECUTE FUNCTION reject_ui_installation_immutable();
CREATE TRIGGER ui_installation_generations_immutable
BEFORE UPDATE OR DELETE ON ui_installation_generations
FOR EACH ROW EXECUTE FUNCTION reject_ui_installation_immutable();
CREATE TRIGGER ui_installation_bindings_immutable
BEFORE UPDATE OR DELETE ON ui_installation_bindings
FOR EACH ROW EXECUTE FUNCTION reject_ui_installation_immutable();
CREATE TRIGGER ui_installation_commands_immutable
BEFORE UPDATE OR DELETE ON ui_installation_commands
FOR EACH ROW EXECUTE FUNCTION reject_ui_installation_immutable();

ALTER TABLE ui_installations ENABLE ROW LEVEL SECURITY;
ALTER TABLE ui_installations FORCE ROW LEVEL SECURITY;
ALTER TABLE ui_installation_generations ENABLE ROW LEVEL SECURITY;
ALTER TABLE ui_installation_generations FORCE ROW LEVEL SECURITY;
ALTER TABLE ui_installation_bindings ENABLE ROW LEVEL SECURITY;
ALTER TABLE ui_installation_bindings FORCE ROW LEVEL SECURITY;
ALTER TABLE ui_installation_commands ENABLE ROW LEVEL SECURITY;
ALTER TABLE ui_installation_commands FORCE ROW LEVEL SECURITY;

CREATE POLICY ui_installations_worker
    ON ui_installations FOR ALL TO hephaestus_worker
    USING (true) WITH CHECK (true);
CREATE POLICY ui_installation_generations_worker
    ON ui_installation_generations FOR ALL TO hephaestus_worker
    USING (true) WITH CHECK (true);
CREATE POLICY ui_installation_bindings_worker
    ON ui_installation_bindings FOR ALL TO hephaestus_worker
    USING (true) WITH CHECK (true);
CREATE POLICY ui_installation_commands_worker
    ON ui_installation_commands FOR ALL TO hephaestus_worker
    USING (true) WITH CHECK (true);

CREATE POLICY ui_installations_app_read
    ON ui_installations FOR SELECT TO hephaestus_app
    USING (
        check_permission('user', hephaestus_actor_id(), 'can_read',
            'project', project_id::text) = 1
        AND (
            repository_id IS NULL
            OR check_permission('user', hephaestus_actor_id(), 'can_read',
                'repository', repository_id::text) = 1
        )
    );
CREATE POLICY ui_installation_generations_app_read
    ON ui_installation_generations FOR SELECT TO hephaestus_app
    USING (EXISTS (
        SELECT 1 FROM ui_installations installation
        WHERE installation.id = ui_installation_generations.installation_id
    ));
CREATE POLICY ui_installation_bindings_app_read
    ON ui_installation_bindings FOR SELECT TO hephaestus_app
    USING (EXISTS (
        SELECT 1 FROM ui_installations installation
        WHERE installation.id = ui_installation_bindings.installation_id
    ));
CREATE POLICY ui_installation_commands_app_read
    ON ui_installation_commands FOR SELECT TO hephaestus_app
    USING (EXISTS (
        SELECT 1 FROM ui_installations installation
        WHERE installation.id = ui_installation_commands.installation_id
    ));

REVOKE ALL ON ui_installations, ui_installation_generations,
    ui_installation_bindings, ui_installation_commands
    FROM PUBLIC, hephaestus_app, hephaestus_worker;
GRANT SELECT ON ui_installations, ui_installation_generations,
    ui_installation_bindings, ui_installation_commands TO hephaestus_app;
GRANT SELECT, INSERT, UPDATE ON ui_installations TO hephaestus_worker;
GRANT SELECT, INSERT ON ui_installation_generations,
    ui_installation_bindings, ui_installation_commands TO hephaestus_worker;
