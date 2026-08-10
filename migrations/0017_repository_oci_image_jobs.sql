-- Durable, worker-owned repository-image production and generic OCI-image
-- materialization queues. Application callers request production; only
-- workers record immutable outputs or materialized filesystem roots.

CREATE TABLE repository_oci_image_production_jobs (
    id uuid PRIMARY KEY,
    definition_id uuid NOT NULL UNIQUE
        REFERENCES repository_oci_image_definitions(id) ON DELETE CASCADE,
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
    failure_reason text CHECK (failure_reason IS NULL OR length(failure_reason) BETWEEN 1 AND 2048),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (
        (state = 'succeeded' AND output_image_reference IS NOT NULL
            AND output_image_digest IS NOT NULL AND provenance IS NOT NULL
            AND scan_reference IS NOT NULL AND failure_reason IS NULL)
        OR (state = 'failed' AND output_image_reference IS NULL
            AND output_image_digest IS NULL AND provenance IS NULL
            AND scan_reference IS NULL AND failure_reason IS NOT NULL)
        OR (state IN ('queued', 'claimed') AND output_image_reference IS NULL
            AND output_image_digest IS NULL AND provenance IS NULL
            AND scan_reference IS NULL AND failure_reason IS NULL)
    ),
    CHECK (
        output_image_reference IS NULL
        OR split_part(output_image_reference, '@', 2) = output_image_digest
    )
);

CREATE INDEX repository_oci_image_production_jobs_pending
    ON repository_oci_image_production_jobs (created_at, id)
    WHERE state = 'queued';

-- Materializers consume immutable forge-registry references. This cache is
-- deliberately shared by platform and repository-produced images: OCI image
-- identity, rather than its origin or execution phase, is the cache key.
CREATE TABLE oci_image_materialization_jobs (
    id uuid PRIMARY KEY,
    worker_name text NOT NULL CHECK (length(worker_name) BETWEEN 1 AND 200),
    image_reference text NOT NULL CHECK (
        image_reference ~ '^.+@sha256:[0-9a-f]{64}$'
    ),
    state text NOT NULL CHECK (state IN ('queued', 'claimed', 'materialized', 'failed')),
    lease_expires_at timestamptz,
    root_path text CHECK (root_path IS NULL OR length(root_path) BETWEEN 1 AND 4096),
    failure_reason text CHECK (failure_reason IS NULL OR length(failure_reason) BETWEEN 1 AND 2048),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (worker_name, image_reference),
    CHECK (
        (state = 'materialized' AND root_path IS NOT NULL AND failure_reason IS NULL)
        OR (state = 'failed' AND root_path IS NULL AND failure_reason IS NOT NULL)
        OR (state IN ('queued', 'claimed') AND root_path IS NULL AND failure_reason IS NULL)
    )
);

CREATE INDEX oci_image_materialization_jobs_pending
    ON oci_image_materialization_jobs (worker_name, created_at, id)
    WHERE state = 'queued';

-- Entering production is the sole application-owned request. Queue insertion
-- is part of the same transaction as the state transition.
CREATE FUNCTION enqueue_repository_oci_image_production_job() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET row_security = off
AS $$
BEGIN
    IF NEW.status = 'producing'
       AND (TG_OP = 'INSERT' OR OLD.status <> 'producing') THEN
        INSERT INTO repository_oci_image_production_jobs (id, definition_id, state, attempt)
        VALUES (gen_random_uuid(), NEW.id, 'queued', 1)
        ON CONFLICT (definition_id) DO UPDATE
            SET state = 'queued',
                attempt = repository_oci_image_production_jobs.attempt + 1,
                lease_expires_at = NULL,
                output_image_reference = NULL,
                output_image_digest = NULL,
                provenance = NULL,
                scan_reference = NULL,
                failure_reason = NULL,
                updated_at = now();
    END IF;
    RETURN NEW;
END
$$;

ALTER FUNCTION enqueue_repository_oci_image_production_job() OWNER TO hephaestus_authz_owner;

CREATE TRIGGER enqueue_repository_oci_image_production_job
AFTER INSERT OR UPDATE OF status ON repository_oci_image_definitions
FOR EACH ROW EXECUTE FUNCTION enqueue_repository_oci_image_production_job();

-- A ready image must exactly match a successful worker-owned production
-- outcome. RPC callers cannot promote arbitrary registry digests.
CREATE FUNCTION verify_repository_oci_image_worker_completion() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET row_security = off
AS $$
BEGIN
    IF NEW.status = 'ready' AND OLD.status <> 'ready' THEN
        IF NOT EXISTS (
            SELECT 1 FROM repository_oci_image_production_jobs AS job
             WHERE job.definition_id = NEW.id
               AND job.state = 'succeeded'
               AND job.output_image_reference = NEW.image_reference
               AND job.output_image_digest = NEW.image_digest
               AND job.provenance = NEW.provenance
        ) THEN
            RAISE EXCEPTION 'ready repository OCI image requires successful worker production output';
        END IF;
    END IF;
    RETURN NEW;
END
$$;

ALTER FUNCTION verify_repository_oci_image_worker_completion() OWNER TO hephaestus_authz_owner;

CREATE TRIGGER verify_repository_oci_image_worker_completion
BEFORE UPDATE OF status, image_reference, image_digest, provenance
ON repository_oci_image_definitions
FOR EACH ROW EXECUTE FUNCTION verify_repository_oci_image_worker_completion();

ALTER TABLE repository_oci_image_production_jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE repository_oci_image_production_jobs FORCE ROW LEVEL SECURITY;
ALTER TABLE oci_image_materialization_jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE oci_image_materialization_jobs FORCE ROW LEVEL SECURITY;

CREATE POLICY repository_oci_image_production_jobs_worker
    ON repository_oci_image_production_jobs FOR ALL
    USING (current_user = 'hephaestus_worker')
    WITH CHECK (current_user = 'hephaestus_worker');

CREATE POLICY oci_image_materialization_jobs_worker
    ON oci_image_materialization_jobs FOR ALL
    USING (current_user = 'hephaestus_worker')
    WITH CHECK (current_user = 'hephaestus_worker');

CREATE POLICY repository_oci_image_definitions_worker
    ON repository_oci_image_definitions FOR ALL
    USING (current_user = 'hephaestus_worker')
    WITH CHECK (current_user = 'hephaestus_worker');

GRANT SELECT, INSERT, UPDATE ON repository_oci_image_production_jobs
    TO hephaestus_worker;
GRANT SELECT, INSERT, UPDATE ON oci_image_materialization_jobs
    TO hephaestus_worker;
GRANT SELECT, INSERT, UPDATE ON repository_oci_image_production_jobs
    TO hephaestus_authz_owner;
GRANT UPDATE (status, image_reference, image_digest, provenance,
              failure_reason, updated_at)
    ON repository_oci_image_definitions TO hephaestus_worker;
GRANT SELECT ON repository_oci_image_definitions TO hephaestus_worker;
