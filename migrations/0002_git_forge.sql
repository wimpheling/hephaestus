CREATE TABLE projects (
    id uuid PRIMARY KEY,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    settings jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE repositories (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id),
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    default_branch text NOT NULL DEFAULT 'refs/heads/main'
        CHECK (default_branch LIKE 'refs/heads/%'),
    settings jsonb NOT NULL DEFAULT '{"agent_runs_enabled": true}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, name)
);

CREATE TABLE git_receives (
    id uuid PRIMARY KEY,
    repository_id uuid NOT NULL REFERENCES repositories(id),
    principal text NOT NULL,
    status text NOT NULL CHECK (status IN ('accepted', 'rejected')),
    error text,
    accepted_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE git_ref_updates (
    receive_id uuid NOT NULL REFERENCES git_receives(id),
    sequence integer NOT NULL CHECK (sequence > 0),
    git_ref text NOT NULL CHECK (git_ref LIKE 'refs/%'),
    old_commit text CHECK (
        old_commit IS NULL
        OR old_commit ~ '^[0-9a-f]{40}$'
        OR old_commit ~ '^[0-9a-f]{64}$'
    ),
    new_commit text CHECK (
        new_commit IS NULL
        OR new_commit ~ '^[0-9a-f]{40}$'
        OR new_commit ~ '^[0-9a-f]{64}$'
    ),
    PRIMARY KEY (receive_id, sequence),
    CHECK (old_commit IS NOT NULL OR new_commit IS NOT NULL)
);

CREATE TABLE git_refs (
    repository_id uuid NOT NULL REFERENCES repositories(id),
    git_ref text NOT NULL CHECK (git_ref LIKE 'refs/%'),
    commit_sha text NOT NULL CHECK (
        commit_sha ~ '^[0-9a-f]{40}$'
        OR commit_sha ~ '^[0-9a-f]{64}$'
    ),
    updated_by_receive_id uuid NOT NULL REFERENCES git_receives(id),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (repository_id, git_ref)
);

CREATE INDEX git_ref_updates_by_repository_ref
    ON git_ref_updates (git_ref, receive_id);

CREATE TABLE agent_config_revisions (
    id uuid PRIMARY KEY,
    repository_id uuid NOT NULL REFERENCES repositories(id),
    receive_id uuid NOT NULL REFERENCES git_receives(id),
    commit_sha text NOT NULL CHECK (
        commit_sha ~ '^[0-9a-f]{40}$'
        OR commit_sha ~ '^[0-9a-f]{64}$'
    ),
    config_hash text NOT NULL CHECK (config_hash ~ '^[0-9a-f]{64}$'),
    schema_version integer,
    agent_id uuid,
    status text NOT NULL CHECK (status IN ('valid', 'invalid')),
    config jsonb,
    diagnostics jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (repository_id, commit_sha, config_hash)
);

CREATE TABLE run_requests (
    id uuid PRIMARY KEY,
    repository_id uuid NOT NULL REFERENCES repositories(id),
    commit_sha text NOT NULL CHECK (
        commit_sha ~ '^[0-9a-f]{40}$'
        OR commit_sha ~ '^[0-9a-f]{64}$'
    ),
    git_ref text NOT NULL CHECK (git_ref LIKE 'refs/%'),
    config_hash text NOT NULL CHECK (config_hash ~ '^[0-9a-f]{64}$'),
    receive_id uuid NOT NULL REFERENCES git_receives(id),
    config_revision_id uuid NOT NULL REFERENCES agent_config_revisions(id),
    agent_id uuid NOT NULL,
    run_id uuid NOT NULL UNIQUE,
    command_id uuid NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (repository_id, commit_sha, git_ref, config_hash, receive_id)
);

CREATE INDEX run_requests_by_repository
    ON run_requests (repository_id, created_at);
