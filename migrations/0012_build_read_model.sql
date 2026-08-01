-- Durable build summary/detail provenance.
--
-- The request and execution tables remain the authoritative write model. These
-- columns freeze the parsed, non-secret build inputs at request time so a
-- later agent.toml revision or catalog change cannot rewrite build history.
ALTER TABLE build_requests
    ADD COLUMN build_trigger text NOT NULL DEFAULT 'manual'
        CHECK (build_trigger IN ('push', 'manual', 'recovery')),
    ADD COLUMN agent_key text
        CHECK (agent_key IS NULL OR agent_key ~ '^[a-z0-9][a-z0-9_-]{0,63}$'),
    ADD COLUMN builder_image_id uuid REFERENCES builder_images(id),
    ADD COLUMN builder_image_key text
        CHECK (builder_image_key IS NULL OR builder_image_key ~ '^[a-z0-9][a-z0-9_-]{0,63}$'),
    ADD COLUMN builder_image_reference text
        CHECK (builder_image_reference IS NULL OR builder_image_reference ~ '@sha256:[0-9a-f]{64}$'),
    ADD COLUMN configuration_hash bytea
        CHECK (configuration_hash IS NULL OR octet_length(configuration_hash) = 32),
    ADD COLUMN build_declaration jsonb NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(build_declaration) = 'object'),
    ADD COLUMN build_policy jsonb NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(build_policy) = 'object'),
    ADD COLUMN declared_artifacts jsonb NOT NULL DEFAULT '[]'::jsonb
        CHECK (jsonb_typeof(declared_artifacts) = 'array');

-- Existing receive-linked requests were created by the push workflow. New
-- callers set the value explicitly, while old fixture rows remain readable.
UPDATE build_requests
   SET build_trigger = 'push'
 WHERE origin_receive_id IS NOT NULL;

-- A build timeline is an append-only projection of lifecycle transitions. It
-- deliberately contains no logs or source-controlled values.
CREATE TABLE build_state_transitions (
    id uuid PRIMARY KEY,
    build_request_id uuid NOT NULL REFERENCES build_requests(id),
    from_state text
        CHECK (from_state IS NULL OR from_state IN (
            'queued', 'running', 'importing', 'succeeded', 'failed', 'cancelled'
        )),
    to_state text NOT NULL CHECK (to_state IN (
        'queued', 'running', 'importing', 'succeeded', 'failed', 'cancelled'
    )),
    reason text NOT NULL CHECK (length(reason) BETWEEN 1 AND 128),
    occurred_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (build_request_id, id)
);
CREATE INDEX build_state_transitions_by_build
    ON build_state_transitions (build_request_id, occurred_at, id);

ALTER TABLE build_state_transitions ENABLE ROW LEVEL SECURITY;
ALTER TABLE build_state_transitions FORCE ROW LEVEL SECURITY;
CREATE POLICY build_state_transitions_select ON build_state_transitions
    FOR SELECT USING (
        check_permission(
            'user', hephaestus_actor_id(), 'can_read',
            'build', build_request_id::text
        ) = 1
    );
GRANT SELECT ON build_state_transitions TO hephaestus_app;
GRANT SELECT, INSERT ON build_state_transitions TO hephaestus_worker;

CREATE FUNCTION capture_build_state_transition() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    IF TG_OP = 'INSERT' OR NEW.state IS DISTINCT FROM OLD.state THEN
        INSERT INTO build_state_transitions
            (id, build_request_id, from_state, to_state, reason, occurred_at)
        VALUES (
            gen_random_uuid(),
            NEW.id,
            CASE WHEN TG_OP = 'INSERT' THEN NULL ELSE OLD.state END,
            NEW.state,
            CASE
                WHEN TG_OP = 'INSERT' THEN 'requested'
                WHEN NEW.state = 'running' THEN 'execution_started'
                WHEN NEW.state = 'importing' THEN 'artifact_import_started'
                WHEN NEW.state = 'succeeded' THEN 'draft_release_created'
                WHEN NEW.state = 'failed' THEN 'build_failed'
                WHEN NEW.state = 'cancelled' THEN 'build_cancelled'
                ELSE 'state_changed'
            END,
            COALESCE(NEW.completed_at, NEW.started_at, now())
        );
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION capture_build_state_transition() FROM PUBLIC;

CREATE TRIGGER build_requests_state_timeline
AFTER INSERT OR UPDATE OF state ON build_requests
FOR EACH ROW EXECUTE FUNCTION capture_build_state_transition();

-- Backfill one truthful initial observation for historical rows. Future state
-- changes are captured by the trigger above.
INSERT INTO build_state_transitions
    (id, build_request_id, from_state, to_state, reason, occurred_at)
SELECT gen_random_uuid(), id, NULL, state, 'historical_backfill', created_at
  FROM build_requests request
 WHERE NOT EXISTS (
    SELECT 1 FROM build_state_transitions transition
     WHERE transition.build_request_id = request.id
 );

GRANT SELECT ON build_requests TO hephaestus_app;
GRANT SELECT, INSERT, UPDATE ON build_requests TO hephaestus_worker;
