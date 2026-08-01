-- Verification rebuilds execute the immutable build inputs again without
-- mutating the original draft or published release.
CREATE TABLE build_verifications (
    id uuid PRIMARY KEY,
    build_request_id uuid NOT NULL REFERENCES build_requests(id),
    state text NOT NULL CHECK (state IN ('running', 'succeeded', 'failed')),
    expected_manifest jsonb NOT NULL CHECK (jsonb_typeof(expected_manifest) = 'array'),
    actual_manifest jsonb CHECK (actual_manifest IS NULL OR jsonb_typeof(actual_manifest) = 'array'),
    failure_code text,
    created_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    CHECK (
        (state = 'failed' AND failure_code IS NOT NULL)
        OR (state <> 'failed' AND failure_code IS NULL)
    )
);

CREATE INDEX build_verifications_by_request
    ON build_verifications (build_request_id, created_at, id);

ALTER TABLE build_verifications ENABLE ROW LEVEL SECURITY;
ALTER TABLE build_verifications FORCE ROW LEVEL SECURITY;
CREATE POLICY build_verifications_select ON build_verifications FOR SELECT
    USING (
        check_permission(
            'user', hephaestus_actor_id(), 'can_read',
            'build', build_request_id::text
        ) = 1
    );
GRANT SELECT ON build_verifications TO hephaestus_app;
GRANT SELECT, INSERT, UPDATE ON build_verifications TO hephaestus_worker;
