-- Reusable release/instance model and non-disclosing secret delegation.
-- Existing POC `agents` remain temporarily for compatibility with Phase 3
-- run records; all new product behavior uses `agent_instances`.

ALTER TABLE projects ADD CONSTRAINT projects_id_organization_unique
    UNIQUE (id, organization_id);
ALTER TABLE repositories ADD CONSTRAINT repositories_id_project_unique
    UNIQUE (id, project_id);
ALTER TABLE agent_config_revisions
    ADD COLUMN normalized_config_hash text
    CHECK (
        normalized_config_hash IS NULL
        OR normalized_config_hash ~ '^[0-9a-f]{64}$'
    );

CREATE TABLE organization_secret_managers (
    organization_id uuid NOT NULL REFERENCES organizations(id),
    user_id uuid NOT NULL REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (organization_id, user_id)
);

CREATE TABLE project_secret_roles (
    project_id uuid NOT NULL REFERENCES projects(id),
    user_id uuid NOT NULL REFERENCES users(id),
    role text NOT NULL CHECK (
        role IN ('secret_manager', 'brokered_secret_binder', 'raw_secret_binder')
    ),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, user_id, role)
);

CREATE TABLE repository_managers (
    repository_id uuid NOT NULL REFERENCES repositories(id),
    user_id uuid NOT NULL REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (repository_id, user_id)
);

CREATE TABLE repository_secret_roles (
    repository_id uuid NOT NULL REFERENCES repositories(id),
    user_id uuid NOT NULL REFERENCES users(id),
    role text NOT NULL CHECK (
        role IN ('secret_manager', 'brokered_secret_binder', 'raw_secret_binder')
    ),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (repository_id, user_id, role)
);

CREATE TABLE agent_families (
    id uuid PRIMARY KEY,
    repository_id uuid NOT NULL REFERENCES repositories(id),
    agent_key text NOT NULL CHECK (
        agent_key ~ '^[a-z0-9][a-z0-9_-]{0,63}$'
    ),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (repository_id, agent_key),
    UNIQUE (id, repository_id)
);

CREATE TABLE build_requests (
    id uuid PRIMARY KEY,
    repository_id uuid NOT NULL REFERENCES repositories(id),
    source_commit text NOT NULL CHECK (
        source_commit ~ '^[0-9a-f]{40}$'
        OR source_commit ~ '^[0-9a-f]{64}$'
    ),
    source_ref text NOT NULL CHECK (source_ref LIKE 'refs/%'),
    origin_receive_id uuid REFERENCES git_receives(id),
    build_definition_hash bytea NOT NULL
        CHECK (octet_length(build_definition_hash) = 32),
    state text NOT NULL CHECK (
        state IN ('queued', 'running', 'importing', 'succeeded', 'failed', 'cancelled')
    ),
    diagnostics jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_by uuid REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    started_at timestamptz,
    completed_at timestamptz,
    UNIQUE (
        repository_id, source_commit, source_ref, build_definition_hash
    ),
    UNIQUE (id, repository_id)
);
CREATE INDEX build_requests_by_repository_state
    ON build_requests (repository_id, state, created_at, id);

CREATE TABLE build_request_sources (
    build_request_id uuid NOT NULL REFERENCES build_requests(id),
    receive_id uuid NOT NULL REFERENCES git_receives(id),
    source_ref text NOT NULL CHECK (source_ref LIKE 'refs/%'),
    source_commit text NOT NULL CHECK (
        source_commit ~ '^[0-9a-f]{40}$'
        OR source_commit ~ '^[0-9a-f]{64}$'
    ),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (build_request_id, receive_id, source_ref)
);

CREATE TABLE releases (
    id uuid PRIMARY KEY,
    repository_id uuid NOT NULL REFERENCES repositories(id),
    version text NOT NULL CHECK (
        length(version) BETWEEN 1 AND 128
        AND version !~ '[[:space:]/\\]'
    ),
    source_commit text NOT NULL CHECK (
        source_commit ~ '^[0-9a-f]{40}$'
        OR source_commit ~ '^[0-9a-f]{64}$'
    ),
    source_ref text NOT NULL CHECK (source_ref LIKE 'refs/%'),
    build_request_id uuid NOT NULL,
    build_definition_hash bytea NOT NULL
        CHECK (octet_length(build_definition_hash) = 32),
    configuration jsonb NOT NULL,
    configuration_hash bytea NOT NULL
        CHECK (octet_length(configuration_hash) = 32),
    manifest_hash bytea NOT NULL CHECK (octet_length(manifest_hash) = 32),
    state text NOT NULL CHECK (state IN ('draft', 'published', 'revoked')),
    publication_actor_id uuid REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    published_at timestamptz,
    revoked_at timestamptz,
    FOREIGN KEY (build_request_id, repository_id)
        REFERENCES build_requests(id, repository_id),
    UNIQUE (repository_id, version),
    UNIQUE (id, repository_id),
    CHECK (
        (state = 'draft' AND published_at IS NULL AND revoked_at IS NULL)
        OR (state = 'published' AND published_at IS NOT NULL AND revoked_at IS NULL)
        OR (state = 'revoked' AND revoked_at IS NOT NULL)
    )
);
CREATE INDEX releases_by_repository_state
    ON releases (repository_id, state, created_at DESC, id);

CREATE FUNCTION reject_published_release_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.state IN ('published', 'revoked')
       AND (
           NEW.repository_id, NEW.version, NEW.source_commit, NEW.source_ref,
           NEW.build_request_id, NEW.build_definition_hash, NEW.configuration,
           NEW.configuration_hash, NEW.manifest_hash
       ) IS DISTINCT FROM (
           OLD.repository_id, OLD.version, OLD.source_commit, OLD.source_ref,
           OLD.build_request_id, OLD.build_definition_hash, OLD.configuration,
           OLD.configuration_hash, OLD.manifest_hash
       )
    THEN
        RAISE EXCEPTION 'published release provenance is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.state = 'published' AND NEW.state NOT IN ('published', 'revoked') THEN
        RAISE EXCEPTION 'published release cannot return to draft'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.state = 'revoked' AND NEW IS DISTINCT FROM OLD THEN
        RAISE EXCEPTION 'revoked release is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER releases_immutable_after_publish
