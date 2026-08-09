-- Durable gateway declarations are the authority. Provider configuration is
-- derived state and is deliberately absent from this migration.

CREATE FUNCTION gateway_text_array_is_unique(values text[]) RETURNS boolean
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT cardinality(values) = (SELECT count(DISTINCT value) FROM unnest(values) AS value)
$$;
REVOKE ALL ON FUNCTION gateway_text_array_is_unique(text[]) FROM PUBLIC;

CREATE TABLE gateways (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id),
    repository_id uuid NOT NULL REFERENCES repositories(id),
    name text NOT NULL CHECK (name ~ '^[a-z][a-z0-9_-]{0,63}$'),
    lifecycle text NOT NULL CHECK (lifecycle IN ('enabled', 'paused', 'removed')),
    active_revision_id uuid,
    removed_at timestamptz,
    created_by uuid NOT NULL REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (repository_id, name),
    UNIQUE (id, project_id),
    CHECK ((lifecycle = 'removed') = (removed_at IS NOT NULL))
);
CREATE INDEX gateways_project_lifecycle ON gateways (project_id, lifecycle, created_at DESC, id);

CREATE TABLE gateway_revisions (
    id uuid PRIMARY KEY,
    gateway_id uuid NOT NULL REFERENCES gateways(id),
    project_id uuid NOT NULL REFERENCES projects(id),
    repository_id uuid NOT NULL REFERENCES repositories(id),
    release_id uuid REFERENCES releases(id),
    handler_contract text NOT NULL CHECK (handler_contract = 'http.v1'),
    exposure text NOT NULL CHECK (exposure IN ('public', 'heph_authenticated')),
    parameters jsonb NOT NULL CHECK (jsonb_typeof(parameters) = 'object'),
    secret_slots text[] NOT NULL DEFAULT '{}' CHECK (cardinality(secret_slots) <= 32),
    normalized_hash bytea NOT NULL CHECK (octet_length(normalized_hash) = 32),
    created_by uuid NOT NULL REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (id, gateway_id),
    UNIQUE (id, project_id),
    UNIQUE (gateway_id, normalized_hash),
    FOREIGN KEY (gateway_id, project_id) REFERENCES gateways(id, project_id)
);
CREATE INDEX gateway_revisions_gateway_created ON gateway_revisions (gateway_id, created_at DESC, id);
ALTER TABLE gateways ADD CONSTRAINT gateways_active_revision_fk
    FOREIGN KEY (active_revision_id, id) REFERENCES gateway_revisions(id, gateway_id)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE gateway_routes (
    id uuid PRIMARY KEY,
    gateway_revision_id uuid NOT NULL,
    gateway_id uuid NOT NULL,
    project_id uuid NOT NULL REFERENCES projects(id),
    path text NOT NULL CHECK (path ~ '^/[A-Za-z0-9._~-]+(?:/[A-Za-z0-9._~-]+)*$'),
    methods text[] NOT NULL CHECK (cardinality(methods) BETWEEN 1 AND 7),
    enabled boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (id, gateway_revision_id),
    UNIQUE (gateway_revision_id, path),
    FOREIGN KEY (gateway_revision_id, gateway_id) REFERENCES gateway_revisions(id, gateway_id),
    FOREIGN KEY (gateway_id, project_id) REFERENCES gateways(id, project_id),
    CHECK (methods <@ ARRAY['GET','POST','PUT','PATCH','DELETE','HEAD','OPTIONS']::text[]),
    CHECK (gateway_text_array_is_unique(methods))
);
CREATE INDEX gateway_routes_active_path ON gateway_routes (project_id, path) WHERE enabled;

CREATE TABLE gateway_lifecycle_transitions (
    id uuid PRIMARY KEY,
    gateway_id uuid NOT NULL REFERENCES gateways(id),
    project_id uuid NOT NULL REFERENCES projects(id),
    from_lifecycle text NOT NULL CHECK (from_lifecycle IN ('enabled', 'paused', 'removed')),
    to_lifecycle text NOT NULL CHECK (to_lifecycle IN ('enabled', 'paused', 'removed')),
    actor_id uuid NOT NULL REFERENCES users(id),
    correlation_id uuid,
    occurred_at timestamptz NOT NULL DEFAULT now(),
    CHECK (from_lifecycle <> to_lifecycle)
);

