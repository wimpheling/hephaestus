-- A mailbox attempt is claimed before runtime authority is minted.  If a
-- dispatch-time authorization, artifact, or resource check fails in that
-- interval, it has a durable run but deliberately has no authority snapshot
-- and must still be settled without being retried as an unknown guest effect.
-- Successful execution continues to require the exact snapshot evidence.

ALTER TABLE mailbox_delivery_attempts
    DROP CONSTRAINT mailbox_delivery_attempts_run_snapshot_evidence;

ALTER TABLE mailbox_delivery_attempts
    ADD CONSTRAINT mailbox_delivery_attempts_run_snapshot_evidence
    CHECK (
        (run_id IS NULL AND authorization_snapshot_id IS NULL)
        OR (run_id IS NOT NULL AND authorization_snapshot_id IS NOT NULL)
        OR (
            run_id IS NOT NULL
            AND authorization_snapshot_id IS NULL
            AND state IN ('leased', 'failed', 'uncertain')
        )
    );
