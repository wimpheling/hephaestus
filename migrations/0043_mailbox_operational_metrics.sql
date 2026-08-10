-- Aggregate-only operational evidence. No ID, route, producer, header, trace
-- or payload value is exposed as a metric label or JSON field.
CREATE VIEW mailbox_operation_metrics
WITH (security_barrier = true) AS
SELECT
    (SELECT count(*) FROM mailbox_deliveries
     WHERE disposition IN ('pending', 'eligible', 'retryable')) AS queue_depth,
    (SELECT count(*) FROM mailbox_deliveries WHERE disposition = 'running') AS active_runs,
    (SELECT count(*) FROM mailbox_deliveries WHERE disposition = 'retryable') AS retries,
    (SELECT count(*) FROM mailbox_deliveries WHERE disposition = 'dead_lettered') AS dead_letters,
    (SELECT count(*) FROM mailbox_deliveries WHERE disposition = 'denied') AS denials,
    (SELECT count(*) FROM outbox
     WHERE published_at IS NULL AND subject LIKE 'heph.mailbox.v1.%') AS unpublished_commands,
    (SELECT count(*) FROM mailbox_payloads WHERE body_purged_at IS NOT NULL) AS payloads_purged,
    (SELECT count(*) FROM mailbox_delivery_attempts WHERE state = 'uncertain')
        AS reconciliation_uncertain_attempts,
    (SELECT COALESCE(
        avg(EXTRACT(EPOCH FROM (attempt.created_at - event.received_at)) * 1000)::bigint,
        0
     )
     FROM mailbox_delivery_attempts AS attempt
     JOIN mailbox_events AS event ON event.id = attempt.event_id) AS acceptance_to_dispatch_milliseconds;

GRANT SELECT ON mailbox_operation_metrics TO hephaestus_app, hephaestus_worker;
