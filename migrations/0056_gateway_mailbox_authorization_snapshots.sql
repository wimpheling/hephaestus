-- A gateway session must retain the exact mailbox publication ceiling that
-- was live at issuance.  The guest never receives these identifiers; they are
-- durable, immutable host-side authorization evidence for the invocation.
ALTER TABLE gateway_mailbox_binding_grants
    ADD CONSTRAINT gateway_mailbox_binding_grants_id_binding_unique UNIQUE (id, binding_id);

CREATE TABLE gateway_authorization_snapshot_bindings (
    snapshot_id uuid NOT NULL REFERENCES gateway_authorization_snapshots(id),
    gateway_revision_id uuid NOT NULL,
    ordinal integer NOT NULL CHECK (ordinal >= 0),
    binding_id uuid NOT NULL,
    grant_id uuid NOT NULL,
    binding_hash bytea NOT NULL CHECK (octet_length(binding_hash) = 32),
    slot_key text NOT NULL CHECK (slot_key ~ '^[a-z][a-z0-9_-]{0,63}$'),
    resource_kind text NOT NULL CHECK (resource_kind = 'mailbox'),
    resource_id uuid NOT NULL REFERENCES mailboxes(id),
    granted_operations text[] NOT NULL CHECK (granted_operations = ARRAY['publish']::text[]),
    PRIMARY KEY (snapshot_id, ordinal),
    UNIQUE (snapshot_id, binding_id),
    FOREIGN KEY (snapshot_id, gateway_revision_id)
        REFERENCES gateway_authorization_snapshots(id, gateway_revision_id),
    FOREIGN KEY (binding_id, gateway_revision_id)
        REFERENCES gateway_mailbox_bindings(id, gateway_revision_id),
    FOREIGN KEY (grant_id, binding_id)
        REFERENCES gateway_mailbox_binding_grants(id, binding_id)
);

CREATE FUNCTION enforce_gateway_snapshot_mailbox_binding_copy() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE binding gateway_mailbox_bindings%ROWTYPE;
        binding_grant gateway_mailbox_binding_grants%ROWTYPE;
BEGIN
    SELECT * INTO binding FROM gateway_mailbox_bindings
      WHERE id = NEW.binding_id AND gateway_revision_id = NEW.gateway_revision_id;
    SELECT * INTO binding_grant FROM gateway_mailbox_binding_grants
      WHERE id = NEW.grant_id AND binding_id = NEW.binding_id;
    IF binding.id IS NULL OR binding_grant.id IS NULL OR binding_grant.status <> 'active'
       OR NEW.slot_key <> binding.slot_key OR NEW.resource_id <> binding.mailbox_id
       OR NEW.resource_kind <> 'mailbox'
       OR NEW.granted_operations <> ARRAY['publish']::text[] THEN
        RAISE EXCEPTION 'gateway authorization snapshot binding does not match active mailbox publication authority'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_gateway_snapshot_mailbox_binding_copy() FROM PUBLIC;
CREATE TRIGGER gateway_authorization_snapshot_bindings_exact_copy
BEFORE INSERT ON gateway_authorization_snapshot_bindings
FOR EACH ROW EXECUTE FUNCTION enforce_gateway_snapshot_mailbox_binding_copy();
CREATE TRIGGER gateway_authorization_snapshot_bindings_immutable
BEFORE UPDATE OR DELETE ON gateway_authorization_snapshot_bindings
FOR EACH ROW EXECUTE FUNCTION reject_runtime_authority_immutable_record();

ALTER TABLE gateway_authorization_snapshot_bindings ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_authorization_snapshot_bindings FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_authorization_snapshot_bindings_worker
    ON gateway_authorization_snapshot_bindings TO hephaestus_worker
    USING (true) WITH CHECK (true);
GRANT SELECT, INSERT ON gateway_authorization_snapshot_bindings TO hephaestus_worker;
