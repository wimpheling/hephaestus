-- Gateway readers may inspect value-free authorization provenance for their
-- own gateway. Session credentials, request identities, payloads, headers,
-- deduplication keys, and denial detail remain absent from this projection.
CREATE POLICY gateway_authorization_snapshots_gateway_reader
    ON gateway_authorization_snapshots FOR SELECT TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'gateway', gateway_id::text) = 1);

CREATE POLICY gateway_runtime_authority_sessions_gateway_reader
    ON gateway_runtime_authority_sessions FOR SELECT TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'gateway', gateway_id::text) = 1);

CREATE POLICY gateway_authorization_snapshot_bindings_gateway_reader
    ON gateway_authorization_snapshot_bindings FOR SELECT TO hephaestus_app
    USING (EXISTS (
        SELECT 1 FROM gateway_authorization_snapshots snapshot
        WHERE snapshot.id = gateway_authorization_snapshot_bindings.snapshot_id
          AND check_permission('user', hephaestus_actor_id(), 'can_read',
              'gateway', snapshot.gateway_id::text) = 1
    ));

GRANT SELECT ON gateway_authorization_snapshots,
    gateway_runtime_authority_sessions,
    gateway_authorization_snapshot_bindings TO hephaestus_app;