BEFORE UPDATE ON releases
FOR EACH ROW EXECUTE FUNCTION reject_published_release_mutation();

CREATE TABLE release_artifacts (
    id uuid PRIMARY KEY,
    release_id uuid NOT NULL REFERENCES releases(id),
    path text NOT NULL CHECK (
        length(path) BETWEEN 1 AND 1024
        AND path !~ '(^/|(^|/)\.\.?(/|$)|(^|/)\.git(/|$)|\\)'
    ),
    kind text NOT NULL CHECK (
        kind IN ('executable', 'file', 'manifest', 'build_log')
    ),
    mode integer NOT NULL CHECK (mode BETWEEN 0 AND 4095),
    content_hash bytea NOT NULL CHECK (octet_length(content_hash) = 32),
    size_bytes bigint NOT NULL CHECK (size_bytes >= 0),
    media_type text NOT NULL CHECK (length(media_type) BETWEEN 1 AND 256),
    storage_key uuid NOT NULL,
    provenance jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (release_id, path),
    UNIQUE (release_id, storage_key),
    UNIQUE (id, release_id)
);
CREATE INDEX release_artifacts_by_release
    ON release_artifacts (release_id, kind, path, id);

CREATE TABLE release_agents (
    id uuid PRIMARY KEY,
    release_id uuid NOT NULL REFERENCES releases(id),
    family_id uuid NOT NULL REFERENCES agent_families(id),
    agent_key text NOT NULL CHECK (
        agent_key ~ '^[a-z0-9][a-z0-9_-]{0,63}$'
    ),
    display_name text NOT NULL CHECK (length(display_name) BETWEEN 1 AND 200),
    runtime_contract jsonb NOT NULL,
    runtime_contract_hash bytea NOT NULL
        CHECK (octet_length(runtime_contract_hash) = 32),
    parameter_schema jsonb NOT NULL DEFAULT '[]'::jsonb,
    secret_slot_schema jsonb NOT NULL DEFAULT '[]'::jsonb,
    requires_state boolean NOT NULL,
    update_hook jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (release_id, agent_key),
    UNIQUE (id, release_id),
    UNIQUE (id, family_id)
);
CREATE INDEX release_agents_by_release_key
    ON release_agents (release_id, agent_key, id);
CREATE INDEX release_agents_by_family
    ON release_agents (family_id, release_id, id);

CREATE TABLE agent_instances (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id),
    family_id uuid NOT NULL REFERENCES agent_families(id),
    name text NOT NULL CHECK (
        name ~ '^[a-z0-9][a-z0-9_-]{0,127}$'
    ),
    state text NOT NULL CHECK (
        state IN (
            'active', 'disabled', 'update_draining', 'updating',
            'update_rejected', 'paused_unknown_state',
            'paused_activation_recovery', 'recovering', 'removed'
        )
    ),
    active_revision_id uuid,
    state_volume_id uuid UNIQUE,
    version bigint NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_by uuid REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    removed_at timestamptz,
    UNIQUE (project_id, name),
    UNIQUE (id, project_id),
    UNIQUE (id, active_revision_id),
    CHECK (
        (state = 'removed' AND removed_at IS NOT NULL)
        OR (state <> 'removed' AND removed_at IS NULL)
    )
);
CREATE INDEX agent_instances_by_project_state
    ON agent_instances (project_id, state, created_at DESC, id);

CREATE TABLE agent_instance_state_volumes (
    id uuid PRIMARY KEY,
    instance_id uuid NOT NULL UNIQUE REFERENCES agent_instances(id),
    state text NOT NULL CHECK (
        state IN ('uninitialized', 'ready', 'attached', 'recovering')
    ),
    host_id text,
    host_path text UNIQUE,
    capacity_bytes bigint NOT NULL CHECK (capacity_bytes > 0),
    filesystem_uuid uuid UNIQUE,
    lease_generation bigint NOT NULL DEFAULT 0 CHECK (lease_generation >= 0),
    key_reference text,
    encryption_version integer,
    backup_revision bigint,
    checksum text,
    last_successful_backup_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (
        (state = 'uninitialized'
            AND host_id IS NULL AND host_path IS NULL
            AND filesystem_uuid IS NULL)
        OR (state <> 'unallocated'
            AND host_id IS NOT NULL AND host_path IS NOT NULL
            AND filesystem_uuid IS NOT NULL)
    ),
    UNIQUE (id, instance_id)
);
ALTER TABLE agent_instances
    ADD CONSTRAINT agent_instances_state_volume_fk
    FOREIGN KEY (state_volume_id, id)
    REFERENCES agent_instance_state_volumes(id, instance_id)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE agent_instance_revisions (
    id uuid PRIMARY KEY,
    instance_id uuid NOT NULL REFERENCES agent_instances(id),
    release_agent_id uuid NOT NULL REFERENCES release_agents(id),
    parameters jsonb NOT NULL,
    parameter_hash bytea NOT NULL CHECK (octet_length(parameter_hash) = 32),
    secret_bindings jsonb NOT NULL DEFAULT '[]'::jsonb,
    resource_selection jsonb NOT NULL,
    network_restriction jsonb NOT NULL,
    effective_runtime_policy jsonb NOT NULL,
    effective_policy_hash bytea NOT NULL
        CHECK (octet_length(effective_policy_hash) = 32),
    platform_policy_version text NOT NULL
        CHECK (length(platform_policy_version) BETWEEN 1 AND 128),
    runnable boolean NOT NULL,
    diagnostics jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_by uuid REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (id, instance_id),
    UNIQUE (instance_id, id)
);
CREATE INDEX agent_instance_revisions_by_instance
    ON agent_instance_revisions (instance_id, created_at DESC, id);

