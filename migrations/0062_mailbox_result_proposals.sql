-- Mailbox runs retain their frozen Git target in the delivery attempt rather
-- than run_requests. Both origins must create the same controlled proposal.
CREATE OR REPLACE FUNCTION create_review_proposal() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.state = 'completed' AND NEW.result_commit IS NOT NULL THEN
        INSERT INTO review_proposals (
            id, result_id, repository_id, run_id, target_ref,
            input_commit, result_commit, result_ref
        )
        SELECT
            gen_random_uuid(), NEW.id, NEW.repository_id, NEW.run_id,
            COALESCE(request.git_ref, attempt.target_ref),
            NEW.input_commit, NEW.result_commit, NEW.result_ref
        FROM runs run
        LEFT JOIN run_requests request ON request.run_id = run.id
        LEFT JOIN mailbox_delivery_attempts attempt ON attempt.run_id = run.id
        LEFT JOIN agent_attachments attachment
          ON attachment.id = run.attachment_id AND attachment.instance_id = run.instance_id
        WHERE run.id = NEW.run_id
          AND COALESCE(request.repository_id, attachment.repository_id) = NEW.repository_id
          AND COALESCE(request.commit_sha, attempt.target_commit) = NEW.input_commit
        ON CONFLICT (result_id) DO NOTHING;
    END IF;
    RETURN NEW;
END
$$;

-- Restore reviewability for completed historical mailbox results without
-- changing canonical Git or overriding an existing review decision.
INSERT INTO review_proposals (
    id, result_id, repository_id, run_id, target_ref,
    input_commit, result_commit, result_ref
)
SELECT gen_random_uuid(), result.id, result.repository_id, result.run_id,
       attempt.target_ref, result.input_commit, result.result_commit, result.result_ref
FROM run_results result
JOIN runs run ON run.id = result.run_id
JOIN mailbox_delivery_attempts attempt ON attempt.run_id = run.id
JOIN agent_attachments attachment
  ON attachment.id = run.attachment_id AND attachment.instance_id = run.instance_id
WHERE result.state = 'completed' AND result.result_commit IS NOT NULL
  AND attachment.repository_id = result.repository_id
  AND attempt.target_commit = result.input_commit
ON CONFLICT (result_id) DO NOTHING;
