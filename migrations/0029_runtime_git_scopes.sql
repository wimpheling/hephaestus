-- Typed release ceilings, exact immutable revision bindings, and dispatch
-- snapshots for runtime Git. Ordinary repository capability rows remain the
-- identity/operation envelope; these rows carry the receive policy that may
-- only attenuate it.

CREATE FUNCTION git_operations_are_valid(operations text[]) RETURNS boolean
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT cardinality(operations) BETWEEN 1 AND 3
       AND operations <@ ARRAY['discover', 'fetch', 'receive']::text[]
       AND cardinality(operations) = (
           SELECT count(DISTINCT operation) FROM unnest(operations) AS operation
       )
$$;

CREATE FUNCTION git_scope_values_are_unique(values_to_check text[]) RETURNS boolean
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT cardinality(values_to_check) = count(DISTINCT value)
    FROM unnest(values_to_check) AS value
$$;

CREATE FUNCTION git_scope_values_are_bounded(
    values_to_check text[], maximum_bytes integer
) RETURNS boolean
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT COALESCE(bool_and(octet_length(value) BETWEEN 1 AND maximum_bytes), true)
    FROM unnest(values_to_check) AS value
$$;

CREATE TABLE release_git_capability_ceilings (
    requirement_id uuid PRIMARY KEY,
    release_agent_id uuid NOT NULL,
    grammar_version smallint NOT NULL CHECK (grammar_version > 0),
    git_operations text[] NOT NULL CHECK (git_operations_are_valid(git_operations)),
    ref_globs text[] NOT NULL CHECK (cardinality(ref_globs) BETWEEN 1 AND 256),
    changed_path_globs text[] NOT NULL CHECK (cardinality(changed_path_globs) <= 256),
    branch_update_policy text NOT NULL CHECK (
        branch_update_policy IN ('fast_forward_only', 'allow_force')
    ),
    branch_create boolean NOT NULL,
    branch_delete boolean NOT NULL,
    tag_create boolean NOT NULL,
    tag_update boolean NOT NULL,
    tag_delete boolean NOT NULL,
    other_create boolean NOT NULL,
    other_update boolean NOT NULL,
    other_delete boolean NOT NULL,
    request_bytes bigint NOT NULL CHECK (request_bytes BETWEEN 1 AND 16777216),
    pack_bytes bigint NOT NULL CHECK (pack_bytes BETWEEN 1 AND 1073741824),
    object_count integer NOT NULL CHECK (object_count BETWEEN 1 AND 1000000),
    ref_updates integer NOT NULL CHECK (ref_updates BETWEEN 1 AND 256),
    exact_parent_required boolean NOT NULL,
    normalized_hash bytea NOT NULL CHECK (octet_length(normalized_hash) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (requirement_id, release_agent_id)
        REFERENCES release_capability_requirements(id, release_agent_id),
    CHECK (git_scope_values_are_unique(ref_globs)),
    CHECK (git_scope_values_are_unique(changed_path_globs)),
    CHECK (git_scope_values_are_bounded(ref_globs, 512)),
    CHECK (git_scope_values_are_bounded(changed_path_globs, 1024)),
    CHECK (
        ('receive' = ANY(git_operations)) = (cardinality(changed_path_globs) > 0)
    ),
    CHECK (NOT exact_parent_required OR 'receive' = ANY(git_operations))
);

CREATE FUNCTION enforce_release_git_ceiling() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    requirement release_capability_requirements%ROWTYPE;
    declared_operations text[];
BEGIN
    SELECT * INTO requirement FROM release_capability_requirements
    WHERE id = NEW.requirement_id AND release_agent_id = NEW.release_agent_id;
    declared_operations := requirement.required_operations
        || requirement.optional_operations;
    IF requirement.id IS NULL OR requirement.resource_kind <> 'repository'
       OR (('discover' = ANY(NEW.git_operations)
            OR 'fetch' = ANY(NEW.git_operations))
           AND NOT ('git_read' = ANY(declared_operations)))
       OR ('receive' = ANY(NEW.git_operations)
           AND NOT ('update_ref' = ANY(declared_operations)))
       OR (NEW.branch_update_policy = 'allow_force'
           AND NOT ('force_update_ref' = ANY(declared_operations)))
       OR (NEW.branch_create AND NOT ('create_ref' = ANY(declared_operations)))
       OR (NEW.branch_delete AND NOT ('delete_ref' = ANY(declared_operations)))
       OR (NEW.tag_create AND NOT ('create_tag' = ANY(declared_operations)))
       OR (NEW.tag_update AND NOT ('force_update_ref' = ANY(declared_operations)))
       OR (NEW.tag_delete AND NOT ('delete_tag' = ANY(declared_operations)))
       OR (NEW.other_create AND NOT ('create_ref' = ANY(declared_operations)))
       OR (NEW.other_update AND NOT ('force_update_ref' = ANY(declared_operations)))
       OR (NEW.other_delete AND NOT ('delete_ref' = ANY(declared_operations)))
    THEN
        RAISE EXCEPTION 'Git ceiling requires an exact repository capability requirement'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_release_git_ceiling() FROM PUBLIC;
CREATE TRIGGER release_git_capability_ceilings_valid
BEFORE INSERT ON release_git_capability_ceilings
FOR EACH ROW EXECUTE FUNCTION enforce_release_git_ceiling();
CREATE TRIGGER release_git_capability_ceilings_immutable
BEFORE UPDATE OR DELETE ON release_git_capability_ceilings
FOR EACH ROW EXECUTE FUNCTION reject_capability_record_mutation();

CREATE TABLE agent_git_capability_bindings (
    binding_id uuid PRIMARY KEY,
    instance_revision_id uuid NOT NULL,
    requirement_id uuid NOT NULL,
    grammar_version smallint NOT NULL CHECK (grammar_version > 0),
    git_operations text[] NOT NULL CHECK (git_operations_are_valid(git_operations)),
    ref_globs text[] NOT NULL CHECK (cardinality(ref_globs) BETWEEN 1 AND 256),
    changed_path_globs text[] NOT NULL CHECK (cardinality(changed_path_globs) <= 256),
    branch_update_policy text NOT NULL CHECK (
        branch_update_policy IN ('fast_forward_only', 'allow_force')
    ),
    branch_create boolean NOT NULL,
    branch_delete boolean NOT NULL,
    tag_create boolean NOT NULL,
    tag_update boolean NOT NULL,
    tag_delete boolean NOT NULL,
    other_create boolean NOT NULL,
    other_update boolean NOT NULL,
    other_delete boolean NOT NULL,
    request_bytes bigint NOT NULL CHECK (request_bytes BETWEEN 1 AND 16777216),
    pack_bytes bigint NOT NULL CHECK (pack_bytes BETWEEN 1 AND 1073741824),
    object_count integer NOT NULL CHECK (object_count BETWEEN 1 AND 1000000),
    ref_updates integer NOT NULL CHECK (ref_updates BETWEEN 1 AND 256),
    exact_parent_required boolean NOT NULL,
    normalized_hash bytea NOT NULL CHECK (octet_length(normalized_hash) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (binding_id, instance_revision_id)
        REFERENCES agent_capability_bindings(id, instance_revision_id),
    FOREIGN KEY (requirement_id)
        REFERENCES release_git_capability_ceilings(requirement_id),
    UNIQUE (binding_id, instance_revision_id),
    CHECK (git_scope_values_are_unique(ref_globs)),
    CHECK (git_scope_values_are_unique(changed_path_globs)),
    CHECK (git_scope_values_are_bounded(ref_globs, 512)),
    CHECK (git_scope_values_are_bounded(changed_path_globs, 1024)),
    CHECK (
        ('receive' = ANY(git_operations)) = (cardinality(changed_path_globs) > 0)
    ),
    CHECK (NOT exact_parent_required OR 'receive' = ANY(git_operations))
);

CREATE FUNCTION enforce_git_binding_attenuation() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    ceiling release_git_capability_ceilings%ROWTYPE;
    generic_binding agent_capability_bindings%ROWTYPE;
BEGIN
    SELECT * INTO ceiling FROM release_git_capability_ceilings
    WHERE requirement_id = NEW.requirement_id;
    SELECT * INTO generic_binding FROM agent_capability_bindings
    WHERE id = NEW.binding_id AND instance_revision_id = NEW.instance_revision_id;
    IF ceiling.requirement_id IS NULL OR generic_binding.id IS NULL
       OR generic_binding.requirement_id <> NEW.requirement_id
       OR generic_binding.resource_kind <> 'repository'
       OR (('discover' = ANY(NEW.git_operations)
            OR 'fetch' = ANY(NEW.git_operations))
           AND NOT ('git_read' = ANY(generic_binding.granted_operations)))
       OR ('receive' = ANY(NEW.git_operations)
           AND NOT ('update_ref' = ANY(generic_binding.granted_operations)))
       OR (NEW.branch_update_policy = 'allow_force'
           AND NOT ('force_update_ref' = ANY(generic_binding.granted_operations)))
       OR (NEW.branch_create
           AND NOT ('create_ref' = ANY(generic_binding.granted_operations)))
       OR (NEW.branch_delete
           AND NOT ('delete_ref' = ANY(generic_binding.granted_operations)))
       OR (NEW.tag_create
           AND NOT ('create_tag' = ANY(generic_binding.granted_operations)))
       OR (NEW.tag_update
           AND NOT ('force_update_ref' = ANY(generic_binding.granted_operations)))
       OR (NEW.tag_delete
           AND NOT ('delete_tag' = ANY(generic_binding.granted_operations)))
       OR (NEW.other_create
           AND NOT ('create_ref' = ANY(generic_binding.granted_operations)))
       OR (NEW.other_update
           AND NOT ('force_update_ref' = ANY(generic_binding.granted_operations)))
       OR (NEW.other_delete
           AND NOT ('delete_ref' = ANY(generic_binding.granted_operations)))
       OR NEW.grammar_version <> ceiling.grammar_version
       OR NOT (NEW.git_operations <@ ceiling.git_operations)
       OR NOT (NEW.ref_globs <@ ceiling.ref_globs)
       OR NOT (NEW.changed_path_globs <@ ceiling.changed_path_globs)
       OR (NEW.branch_update_policy = 'allow_force'
           AND ceiling.branch_update_policy <> 'allow_force')
       OR (NEW.branch_create AND NOT ceiling.branch_create)
       OR (NEW.branch_delete AND NOT ceiling.branch_delete)
       OR (NEW.tag_create AND NOT ceiling.tag_create)
       OR (NEW.tag_update AND NOT ceiling.tag_update)
       OR (NEW.tag_delete AND NOT ceiling.tag_delete)
       OR (NEW.other_create AND NOT ceiling.other_create)
       OR (NEW.other_update AND NOT ceiling.other_update)
       OR (NEW.other_delete AND NOT ceiling.other_delete)
       OR NEW.request_bytes > ceiling.request_bytes
       OR NEW.pack_bytes > ceiling.pack_bytes
       OR NEW.object_count > ceiling.object_count
       OR NEW.ref_updates > ceiling.ref_updates
       OR (ceiling.exact_parent_required AND NOT NEW.exact_parent_required)
    THEN
        RAISE EXCEPTION 'runtime Git binding exceeds or mismatches release ceiling'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_git_binding_attenuation() FROM PUBLIC;
CREATE TRIGGER agent_git_capability_bindings_attenuate
BEFORE INSERT ON agent_git_capability_bindings
FOR EACH ROW EXECUTE FUNCTION enforce_git_binding_attenuation();
CREATE TRIGGER agent_git_capability_bindings_immutable
BEFORE UPDATE OR DELETE ON agent_git_capability_bindings
FOR EACH ROW EXECUTE FUNCTION reject_capability_record_mutation();

CREATE FUNCTION require_typed_git_binding() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM release_git_capability_ceilings
        WHERE requirement_id = NEW.requirement_id
    ) AND NOT EXISTS (
        SELECT 1 FROM agent_git_capability_bindings
        WHERE binding_id = NEW.id AND instance_revision_id = NEW.instance_revision_id
    ) THEN
        RAISE EXCEPTION 'repository capability binding is missing its typed Git authority'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION require_typed_git_binding() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER agent_capability_bindings_require_typed_git
AFTER INSERT ON agent_capability_bindings
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION require_typed_git_binding();

CREATE TABLE run_git_authority_snapshots (
    snapshot_id uuid PRIMARY KEY,
    instance_revision_id uuid NOT NULL,
    binding_id uuid NOT NULL,
    repository_id uuid NOT NULL,
    grammar_version smallint NOT NULL CHECK (grammar_version > 0),
    git_operations text[] NOT NULL CHECK (git_operations_are_valid(git_operations)),
    ref_globs text[] NOT NULL,
    changed_path_globs text[] NOT NULL,
    branch_update_policy text NOT NULL CHECK (
        branch_update_policy IN ('fast_forward_only', 'allow_force')
    ),
    branch_create boolean NOT NULL,
    branch_delete boolean NOT NULL,
    tag_create boolean NOT NULL,
    tag_update boolean NOT NULL,
    tag_delete boolean NOT NULL,
    other_create boolean NOT NULL,
    other_update boolean NOT NULL,
    other_delete boolean NOT NULL,
    request_bytes bigint NOT NULL CHECK (request_bytes BETWEEN 1 AND 16777216),
    pack_bytes bigint NOT NULL CHECK (pack_bytes BETWEEN 1 AND 1073741824),
    object_count integer NOT NULL CHECK (object_count BETWEEN 1 AND 1000000),
    ref_updates integer NOT NULL CHECK (ref_updates BETWEEN 1 AND 256),
    exact_parent_required boolean NOT NULL,
    expected_parent text CHECK (
        expected_parent IS NULL
        OR expected_parent ~ '^[0-9a-f]{40}$'
        OR expected_parent ~ '^[0-9a-f]{64}$'
    ),
    normalized_hash bytea NOT NULL CHECK (octet_length(normalized_hash) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (snapshot_id, instance_revision_id)
        REFERENCES run_authorization_snapshots(id, instance_revision_id),
    FOREIGN KEY (snapshot_id, binding_id)
        REFERENCES run_authorization_snapshot_bindings(snapshot_id, binding_id),
    FOREIGN KEY (binding_id, instance_revision_id)
        REFERENCES agent_git_capability_bindings(binding_id, instance_revision_id),
    FOREIGN KEY (repository_id) REFERENCES repositories(id),
    CHECK (exact_parent_required = (expected_parent IS NOT NULL))
);

CREATE FUNCTION enforce_git_snapshot_copy() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    stored agent_git_capability_bindings%ROWTYPE;
    generic_binding agent_capability_bindings%ROWTYPE;
    snapshot_run_id uuid;
    trigger_repository_id uuid;
    trigger_commit text;
BEGIN
    SELECT * INTO stored FROM agent_git_capability_bindings
    WHERE binding_id = NEW.binding_id
      AND instance_revision_id = NEW.instance_revision_id;
    SELECT * INTO generic_binding FROM agent_capability_bindings
    WHERE id = NEW.binding_id
      AND instance_revision_id = NEW.instance_revision_id;
    SELECT run_id INTO snapshot_run_id FROM run_authorization_snapshots
    WHERE id = NEW.snapshot_id;
    SELECT target_repository_id, target_commit
      INTO trigger_repository_id, trigger_commit
    FROM run_instance_provenance WHERE run_id = snapshot_run_id;

    IF stored.binding_id IS NULL OR generic_binding.id IS NULL
       OR NEW.repository_id <> generic_binding.resource_id
       OR NEW.grammar_version <> stored.grammar_version
       OR NEW.git_operations <> stored.git_operations
       OR NEW.ref_globs <> stored.ref_globs
       OR NEW.changed_path_globs <> stored.changed_path_globs
       OR NEW.branch_update_policy <> stored.branch_update_policy
       OR NEW.branch_create <> stored.branch_create
       OR NEW.branch_delete <> stored.branch_delete
       OR NEW.tag_create <> stored.tag_create
       OR NEW.tag_update <> stored.tag_update
       OR NEW.tag_delete <> stored.tag_delete
       OR NEW.other_create <> stored.other_create
       OR NEW.other_update <> stored.other_update
       OR NEW.other_delete <> stored.other_delete
       OR NEW.request_bytes <> stored.request_bytes
       OR NEW.pack_bytes <> stored.pack_bytes
       OR NEW.object_count <> stored.object_count
       OR NEW.ref_updates <> stored.ref_updates
       OR NEW.exact_parent_required <> stored.exact_parent_required
       OR NEW.normalized_hash <> stored.normalized_hash
       OR (stored.exact_parent_required AND (
           trigger_repository_id IS DISTINCT FROM generic_binding.resource_id
           OR NEW.expected_parent IS DISTINCT FROM trigger_commit
       ))
       OR (NOT stored.exact_parent_required AND NEW.expected_parent IS NOT NULL)
    THEN
        RAISE EXCEPTION 'runtime Git snapshot does not exactly copy its immutable binding and trigger parent'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_git_snapshot_copy() FROM PUBLIC;
CREATE TRIGGER run_git_authority_snapshots_exact_copy
BEFORE INSERT ON run_git_authority_snapshots
FOR EACH ROW EXECUTE FUNCTION enforce_git_snapshot_copy();
CREATE TRIGGER run_git_authority_snapshots_immutable
BEFORE UPDATE OR DELETE ON run_git_authority_snapshots
FOR EACH ROW EXECUTE FUNCTION reject_runtime_authority_immutable_record();

GRANT SELECT ON release_git_capability_ceilings,
    agent_git_capability_bindings TO hephaestus_app, hephaestus_worker;
GRANT INSERT ON release_git_capability_ceilings TO hephaestus_worker;
GRANT INSERT ON agent_git_capability_bindings TO hephaestus_app, hephaestus_worker;
GRANT SELECT, INSERT ON run_git_authority_snapshots TO hephaestus_worker;

ALTER TABLE release_git_capability_ceilings ENABLE ROW LEVEL SECURITY;
ALTER TABLE release_git_capability_ceilings FORCE ROW LEVEL SECURITY;
CREATE POLICY release_git_capability_ceilings_worker
    ON release_git_capability_ceilings TO hephaestus_worker
    USING (true) WITH CHECK (true);
CREATE POLICY release_git_capability_ceilings_user_select
    ON release_git_capability_ceilings FOR SELECT TO hephaestus_app
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'release_agent', release_agent_id::text
    ) = 1);