CREATE TABLE gateway_derived_configurations (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id),
    desired_hash bytea NOT NULL CHECK (octet_length(desired_hash) = 32),
    observed_hash bytea CHECK (observed_hash IS NULL OR octet_length(observed_hash) = 32),
    state text NOT NULL CHECK (state IN ('pending', 'applied', 'failed')),
    reconciliation_id uuid NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    observed_at timestamptz
);

CREATE FUNCTION enforce_gateway_revision_integrity() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public AS $$
DECLARE gateway_project uuid; gateway_repository uuid;
BEGIN
  SELECT project_id, repository_id INTO gateway_project, gateway_repository FROM gateways WHERE id = NEW.gateway_id;
  IF gateway_project IS NULL OR gateway_project <> NEW.project_id OR gateway_repository <> NEW.repository_id THEN
    RAISE EXCEPTION 'gateway revision crosses its durable gateway boundary' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  IF NEW.release_id IS NOT NULL AND NOT EXISTS (
      SELECT 1 FROM releases WHERE id = NEW.release_id AND repository_id = NEW.repository_id
  ) THEN
    RAISE EXCEPTION 'gateway revision release is outside its repository' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_gateway_revision_integrity() FROM PUBLIC;
CREATE TRIGGER gateway_revisions_integrity BEFORE INSERT ON gateway_revisions FOR EACH ROW EXECUTE FUNCTION enforce_gateway_revision_integrity();

CREATE FUNCTION gateway_transition_lifecycle(p_gateway_id uuid, p_expected text, p_next text, p_actor_id uuid, p_correlation_id uuid DEFAULT NULL) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE changed integer;
BEGIN
  UPDATE gateways SET lifecycle = p_next, removed_at = CASE WHEN p_next = 'removed' THEN now() ELSE NULL END, updated_at = now()
   WHERE id = p_gateway_id AND lifecycle = p_expected AND lifecycle <> 'removed';
  GET DIAGNOSTICS changed = ROW_COUNT;
  IF changed = 1 THEN INSERT INTO gateway_lifecycle_transitions(id, gateway_id, project_id, from_lifecycle, to_lifecycle, actor_id, correlation_id)
    SELECT gen_random_uuid(), id, project_id, p_expected, p_next, p_actor_id, p_correlation_id FROM gateways WHERE id = p_gateway_id; END IF;
  RETURN changed = 1;
END $$;
REVOKE ALL ON FUNCTION gateway_transition_lifecycle(uuid, text, text, uuid, uuid) FROM PUBLIC;

CREATE FUNCTION reject_gateway_revision_mutation() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
  RAISE EXCEPTION 'gateway revisions and route declarations are immutable' USING ERRCODE = 'integrity_constraint_violation'; END $$;
CREATE TRIGGER gateway_revisions_immutable BEFORE UPDATE OR DELETE ON gateway_revisions FOR EACH ROW EXECUTE FUNCTION reject_gateway_revision_mutation();
CREATE TRIGGER gateway_routes_immutable BEFORE UPDATE OR DELETE ON gateway_routes FOR EACH ROW EXECUTE FUNCTION reject_gateway_revision_mutation();

-- An enabled current route reserves the exact path within its project. The
-- provider may choose hosts later, but cannot make two active targets win.
CREATE FUNCTION enforce_gateway_route_conflict() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public AS $$
BEGIN
  IF NEW.enabled AND EXISTS (SELECT 1 FROM gateway_routes route JOIN gateways gateway ON gateway.active_revision_id = route.gateway_revision_id
      WHERE route.project_id = NEW.project_id AND route.path = NEW.path AND route.enabled AND route.gateway_id <> NEW.gateway_id) THEN
    RAISE EXCEPTION 'active gateway route conflicts with an existing gateway' USING ERRCODE = 'unique_violation';
  END IF;
  RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_gateway_route_conflict() FROM PUBLIC;
CREATE TRIGGER gateway_routes_conflict BEFORE INSERT ON gateway_routes FOR EACH ROW EXECUTE FUNCTION enforce_gateway_route_conflict();

