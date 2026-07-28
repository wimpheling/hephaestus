-- Hephaestus Phase 3 POC baseline. Earlier POC migrations are intentionally
-- squashed: Phase 3 test and development databases are disposable.

CREATE TABLE users (
    id uuid PRIMARY KEY,
    display_name text NOT NULL CHECK (length(display_name) BETWEEN 1 AND 200),
    status text NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'suspended', 'disabled')),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE external_identities (
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    issuer text NOT NULL CHECK (length(issuer) BETWEEN 1 AND 2048),
    subject text NOT NULL CHECK (length(subject) BETWEEN 1 AND 2048),
    provider_metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (issuer, subject),
    UNIQUE (user_id, issuer)
);

CREATE TABLE user_profiles (
    user_id uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    validated_claims jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE organizations (
    id uuid PRIMARY KEY,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE organization_members (
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role text NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (organization_id, user_id)
);
CREATE INDEX organization_members_by_user
    ON organization_members (user_id, organization_id, role);

CREATE TABLE projects (
    id uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    settings jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (organization_id, name)
);
CREATE INDEX projects_by_organization ON projects (organization_id, id);

CREATE TABLE project_maintainers (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, user_id)
);
CREATE INDEX project_maintainers_by_user
    ON project_maintainers (user_id, project_id);

CREATE TABLE repositories (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    default_branch text NOT NULL DEFAULT 'refs/heads/main'
        CHECK (default_branch LIKE 'refs/heads/%'),
    is_public boolean NOT NULL DEFAULT false,
    settings jsonb NOT NULL DEFAULT '{"agent_runs_enabled": true}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, name)
);
CREATE INDEX repositories_by_project ON repositories (project_id, id);

CREATE TABLE agents (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 128),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, name)
);
CREATE INDEX agents_by_project ON agents (project_id, id);

CREATE TABLE agent_state_volumes (
    id uuid PRIMARY KEY,
    agent_id uuid NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind = 'agent_state'),
    host_id text NOT NULL,
    host_path text NOT NULL UNIQUE,
    capacity_bytes bigint NOT NULL CHECK (capacity_bytes > 0),
    filesystem_uuid uuid NOT NULL UNIQUE,
    state text NOT NULL CHECK (
        state IN ('uninitialized', 'ready', 'attached', 'recovering')
    ),
    lease_generation bigint NOT NULL DEFAULT 0 CHECK (lease_generation >= 0),
    key_reference text,
    encryption_version integer,
    backup_revision bigint,
    checksum text,
    last_successful_backup_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (agent_id, kind)
);
CREATE INDEX state_volumes_by_agent ON agent_state_volumes (agent_id, id);

CREATE TABLE volume_leases (
    id uuid PRIMARY KEY,
    volume_id uuid NOT NULL REFERENCES agent_state_volumes(id) ON DELETE CASCADE,
    run_id uuid NOT NULL,
    host_id text NOT NULL,
    fencing_token bigint NOT NULL CHECK (fencing_token > 0),
    acquired_at timestamptz NOT NULL,
    heartbeat_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    attached_at timestamptz,
    recovering_at timestamptz,
    released_at timestamptz,
    UNIQUE (volume_id, fencing_token)
);
CREATE UNIQUE INDEX one_active_volume_lease
    ON volume_leases (volume_id) WHERE released_at IS NULL;
CREATE UNIQUE INDEX one_active_run_lease
    ON volume_leases (run_id) WHERE released_at IS NULL;

CREATE TABLE runs (
    id uuid PRIMARY KEY,
    agent_id uuid NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    command_id uuid NOT NULL UNIQUE,
    volume_id uuid REFERENCES agent_state_volumes(id),
    lease_id uuid REFERENCES volume_leases(id),
    vm_id text,
    state text NOT NULL CHECK (
        state IN (
            'queued', 'leasing_volume', 'provisioning', 'starting', 'running',
            'succeeded', 'failed', 'cancelled', 'cleaning_up', 'cleaned_up'
        )
    ),
    outcome text CHECK (outcome IN ('succeeded', 'failed', 'cancelled')),
    exit_code integer,
    exit_signal integer,
    failure text,
    cancel_requested_at timestamptz,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    state_version bigint NOT NULL DEFAULT 0 CHECK (state_version >= 0),
    CHECK (NOT (exit_code IS NOT NULL AND exit_signal IS NOT NULL))
);
CREATE INDEX runs_by_agent ON runs (agent_id, created_at, id);

CREATE TABLE run_events (
    id uuid PRIMARY KEY,
    run_id uuid NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    sequence bigint NOT NULL CHECK (sequence > 0),
    event_type text NOT NULL,
    payload jsonb NOT NULL,
    occurred_at timestamptz NOT NULL,
    UNIQUE (run_id, sequence)
);

