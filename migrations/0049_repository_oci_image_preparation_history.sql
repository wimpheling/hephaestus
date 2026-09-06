-- Immutable, redacted lifecycle evidence for project-owned OCI preparation.
-- The evidence deliberately records phase transitions and durable identifiers,
-- never private checkout paths, command lines, credentials, or raw tool output.

CREATE TABLE repository_oci_image_preparation_events (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    definition_id uuid NOT NULL REFERENCES repository_oci_image_definitions(id) ON DELETE CASCADE,
    job_id uuid REFERENCES repository_oci_image_production_jobs(id) ON DELETE SET NULL,
    attempt integer CHECK (attempt IS NULL OR attempt > 0),
    phase text NOT NULL CHECK (phase IN ('requested', 'preparing', 'published', 'materializing', 'ready', 'failed')),
    outcome text NOT NULL CHECK (outcome IN ('pending', 'succeeded', 'failed')),
    output_digest text CHECK (output_digest IS NULL OR output_digest ~ '^sha256:[0-9a-f]{64}$'),
    safe_reason text CHECK (safe_reason IS NULL OR length(safe_reason) BETWEEN 1 AND 2048),
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((outcome <> 'failed' AND safe_reason IS NULL) OR outcome = 'failed')
);

CREATE INDEX repository_oci_image_preparation_events_by_definition
    ON repository_oci_image_preparation_events (definition_id, created_at, id);

ALTER TABLE repository_oci_image_preparation_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE repository_oci_image_preparation_events FORCE ROW LEVEL SECURITY;

CREATE POLICY repository_oci_image_preparation_events_select
    ON repository_oci_image_preparation_events FOR SELECT
    USING (EXISTS (
        SELECT 1
        FROM repository_oci_image_definitions AS definition
        WHERE definition.id = repository_oci_image_preparation_events.definition_id
    ));

CREATE POLICY repository_oci_image_preparation_events_worker
    ON repository_oci_image_preparation_events FOR ALL
    USING (current_user = 'hephaestus_worker')
    WITH CHECK (current_user = 'hephaestus_worker');

GRANT SELECT ON repository_oci_image_preparation_events TO hephaestus_app, hephaestus_worker;
GRANT INSERT ON repository_oci_image_preparation_events TO hephaestus_worker;

CREATE FUNCTION record_repository_oci_image_job_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET row_security = off
AS $$
BEGIN
    INSERT INTO repository_oci_image_preparation_events
        (definition_id, job_id, attempt, phase, outcome, output_digest, safe_reason)
    VALUES (
        NEW.definition_id,
        NEW.id,
        NEW.attempt,
        CASE NEW.state
            WHEN 'queued' THEN 'requested'
            WHEN 'claimed' THEN 'preparing'
            WHEN 'succeeded' THEN 'published'
            ELSE 'failed'
        END,
        CASE NEW.state
            WHEN NEW.state IN ('queued', 'claimed') THEN 'pending'
            WHEN 'succeeded' THEN 'succeeded'
            ELSE 'failed'
        END,
        NEW.output_image_digest,
        NEW.failure_reason
    );
    RETURN NEW;
END
$$;

ALTER FUNCTION record_repository_oci_image_job_event() OWNER TO hephaestus_authz_owner;
GRANT EXECUTE ON FUNCTION record_repository_oci_image_job_event() TO hephaestus_worker;

CREATE TRIGGER repository_oci_image_job_event
AFTER INSERT OR UPDATE OF state ON repository_oci_image_production_jobs
FOR EACH ROW EXECUTE FUNCTION record_repository_oci_image_job_event();

CREATE FUNCTION record_repository_oci_image_materialization_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    image_definition_id uuid;
BEGIN
    SELECT definition.id INTO image_definition_id
      FROM repository_oci_image_definitions AS definition
     WHERE definition.image_reference = NEW.image_reference;
    IF image_definition_id IS NULL THEN
        RETURN NEW;
    END IF;
    INSERT INTO repository_oci_image_preparation_events
        (definition_id, phase, outcome, safe_reason)
    VALUES (
        image_definition_id,
        'materializing',
        CASE NEW.state WHEN 'materialized' THEN 'succeeded' WHEN 'failed' THEN 'failed' ELSE 'pending' END,
        NEW.failure_reason
    );
    RETURN NEW;
END
$$;

ALTER FUNCTION record_repository_oci_image_materialization_event() OWNER TO hephaestus_authz_owner;
GRANT EXECUTE ON FUNCTION record_repository_oci_image_materialization_event() TO hephaestus_worker;

CREATE TRIGGER repository_oci_image_materialization_event
AFTER INSERT OR UPDATE OF state ON oci_image_materialization_jobs
FOR EACH ROW EXECUTE FUNCTION record_repository_oci_image_materialization_event();

CREATE FUNCTION record_repository_oci_image_ready_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
BEGIN
    IF NEW.status = 'ready' AND OLD.status <> 'ready' THEN
        INSERT INTO repository_oci_image_preparation_events
            (definition_id, phase, outcome, output_digest)
        VALUES (NEW.id, 'ready', 'succeeded', NEW.image_digest);
    END IF;
    RETURN NEW;
END
$$;

ALTER FUNCTION record_repository_oci_image_ready_event() OWNER TO hephaestus_authz_owner;
GRANT EXECUTE ON FUNCTION record_repository_oci_image_ready_event() TO hephaestus_worker;

CREATE TRIGGER repository_oci_image_ready_event
AFTER UPDATE OF status ON repository_oci_image_definitions
FOR EACH ROW EXECUTE FUNCTION record_repository_oci_image_ready_event();
