-- A mailbox delivery is a normal reusable-instance execution even though it
-- has no Git receive and therefore no run_request.  Retain the exact source
-- selected when the dispatcher claims the attempt so secret authorization and
-- later inspection never need to follow a mutable repository ref.

ALTER TABLE mailbox_delivery_attempts
    ADD COLUMN target_ref text,
    ADD COLUMN target_commit text,
    ADD CONSTRAINT mailbox_delivery_attempts_source_provenance
        CHECK (
            (target_ref IS NULL AND target_commit IS NULL)
            OR (
                target_ref LIKE 'refs/%'
                AND (
                    target_commit ~ '^[0-9a-f]{40}$'
                    OR target_commit ~ '^[0-9a-f]{64}$'
                )
            )
        );