ALTER TABLE agent_git_capability_bindings ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_git_capability_bindings FORCE ROW LEVEL SECURITY;
CREATE POLICY agent_git_capability_bindings_worker
    ON agent_git_capability_bindings TO hephaestus_worker
    USING (true) WITH CHECK (true);
CREATE POLICY agent_git_capability_bindings_user_select
    ON agent_git_capability_bindings FOR SELECT TO hephaestus_app
    USING (EXISTS (
        SELECT 1 FROM agent_capability_bindings AS binding
        JOIN agent_instance_revisions AS revision
          ON revision.id = binding.instance_revision_id
        WHERE binding.id = binding_id
          AND check_permission(
              'user', hephaestus_actor_id(), 'can_read',
              'agent_instance', revision.instance_id::text
          ) = 1
    ));
CREATE POLICY agent_git_capability_bindings_user_insert
    ON agent_git_capability_bindings FOR INSERT TO hephaestus_app
    WITH CHECK (EXISTS (
        SELECT 1 FROM agent_capability_bindings AS binding
        JOIN agent_instance_revisions AS revision
          ON revision.id = binding.instance_revision_id
        WHERE binding.id = binding_id
          AND check_permission(
              'user', hephaestus_actor_id(), 'can_manage',
              'agent_instance', revision.instance_id::text
          ) = 1
    ));

ALTER TABLE run_git_authority_snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_git_authority_snapshots FORCE ROW LEVEL SECURITY;
CREATE POLICY run_git_authority_snapshots_worker
    ON run_git_authority_snapshots TO hephaestus_worker
    USING (true) WITH CHECK (true);

COMMENT ON TABLE release_git_capability_ceilings IS
    'Versioned normalized Git policy ceilings declared by released repository slots.';
COMMENT ON TABLE agent_git_capability_bindings IS
    'Exact immutable attenuations bound to one repository through the generic capability row.';
COMMENT ON TABLE run_git_authority_snapshots IS
    'Historical dispatch-time copies, including an optional trigger-bound expected parent.';
