-- Durable admission only. Provider claims and execution are deliberately separate.
CREATE TABLE recipe_definitions (
    project_id uuid NOT NULL REFERENCES projects(id),
    recipe_id text NOT NULL CHECK (recipe_id ~ '^[a-z][a-z0-9_-]{0,63}$'),
    recipe_version text NOT NULL CHECK (octet_length(recipe_version) BETWEEN 1 AND 128),
    contract_version integer NOT NULL CHECK (contract_version = 1),
    canonical_toml text NOT NULL CHECK (octet_length(canonical_toml) BETWEEN 1 AND 65536),
    declaration_hash bytea NOT NULL CHECK (octet_length(declaration_hash) = 32),
    created_by uuid NOT NULL REFERENCES users(id),
    first_request_id uuid NOT NULL CHECK (first_request_id <> '00000000-0000-0000-0000-000000000000'),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, recipe_id, recipe_version)
);

CREATE TABLE recipe_deployments (
    id uuid PRIMARY KEY CHECK (id <> '00000000-0000-0000-0000-000000000000'),
    project_id uuid NOT NULL REFERENCES projects(id),
    deployment_key text NOT NULL CHECK (octet_length(deployment_key) BETWEEN 1 AND 128
        AND deployment_key = btrim(deployment_key) AND deployment_key !~ '[[:cntrl:]]'),
    recipe_id text NOT NULL,
    recipe_version text NOT NULL,
    declaration_hash bytea NOT NULL CHECK (octet_length(declaration_hash) = 32),
    resolved_json bytea NOT NULL CHECK (octet_length(resolved_json) BETWEEN 1 AND 33554432),
    resolved_hash bytea NOT NULL CHECK (octet_length(resolved_hash) = 32),
    input_hash bytea NOT NULL CHECK (octet_length(input_hash) = 32),
    ordinary_inputs jsonb NOT NULL CHECK (jsonb_typeof(ordinary_inputs) = 'object'),
    admission_evidence jsonb NOT NULL CHECK (jsonb_typeof(admission_evidence) = 'object'
        AND octet_length(admission_evidence::text) <= 33554432),
    resource_count integer NOT NULL CHECK (resource_count BETWEEN 1 AND 64),
    lifecycle text NOT NULL DEFAULT 'installing' CHECK (lifecycle IN
        ('installing', 'installed', 'removing', 'removed', 'recovery_required')),
    version bigint NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_by uuid NOT NULL REFERENCES users(id),
    first_request_id uuid NOT NULL CHECK (first_request_id <> '00000000-0000-0000-0000-000000000000'),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, deployment_key),
    UNIQUE (id, project_id),
    FOREIGN KEY (project_id, recipe_id, recipe_version)
        REFERENCES recipe_definitions(project_id, recipe_id, recipe_version)
);

