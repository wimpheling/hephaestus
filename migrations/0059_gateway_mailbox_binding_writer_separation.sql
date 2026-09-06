-- The runtime worker settles publications and snapshots, but it must never be
-- able to mint, revoke, or impersonate an operator's mailbox publication
-- grant. Those lifecycle writes are actor-scoped application operations.
DROP POLICY gateway_mailbox_bindings_worker ON gateway_mailbox_bindings;
DROP POLICY gateway_mailbox_binding_grants_worker ON gateway_mailbox_binding_grants;

CREATE POLICY gateway_mailbox_bindings_worker_read
    ON gateway_mailbox_bindings FOR SELECT TO hephaestus_worker USING (true);
CREATE POLICY gateway_mailbox_binding_grants_worker_read
    ON gateway_mailbox_binding_grants FOR SELECT TO hephaestus_worker USING (true);

CREATE POLICY gateway_mailbox_bindings_manage
    ON gateway_mailbox_bindings FOR INSERT TO hephaestus_app
    WITH CHECK (
        created_by::text = hephaestus_actor_id()
        AND check_permission('user', hephaestus_actor_id(), 'can_grant_agent_capability',
            'gateway', gateway_id::text) = 1
        AND check_permission('user', hephaestus_actor_id(), 'can_grant_agent_capability',
            'agent_instance', (SELECT instance_id::text FROM mailboxes WHERE id = mailbox_id)) = 1
    );

CREATE POLICY gateway_mailbox_binding_grants_manage
    ON gateway_mailbox_binding_grants FOR INSERT TO hephaestus_app
    WITH CHECK (
        granted_by::text = hephaestus_actor_id()
        AND EXISTS (
            SELECT 1
            FROM gateway_mailbox_bindings binding
            JOIN mailboxes mailbox ON mailbox.id = binding.mailbox_id
            WHERE binding.id = binding_id
              AND check_permission('user', hephaestus_actor_id(), 'can_grant_agent_capability',
                    'gateway', binding.gateway_id::text) = 1
              AND check_permission('user', hephaestus_actor_id(), 'can_grant_agent_capability',
                    'agent_instance', mailbox.instance_id::text) = 1
        )
    );

CREATE POLICY gateway_mailbox_binding_grants_revoke
    ON gateway_mailbox_binding_grants FOR UPDATE TO hephaestus_app
    USING (
        EXISTS (
            SELECT 1
            FROM gateway_mailbox_bindings binding
            JOIN mailboxes mailbox ON mailbox.id = binding.mailbox_id
            WHERE binding.id = binding_id
              AND check_permission('user', hephaestus_actor_id(), 'can_grant_agent_capability',
                    'gateway', binding.gateway_id::text) = 1
              AND check_permission('user', hephaestus_actor_id(), 'can_grant_agent_capability',
                    'agent_instance', mailbox.instance_id::text) = 1
        )
    ) WITH CHECK (revoked_by::text = hephaestus_actor_id());

REVOKE INSERT, UPDATE ON gateway_mailbox_bindings, gateway_mailbox_binding_grants
    FROM hephaestus_worker;
-- Keep the worker's authority check readable after removing its lifecycle
-- write privileges above.
GRANT SELECT ON gateway_mailbox_bindings, gateway_mailbox_binding_grants TO hephaestus_worker;
GRANT INSERT ON gateway_mailbox_bindings, gateway_mailbox_binding_grants TO hephaestus_app;
GRANT UPDATE ON gateway_mailbox_binding_grants TO hephaestus_app;
