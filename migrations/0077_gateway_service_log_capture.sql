-- Persist the release-owned, default-off application log capture declaration.
-- Lifecycle failure records remain content-free; this policy only selects a
-- later bounded project-scoped application stream.
ALTER TABLE gateway_revisions
    ADD COLUMN service_log_capture_mode text NOT NULL DEFAULT 'disabled',
    ADD CONSTRAINT gateway_revisions_service_log_capture_mode_check
        CHECK (service_log_capture_mode IN ('disabled', 'application')),
    ADD CONSTRAINT gateway_revisions_service_log_capture_contract_check
        CHECK (
            handler_contract = 'http.service.v1'
            OR service_log_capture_mode = 'disabled'
        );