CREATE TABLE recipe_deployment_resources (
    deployment_id uuid NOT NULL,
    project_id uuid NOT NULL,
    resource_name text NOT NULL CHECK (resource_name ~ '^[a-z][a-z0-9_-]{0,63}$'),
    resource_kind text NOT NULL CHECK (resource_kind IN ('volume', 'instance')),
    resource_id uuid NOT NULL CHECK (resource_id <> '00000000-0000-0000-0000-000000000000'),
    revision_id uuid CHECK (revision_id <> '00000000-0000-0000-0000-000000000000'),
    filesystem_uuid uuid CHECK (filesystem_uuid <> '00000000-0000-0000-0000-000000000000'),
    ownership text NOT NULL CHECK (ownership IN ('owned', 'external')),
    removal_policy text NOT NULL CHECK (removal_policy IN ('retain', 'delete')),
    plan_json jsonb NOT NULL CHECK (jsonb_typeof(plan_json) = 'object'),
    input_hash bytea NOT NULL CHECK (octet_length(input_hash) = 32),
    install_progress text NOT NULL DEFAULT 'pending' CHECK (install_progress IN
        ('pending', 'creating', 'ready', 'failed', 'recovery_required')),
    removal_progress text NOT NULL DEFAULT 'pending' CHECK (removal_progress IN
        ('pending', 'draining', 'detached', 'retained', 'deleted', 'recovery_required')),
    version bigint NOT NULL DEFAULT 0 CHECK (version >= 0),
    active_attempt_id uuid,
    diagnostic text CHECK (diagnostic IN ('provider_failure', 'provider_outcome_unknown',
        'dependency_unavailable', 'authorization_changed', 'drain_pending', 'fencing_unproven', 'intent_conflict')),
    PRIMARY KEY (deployment_id, resource_name),
    UNIQUE (deployment_id, resource_id),
    FOREIGN KEY (deployment_id, project_id) REFERENCES recipe_deployments(id, project_id),
    CHECK ((resource_kind = 'instance' AND ownership = 'owned' AND revision_id IS NOT NULL AND filesystem_uuid IS NULL)
        OR (resource_kind = 'volume' AND revision_id IS NULL
            AND ((ownership = 'owned' AND filesystem_uuid IS NOT NULL)
                OR (ownership = 'external' AND filesystem_uuid IS NULL)))),
    CHECK (ownership <> 'external' OR (removal_policy = 'retain' AND removal_progress = 'retained')),
    CHECK (removal_progress <> 'deleted' OR (ownership = 'owned' AND removal_policy = 'delete')),
    CHECK (removal_progress <> 'retained' OR removal_policy = 'retain')
);