ALTER TABLE agent_instances
    ADD CONSTRAINT agent_instances_active_revision_fk
    FOREIGN KEY (id, active_revision_id)
    REFERENCES agent_instance_revisions(instance_id, id)
    DEFERRABLE INITIALLY DEFERRED;

CREATE FUNCTION reject_instance_revision_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'agent instance revisions are immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
CREATE TRIGGER agent_instance_revisions_immutable
BEFORE UPDATE OR DELETE ON agent_instance_revisions
FOR EACH ROW EXECUTE FUNCTION reject_instance_revision_mutation();

CREATE TABLE agent_attachments (
    id uuid PRIMARY KEY,
    instance_id uuid NOT NULL REFERENCES agent_instances(id),
    project_id uuid NOT NULL,
    repository_id uuid NOT NULL,
    ref_selector text NOT NULL CHECK (
        ref_selector LIKE 'refs/%'
        AND length(ref_selector) BETWEEN 6 AND 1024
    ),
    trigger_policy text NOT NULL CHECK (
        trigger_policy IN ('push', 'manual', 'push_and_manual')
    ),
    enabled boolean NOT NULL DEFAULT true,
    created_by uuid REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    removed_at timestamptz,
    FOREIGN KEY (instance_id, project_id)
        REFERENCES agent_instances(id, project_id),
    FOREIGN KEY (repository_id, project_id)
        REFERENCES repositories(id, project_id),
    UNIQUE (instance_id, repository_id, ref_selector),
    UNIQUE (id, instance_id),
    CHECK (removed_at IS NULL OR enabled = false)
);
CREATE INDEX agent_attachments_by_target
    ON agent_attachments (repository_id, enabled, ref_selector, id)
    WHERE removed_at IS NULL;
CREATE INDEX agent_attachments_by_instance
    ON agent_attachments (instance_id, created_at, id);

ALTER TABLE agent_instances
    ADD COLUMN run_gate_open boolean NOT NULL DEFAULT true;

-- Run requests are exclusively reusable-instance requests. Source
-- configuration may request builds, but never creates an implicit runnable
-- agent in the target repository.
ALTER TABLE run_requests
    ALTER COLUMN config_hash DROP NOT NULL,
    ALTER COLUMN config_revision_id DROP NOT NULL,
    ALTER COLUMN agent_id DROP NOT NULL;
ALTER TABLE run_requests
    ADD COLUMN instance_id uuid,
    ADD COLUMN instance_revision_id uuid,
    ADD COLUMN release_id uuid,
    ADD COLUMN release_agent_id uuid,
    ADD COLUMN attachment_id uuid,
    ADD COLUMN request_kind text NOT NULL DEFAULT 'instance_normal'
        CHECK (request_kind = 'instance_normal'),
    ADD COLUMN platform_policy_version text,
    ADD COLUMN dispatch_state text NOT NULL DEFAULT 'pending'
        CHECK (dispatch_state IN ('pending', 'dispatched', 'denied', 'cancelled')),
    ADD COLUMN dispatch_diagnostics jsonb NOT NULL DEFAULT '[]',
    ADD FOREIGN KEY (instance_id, instance_revision_id)
        REFERENCES agent_instance_revisions(instance_id, id),
    ADD FOREIGN KEY (release_id) REFERENCES releases(id),
    ADD FOREIGN KEY (release_agent_id, release_id)
        REFERENCES release_agents(id, release_id),
    ADD FOREIGN KEY (attachment_id, instance_id)
        REFERENCES agent_attachments(id, instance_id),
    ADD CHECK (
        request_kind = 'instance_normal'
        AND agent_id IS NULL AND config_hash IS NULL
        AND config_revision_id IS NULL
        AND num_nonnulls(
            instance_id, instance_revision_id, release_id,
            release_agent_id, attachment_id,
            platform_policy_version
        ) = 6
    );
CREATE UNIQUE INDEX one_instance_run_request_per_receive
    ON run_requests (
        attachment_id, instance_revision_id, commit_sha, git_ref,
        receive_id, attempt
    )
    WHERE request_kind = 'instance_normal';
CREATE INDEX instance_run_requests_by_instance
    ON run_requests (instance_id, created_at, id);

ALTER TABLE runs
    ALTER COLUMN agent_id DROP NOT NULL;
ALTER TABLE runs
    ADD COLUMN instance_id uuid,
    ADD COLUMN instance_revision_id uuid,
    ADD COLUMN release_id uuid,
    ADD COLUMN release_agent_id uuid,
    ADD COLUMN attachment_id uuid,
    ADD COLUMN run_kind text NOT NULL
        CHECK (run_kind IN ('normal', 'update')),
    ADD FOREIGN KEY (instance_id, instance_revision_id)
        REFERENCES agent_instance_revisions(instance_id, id),
    ADD FOREIGN KEY (release_id) REFERENCES releases(id),
    ADD FOREIGN KEY (release_agent_id, release_id)
        REFERENCES release_agents(id, release_id),
    ADD FOREIGN KEY (attachment_id, instance_id)
        REFERENCES agent_attachments(id, instance_id),
    ADD CHECK (
        (
            run_kind = 'normal' AND agent_id IS NULL
            AND num_nonnulls(
                instance_id, instance_revision_id, release_id,
                release_agent_id, attachment_id
            ) = 5
        )
        OR (
            run_kind = 'update' AND agent_id IS NULL
            AND num_nonnulls(
                instance_id, instance_revision_id, release_id,
                release_agent_id
            ) = 4
            AND attachment_id IS NULL
        )
    );
CREATE INDEX runs_by_instance
    ON runs (instance_id, created_at, id);

ALTER TABLE run_results
    DROP CONSTRAINT run_results_agent_id_fkey;
ALTER TABLE run_results
    RENAME COLUMN agent_id TO instance_id;
