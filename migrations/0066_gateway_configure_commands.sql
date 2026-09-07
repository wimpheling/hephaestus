-- Immutable request identity for runtime gateway configuration. Product
-- events are aggregate evidence; this ledger preserves exact replay results
-- even after a later revision becomes active.
CREATE TABLE gateway_configure_commands (
    command_key bytea PRIMARY KEY CHECK (octet_length(command_key) = 32),
    operation text NOT NULL CHECK (operation = 'configure_gateway'),
    gateway_id uuid NOT NULL REFERENCES gateways(id),
    expected_revision_id uuid NOT NULL REFERENCES gateway_revisions(id),
    payload_hash bytea NOT NULL CHECK (octet_length(payload_hash) = 32),
    actor_id uuid NOT NULL REFERENCES users(id),
    request_id uuid,
    result_revision_id uuid REFERENCES gateway_revisions(id),
    completed_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE gateway_configure_commands ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_configure_commands FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_configure_commands_manage ON gateway_configure_commands
    FOR ALL TO hephaestus_app
    USING (EXISTS (
        SELECT 1 FROM gateways
        WHERE gateways.id = gateway_configure_commands.gateway_id
          AND check_permission('user', hephaestus_actor_id(), 'can_manage',
              'project', gateways.project_id::text) = 1
    ))
    WITH CHECK (EXISTS (
        SELECT 1 FROM gateways
        WHERE gateways.id = gateway_configure_commands.gateway_id
          AND check_permission('user', hephaestus_actor_id(), 'can_manage',
              'project', gateways.project_id::text) = 1
    ));
CREATE POLICY gateway_configure_commands_worker ON gateway_configure_commands
    TO hephaestus_worker USING (true) WITH CHECK (true);
GRANT SELECT, INSERT ON gateway_configure_commands TO hephaestus_app;
GRANT SELECT, INSERT, UPDATE ON gateway_configure_commands TO hephaestus_worker;
