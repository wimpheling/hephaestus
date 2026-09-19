-- Durable, opt-in application logs.  A worker may append only through the
-- adapter's exact instance/fence transaction; readers and eviction are later
-- slices.  The usage row is deliberately separate from the project entity so
-- append and maintenance share the quota -> gateway -> instance -> epoch lock
-- order.
CREATE TABLE gateway_service_log_project_usage (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    retained_bytes bigint NOT NULL DEFAULT 0 CHECK (retained_bytes >= 0),
    retained_chunks bigint NOT NULL DEFAULT 0 CHECK (retained_chunks >= 0),
    retained_epochs integer NOT NULL DEFAULT 0 CHECK (retained_epochs >= 0),
    storage_dropped_chunks bigint NOT NULL DEFAULT 0 CHECK (storage_dropped_chunks >= 0),
    storage_dropped_bytes bigint NOT NULL DEFAULT 0 CHECK (storage_dropped_bytes >= 0),
    updated_at timestamptz NOT NULL DEFAULT now()
);
COMMENT ON COLUMN gateway_service_log_project_usage.storage_dropped_chunks IS
    'Rejected metadata-cap submission totals; Capacity is terminal for an acknowledged batch, but an ambiguous commit followed by retry may count bytes again because no epoch watermark exists.';

CREATE TABLE gateway_service_log_epochs (
    instance_id uuid NOT NULL,
    gateway_id uuid NOT NULL,
    revision_id uuid NOT NULL,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    fencing_token bigint NOT NULL CHECK (fencing_token > 0),
    acknowledged_through bigint NOT NULL DEFAULT -1 CHECK (acknowledged_through >= -1),
    retained_bytes bigint NOT NULL DEFAULT 0 CHECK (retained_bytes >= 0),
    retained_chunks bigint NOT NULL DEFAULT 0 CHECK (retained_chunks >= 0),
    producer_dropped_chunks bigint NOT NULL DEFAULT 0 CHECK (producer_dropped_chunks >= 0),
    producer_dropped_bytes bigint NOT NULL DEFAULT 0 CHECK (producer_dropped_bytes >= 0),
    provider_lagged_events bigint NOT NULL DEFAULT 0 CHECK (provider_lagged_events >= 0),
    storage_dropped_chunks bigint NOT NULL DEFAULT 0 CHECK (storage_dropped_chunks >= 0),
    storage_dropped_bytes bigint NOT NULL DEFAULT 0 CHECK (storage_dropped_bytes >= 0),
    evicted_chunks bigint NOT NULL DEFAULT 0 CHECK (evicted_chunks >= 0),
    evicted_bytes bigint NOT NULL DEFAULT 0 CHECK (evicted_bytes >= 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (instance_id, fencing_token),
    UNIQUE (instance_id, gateway_id, revision_id, project_id, fencing_token),
    FOREIGN KEY (instance_id, gateway_id, revision_id)
        REFERENCES gateway_service_instances(id, gateway_id, revision_id),
    FOREIGN KEY (gateway_id, project_id)
        REFERENCES gateways(id, project_id)
);
COMMENT ON COLUMN gateway_service_log_epochs.acknowledged_through IS
    'Monotonic worker sequence watermark; payload eviction must retain this tombstone.';

CREATE TABLE gateway_service_log_chunks (
    instance_id uuid NOT NULL,
    gateway_id uuid NOT NULL,
    revision_id uuid NOT NULL,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    fencing_token bigint NOT NULL,
    sequence bigint NOT NULL CHECK (sequence >= 0),
    stream text NOT NULL CHECK (stream IN ('stdout', 'stderr')),
    observed_at timestamptz NOT NULL,
    bytes bytea NOT NULL CHECK (octet_length(bytes) BETWEEN 0 AND 65536),
    stored_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (instance_id, fencing_token, sequence),
    FOREIGN KEY (instance_id, gateway_id, revision_id, project_id, fencing_token)
        REFERENCES gateway_service_log_epochs(
            instance_id, gateway_id, revision_id, project_id, fencing_token
        )
        ON DELETE CASCADE
);

CREATE INDEX gateway_service_log_chunks_project_time
    ON gateway_service_log_chunks (project_id, stored_at, instance_id, sequence);

ALTER TABLE gateway_service_log_project_usage ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_service_log_project_usage FORCE ROW LEVEL SECURITY;
ALTER TABLE gateway_service_log_epochs ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_service_log_epochs FORCE ROW LEVEL SECURITY;
ALTER TABLE gateway_service_log_chunks ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_service_log_chunks FORCE ROW LEVEL SECURITY;

CREATE POLICY gateway_service_log_usage_worker ON gateway_service_log_project_usage
    FOR ALL TO hephaestus_worker USING (true) WITH CHECK (true);
CREATE POLICY gateway_service_log_epochs_worker ON gateway_service_log_epochs
    FOR ALL TO hephaestus_worker USING (true) WITH CHECK (true);
CREATE POLICY gateway_service_log_chunks_worker ON gateway_service_log_chunks
    FOR ALL TO hephaestus_worker USING (true) WITH CHECK (true);

CREATE POLICY gateway_service_log_epochs_read ON gateway_service_log_epochs
    FOR SELECT TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_read', 'project', project_id::text) = 1);
CREATE POLICY gateway_service_log_chunks_read ON gateway_service_log_chunks
    FOR SELECT TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_read', 'project', project_id::text) = 1);

GRANT SELECT ON gateway_service_log_epochs, gateway_service_log_chunks TO hephaestus_app;
GRANT SELECT, INSERT, UPDATE ON gateway_service_log_project_usage TO hephaestus_worker;
GRANT SELECT, INSERT, UPDATE ON gateway_service_log_epochs TO hephaestus_worker;
GRANT SELECT, INSERT ON gateway_service_log_chunks TO hephaestus_worker;