ALTER TABLE run_results
    ADD COLUMN instance_revision_id uuid,
    ADD COLUMN release_id uuid,
    ADD COLUMN release_agent_id uuid,
    ADD FOREIGN KEY (instance_id, instance_revision_id)
        REFERENCES agent_instance_revisions(instance_id, id),
    ADD FOREIGN KEY (release_id) REFERENCES releases(id),
    ADD FOREIGN KEY (release_agent_id, release_id)
        REFERENCES release_agents(id, release_id);
DROP INDEX run_results_by_agent;
CREATE INDEX run_results_by_instance
    ON run_results (instance_id, created_at, id);

CREATE TABLE agent_instance_volume_leases (
    id uuid PRIMARY KEY,
    volume_id uuid NOT NULL REFERENCES agent_instance_state_volumes(id),
    instance_id uuid NOT NULL,
    run_id uuid REFERENCES runs(id),
    update_id uuid UNIQUE,
    host_id text NOT NULL,
    fencing_token bigint NOT NULL CHECK (fencing_token > 0),
    state text NOT NULL CHECK (
        state IN ('active', 'released', 'recovery_required')
    ),
    acquired_at timestamptz NOT NULL DEFAULT now(),
    heartbeat_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    attached_at timestamptz,
    recovering_at timestamptz,
    released_at timestamptz,
    FOREIGN KEY (volume_id, instance_id)
        REFERENCES agent_instance_state_volumes(id, instance_id),
    UNIQUE (volume_id, fencing_token),
    CHECK (expires_at > acquired_at),
    CHECK (num_nonnulls(run_id, update_id) = 1)
);
CREATE UNIQUE INDEX one_active_instance_volume_lease
    ON agent_instance_volume_leases (volume_id)
    WHERE released_at IS NULL;
CREATE UNIQUE INDEX one_active_instance_run_lease
    ON agent_instance_volume_leases (run_id)
    WHERE released_at IS NULL AND run_id IS NOT NULL;

CREATE TABLE agent_updates (
    id uuid PRIMARY KEY,
    instance_id uuid NOT NULL REFERENCES agent_instances(id),
    expected_current_revision_id uuid NOT NULL,
    candidate_revision_id uuid NOT NULL,
    state text NOT NULL CHECK (
        state IN (
            'candidate', 'draining', 'hook_running', 'hook_committed',
            'activated', 'rejected', 'compatibility_unknown',
            'activation_recovery'
        )
    ),
    hook_run_id uuid REFERENCES runs(id),
    hook_exit_code integer,
    hook_exit_signal integer,
    diagnostics jsonb NOT NULL DEFAULT '[]'::jsonb,
    final_decision text CHECK (
        final_decision IN ('activated', 'agent_rejected', 'unknown', 'recovery')
    ),
    actor_id uuid REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    FOREIGN KEY (instance_id, expected_current_revision_id)
        REFERENCES agent_instance_revisions(instance_id, id),
    FOREIGN KEY (instance_id, candidate_revision_id)
        REFERENCES agent_instance_revisions(instance_id, id),
    CHECK (expected_current_revision_id <> candidate_revision_id),
    CHECK (NOT (hook_exit_code IS NOT NULL AND hook_exit_signal IS NOT NULL))
);
CREATE UNIQUE INDEX one_active_agent_update
    ON agent_updates (instance_id)
    WHERE state IN (
        'candidate', 'draining', 'hook_running', 'hook_committed',
        'activation_recovery'
    );
CREATE INDEX agent_updates_by_instance
    ON agent_updates (instance_id, created_at DESC, id);
ALTER TABLE agent_instance_volume_leases
    ADD FOREIGN KEY (update_id) REFERENCES agent_updates(id);

CREATE TABLE deferred_agent_triggers (
    id uuid PRIMARY KEY,
    instance_id uuid NOT NULL REFERENCES agent_instances(id),
    attachment_id uuid NOT NULL,
    repository_id uuid NOT NULL REFERENCES repositories(id),
    target_ref text NOT NULL CHECK (target_ref LIKE 'refs/%'),
    target_commit text NOT NULL CHECK (
        target_commit ~ '^[0-9a-f]{40}$'
        OR target_commit ~ '^[0-9a-f]{64}$'
    ),
    source_id uuid NOT NULL,
    state text NOT NULL DEFAULT 'deferred' CHECK (
        state IN ('deferred', 'materialized', 'denied')
    ),
    run_request_id uuid REFERENCES run_requests(id),
    diagnostics jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    resolved_at timestamptz,
    FOREIGN KEY (attachment_id, instance_id)
        REFERENCES agent_attachments(id, instance_id),
    UNIQUE (attachment_id, repository_id, target_ref, target_commit, source_id)
);
CREATE INDEX deferred_agent_triggers_by_instance
    ON deferred_agent_triggers (instance_id, created_at, id)
    WHERE state = 'deferred';

CREATE TABLE agent_instance_events (
    id uuid PRIMARY KEY,
    instance_id uuid NOT NULL REFERENCES agent_instances(id),
    revision_id uuid,
    update_id uuid REFERENCES agent_updates(id),
    event_type text NOT NULL,
    actor_id uuid REFERENCES users(id),
    request_id uuid,
    payload jsonb NOT NULL DEFAULT '{}'::jsonb,
    occurred_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (instance_id, revision_id)
        REFERENCES agent_instance_revisions(instance_id, id)
);
CREATE INDEX agent_instance_events_by_instance
    ON agent_instance_events (instance_id, occurred_at, id);

CREATE TABLE release_command_inbox (
    command_key bytea PRIMARY KEY CHECK (octet_length(command_key) = 32),
    operation text NOT NULL,
    aggregate_id uuid NOT NULL,
    secondary_id uuid,
    actor_id uuid REFERENCES users(id),
    request_id uuid,
    completed_at timestamptz NOT NULL DEFAULT now()
);

