-- A gateway revision has no ambient mailbox authority.  This records one
-- immutable, declared slot to one exact same-project mailbox and gives that
-- binding a separately revocable publication grant.
CREATE FUNCTION gateway_slot_array_is_valid(input_values text[]) RETURNS boolean
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT COALESCE(bool_and(value ~ '^[a-z][a-z0-9_-]{0,63}$'), true)
    FROM unnest(input_values) AS value
$$;
REVOKE ALL ON FUNCTION gateway_slot_array_is_valid(text[]) FROM PUBLIC;

ALTER TABLE gateway_revisions
    ADD COLUMN mailbox_slots text[] NOT NULL DEFAULT '{}'
        CHECK (cardinality(mailbox_slots) <= 32)
        CHECK (gateway_text_array_is_unique(mailbox_slots))
        CHECK (gateway_slot_array_is_valid(mailbox_slots));

CREATE TABLE gateway_mailbox_bindings (
    id uuid PRIMARY KEY,
    gateway_revision_id uuid NOT NULL,
    gateway_id uuid NOT NULL,
    project_id uuid NOT NULL REFERENCES projects(id),
    slot_key text NOT NULL CHECK (slot_key ~ '^[a-z][a-z0-9_-]{0,63}$'),
    mailbox_id uuid NOT NULL REFERENCES mailboxes(id),
    producer_id text NOT NULL CHECK (length(producer_id) BETWEEN 1 AND 128)
        CHECK (producer_id = btrim(producer_id) AND producer_id !~ '[[:cntrl:]]'),
    created_by uuid NOT NULL REFERENCES users(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (gateway_revision_id, slot_key),
    UNIQUE (id, gateway_revision_id),
    FOREIGN KEY (gateway_revision_id, gateway_id)
        REFERENCES gateway_revisions(id, gateway_id),
    FOREIGN KEY (gateway_id, project_id) REFERENCES gateways(id, project_id),
    FOREIGN KEY (mailbox_id, project_id) REFERENCES mailboxes(id, project_id)
);

CREATE FUNCTION enforce_gateway_mailbox_binding_scope() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE revision gateway_revisions%ROWTYPE; mailbox mailboxes%ROWTYPE;
BEGIN
    SELECT * INTO revision FROM gateway_revisions WHERE id = NEW.gateway_revision_id;
    SELECT * INTO mailbox FROM mailboxes WHERE id = NEW.mailbox_id;
    IF revision.id IS NULL OR mailbox.id IS NULL
       OR revision.gateway_id <> NEW.gateway_id OR revision.project_id <> NEW.project_id
       OR mailbox.project_id <> NEW.project_id OR mailbox.state <> 'active'
       OR NOT (NEW.slot_key = ANY(revision.mailbox_slots)) THEN
        RAISE EXCEPTION 'gateway mailbox binding is outside its exact revision authority'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_gateway_mailbox_binding_scope() FROM PUBLIC;
CREATE TRIGGER gateway_mailbox_bindings_exact_scope
BEFORE INSERT ON gateway_mailbox_bindings FOR EACH ROW
EXECUTE FUNCTION enforce_gateway_mailbox_binding_scope();
CREATE TRIGGER gateway_mailbox_bindings_immutable
BEFORE UPDATE OR DELETE ON gateway_mailbox_bindings FOR EACH ROW
EXECUTE FUNCTION reject_gateway_revision_mutation();

CREATE TABLE gateway_mailbox_binding_grants (
    id uuid PRIMARY KEY,
    binding_id uuid NOT NULL UNIQUE REFERENCES gateway_mailbox_bindings(id),
    status text NOT NULL CHECK (status IN ('active', 'revoked')),
    granted_by uuid NOT NULL REFERENCES users(id),
    granted_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    revoked_by uuid REFERENCES users(id),
    CHECK ((status = 'active' AND revoked_at IS NULL AND revoked_by IS NULL)
        OR (status = 'revoked' AND revoked_at IS NOT NULL AND revoked_by IS NOT NULL))
);

CREATE FUNCTION enforce_gateway_mailbox_grant_scope() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE binding gateway_mailbox_bindings%ROWTYPE;
BEGIN
    SELECT * INTO binding FROM gateway_mailbox_bindings WHERE id = NEW.binding_id;
    IF binding.id IS NULL OR check_permission('user', NEW.granted_by::text,
        'can_grant_agent_capability', 'gateway', binding.gateway_id::text) <> 1
       OR check_permission('user', NEW.granted_by::text,
        'can_grant_agent_capability', 'agent_instance',
        (SELECT instance_id::text FROM mailboxes WHERE id = binding.mailbox_id)) <> 1 THEN
        RAISE EXCEPTION 'gateway mailbox publication grant is not authorized'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_gateway_mailbox_grant_scope() FROM PUBLIC;
CREATE TRIGGER gateway_mailbox_binding_grants_scope
BEFORE INSERT ON gateway_mailbox_binding_grants FOR EACH ROW
EXECUTE FUNCTION enforce_gateway_mailbox_grant_scope();
CREATE FUNCTION enforce_gateway_mailbox_grant_lifecycle() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE binding gateway_mailbox_bindings%ROWTYPE;
BEGIN
    IF NEW.id <> OLD.id OR NEW.binding_id <> OLD.binding_id OR NEW.granted_by <> OLD.granted_by
       OR NEW.granted_at <> OLD.granted_at THEN
        RAISE EXCEPTION 'gateway mailbox grant identity is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.status <> 'active' OR NEW.status <> 'revoked' OR NEW.revoked_at IS NULL
       OR NEW.revoked_by IS NULL THEN
        RAISE EXCEPTION 'invalid gateway mailbox grant lifecycle transition'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    SELECT * INTO binding FROM gateway_mailbox_bindings WHERE id = OLD.binding_id;
    IF binding.id IS NULL OR check_permission('user', NEW.revoked_by::text,
        'can_grant_agent_capability', 'gateway', binding.gateway_id::text) <> 1
       OR check_permission('user', NEW.revoked_by::text,
        'can_grant_agent_capability', 'agent_instance',
        (SELECT instance_id::text FROM mailboxes WHERE id = binding.mailbox_id)) <> 1 THEN
        RAISE EXCEPTION 'gateway mailbox publication revocation is not authorized'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER gateway_mailbox_binding_grants_lifecycle
BEFORE UPDATE ON gateway_mailbox_binding_grants FOR EACH ROW
EXECUTE FUNCTION enforce_gateway_mailbox_grant_lifecycle();

-- The record is value-free provenance.  It has an idempotency boundary before
-- mailbox acceptance so duplicate guest frames cannot create a second body or
-- event even while concurrent requests race the mailbox uniqueness boundary.
CREATE TABLE gateway_mailbox_publications (
    id uuid PRIMARY KEY,
    invocation_id uuid NOT NULL REFERENCES gateway_invocations(id),
    runtime_session_id uuid NOT NULL REFERENCES gateway_runtime_authority_sessions(id),
    gateway_revision_id uuid NOT NULL,
    binding_id uuid REFERENCES gateway_mailbox_bindings(id),
    grant_id uuid REFERENCES gateway_mailbox_binding_grants(id),
    mailbox_id uuid REFERENCES mailboxes(id),
    producer_id text,
    slot_key text NOT NULL CHECK (slot_key ~ '^[a-z][a-z0-9_-]{0,63}$'),
    deduplication_key text NOT NULL CHECK (length(deduplication_key) BETWEEN 1 AND 256),
    event_id uuid REFERENCES mailbox_events(id),
    outcome text NOT NULL CHECK (outcome IN ('accepted', 'duplicate', 'denied')),
    denial_code text CHECK (denial_code IS NULL OR denial_code IN ('authority_unavailable')),
    accepted_at timestamptz NOT NULL DEFAULT now(),
    settled_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (invocation_id, slot_key, deduplication_key),
    FOREIGN KEY (binding_id, gateway_revision_id)
        REFERENCES gateway_mailbox_bindings(id, gateway_revision_id),
    CHECK ((outcome IN ('accepted', 'duplicate') AND event_id IS NOT NULL
            AND binding_id IS NOT NULL AND grant_id IS NOT NULL AND mailbox_id IS NOT NULL AND producer_id IS NOT NULL
            AND denial_code IS NULL)
        OR (outcome = 'denied' AND event_id IS NULL AND denial_code IS NOT NULL))
);
CREATE INDEX gateway_mailbox_publications_invocation
    ON gateway_mailbox_publications (invocation_id, accepted_at DESC, id);
CREATE TRIGGER gateway_mailbox_publications_immutable
BEFORE UPDATE OR DELETE ON gateway_mailbox_publications FOR EACH ROW
EXECUTE FUNCTION reject_gateway_revision_mutation();

ALTER TABLE gateway_mailbox_bindings ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_mailbox_bindings FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_mailbox_bindings_read ON gateway_mailbox_bindings FOR SELECT TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_read', 'gateway', gateway_id::text) = 1);
CREATE POLICY gateway_mailbox_bindings_worker ON gateway_mailbox_bindings TO hephaestus_worker USING (true) WITH CHECK (true);
ALTER TABLE gateway_mailbox_binding_grants ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_mailbox_binding_grants FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_mailbox_binding_grants_read ON gateway_mailbox_binding_grants FOR SELECT TO hephaestus_app
    USING (EXISTS (SELECT 1 FROM gateway_mailbox_bindings binding WHERE binding.id = binding_id
        AND check_permission('user', hephaestus_actor_id(), 'can_read', 'gateway', binding.gateway_id::text) = 1));
CREATE POLICY gateway_mailbox_binding_grants_worker ON gateway_mailbox_binding_grants TO hephaestus_worker USING (true) WITH CHECK (true);
ALTER TABLE gateway_mailbox_publications ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_mailbox_publications FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_mailbox_publications_read ON gateway_mailbox_publications FOR SELECT TO hephaestus_app
    USING (EXISTS (SELECT 1 FROM gateway_invocations invocation WHERE invocation.id = invocation_id
        AND check_permission('user', hephaestus_actor_id(), 'can_read', 'gateway', invocation.gateway_id::text) = 1));
CREATE POLICY gateway_mailbox_publications_worker ON gateway_mailbox_publications TO hephaestus_worker USING (true) WITH CHECK (true);
GRANT SELECT, INSERT ON gateway_mailbox_bindings, gateway_mailbox_binding_grants,
    gateway_mailbox_publications TO hephaestus_worker;
GRANT UPDATE ON gateway_mailbox_binding_grants TO hephaestus_worker;
GRANT SELECT ON gateway_mailbox_bindings, gateway_mailbox_binding_grants,
    gateway_mailbox_publications TO hephaestus_app;
