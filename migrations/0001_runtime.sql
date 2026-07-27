CREATE TABLE agents (
    id uuid PRIMARY KEY,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE agent_state_volumes (
    id uuid PRIMARY KEY,
    agent_id uuid NOT NULL REFERENCES agents(id),
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

CREATE TABLE volume_leases (
    id uuid PRIMARY KEY,
    volume_id uuid NOT NULL REFERENCES agent_state_volumes(id),
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
    ON volume_leases (volume_id)
    WHERE released_at IS NULL;

CREATE UNIQUE INDEX one_active_run_lease
    ON volume_leases (run_id)
    WHERE released_at IS NULL;

CREATE TABLE runs (
    id uuid PRIMARY KEY,
    agent_id uuid NOT NULL REFERENCES agents(id),
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

CREATE TABLE run_events (
    id uuid PRIMARY KEY,
    run_id uuid NOT NULL REFERENCES runs(id),
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
    ON outbox (occurred_at, id)
    WHERE published_at IS NULL;