-- Secret metadata and ciphertext are separated so RLS-facing metadata queries
-- never need access to cryptographic material.
CREATE TABLE secrets (
    id uuid PRIMARY KEY,
    owner_organization_id uuid NOT NULL REFERENCES organizations(id),
    organization_id uuid REFERENCES organizations(id),
    project_id uuid,
    name text NOT NULL CHECK (
        name ~ '^[a-z0-9][a-z0-9_-]{0,127}$'
    ),
    status text NOT NULL CHECK (
        status IN ('active', 'disabled', 'revoked', 'tombstoned', 'purged')
    ),
    allowed_delivery_modes text[] NOT NULL CHECK (
        cardinality(allowed_delivery_modes) BETWEEN 1 AND 2
        AND allowed_delivery_modes <@ ARRAY['raw', 'brokered']::text[]
    ),
    active_version_id uuid,
    policy jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_by uuid REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    tombstoned_at timestamptz,
    purged_at timestamptz,
    FOREIGN KEY (project_id, owner_organization_id)
        REFERENCES projects(id, organization_id),
    CHECK (num_nonnulls(organization_id, project_id) = 1),
    CHECK (organization_id IS NULL OR organization_id = owner_organization_id),
    UNIQUE NULLS NOT DISTINCT (owner_organization_id, organization_id, project_id, name),
    UNIQUE (id, owner_organization_id)
);

CREATE TABLE secret_versions (
    id uuid PRIMARY KEY,
    secret_id uuid NOT NULL REFERENCES secrets(id),
    sequence bigint NOT NULL CHECK (sequence > 0),
    status text NOT NULL CHECK (status IN ('active', 'revoked', 'purged')),
    algorithm text NOT NULL CHECK (
        algorithm = 'AES-256-GCM+AES-256-GCM-KW/v1'
    ),
    key_reference text NOT NULL CHECK (
        length(key_reference) BETWEEN 1 AND 128
    ),
    data_nonce bytea CHECK (
        data_nonce IS NULL OR octet_length(data_nonce) = 12
    ),
    ciphertext bytea,
    wrap_nonce bytea CHECK (
        wrap_nonce IS NULL OR octet_length(wrap_nonce) = 12
    ),
    wrapped_data_key bytea,
    associated_data_hash bytea CHECK (
        associated_data_hash IS NULL OR octet_length(associated_data_hash) = 32
    ),
    content_length integer NOT NULL CHECK (content_length BETWEEN 1 AND 65536),
    created_by uuid REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    purged_at timestamptz,
    UNIQUE (secret_id, sequence),
    UNIQUE (id, secret_id),
    CHECK (
        (status <> 'purged'
            AND data_nonce IS NOT NULL
            AND ciphertext IS NOT NULL
            AND wrap_nonce IS NOT NULL
            AND wrapped_data_key IS NOT NULL
            AND associated_data_hash IS NOT NULL)
        OR (status = 'purged'
            AND data_nonce IS NULL
            AND ciphertext IS NULL
            AND wrap_nonce IS NULL
            AND wrapped_data_key IS NULL
            AND associated_data_hash IS NULL
            AND purged_at IS NOT NULL)
    )
);

ALTER TABLE secrets
    ADD CONSTRAINT secrets_active_version_fk
    FOREIGN KEY (active_version_id, id)
    REFERENCES secret_versions(id, secret_id)
    DEFERRABLE INITIALLY DEFERRED;
CREATE INDEX secrets_by_organization_status
    ON secrets (owner_organization_id, status, id);
CREATE INDEX secret_versions_by_secret
    ON secret_versions (secret_id, sequence DESC, id);

CREATE FUNCTION reject_secret_version_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.status = 'purged' THEN
        RAISE EXCEPTION 'purged secret version is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF (
        NEW.secret_id, NEW.sequence, NEW.algorithm, NEW.key_reference,
        NEW.content_length, NEW.created_by, NEW.created_at
    ) IS DISTINCT FROM (
        OLD.secret_id, OLD.sequence, OLD.algorithm, OLD.key_reference,
        OLD.content_length, OLD.created_by, OLD.created_at
    ) THEN
        RAISE EXCEPTION 'secret version provenance is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NEW.status = 'purged' THEN
        IF NEW.data_nonce IS NOT NULL OR NEW.ciphertext IS NOT NULL
           OR NEW.wrap_nonce IS NOT NULL OR NEW.wrapped_data_key IS NOT NULL
           OR NEW.associated_data_hash IS NOT NULL OR NEW.purged_at IS NULL
        THEN
            RAISE EXCEPTION 'purge must remove all encrypted material'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    ELSIF (
        NEW.data_nonce, NEW.ciphertext, NEW.wrap_nonce,
        NEW.wrapped_data_key, NEW.associated_data_hash
    ) IS DISTINCT FROM (
        OLD.data_nonce, OLD.ciphertext, OLD.wrap_nonce,
        OLD.wrapped_data_key, OLD.associated_data_hash
    ) THEN
        RAISE EXCEPTION 'encrypted secret version is immutable before purge'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER secret_versions_immutable
BEFORE UPDATE ON secret_versions
FOR EACH ROW EXECUTE FUNCTION reject_secret_version_mutation();

CREATE TABLE secret_grants (
    id uuid PRIMARY KEY,
    secret_id uuid NOT NULL,
    owner_organization_id uuid NOT NULL,
    target_kind text NOT NULL CHECK (target_kind IN ('project', 'repository')),
    target_id uuid NOT NULL,
    target_project_id uuid NOT NULL,
    delivery_modes text[] NOT NULL CHECK (
        cardinality(delivery_modes) BETWEEN 1 AND 2
        AND delivery_modes <@ ARRAY['raw', 'brokered']::text[]
    ),
    phases text[] NOT NULL CHECK (
        cardinality(phases) BETWEEN 1 AND 2
        AND phases <@ ARRAY['normal', 'update']::text[]
    ),
    destinations text[] NOT NULL DEFAULT '{}'::text[]
        CHECK (cardinality(destinations) <= 32),
    status text NOT NULL CHECK (status IN ('active', 'revoked', 'expired')),
    expires_at timestamptz,
    created_by uuid REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    FOREIGN KEY (secret_id, owner_organization_id)
        REFERENCES secrets(id, owner_organization_id),
    FOREIGN KEY (target_project_id, owner_organization_id)
        REFERENCES projects(id, organization_id),
    UNIQUE (id, secret_id),
    UNIQUE (id, target_kind, target_id),
    CHECK (
        (target_kind = 'project' AND target_id = target_project_id)
        OR target_kind = 'repository'
    )
);
CREATE INDEX secret_grants_by_secret_target
    ON secret_grants (secret_id, target_kind, target_id, status, id);

