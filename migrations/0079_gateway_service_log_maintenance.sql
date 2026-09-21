-- Bounded worker-owned retention may delete payload rows and permanently
-- ineligible epoch metadata.  The application reader remains SELECT-only.
GRANT DELETE ON gateway_service_log_chunks, gateway_service_log_epochs
    TO hephaestus_worker;

ALTER TABLE gateway_service_log_project_usage
    ADD COLUMN pressure_cleanup_pending boolean NOT NULL DEFAULT false;
ALTER TABLE gateway_service_log_epochs
    ADD COLUMN pressure_cleanup_pending boolean NOT NULL DEFAULT false;

CREATE INDEX gateway_service_log_chunks_project_retention
    ON gateway_service_log_chunks
        (project_id, stored_at, instance_id, fencing_token, sequence);
