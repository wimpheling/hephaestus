-- Repair the lifecycle-event trigger introduced with the preparation-history
-- table. A searched predicate was accidentally written as a simple CASE arm,
-- which caused every newly queued repository image to fail before a worker
-- could claim it. Replacing the function is safe for existing durable jobs:
-- their next state transition records the correct bounded outcome.

CREATE OR REPLACE FUNCTION record_repository_oci_image_job_event() RETURNS trigger
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
        CASE
            WHEN NEW.state IN ('queued', 'claimed') THEN 'pending'
            WHEN NEW.state = 'succeeded' THEN 'succeeded'
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
