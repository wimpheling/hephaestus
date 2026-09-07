-- Preserve exact create/revoke mailbox-binding results across transport
-- retries. Product events remain aggregate evidence; this ledger is the
-- request identity needed to replay the original immutable binding.
CREATE TABLE gateway_mailbox_binding_commands (
    command_key bytea PRIMARY KEY CHECK (octet_length(command_key) = 32),
    operation text NOT NULL CHECK (operation IN (
        'create_gateway_mailbox_binding',
        'revoke_gateway_mailbox_binding_grant'
    )),
    gateway_revision_id uuid NOT NULL REFERENCES gateway_revisions(id),
    slot_key text,
    mailbox_id uuid,
    producer_id text,
    target_binding_id uuid REFERENCES gateway_mailbox_bindings(id),
    payload_hash bytea NOT NULL CHECK (octet_length(payload_hash) = 32),
    actor_id uuid NOT NULL REFERENCES users(id),
    request_id uuid,
    result_binding_id uuid REFERENCES gateway_mailbox_bindings(id),
    completed_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((operation = 'create_gateway_mailbox_binding'
            AND slot_key IS NOT NULL AND mailbox_id IS NOT NULL
            AND producer_id IS NOT NULL AND target_binding_id IS NULL)
        OR (operation = 'revoke_gateway_mailbox_binding_grant'
            AND target_binding_id IS NOT NULL
            AND slot_key IS NULL AND mailbox_id IS NULL AND producer_id IS NULL))
);

ALTER TABLE gateway_mailbox_binding_commands ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_mailbox_binding_commands FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_mailbox_binding_commands_manage
    ON gateway_mailbox_binding_commands FOR ALL TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(),
        'can_grant_agent_capability', 'gateway_revision', gateway_revision_id::text) = 1)
    WITH CHECK (check_permission('user', hephaestus_actor_id(),
        'can_grant_agent_capability', 'gateway_revision', gateway_revision_id::text) = 1);
GRANT SELECT, INSERT, UPDATE ON gateway_mailbox_binding_commands TO hephaestus_app;
