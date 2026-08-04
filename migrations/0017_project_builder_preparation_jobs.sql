-- Durable, worker-owned preparation and rootfs-materialization queues for
-- repository OCI builders. Application callers may request preparation, but
-- only the worker role can produce OCI outputs or materialized roots.

CREATE TABLE project_builder_preparation_jobs (
    id uuid PRIMARY KEY,
    builder_id uuid NOT NULL UNIQUE REFERENCES project_builder_definitions(id) ON DELETE CASCADE,
    state text NOT NULL CHECK (state IN ('queued', 'claimed', 'succeeded', 'failed')),
    attempt integer NOT NULL CHECK (attempt > 0),
    lease_expires_at timestamptz,
    output_image_reference text CHECK (
        output_image_reference IS NULL
        OR output_image_reference ~ '^.+@sha256:[0-9a-f]{64}$'
    ),
    output_image_digest text CHECK (
        output_image_digest IS NULL OR output_image_digest ~ '^sha256:[0-9a-f]{64}$'
    ),
    provenance jsonb CHECK (provenance IS NULL OR jsonb_typeof(provenance) = 'object'),
    scan_reference text CHECK (scan_reference IS NULL OR length(scan_reference) BETWEEN 1 AND 2048),
    local_oci_layout text CHECK (local_oci_layout IS NULL OR left(local_oci_layout, 1) = '/'),
    failure_reason text CHECK (failure_reason IS NULL OR length(failure_reason) BETWEEN 1 AND 2048),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (
        (state = 'succeeded' AND output_image_reference IS NOT NULL
            AND output_image_digest IS NOT NULL AND provenance IS NOT NULL
            AND scan_reference IS NOT NULL AND local_oci_layout IS NOT NULL
            AND failure_reason IS NULL)
        OR (state = 'failed' AND output_image_reference IS NULL
            AND output_image_digest IS NULL AND provenance IS NULL
            AND scan_reference IS NULL AND local_oci_layout IS NULL
            AND failure_reason IS NOT NULL)
        OR (state IN ('queued', 'claimed') AND output_image_reference IS NULL
            AND output_image_digest IS NULL AND provenance IS NULL
            AND scan_reference IS NULL AND local_oci_layout IS NULL
            AND failure_reason IS NULL)
    ),
    CHECK (
        output_image_reference IS NULL
        OR split_part(output_image_reference, '@', 2) = output_image_digest
    )
);

CREATE INDEX project_builder_preparation_jobs_pending
    ON project_builder_preparation_jobs (created_at, id)
    WHERE state = 'queued';

CREATE TABLE project_builder_root_materialization_jobs (
    id uuid PRIMARY KEY,
    builder_id uuid NOT NULL REFERENCES project_builder_definitions(id) ON DELETE CASCADE,
    worker_name text NOT NULL CHECK (length(worker_name) BETWEEN 1 AND 200),
    output_image_reference text NOT NULL CHECK (
        output_image_reference ~ '^.+@sha256:[0-9a-f]{64}$'
    ),
    local_oci_layout text NOT NULL CHECK (left(local_oci_layout, 1) = '/'),
    state text NOT NULL CHECK (state IN ('queued', 'claimed', 'materialized', 'failed')),
    lease_expires_at timestamptz,
    root_path text CHECK (root_path IS NULL OR length(root_path) BETWEEN 1 AND 4096),
    failure_reason text CHECK (failure_reason IS NULL OR length(failure_reason) BETWEEN 1 AND 2048),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (builder_id, worker_name),
    CHECK (
        (state = 'materialized' AND root_path IS NOT NULL AND failure_reason IS NULL)
        OR (state = 'failed' AND root_path IS NULL AND failure_reason IS NOT NULL)
        OR (state IN ('queued', 'claimed') AND root_path IS NULL AND failure_reason IS NULL)
    )
);

CREATE INDEX project_builder_root_materialization_jobs_pending
    ON project_builder_root_materialization_jobs (worker_name, created_at, id)
    WHERE state = 'queued';