CREATE FUNCTION validate_secret_grant_target() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.target_kind = 'repository'
       AND NOT EXISTS (
           SELECT 1 FROM repositories
           WHERE id = NEW.target_id AND project_id = NEW.target_project_id
       )
    THEN
        RAISE EXCEPTION 'secret grant repository is outside target project'
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER secret_grants_target_boundary
BEFORE INSERT OR UPDATE ON secret_grants
FOR EACH ROW EXECUTE FUNCTION validate_secret_grant_target();

CREATE TABLE secret_imports (
    id uuid PRIMARY KEY,
    grant_id uuid NOT NULL REFERENCES secret_grants(id),
    secret_id uuid NOT NULL,
    target_kind text NOT NULL CHECK (target_kind IN ('project', 'repository')),
    target_id uuid NOT NULL,
    alias text NOT NULL CHECK (
        alias ~ '^[a-z0-9][a-z0-9_-]{0,127}$'
    ),
    status text NOT NULL CHECK (status IN ('active', 'revoked', 'expired')),
    accepted_by uuid REFERENCES users(id),
    accepted_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    FOREIGN KEY (grant_id, secret_id) REFERENCES secret_grants(id, secret_id),
    FOREIGN KEY (grant_id, target_kind, target_id)
        REFERENCES secret_grants(id, target_kind, target_id),
    UNIQUE (target_kind, target_id, alias),
    UNIQUE (id, grant_id)
);
CREATE INDEX secret_imports_by_target
    ON secret_imports (target_kind, target_id, status, alias, id);

CREATE TABLE agent_secret_bindings (
    id uuid PRIMARY KEY,
    instance_revision_id uuid NOT NULL REFERENCES agent_instance_revisions(id),
    import_id uuid NOT NULL REFERENCES secret_imports(id),
    slot_key text NOT NULL CHECK (
        slot_key ~ '^[a-z0-9][a-z0-9_-]{0,63}$'
    ),
    delivery_mode text NOT NULL CHECK (
        delivery_mode IN ('raw', 'brokered')
    ),
    phases text[] NOT NULL CHECK (
        cardinality(phases) BETWEEN 1 AND 2
        AND phases <@ ARRAY['normal', 'update']::text[]
    ),
    attachment_ids uuid[] NOT NULL DEFAULT '{}'::uuid[],
    destinations text[] NOT NULL DEFAULT '{}'::text[]
        CHECK (cardinality(destinations) <= 32),
    effective_policy jsonb NOT NULL,
    effective_policy_hash bytea NOT NULL
        CHECK (octet_length(effective_policy_hash) = 32),
    status text NOT NULL CHECK (status IN ('active', 'revoked', 'expired')),
    created_by uuid REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    UNIQUE (instance_revision_id, slot_key),
    UNIQUE (id, instance_revision_id)
);
CREATE INDEX agent_secret_bindings_by_import
    ON agent_secret_bindings (import_id, status, id);

CREATE FUNCTION reject_agent_secret_binding_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF (
        NEW.instance_revision_id, NEW.import_id, NEW.slot_key,
        NEW.delivery_mode, NEW.phases, NEW.attachment_ids,
        NEW.destinations, NEW.effective_policy, NEW.effective_policy_hash,
        NEW.created_by, NEW.created_at
    ) IS DISTINCT FROM (
        OLD.instance_revision_id, OLD.import_id, OLD.slot_key,
        OLD.delivery_mode, OLD.phases, OLD.attachment_ids,
        OLD.destinations, OLD.effective_policy, OLD.effective_policy_hash,
        OLD.created_by, OLD.created_at
    ) THEN
        RAISE EXCEPTION 'agent secret binding is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER agent_secret_bindings_immutable
BEFORE UPDATE ON agent_secret_bindings
FOR EACH ROW EXECUTE FUNCTION reject_agent_secret_binding_mutation();

CREATE TABLE run_instance_provenance (
    run_id uuid PRIMARY KEY REFERENCES runs(id),
    instance_id uuid NOT NULL REFERENCES agent_instances(id),
    instance_revision_id uuid NOT NULL,
    release_id uuid NOT NULL REFERENCES releases(id),
    release_agent_id uuid NOT NULL,
    attachment_id uuid NOT NULL,
    target_repository_id uuid NOT NULL REFERENCES repositories(id),
    target_ref text NOT NULL CHECK (target_ref LIKE 'refs/%'),
    target_commit text NOT NULL CHECK (
        target_commit ~ '^[0-9a-f]{40}$'
        OR target_commit ~ '^[0-9a-f]{64}$'
    ),
    parameter_hash bytea NOT NULL CHECK (octet_length(parameter_hash) = 32),
    platform_policy_version text NOT NULL,
    phase text NOT NULL CHECK (phase IN ('normal', 'update')),
    authorization_model_version text NOT NULL,
    resolved_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (instance_id, instance_revision_id)
        REFERENCES agent_instance_revisions(instance_id, id),
    FOREIGN KEY (release_agent_id, release_id)
        REFERENCES release_agents(id, release_id),
    FOREIGN KEY (attachment_id, instance_id)
        REFERENCES agent_attachments(id, instance_id)
);
CREATE INDEX run_instance_provenance_by_instance
    ON run_instance_provenance (instance_id, resolved_at DESC, run_id);

