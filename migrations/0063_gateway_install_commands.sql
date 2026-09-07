-- Durable command identity and exact result rows for release gateway
-- installation.  Product events describe aggregate changes; they cannot
-- serve as a request ledger because a later release may activate another
-- revision for the same gateway.
CREATE TABLE gateway_install_commands (
    command_key bytea PRIMARY KEY CHECK (octet_length(command_key) = 32),
    operation text NOT NULL CHECK (operation = 'install_release_gateways'),
    project_id uuid NOT NULL REFERENCES projects(id),
    repository_id uuid NOT NULL REFERENCES repositories(id),
    release_id uuid NOT NULL REFERENCES releases(id),
    actor_id uuid NOT NULL REFERENCES users(id),
    request_id uuid,
    completed_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (command_key, project_id, repository_id, release_id)
);

CREATE TABLE gateway_install_command_results (
    command_key bytea NOT NULL REFERENCES gateway_install_commands(command_key),
    ordinal integer NOT NULL CHECK (ordinal > 0),
    gateway_id uuid NOT NULL REFERENCES gateways(id),
    revision_id uuid NOT NULL,
    PRIMARY KEY (command_key, ordinal),
    UNIQUE (command_key, gateway_id),
    FOREIGN KEY (revision_id, gateway_id)
        REFERENCES gateway_revisions(id, gateway_id)
);

ALTER TABLE gateway_install_commands ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_install_commands FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_install_commands_manage ON gateway_install_commands
    FOR ALL TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'project', project_id::text) = 1)
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'project', project_id::text) = 1);
CREATE POLICY gateway_install_commands_worker ON gateway_install_commands
    TO hephaestus_worker USING (true) WITH CHECK (true);

ALTER TABLE gateway_install_command_results ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_install_command_results FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_install_command_results_manage ON gateway_install_command_results
    FOR ALL TO hephaestus_app
    USING (EXISTS (
        SELECT 1 FROM gateway_install_commands command
        WHERE command.command_key = gateway_install_command_results.command_key
          AND check_permission('user', hephaestus_actor_id(), 'can_manage',
              'project', command.project_id::text) = 1
    ))
    WITH CHECK (EXISTS (
        SELECT 1 FROM gateway_install_commands command
        WHERE command.command_key = gateway_install_command_results.command_key
          AND check_permission('user', hephaestus_actor_id(), 'can_manage',
              'project', command.project_id::text) = 1
    ));
CREATE POLICY gateway_install_command_results_worker ON gateway_install_command_results
    TO hephaestus_worker USING (true) WITH CHECK (true);

GRANT SELECT, INSERT ON gateway_install_commands,
    gateway_install_command_results TO hephaestus_app, hephaestus_worker;

-- Installation is authorized as the application actor, then performs the
-- immutable gateway writes through the dedicated worker role. The worker
-- already has read access from the gateway edge migration; these are the
-- narrow write privileges required by this release installation command.
GRANT INSERT, UPDATE ON gateways, gateway_revisions, gateway_routes
    TO hephaestus_worker;
GRANT EXECUTE ON FUNCTION gateway_text_array_is_unique(text[]) TO hephaestus_worker;
GRANT EXECUTE ON FUNCTION gateway_slot_array_is_valid(text[]) TO hephaestus_worker;