CREATE FUNCTION enforce_gateway_activation_conflict() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public AS $$
BEGIN
  IF NEW.active_revision_id IS NOT NULL AND NEW.lifecycle = 'enabled' AND EXISTS (
      SELECT 1 FROM gateway_routes candidate
      JOIN gateway_routes active ON active.project_id = NEW.project_id AND active.path = candidate.path AND active.enabled
      JOIN gateways other ON other.active_revision_id = active.gateway_revision_id
      WHERE candidate.gateway_revision_id = NEW.active_revision_id AND candidate.enabled
        AND other.id <> NEW.id AND other.lifecycle = 'enabled'
  ) THEN
    RAISE EXCEPTION 'gateway activation conflicts with an active route' USING ERRCODE = 'unique_violation';
  END IF;
  RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_gateway_activation_conflict() FROM PUBLIC;
CREATE TRIGGER gateways_activation_conflict BEFORE INSERT OR UPDATE OF active_revision_id, lifecycle ON gateways
FOR EACH ROW EXECUTE FUNCTION enforce_gateway_activation_conflict();

ALTER TABLE gateways ENABLE ROW LEVEL SECURITY; ALTER TABLE gateways FORCE ROW LEVEL SECURITY;
CREATE POLICY gateways_read ON gateways FOR SELECT TO hephaestus_app USING (check_permission('user', hephaestus_actor_id(), 'can_read', 'project', project_id::text) = 1);
CREATE POLICY gateways_manage ON gateways FOR ALL TO hephaestus_app USING (check_permission('user', hephaestus_actor_id(), 'can_manage', 'project', project_id::text) = 1) WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_manage', 'project', project_id::text) = 1);
ALTER TABLE gateway_revisions ENABLE ROW LEVEL SECURITY; ALTER TABLE gateway_revisions FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_revisions_read ON gateway_revisions FOR SELECT TO hephaestus_app USING (check_permission('user', hephaestus_actor_id(), 'can_read', 'project', project_id::text) = 1);
CREATE POLICY gateway_revisions_manage ON gateway_revisions FOR ALL TO hephaestus_app USING (check_permission('user', hephaestus_actor_id(), 'can_manage', 'project', project_id::text) = 1) WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_manage', 'project', project_id::text) = 1);
ALTER TABLE gateway_routes ENABLE ROW LEVEL SECURITY; ALTER TABLE gateway_routes FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_routes_read ON gateway_routes FOR SELECT TO hephaestus_app USING (check_permission('user', hephaestus_actor_id(), 'can_read', 'project', project_id::text) = 1);
CREATE POLICY gateway_routes_manage ON gateway_routes FOR ALL TO hephaestus_app USING (check_permission('user', hephaestus_actor_id(), 'can_manage', 'project', project_id::text) = 1) WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_manage', 'project', project_id::text) = 1);
ALTER TABLE gateway_lifecycle_transitions ENABLE ROW LEVEL SECURITY; ALTER TABLE gateway_lifecycle_transitions FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_lifecycle_read ON gateway_lifecycle_transitions FOR SELECT TO hephaestus_app USING (check_permission('user', hephaestus_actor_id(), 'can_read', 'project', project_id::text) = 1);
ALTER TABLE gateway_derived_configurations ENABLE ROW LEVEL SECURITY; ALTER TABLE gateway_derived_configurations FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_derived_configuration_read ON gateway_derived_configurations FOR SELECT TO hephaestus_app USING (check_permission('user', hephaestus_actor_id(), 'can_read', 'project', project_id::text) = 1);