CREATE TABLE run_secret_provenance (
    run_id uuid NOT NULL REFERENCES runs(id),
    binding_id uuid NOT NULL REFERENCES agent_secret_bindings(id),
    secret_id uuid NOT NULL REFERENCES secrets(id),
    secret_version_id uuid NOT NULL,
    grant_id uuid NOT NULL REFERENCES secret_grants(id),
    import_id uuid NOT NULL REFERENCES secret_imports(id),
    authorization_model_version text NOT NULL,
    delivery_policy_hash bytea NOT NULL
        CHECK (octet_length(delivery_policy_hash) = 32),
    resolved_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (secret_version_id, secret_id)
        REFERENCES secret_versions(id, secret_id),
    PRIMARY KEY (run_id, binding_id)
);

CREATE TABLE secret_runtime_sessions (
    id uuid PRIMARY KEY,
    run_id uuid NOT NULL UNIQUE REFERENCES runs(id),
    instance_id uuid NOT NULL,
    instance_revision_id uuid NOT NULL,
    attachment_id uuid NOT NULL,
    phase text NOT NULL CHECK (phase IN ('normal', 'update')),
    runtime_credential_hash bytea NOT NULL UNIQUE
        CHECK (octet_length(runtime_credential_hash) = 32),
    status text NOT NULL CHECK (
        status IN ('active', 'revoked', 'expired', 'released')
    ),
    issued_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    released_at timestamptz,
    FOREIGN KEY (run_id) REFERENCES run_instance_provenance(run_id),
    FOREIGN KEY (instance_id, instance_revision_id)
        REFERENCES agent_instance_revisions(instance_id, id),
    FOREIGN KEY (attachment_id, instance_id)
        REFERENCES agent_attachments(id, instance_id),
    UNIQUE (id, run_id),
    CHECK (expires_at > issued_at)
);
CREATE INDEX active_secret_runtime_sessions
    ON secret_runtime_sessions (expires_at, id)
    WHERE status = 'active';

CREATE TABLE secret_leases (
    id uuid PRIMARY KEY,
    session_id uuid NOT NULL REFERENCES secret_runtime_sessions(id),
    run_id uuid NOT NULL REFERENCES runs(id),
    binding_id uuid NOT NULL REFERENCES agent_secret_bindings(id),
    secret_version_id uuid NOT NULL REFERENCES secret_versions(id),
    delivery_mode text NOT NULL CHECK (
        delivery_mode IN ('raw', 'brokered')
    ),
    slot_key text NOT NULL CHECK (
        slot_key ~ '^[a-z0-9][a-z0-9_-]{0,63}$'
    ),
    destinations text[] NOT NULL DEFAULT '{}'::text[],
    status text NOT NULL CHECK (
        status IN ('active', 'revoked', 'expired', 'released')
    ),
    raw_material_observed boolean NOT NULL DEFAULT false,
    issued_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    released_at timestamptz,
    UNIQUE (session_id, binding_id),
    UNIQUE (session_id, slot_key),
    CHECK (expires_at > issued_at),
    FOREIGN KEY (session_id, run_id)
        REFERENCES secret_runtime_sessions(id, run_id)
);
CREATE INDEX active_secret_leases_by_version
    ON secret_leases (secret_version_id, expires_at, id)
    WHERE status = 'active';
CREATE INDEX active_secret_leases_by_run
    ON secret_leases (run_id, expires_at, id)
    WHERE status = 'active';

