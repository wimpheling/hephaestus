-- Aggregate-only gateway telemetry. This view intentionally contains no route,
-- request, response, public authority, trace, or secret value; callers can
-- observe only live-authorized operational totals for the gateways they may
-- inspect.
CREATE VIEW gateway_operation_metrics
WITH (security_barrier = true) AS
WITH visible_gateways AS MATERIALIZED (
    SELECT gateway.id
    FROM gateways AS gateway
    WHERE check_permission(
        hephaestus_subject_type(), hephaestus_actor_id(), 'can_read',
        'gateway', gateway.id::text
    ) = 1
)
SELECT
    (SELECT count(*) FROM gateways
     WHERE id IN (SELECT id FROM visible_gateways)
       AND lifecycle = 'enabled') AS enabled_gateways,
    (SELECT count(*) FROM gateways
     WHERE id IN (SELECT id FROM visible_gateways)
       AND lifecycle = 'paused') AS paused_gateways,
    (SELECT count(*) FROM gateways
     WHERE id IN (SELECT id FROM visible_gateways)
       AND lifecycle = 'removed') AS removed_gateways,
    (SELECT count(*) FROM gateway_routes AS route
     JOIN gateway_revisions AS revision ON revision.id = route.gateway_revision_id
     JOIN gateways AS gateway ON gateway.id = revision.gateway_id
     WHERE gateway.id IN (SELECT id FROM visible_gateways)
       AND route.enabled AND gateway.lifecycle = 'enabled'
       AND gateway.active_revision_id = revision.id) AS active_routes,
    (SELECT count(*) FROM gateway_invocations
     WHERE gateway_id IN (SELECT id FROM visible_gateways)
       AND outcome = 'accepted') AS in_flight_invocations,
    (SELECT count(*) FROM gateway_invocations
     WHERE gateway_id IN (SELECT id FROM visible_gateways)
       AND outcome = 'completed') AS completed_invocations,
    (SELECT count(*) FROM gateway_invocations
     WHERE gateway_id IN (SELECT id FROM visible_gateways)
       AND outcome = 'rejected') AS rejected_invocations,
    (SELECT count(*) FROM gateway_invocations
     WHERE gateway_id IN (SELECT id FROM visible_gateways)
       AND outcome = 'failed') AS failed_invocations,
    (SELECT count(*) FROM gateway_invocations
     WHERE gateway_id IN (SELECT id FROM visible_gateways)
       AND outcome = 'timed_out') AS timed_out_invocations,
    COALESCE((SELECT avg(EXTRACT(EPOCH FROM (completed_at - accepted_at)) * 1000)::bigint
     FROM gateway_invocations
     WHERE gateway_id IN (SELECT id FROM visible_gateways)
       AND completed_at IS NOT NULL), 0) AS average_invocation_milliseconds;

GRANT SELECT ON gateway_operation_metrics TO hephaestus_app, hephaestus_worker;