CREATE TABLE recipe_deployment_commands (
    id uuid PRIMARY KEY CHECK (id <> '00000000-0000-0000-0000-000000000000'),
    deployment_id uuid NOT NULL,
    project_id uuid NOT NULL,
    actor_id uuid NOT NULL REFERENCES users(id),
    operation text NOT NULL CHECK (operation IN ('install', 'remove')),
    idempotency_id uuid NOT NULL CHECK (idempotency_id <> '00000000-0000-0000-0000-000000000000'),
    input_hash bytea NOT NULL CHECK (octet_length(input_hash) = 32),
    first_request_id uuid NOT NULL CHECK (first_request_id <> '00000000-0000-0000-0000-000000000000'),
    receipt_lifecycle text NOT NULL CHECK (receipt_lifecycle IN ('installing', 'removing', 'removed')),
    receipt_version bigint NOT NULL CHECK (receipt_version >= 0),
    event_id uuid NOT NULL,
    event_cursor bigint NOT NULL CHECK (event_cursor > 0),
    event_aggregate_version bigint NOT NULL CHECK (event_aggregate_version > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (actor_id, operation, idempotency_id),
    UNIQUE (id, deployment_id, project_id),
    FOREIGN KEY (deployment_id, project_id) REFERENCES recipe_deployments(id, project_id)
);

-- Every admission attempt is append-only; replay never overwrites first provenance.
CREATE TABLE recipe_deployment_admission_attempts (
    id uuid PRIMARY KEY CHECK (id <> '00000000-0000-0000-0000-000000000000'),
    command_id uuid NOT NULL,
    deployment_id uuid NOT NULL,
    project_id uuid NOT NULL,
    actor_id uuid NOT NULL REFERENCES users(id),
    request_id uuid NOT NULL CHECK (request_id <> '00000000-0000-0000-0000-000000000000'),
    disposition text NOT NULL CHECK (disposition IN ('created', 'resume', 'replay')),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (command_id, deployment_id, project_id)
        REFERENCES recipe_deployment_commands(id, deployment_id, project_id)
);

CREATE FUNCTION recipe_ledger_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    RAISE EXCEPTION 'recipe ledger evidence is immutable' USING ERRCODE = 'integrity_constraint_violation';
END $$;
REVOKE ALL ON FUNCTION recipe_ledger_immutable() FROM PUBLIC;
CREATE TRIGGER recipe_definition_immutable BEFORE UPDATE OR DELETE ON recipe_definitions
FOR EACH ROW EXECUTE FUNCTION recipe_ledger_immutable();
CREATE TRIGGER recipe_command_immutable BEFORE UPDATE OR DELETE ON recipe_deployment_commands
FOR EACH ROW EXECUTE FUNCTION recipe_ledger_immutable();
CREATE TRIGGER recipe_admission_attempt_immutable BEFORE UPDATE OR DELETE ON recipe_deployment_admission_attempts
FOR EACH ROW EXECUTE FUNCTION recipe_ledger_immutable();

CREATE FUNCTION recipe_deployment_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'deployment keys and tombstones cannot be reused' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF NEW.lifecycle <> 'installing' OR NEW.version <> 0
           OR NOT EXISTS (SELECT 1 FROM recipe_definitions definition
               WHERE definition.project_id = NEW.project_id AND definition.recipe_id = NEW.recipe_id
                 AND definition.recipe_version = NEW.recipe_version
                 AND definition.declaration_hash = NEW.declaration_hash) THEN
            RAISE EXCEPTION 'deployment must begin at pending admission' USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    ELSE
        IF (to_jsonb(NEW) - ARRAY['lifecycle', 'version']) IS DISTINCT FROM
           (to_jsonb(OLD) - ARRAY['lifecycle', 'version']) OR NEW.version <> OLD.version + 1
           OR OLD.lifecycle = 'removed'
           OR NOT ((OLD.lifecycle = 'installing' AND NEW.lifecycle IN ('installed', 'removing', 'recovery_required'))
                OR (OLD.lifecycle = 'installed' AND NEW.lifecycle IN ('removing', 'recovery_required'))
                OR (OLD.lifecycle = 'removing' AND NEW.lifecycle IN ('removed', 'recovery_required'))
                OR (OLD.lifecycle = 'recovery_required' AND NEW.lifecycle IN ('installing', 'removing'))) THEN
            RAISE EXCEPTION 'invalid deployment identity or transition' USING ERRCODE = 'integrity_constraint_violation';
        END IF;
        IF NEW.lifecycle = 'installed' AND EXISTS (SELECT 1 FROM recipe_deployment_resources
            WHERE deployment_id = NEW.id AND install_progress <> 'ready') THEN
            RAISE EXCEPTION 'deployment resources are not installed' USING ERRCODE = 'integrity_constraint_violation';
        END IF;
        IF NEW.lifecycle = 'removed' AND EXISTS (SELECT 1 FROM recipe_deployment_resources
            WHERE deployment_id = NEW.id AND removal_progress NOT IN ('retained', 'deleted')) THEN
            RAISE EXCEPTION 'deployment resources are not safely removed' USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION recipe_deployment_guard() FROM PUBLIC;
CREATE TRIGGER recipe_deployment_guard BEFORE INSERT OR UPDATE OR DELETE ON recipe_deployments
FOR EACH ROW EXECUTE FUNCTION recipe_deployment_guard();

CREATE FUNCTION recipe_resource_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
DECLARE admitted_count integer;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'resource provenance cannot be erased' USING ERRCODE = 'integrity_constraint_violation';
    ELSIF TG_OP = 'INSERT' THEN
        SELECT resource_count INTO admitted_count FROM recipe_deployments WHERE id = NEW.deployment_id;
        IF (SELECT count(*) FROM recipe_deployment_resources WHERE deployment_id = NEW.deployment_id) >= admitted_count
           OR NEW.install_progress <> 'pending' OR NEW.version <> 0 OR NEW.active_attempt_id IS NOT NULL
           OR NEW.diagnostic IS NOT NULL OR NEW.removal_progress <>
               (CASE WHEN NEW.ownership = 'external' THEN 'retained' ELSE 'pending' END) THEN
            RAISE EXCEPTION 'invalid initial resource progress' USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    ELSE
        IF (to_jsonb(NEW) - ARRAY['install_progress', 'removal_progress', 'version', 'active_attempt_id', 'diagnostic'])
           IS DISTINCT FROM (to_jsonb(OLD) - ARRAY['install_progress', 'removal_progress', 'version', 'active_attempt_id', 'diagnostic'])
           OR NEW.version <> OLD.version + 1 THEN
            RAISE EXCEPTION 'resource intent and progress fence are immutable' USING ERRCODE = 'integrity_constraint_violation';
        END IF;
        -- Effects are unavailable in the admission-only checkpoint. A later
        -- migration adds exact attempt/generation and transition proof guards.
        RAISE EXCEPTION 'resource effects require the execution ledger' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION recipe_resource_guard() FROM PUBLIC;
CREATE TRIGGER recipe_resource_guard BEFORE INSERT OR UPDATE OR DELETE ON recipe_deployment_resources
FOR EACH ROW EXECUTE FUNCTION recipe_resource_guard();

CREATE FUNCTION recipe_command_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF NEW.actor_id::text <> hephaestus_actor_id()
       OR NEW.first_request_id::text <> current_setting('hephaestus.request_id', true)
       OR NEW.idempotency_id::text <> current_setting('hephaestus.occurrence_id', true)
       OR NOT EXISTS (SELECT 1 FROM application_events event
          WHERE event.id = NEW.event_id AND event.scope_kind = 'project'
            AND event.scope_id = NEW.project_id AND event.aggregate_type = 'project'
            AND event.aggregate_id = NEW.project_id AND event.event_type = 'project.changed'
            AND event.actor_id = NEW.actor_id AND event.request_id = NEW.first_request_id
            AND event.occurrence_id = NEW.idempotency_id AND event.cursor = NEW.event_cursor
            AND event.aggregate_version = NEW.event_aggregate_version)
       OR NOT EXISTS (SELECT 1 FROM recipe_deployments deployment
          WHERE deployment.id = NEW.deployment_id AND deployment.version = NEW.receipt_version
            AND deployment.lifecycle = NEW.receipt_lifecycle)
       OR (NEW.operation = 'install' AND NEW.receipt_lifecycle <> 'installing')
       OR (NEW.operation = 'remove' AND NEW.receipt_lifecycle <> 'removing') THEN
        RAISE EXCEPTION 'recipe command requires exact committed event provenance' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION recipe_command_guard() FROM PUBLIC;
CREATE TRIGGER recipe_command_guard BEFORE INSERT ON recipe_deployment_commands
FOR EACH ROW EXECUTE FUNCTION recipe_command_guard();

CREATE FUNCTION recipe_deployment_commit_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM recipe_deployment_commands command
        WHERE command.deployment_id = NEW.id AND command.project_id = NEW.project_id
          AND command.receipt_version = NEW.version AND command.receipt_lifecycle = NEW.lifecycle
          AND command.actor_id::text = hephaestus_actor_id()
          AND command.first_request_id::text = current_setting('hephaestus.request_id', true))
       OR (SELECT count(*) FROM recipe_deployment_resources WHERE deployment_id = NEW.id) <> NEW.resource_count
       OR NOT EXISTS (SELECT 1 FROM authorization_audit_events audit
          WHERE audit.actor_id::text = hephaestus_actor_id() AND audit.permission = 'can_manage'
            AND audit.object_type = 'project' AND audit.object_id = NEW.project_id AND audit.decision = 'allow'
            AND audit.request_id::text = current_setting('hephaestus.request_id', true)) THEN
        RAISE EXCEPTION 'recipe state requires audited admission and committed receipt' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION recipe_deployment_commit_guard() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER recipe_deployment_commit_guard AFTER INSERT OR UPDATE ON recipe_deployments
DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION recipe_deployment_commit_guard();

DO $$
DECLARE ledger_table text;
BEGIN
    FOREACH ledger_table IN ARRAY ARRAY['recipe_definitions', 'recipe_deployments', 'recipe_deployment_resources',
        'recipe_deployment_commands', 'recipe_deployment_admission_attempts'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', ledger_table);
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', ledger_table);
        EXECUTE format('GRANT SELECT, INSERT ON %I TO hephaestus_app', ledger_table);
        EXECUTE format('CREATE POLICY recipe_ledger_read ON %I FOR SELECT TO hephaestus_app USING
            (check_permission(''user'', hephaestus_actor_id(), ''can_manage'', ''project'', project_id::text) = 1)', ledger_table);
        EXECUTE format('CREATE POLICY recipe_ledger_insert ON %I FOR INSERT TO hephaestus_app WITH CHECK
            (check_permission(''user'', hephaestus_actor_id(), ''can_manage'', ''project'', project_id::text) = 1)', ledger_table);
    END LOOP;
END $$;
GRANT UPDATE ON recipe_deployments TO hephaestus_app;
CREATE POLICY recipe_deployment_update ON recipe_deployments FOR UPDATE TO hephaestus_app
USING (check_permission('user', hephaestus_actor_id(), 'can_manage', 'project', project_id::text) = 1)
WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_manage', 'project', project_id::text) = 1);
CREATE POLICY recipe_definition_actor ON recipe_definitions AS RESTRICTIVE FOR INSERT TO hephaestus_app
WITH CHECK (created_by::text = hephaestus_actor_id() AND first_request_id::text = current_setting('hephaestus.request_id', true));
CREATE POLICY recipe_deployment_actor ON recipe_deployments AS RESTRICTIVE FOR INSERT TO hephaestus_app
WITH CHECK (created_by::text = hephaestus_actor_id() AND first_request_id::text = current_setting('hephaestus.request_id', true));
CREATE POLICY recipe_command_actor ON recipe_deployment_commands AS RESTRICTIVE FOR INSERT TO hephaestus_app
WITH CHECK (actor_id::text = hephaestus_actor_id());
CREATE POLICY recipe_admission_actor ON recipe_deployment_admission_attempts AS RESTRICTIVE FOR INSERT TO hephaestus_app
WITH CHECK (actor_id::text = hephaestus_actor_id() AND request_id::text = current_setting('hephaestus.request_id', true)
    AND EXISTS (SELECT 1 FROM recipe_deployment_commands command
        WHERE command.id = command_id AND command.actor_id::text = hephaestus_actor_id()));

-- Discover only the authorization target before reading a protected snapshot.
-- This supplies no permission or catalog facts; the adapter still audits the
-- exact live Project CanManage decision, including rejected removal/replay.
CREATE FUNCTION recipe_deployment_project(deployment uuid) RETURNS uuid
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT project_id FROM recipe_deployments WHERE id = deployment
      AND NULLIF(current_setting('hephaestus.actor_id', true), '') IS NOT NULL
      AND current_setting('hephaestus.subject_type', true) = 'user'
$$;
REVOKE ALL ON FUNCTION recipe_deployment_project(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION recipe_deployment_project(uuid) TO hephaestus_app;

-- Exact cleanup ownership discovery, including objects hidden by their RLS.
-- Only project identity is returned; cleanup still requires live object rights.
CREATE FUNCTION recipe_owned_resource_project(kind text, resource uuid) RETURNS uuid
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT capability_resource_project(kind, resource)
    WHERE kind IN ('state_volume', 'agent_instance')
      AND NULLIF(current_setting('hephaestus.actor_id', true), '') IS NOT NULL
      AND current_setting('hephaestus.subject_type', true) = 'user'
$$;
REVOKE ALL ON FUNCTION recipe_owned_resource_project(text, uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION recipe_owned_resource_project(text, uuid) TO hephaestus_app;