CREATE TABLE secret_audit_events (
    id uuid PRIMARY KEY,
    owner_organization_id uuid NOT NULL REFERENCES organizations(id),
    requester_id uuid REFERENCES users(id),
    mediator_id uuid REFERENCES users(id),
    runtime_run_id uuid REFERENCES runs(id),
    secret_id uuid REFERENCES secrets(id),
    secret_version_id uuid REFERENCES secret_versions(id),
    grant_id uuid REFERENCES secret_grants(id),
    import_id uuid REFERENCES secret_imports(id),
    binding_id uuid REFERENCES agent_secret_bindings(id),
    lease_id uuid REFERENCES secret_leases(id),
    target_kind text,
    target_id uuid,
    operation text NOT NULL CHECK (
        operation IN (
            'inspect_metadata', 'write_value', 'rotate', 'manage_grants',
            'accept_import', 'bind_brokered', 'bind_raw', 'resolve',
            'use_brokered', 'receive_raw', 'disable', 'enable', 'revoke',
            'purge', 'cleanup'
        )
    ),
    permission text NOT NULL,
    delivery_mode text CHECK (delivery_mode IN ('raw', 'brokered')),
    decision text NOT NULL CHECK (decision IN ('allow', 'deny')),
    outcome text NOT NULL,
    request_id uuid,
    command_id uuid,
    authorization_model_version text NOT NULL,
    policy_version text,
    occurred_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX secret_audit_by_organization
    ON secret_audit_events (owner_organization_id, occurred_at DESC, id);
CREATE INDEX secret_audit_by_secret
    ON secret_audit_events (secret_id, occurred_at DESC, id);

CREATE TABLE secret_command_inbox (
    command_key bytea PRIMARY KEY CHECK (octet_length(command_key) = 32),
    operation text NOT NULL,
    aggregate_id uuid NOT NULL,
    secondary_id uuid,
    requester_id uuid REFERENCES users(id),
    request_id uuid,
    completed_at timestamptz NOT NULL DEFAULT now()
);

CREATE FUNCTION forbid_secret_audit_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'secret audit events are append-only'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
CREATE TRIGGER secret_audit_events_append_only
BEFORE UPDATE OR DELETE ON secret_audit_events
FOR EACH ROW EXECUTE FUNCTION forbid_secret_audit_mutation();

-- Wake LiveViews with opaque identifiers only.
CREATE TRIGGER releases_ui_wakeup
AFTER INSERT OR UPDATE ON releases
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER agent_instances_ui_wakeup
AFTER INSERT OR UPDATE ON agent_instances
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER agent_attachments_ui_wakeup
AFTER INSERT OR UPDATE ON agent_attachments
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER agent_updates_ui_wakeup
AFTER INSERT OR UPDATE ON agent_updates
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER secrets_ui_wakeup
AFTER INSERT OR UPDATE ON secrets
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();

-- New protected data defaults to no access for the application role until an
-- exact policy below permits it. Worker ciphertext access is granted only to
-- the explicitly trusted worker role.
GRANT SELECT, INSERT, UPDATE, DELETE ON
    organization_secret_managers, project_secret_roles, repository_managers,
    repository_secret_roles, agent_families, build_requests, releases,
    build_request_sources, release_artifacts, release_agents,
    agent_instances, agent_instance_revisions, agent_instance_state_volumes,
    agent_instance_volume_leases, agent_attachments, agent_updates,
    deferred_agent_triggers, agent_instance_events, release_command_inbox,
    secrets, secret_grants,
    secret_imports, agent_secret_bindings, run_secret_provenance,
    run_instance_provenance, secret_runtime_sessions, secret_leases,
    secret_audit_events
    , secret_command_inbox
TO hephaestus_app, hephaestus_worker;
GRANT SELECT, INSERT, UPDATE ON secret_versions TO hephaestus_worker;
REVOKE ALL ON secret_versions FROM hephaestus_app;

-- Ciphertext deliberately has no user-facing RLS policy. It is unreachable by
-- `hephaestus_app`; the trusted resolver uses `hephaestus_worker`.
ALTER TABLE secret_versions ENABLE ROW LEVEL SECURITY;
ALTER TABLE secret_versions FORCE ROW LEVEL SECURITY;
CREATE POLICY secret_versions_worker ON secret_versions
    TO hephaestus_worker USING (true) WITH CHECK (true);

-- Metadata tables are forced through Mélange. Generated functions for the new
-- object vocabulary are installed by the following authorization migration.
ALTER TABLE agent_families ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_families FORCE ROW LEVEL SECURITY;
ALTER TABLE build_requests ENABLE ROW LEVEL SECURITY;
ALTER TABLE build_requests FORCE ROW LEVEL SECURITY;
ALTER TABLE build_request_sources ENABLE ROW LEVEL SECURITY;
ALTER TABLE build_request_sources FORCE ROW LEVEL SECURITY;
ALTER TABLE releases ENABLE ROW LEVEL SECURITY;
ALTER TABLE releases FORCE ROW LEVEL SECURITY;
ALTER TABLE release_artifacts ENABLE ROW LEVEL SECURITY;
ALTER TABLE release_artifacts FORCE ROW LEVEL SECURITY;
ALTER TABLE release_agents ENABLE ROW LEVEL SECURITY;
ALTER TABLE release_agents FORCE ROW LEVEL SECURITY;
ALTER TABLE agent_instances ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_instances FORCE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_revisions ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_revisions FORCE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_state_volumes ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_state_volumes FORCE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_volume_leases ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_volume_leases FORCE ROW LEVEL SECURITY;
ALTER TABLE agent_attachments ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_attachments FORCE ROW LEVEL SECURITY;
ALTER TABLE agent_updates ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_updates FORCE ROW LEVEL SECURITY;
ALTER TABLE deferred_agent_triggers ENABLE ROW LEVEL SECURITY;
ALTER TABLE deferred_agent_triggers FORCE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_instance_events FORCE ROW LEVEL SECURITY;
ALTER TABLE release_command_inbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE release_command_inbox FORCE ROW LEVEL SECURITY;
ALTER TABLE secrets ENABLE ROW LEVEL SECURITY;
ALTER TABLE secrets FORCE ROW LEVEL SECURITY;
ALTER TABLE secret_grants ENABLE ROW LEVEL SECURITY;
ALTER TABLE secret_grants FORCE ROW LEVEL SECURITY;
ALTER TABLE secret_imports ENABLE ROW LEVEL SECURITY;
ALTER TABLE secret_imports FORCE ROW LEVEL SECURITY;
ALTER TABLE agent_secret_bindings ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_secret_bindings FORCE ROW LEVEL SECURITY;
ALTER TABLE run_secret_provenance ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_secret_provenance FORCE ROW LEVEL SECURITY;
ALTER TABLE run_instance_provenance ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_instance_provenance FORCE ROW LEVEL SECURITY;
ALTER TABLE secret_runtime_sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE secret_runtime_sessions FORCE ROW LEVEL SECURITY;
ALTER TABLE secret_leases ENABLE ROW LEVEL SECURITY;
ALTER TABLE secret_leases FORCE ROW LEVEL SECURITY;
ALTER TABLE secret_audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE secret_audit_events FORCE ROW LEVEL SECURITY;
ALTER TABLE secret_command_inbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE secret_command_inbox FORCE ROW LEVEL SECURITY;

-- Immutable release rows and audit provenance are retained; deletion is a
-- lifecycle tombstone/revocation operation.
CREATE FUNCTION reject_release_record_delete() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'release and instance provenance uses tombstones'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
CREATE TRIGGER releases_no_delete BEFORE DELETE ON releases
FOR EACH ROW EXECUTE FUNCTION reject_release_record_delete();
CREATE TRIGGER release_artifacts_no_delete BEFORE DELETE ON release_artifacts
FOR EACH ROW EXECUTE FUNCTION reject_release_record_delete();
CREATE TRIGGER release_agents_no_delete BEFORE DELETE ON release_agents
FOR EACH ROW EXECUTE FUNCTION reject_release_record_delete();
CREATE TRIGGER agent_instances_no_delete BEFORE DELETE ON agent_instances
FOR EACH ROW EXECUTE FUNCTION reject_release_record_delete();
CREATE TRIGGER agent_attachments_no_delete BEFORE DELETE ON agent_attachments
FOR EACH ROW EXECUTE FUNCTION reject_release_record_delete();