-- Gateway resources now have exact tenant integrity in existing immutable
-- capability bindings. The generic model remains intentionally unchanged.
CREATE OR REPLACE FUNCTION capability_resource_project(p_kind text, p_id uuid) RETURNS uuid LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
  CASE p_kind
    WHEN 'repository' THEN RETURN (SELECT project_id FROM repositories WHERE id = p_id);
    WHEN 'project' THEN RETURN (SELECT id FROM projects WHERE id = p_id);
    WHEN 'agent_instance' THEN RETURN (SELECT project_id FROM agent_instances WHERE id = p_id);
    WHEN 'gateway' THEN RETURN (SELECT project_id FROM gateways WHERE id = p_id AND lifecycle <> 'removed');
    WHEN 'run' THEN RETURN (SELECT instance.project_id FROM runs run JOIN agent_instances instance ON instance.id = run.instance_id WHERE run.id = p_id);
    WHEN 'state_volume' THEN RETURN (SELECT instance.project_id FROM agent_instance_state_volumes volume JOIN agent_instances instance ON instance.id = volume.instance_id WHERE volume.id = p_id);
  END CASE; RETURN NULL;
END $$;

-- Supersede the pre-gateway trigger body while retaining every release-ceiling
-- check.  Gateway selection is now a real same-project resource, never a
-- synthetic tuple or a provider configuration object.
CREATE OR REPLACE FUNCTION enforce_capability_resource_integrity() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE consuming_project_id uuid; resource_project_id uuid; declared_slot_key text;
  declared_resource_kind text; declared_required_operations text[];
  declared_optional_operations text[]; declared_hash bytea;
BEGIN
  SELECT slot_key, resource_kind, required_operations, optional_operations, normalized_hash
    INTO declared_slot_key, declared_resource_kind, declared_required_operations,
         declared_optional_operations, declared_hash
    FROM release_capability_requirements WHERE id = NEW.requirement_id
      AND release_agent_id = NEW.release_agent_id;
  IF declared_slot_key IS NULL OR NEW.slot_key <> declared_slot_key
     OR NEW.resource_kind <> declared_resource_kind OR NEW.requirement_hash <> declared_hash
     OR NOT (NEW.granted_operations @> declared_required_operations)
     OR NOT (NEW.granted_operations <@ (declared_required_operations || declared_optional_operations)) THEN
    RAISE EXCEPTION 'capability binding exceeds or mismatches release requirement' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  SELECT instance.project_id INTO consuming_project_id FROM agent_instance_revisions revision
    JOIN agent_instances instance ON instance.id = revision.instance_id WHERE revision.id = NEW.instance_revision_id;
  IF consuming_project_id IS NULL THEN RAISE EXCEPTION 'capability binding revision is unavailable' USING ERRCODE = 'foreign_key_violation'; END IF;
  resource_project_id := capability_resource_project(NEW.resource_kind, NEW.resource_id);
  IF resource_project_id IS NULL THEN RAISE EXCEPTION 'capability resource is unavailable' USING ERRCODE = 'foreign_key_violation'; END IF;
  IF resource_project_id <> consuming_project_id THEN RAISE EXCEPTION 'cross-project capability binding requires a sharing contract' USING ERRCODE = 'integrity_constraint_violation'; END IF;
  RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_capability_resource_integrity() FROM PUBLIC;

-- The published relation model uses gateway.project for human lifecycle
-- authorization. Explicit agent gateway operations remain exact immutable
-- bindings and are exposed only while the consuming revision is active.
CREATE OR REPLACE VIEW melange_tuples (subject_type, subject_id, relation, object_type, object_id) AS
SELECT subject_type, subject_id, relation, object_type, object_id FROM melange_base_tuples
UNION ALL SELECT 'project', image.project_id::text, 'project', 'repository_oci_image', image.id::text FROM repository_oci_image_definitions image
UNION ALL SELECT 'user', granter.user_id::text, 'capability_granter', 'project', granter.project_id::text FROM project_capability_granters granter
UNION ALL SELECT 'project', gateway.project_id::text, 'project', 'gateway', gateway.id::text FROM gateways gateway
UNION ALL SELECT 'gateway', revision.gateway_id::text, 'gateway', 'gateway_revision', revision.id::text FROM gateway_revisions revision
UNION ALL SELECT 'agent_instance', revision.instance_id::text, 'agent_' || operation.name, binding.resource_kind, binding.resource_id::text
 FROM agent_capability_bindings binding JOIN agent_instance_revisions revision ON revision.id = binding.instance_revision_id
 JOIN agent_instances instance ON instance.id = revision.instance_id AND instance.active_revision_id = revision.id
 CROSS JOIN LATERAL unnest(binding.granted_operations) operation(name);