-- Entering preparing is the sole application-owned request. The trigger owns
-- queue insertion so state and job are committed together even if the caller
-- is interrupted immediately after the transition.
CREATE FUNCTION enqueue_project_builder_preparation_job() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET row_security = off
AS $$
BEGIN
    IF NEW.status = 'preparing'
       AND (TG_OP = 'INSERT' OR OLD.status <> 'preparing') THEN
        INSERT INTO project_builder_preparation_jobs (id, builder_id, state, attempt)
        VALUES (gen_random_uuid(), NEW.id, 'queued', 1)
        ON CONFLICT (builder_id) DO UPDATE
            SET state = 'queued',
                attempt = project_builder_preparation_jobs.attempt + 1,
                lease_expires_at = NULL,
                output_image_reference = NULL,
                output_image_digest = NULL,
                provenance = NULL,
                scan_reference = NULL,
                local_oci_layout = NULL,
                failure_reason = NULL,
                updated_at = now();
    END IF;
    RETURN NEW;
END
$$;

ALTER FUNCTION enqueue_project_builder_preparation_job() OWNER TO hephaestus_authz_owner;

CREATE TRIGGER enqueue_project_builder_preparation_job
AFTER INSERT OR UPDATE OF status ON project_builder_definitions
FOR EACH ROW EXECUTE FUNCTION enqueue_project_builder_preparation_job();

-- A ready project builder must have a successful worker-owned outcome for the
-- same immutable output. This prevents an RPC caller from promoting an
-- arbitrary digest by invoking the old completion path.
CREATE FUNCTION verify_project_builder_worker_completion() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET row_security = off
AS $$
BEGIN
    IF NEW.status = 'ready' AND OLD.status <> 'ready' THEN
        IF NOT EXISTS (
            SELECT 1 FROM project_builder_preparation_jobs AS job
             WHERE job.builder_id = NEW.id
               AND job.state = 'succeeded'
               AND job.output_image_reference = NEW.oci_image_reference
               AND job.output_image_digest = NEW.oci_image_digest
               AND job.provenance = NEW.provenance
        ) THEN
            RAISE EXCEPTION 'ready project builder requires successful worker preparation output';
        END IF;
    END IF;
    RETURN NEW;
END
$$;

ALTER FUNCTION verify_project_builder_worker_completion() OWNER TO hephaestus_authz_owner;

CREATE TRIGGER verify_project_builder_worker_completion
BEFORE UPDATE OF status, oci_image_reference, oci_image_digest, provenance
ON project_builder_definitions
FOR EACH ROW EXECUTE FUNCTION verify_project_builder_worker_completion();

ALTER TABLE project_builder_preparation_jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE project_builder_preparation_jobs FORCE ROW LEVEL SECURITY;
ALTER TABLE project_builder_root_materialization_jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE project_builder_root_materialization_jobs FORCE ROW LEVEL SECURITY;

CREATE POLICY project_builder_preparation_jobs_worker
    ON project_builder_preparation_jobs FOR ALL
    USING (current_user = 'hephaestus_worker')
    WITH CHECK (current_user = 'hephaestus_worker');

CREATE POLICY project_builder_root_materialization_jobs_worker
    ON project_builder_root_materialization_jobs FOR ALL
    USING (current_user = 'hephaestus_worker')
    WITH CHECK (current_user = 'hephaestus_worker');

CREATE POLICY project_builder_definitions_worker
    ON project_builder_definitions FOR ALL
    USING (current_user = 'hephaestus_worker')
    WITH CHECK (current_user = 'hephaestus_worker');

-- Workers do not accept tenant reads or writes through RLS. They operate with
-- the dedicated role and are constrained by the state-machine functions below.
GRANT SELECT, INSERT, UPDATE ON project_builder_preparation_jobs
    TO hephaestus_worker;
GRANT SELECT, INSERT, UPDATE ON project_builder_root_materialization_jobs
    TO hephaestus_worker;
GRANT SELECT, INSERT, UPDATE ON project_builder_preparation_jobs
    TO hephaestus_authz_owner;
GRANT UPDATE (status, oci_image_reference, oci_image_digest, provenance,
              failure_reason, updated_at)
    ON project_builder_definitions TO hephaestus_worker;
GRANT SELECT ON project_builder_definitions TO hephaestus_worker;
