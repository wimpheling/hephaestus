-- Initial development has no legacy state to preserve. Remove the hollow
-- source-config agent execution model after the canonical instance model and
-- authorization functions are installed.

ALTER TABLE agent_config_revisions
    DROP COLUMN agent_id;

ALTER TABLE run_requests
    DROP COLUMN config_hash CASCADE,
    DROP COLUMN config_revision_id CASCADE,
    DROP COLUMN agent_id CASCADE,
    ADD COLUMN trigger_command_id uuid,
    ADD COLUMN requires_state boolean NOT NULL,
    ALTER COLUMN instance_id SET NOT NULL,
    ALTER COLUMN instance_revision_id SET NOT NULL,
    ALTER COLUMN release_id SET NOT NULL,
    ALTER COLUMN release_agent_id SET NOT NULL,
    ALTER COLUMN attachment_id SET NOT NULL,
    ALTER COLUMN platform_policy_version SET NOT NULL,
    ALTER COLUMN receive_id DROP NOT NULL,
    ADD CHECK (num_nonnulls(receive_id, trigger_command_id) = 1);

CREATE UNIQUE INDEX one_instance_run_request_per_command
    ON run_requests (
        attachment_id, instance_revision_id, commit_sha, git_ref,
        trigger_command_id, attempt
    )
    WHERE trigger_command_id IS NOT NULL;

ALTER TABLE runs
    DROP COLUMN agent_id CASCADE,
    DROP COLUMN volume_id CASCADE,
    DROP COLUMN lease_id CASCADE,
    ADD COLUMN volume_id uuid REFERENCES agent_instance_state_volumes(id),
    ADD COLUMN lease_id uuid REFERENCES agent_instance_volume_leases(id),
    ADD COLUMN requires_state boolean NOT NULL,
    ALTER COLUMN instance_id SET NOT NULL,
    ALTER COLUMN instance_revision_id SET NOT NULL,
    ALTER COLUMN release_id SET NOT NULL,
    ALTER COLUMN release_agent_id SET NOT NULL,
    ADD CHECK (
        (run_kind = 'normal' AND attachment_id IS NOT NULL)
        OR (run_kind = 'update' AND attachment_id IS NULL)
    );

ALTER TABLE run_results
    ALTER COLUMN instance_revision_id SET NOT NULL,
    ALTER COLUMN release_id SET NOT NULL,
    ALTER COLUMN release_agent_id SET NOT NULL;

ALTER TABLE run_instance_provenance
    ALTER COLUMN attachment_id DROP NOT NULL,
    ALTER COLUMN target_repository_id DROP NOT NULL,
    ALTER COLUMN target_ref DROP NOT NULL,
    ALTER COLUMN target_commit DROP NOT NULL,
    ADD CHECK (
        (
            phase = 'normal'
            AND attachment_id IS NOT NULL
            AND target_repository_id IS NOT NULL
            AND target_ref IS NOT NULL
            AND target_commit IS NOT NULL
        )
        OR (
            phase = 'update'
            AND attachment_id IS NULL
            AND target_repository_id IS NULL
            AND target_ref IS NULL
            AND target_commit IS NULL
        )
    );

ALTER TABLE secret_runtime_sessions
    ALTER COLUMN attachment_id DROP NOT NULL,
    ADD CHECK (
        (phase = 'normal' AND attachment_id IS NOT NULL)
        OR (phase = 'update' AND attachment_id IS NULL)
    );

CREATE TABLE secret_runtime_mounts (
    run_id uuid PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    opaque_directory uuid NOT NULL UNIQUE,
    state text NOT NULL CHECK (state IN ('materialized', 'destroyed')),
    materialized_at timestamptz NOT NULL DEFAULT now(),
    destroyed_at timestamptz,
    CHECK (
        (state = 'materialized' AND destroyed_at IS NULL)
        OR (state = 'destroyed' AND destroyed_at IS NOT NULL)
    )
);
CREATE INDEX active_secret_runtime_mounts
    ON secret_runtime_mounts (materialized_at, run_id)
    WHERE state = 'materialized';

CREATE TABLE build_executions (
    build_request_id uuid PRIMARY KEY REFERENCES build_requests(id),
    vm_id text NOT NULL UNIQUE,
    release_id uuid NOT NULL UNIQUE,
    release_agent_id uuid NOT NULL UNIQUE,
    release_version text NOT NULL,
    state text NOT NULL CHECK (
        state IN (
            'claimed', 'running', 'sealed', 'imported', 'drafted', 'failed'
        )
    ),
    exit_code integer,
    exit_signal integer,
    failure_code text,
    logs jsonb NOT NULL DEFAULT '[]',
    metrics jsonb NOT NULL DEFAULT '[]',
    artifact_manifest jsonb,
    started_at timestamptz,
    sealed_at timestamptz,
    imported_at timestamptz,
    completed_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (NOT (exit_code IS NOT NULL AND exit_signal IS NOT NULL)),
    CHECK (
        (state = 'failed' AND failure_code IS NOT NULL)
        OR (state <> 'failed' AND failure_code IS NULL)
    )
);
CREATE INDEX build_executions_by_state
    ON build_executions (state, updated_at, build_request_id);
ALTER TABLE build_executions ENABLE ROW LEVEL SECURITY;
ALTER TABLE build_executions FORCE ROW LEVEL SECURITY;
CREATE POLICY build_executions_select ON build_executions FOR SELECT
    USING (
        check_permission(
            'user', hephaestus_actor_id(), 'can_read',
            'build', build_request_id::text
        ) = 1
    );
GRANT SELECT ON build_executions TO hephaestus_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON build_executions
    TO hephaestus_worker;

DROP TABLE volume_leases;
DROP TABLE agent_state_volumes;
DROP TABLE agents;
