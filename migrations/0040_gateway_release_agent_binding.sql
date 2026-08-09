-- A gateway must select one exact released-agent runtime contract. A release
-- may contain multiple agents, so release identity alone is insufficient.
ALTER TABLE gateway_revisions
    ADD COLUMN release_agent_id uuid REFERENCES release_agents(id),
    ADD COLUMN release_agent_key text;
ALTER TABLE gateway_revisions
    ADD CONSTRAINT gateway_revisions_release_agent_exact
    CHECK ((release_id IS NULL AND release_agent_id IS NULL AND release_agent_key IS NULL)
        OR (release_id IS NOT NULL AND release_agent_id IS NOT NULL AND release_agent_key IS NOT NULL));

-- Pin the handler to an agent from precisely the release that supplied the
-- declaration.  The text key is retained as immutable operator/audit evidence;
-- dispatch uses the UUID, never a later key lookup.
ALTER TABLE gateway_revisions
    ADD CONSTRAINT gateway_revisions_release_agent_release_fk
    FOREIGN KEY (release_agent_id, release_id)
    REFERENCES release_agents (id, release_id);
