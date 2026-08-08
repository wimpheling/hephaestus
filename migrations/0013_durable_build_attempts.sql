-- Durable retry attempts retain the failed execution while allowing the
-- trusted build worker to reset the active execution identity.
ALTER TABLE build_executions
    ADD COLUMN attempt_number integer NOT NULL DEFAULT 1
        CHECK (attempt_number > 0);

CREATE TABLE build_attempts (
    id uuid PRIMARY KEY,
    build_request_id uuid NOT NULL REFERENCES build_requests(id),
    attempt_number integer NOT NULL CHECK (attempt_number > 0),
    state text NOT NULL CHECK (
        state IN ('claimed', 'running', 'sealed', 'imported', 'drafted', 'failed')
    ),
    failure_code text,
    artifact_manifest jsonb,
    started_at timestamptz,
    completed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (build_request_id, attempt_number),
    CHECK (
        (state = 'failed' AND failure_code IS NOT NULL)
        OR (state <> 'failed' AND failure_code IS NULL)
    )
);

CREATE INDEX build_attempts_by_request
    ON build_attempts (build_request_id, attempt_number);

ALTER TABLE build_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE build_attempts FORCE ROW LEVEL SECURITY;
CREATE POLICY build_attempts_select ON build_attempts FOR SELECT
    USING (
        check_permission(
            'user', hephaestus_actor_id(), 'can_read',
            'build', build_request_id::text
        ) = 1
    );
GRANT SELECT ON build_attempts TO hephaestus_app;
GRANT SELECT, INSERT, UPDATE ON build_attempts TO hephaestus_worker;