CREATE TABLE command_inbox (
    command_id uuid PRIMARY KEY,
    command_type text NOT NULL,
    payload jsonb NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    processed_at timestamptz
);

CREATE TABLE outbox (
    id uuid PRIMARY KEY,
    aggregate_type text NOT NULL,
    aggregate_id uuid NOT NULL,
    subject text NOT NULL,
    event_type text NOT NULL,
    payload jsonb NOT NULL,
    occurred_at timestamptz NOT NULL,
    published_at timestamptz,
    attempts integer NOT NULL DEFAULT 0,
    last_error text
);
CREATE INDEX unpublished_outbox
    ON outbox (occurred_at, id) WHERE published_at IS NULL;

CREATE TABLE git_receives (
    id uuid PRIMARY KEY,
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    actor_id uuid REFERENCES users(id),
    principal text NOT NULL,
    request_id uuid,
    status text NOT NULL CHECK (status IN ('accepted', 'rejected')),
    error text,
    accepted_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX git_receives_by_repository
    ON git_receives (repository_id, created_at, id);

CREATE TABLE git_ref_updates (
    receive_id uuid NOT NULL REFERENCES git_receives(id) ON DELETE CASCADE,
    sequence integer NOT NULL CHECK (sequence > 0),
    git_ref text NOT NULL CHECK (git_ref LIKE 'refs/%'),
    old_commit text CHECK (
        old_commit IS NULL OR old_commit ~ '^[0-9a-f]{40}$'
        OR old_commit ~ '^[0-9a-f]{64}$'
    ),
    new_commit text CHECK (
        new_commit IS NULL OR new_commit ~ '^[0-9a-f]{40}$'
        OR new_commit ~ '^[0-9a-f]{64}$'
    ),
    PRIMARY KEY (receive_id, sequence),
    CHECK (old_commit IS NOT NULL OR new_commit IS NOT NULL)
);
CREATE INDEX git_ref_updates_by_ref ON git_ref_updates (git_ref, receive_id);

CREATE TABLE git_refs (
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    git_ref text NOT NULL CHECK (git_ref LIKE 'refs/%'),
    commit_sha text NOT NULL CHECK (
        commit_sha ~ '^[0-9a-f]{40}$' OR commit_sha ~ '^[0-9a-f]{64}$'
    ),
    updated_by_receive_id uuid NOT NULL REFERENCES git_receives(id),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (repository_id, git_ref)
);

CREATE TABLE agent_config_revisions (
    id uuid PRIMARY KEY,
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    receive_id uuid NOT NULL REFERENCES git_receives(id) ON DELETE CASCADE,
    commit_sha text NOT NULL CHECK (
        commit_sha ~ '^[0-9a-f]{40}$' OR commit_sha ~ '^[0-9a-f]{64}$'
    ),
    config_hash text NOT NULL CHECK (config_hash ~ '^[0-9a-f]{64}$'),
    schema_version integer,
    agent_id uuid REFERENCES agents(id),
    status text NOT NULL CHECK (status IN ('valid', 'invalid')),
    config jsonb,
    diagnostics jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (repository_id, commit_sha, config_hash)
);

CREATE TABLE run_requests (
    id uuid PRIMARY KEY,
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    commit_sha text NOT NULL CHECK (
        commit_sha ~ '^[0-9a-f]{40}$' OR commit_sha ~ '^[0-9a-f]{64}$'
    ),
    git_ref text NOT NULL CHECK (git_ref LIKE 'refs/%'),
    config_hash text NOT NULL CHECK (config_hash ~ '^[0-9a-f]{64}$'),
    receive_id uuid NOT NULL REFERENCES git_receives(id) ON DELETE CASCADE,
    config_revision_id uuid NOT NULL REFERENCES agent_config_revisions(id),
    agent_id uuid NOT NULL REFERENCES agents(id),
    run_id uuid NOT NULL UNIQUE,
    command_id uuid NOT NULL UNIQUE,
    actor_id uuid REFERENCES users(id),
    request_id uuid,
    retry_of_run_id uuid REFERENCES runs(id),
    attempt integer NOT NULL DEFAULT 1 CHECK (attempt > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (repository_id, commit_sha, git_ref, config_hash, receive_id, attempt),
    CHECK (
        (attempt = 1 AND retry_of_run_id IS NULL)
        OR (attempt > 1 AND retry_of_run_id IS NOT NULL)
    )
);
CREATE INDEX run_requests_by_repository
    ON run_requests (repository_id, created_at, id);

CREATE TABLE run_workspaces (
    id uuid PRIMARY KEY,
    run_id uuid NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    input_commit text NOT NULL CHECK (
        input_commit ~ '^[0-9a-f]{40}$' OR input_commit ~ '^[0-9a-f]{64}$'
    ),
    input_tree text CHECK (
        input_tree IS NULL OR input_tree ~ '^[0-9a-f]{40}$'
        OR input_tree ~ '^[0-9a-f]{64}$'
    ),
    materialization_hash text CHECK (
        materialization_hash IS NULL
        OR materialization_hash ~ '^[0-9a-f]{64}$'
    ),
    active_path text NOT NULL,
    sealed_path text NOT NULL,
    source_mount_path text NOT NULL DEFAULT '/workspace/repo',
    work_mount_path text NOT NULL DEFAULT '/workspace/work',
    state text NOT NULL CHECK (
        state IN (
            'preparing', 'active', 'finalize_requested', 'sealed',
            'importing', 'cleaned', 'materialization_failed', 'seal_failed',
            'import_rejected', 'abandoned'
        )
    ),
    failure jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    finalized_at timestamptz,
    sealed_at timestamptz,
    cleaned_at timestamptz
);
CREATE INDEX run_workspaces_by_repository
    ON run_workspaces (repository_id, created_at, id);

CREATE TABLE run_results (
    id uuid PRIMARY KEY,
    run_id uuid NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    agent_id uuid NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    input_commit text NOT NULL CHECK (
        input_commit ~ '^[0-9a-f]{40}$' OR input_commit ~ '^[0-9a-f]{64}$'
    ),
    result_tree text CHECK (
        result_tree IS NULL OR result_tree ~ '^[0-9a-f]{40}$'
        OR result_tree ~ '^[0-9a-f]{64}$'
    ),
    result_commit text CHECK (
        result_commit IS NULL OR result_commit ~ '^[0-9a-f]{40}$'
        OR result_commit ~ '^[0-9a-f]{64}$'
    ),
    result_ref text NOT NULL CHECK (
        result_ref LIKE 'refs/heads/hephaestus/%'
    ),
    message text NOT NULL CHECK (octet_length(message) <= 4096),
    artifact_manifest_hash text CHECK (
        artifact_manifest_hash IS NULL
        OR artifact_manifest_hash ~ '^[0-9a-f]{64}$'
    ),
    state text NOT NULL CHECK (
        state IN (
            'pending', 'prepared', 'ref_published', 'completed', 'rejected'
        )
    ),
    diagnostics jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    prepared_at timestamptz,
    published_at timestamptz,
    completed_at timestamptz,
    UNIQUE (repository_id, result_ref)
);
CREATE INDEX run_results_by_repository
    ON run_results (repository_id, created_at, id);
CREATE INDEX run_results_by_agent
    ON run_results (agent_id, created_at, id);

CREATE TABLE result_artifacts (
    id uuid PRIMARY KEY,
    result_id uuid NOT NULL REFERENCES run_results(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (
        kind IN ('manifest', 'patch', 'logs', 'exit', 'declared_file')
    ),
    path text NOT NULL DEFAULT '',
    git_mode integer,
    media_type text,
    size_bytes bigint NOT NULL CHECK (size_bytes >= 0),
    sha256 text NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    storage_key text NOT NULL,
    provenance jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (result_id, kind, path)
);
CREATE INDEX result_artifacts_by_result
    ON result_artifacts (result_id, kind, id);

CREATE TABLE review_proposals (
    id uuid PRIMARY KEY,
    result_id uuid NOT NULL UNIQUE REFERENCES run_results(id) ON DELETE CASCADE,
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    run_id uuid NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    target_ref text NOT NULL CHECK (target_ref LIKE 'refs/heads/%'),
    input_commit text NOT NULL CHECK (
        input_commit ~ '^[0-9a-f]{40}$' OR input_commit ~ '^[0-9a-f]{64}$'
    ),
    result_commit text NOT NULL CHECK (
        result_commit ~ '^[0-9a-f]{40}$' OR result_commit ~ '^[0-9a-f]{64}$'
    ),
    result_ref text NOT NULL CHECK (
        result_ref LIKE 'refs/heads/hephaestus/%'
    ),
    state text NOT NULL DEFAULT 'open' CHECK (
        state IN (
            'open', 'approval_requested', 'approved', 'rejected',
            'conflicted', 'superseded'
        )
    ),
    version bigint NOT NULL DEFAULT 0 CHECK (version >= 0),
    decision_actor_id uuid REFERENCES users(id),
    decision_request_id uuid,
    decision_reason text CHECK (
        decision_reason IS NULL OR octet_length(decision_reason) <= 4096
    ),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    decided_at timestamptz,
    CHECK (
        (state IN ('open', 'approval_requested')
            AND decided_at IS NULL)
        OR (state IN ('approved', 'rejected', 'conflicted', 'superseded')
            AND decided_at IS NOT NULL)
    )
);
CREATE INDEX review_proposals_by_repository
    ON review_proposals (repository_id, created_at DESC, id);
CREATE INDEX review_proposals_by_state
    ON review_proposals (state, created_at, id);

CREATE TABLE control_requests (
    id uuid PRIMARY KEY,
    kind text NOT NULL CHECK (
        kind IN ('cancel_run', 'retry_run', 'approve_result', 'reject_result')
    ),
    actor_id uuid NOT NULL REFERENCES users(id),
    request_id uuid NOT NULL,
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    run_id uuid REFERENCES runs(id) ON DELETE CASCADE,
    proposal_id uuid REFERENCES review_proposals(id) ON DELETE CASCADE,
    reason text NOT NULL DEFAULT '' CHECK (octet_length(reason) <= 4096),
    state text NOT NULL DEFAULT 'pending' CHECK (
        state IN ('pending', 'processing', 'completed', 'failed')
    ),
    diagnostics jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    processed_at timestamptz,
    UNIQUE (actor_id, request_id),
    CHECK (
        (kind IN ('cancel_run', 'retry_run')
            AND run_id IS NOT NULL AND proposal_id IS NULL)
        OR (kind IN ('approve_result', 'reject_result')
            AND run_id IS NULL AND proposal_id IS NOT NULL)
    )
);
CREATE INDEX control_requests_by_actor
    ON control_requests (actor_id, created_at DESC, id);
CREATE INDEX pending_control_requests
    ON control_requests (created_at, id) WHERE state IN ('pending', 'processing');

CREATE FUNCTION create_review_proposal() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.state = 'completed' AND NEW.result_commit IS NOT NULL THEN
        INSERT INTO review_proposals (
            id, result_id, repository_id, run_id, target_ref,
            input_commit, result_commit, result_ref
        )
        SELECT
            gen_random_uuid(), NEW.id, NEW.repository_id, NEW.run_id,
            request.git_ref, NEW.input_commit, NEW.result_commit, NEW.result_ref
        FROM run_requests request
        WHERE request.run_id = NEW.run_id
        ON CONFLICT (result_id) DO NOTHING;
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER completed_result_review_proposal
AFTER INSERT OR UPDATE OF state, result_commit ON run_results
FOR EACH ROW EXECUTE FUNCTION create_review_proposal();

CREATE FUNCTION enqueue_control_request() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    INSERT INTO outbox (
        id, aggregate_type, aggregate_id, subject, event_type,
        payload, occurred_at
    )
    VALUES (
        gen_random_uuid(), 'control_request', NEW.id,
        'hephaestus.control.execute', 'control.requested',
        jsonb_build_object(
            'command_id', NEW.id,
            'kind', NEW.kind,
            'actor_id', NEW.actor_id,
            'request_id', NEW.request_id,
            'repository_id', NEW.repository_id,
            'run_id', NEW.run_id,
            'proposal_id', NEW.proposal_id,
            'reason', NEW.reason
        ),
        NEW.created_at
    );
    RETURN NEW;
END
$$;
CREATE TRIGGER control_request_outbox
AFTER INSERT ON control_requests
FOR EACH ROW EXECUTE FUNCTION enqueue_control_request();

CREATE FUNCTION notify_ui_wakeup() RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    entity_id text;
BEGIN
    entity_id := COALESCE(
        to_jsonb(NEW) ->> 'run_id',
        to_jsonb(NEW) ->> 'id'
    );
    PERFORM pg_notify(
        'hephaestus_ui_wakeup',
        json_build_object('kind', TG_TABLE_NAME, 'id', entity_id)::text
    );
    RETURN NEW;
END
$$;
CREATE TRIGGER runs_ui_wakeup
AFTER INSERT OR UPDATE ON runs
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER run_events_ui_wakeup
AFTER INSERT ON run_events
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER run_results_ui_wakeup
AFTER INSERT OR UPDATE ON run_results
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER review_proposals_ui_wakeup
AFTER INSERT OR UPDATE ON review_proposals
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();

CREATE TABLE authorization_audit_events (
    id uuid PRIMARY KEY,
    actor_id uuid NOT NULL REFERENCES users(id),
    permission text NOT NULL,
    object_type text NOT NULL,
    object_id uuid NOT NULL,
    decision text NOT NULL CHECK (decision IN ('allow', 'deny')),
    request_id uuid NOT NULL,
    authorization_model_version text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX authorization_audit_by_actor
    ON authorization_audit_events (actor_id, created_at, id);
