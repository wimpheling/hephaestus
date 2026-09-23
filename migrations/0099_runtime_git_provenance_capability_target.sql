-- Runtime Git publication targets are release-owned capability repositories.
-- Trigger attachments retain routing provenance but cannot constrain the
-- capability repository selected by the immutable revision binding.
ALTER TABLE run_instance_provenance
    DROP CONSTRAINT run_provenance_exact_trigger_attachment;

COMMENT ON COLUMN run_instance_provenance.target_repository_id IS
    'Exact immutable runtime capability repository; trigger routing remains attachment_id and run_requests.repository_id.';
