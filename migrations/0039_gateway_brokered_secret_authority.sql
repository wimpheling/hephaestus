-- Gateway declarations already contain symbolic secret slots. This migration
-- makes one declared slot resolvable through the existing opaque import and
-- version lifecycle without adding plaintext to gateway, edge, or VM tables.

ALTER TABLE gateway_runtime_authority_sessions
    ADD CONSTRAINT gateway_runtime_authority_sessions_id_invocation_unique
    UNIQUE (id, invocation_id);

CREATE TABLE gateway_secret_bindings (
    id uuid PRIMARY KEY,
    gateway_id uuid NOT NULL,
    gateway_revision_id uuid NOT NULL,
    import_id uuid NOT NULL REFERENCES secret_imports(id),
    slot_key text NOT NULL CHECK (slot_key ~ '^[a-z][a-z0-9_-]{0,63}$'),
    secret_version_id uuid NOT NULL REFERENCES secret_versions(id),
    status text NOT NULL CHECK (status IN ('active', 'revoked', 'expired')),
    normalized_hash bytea NOT NULL CHECK (octet_length(normalized_hash) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    FOREIGN KEY (gateway_revision_id, gateway_id)
        REFERENCES gateway_revisions(id, gateway_id),
    UNIQUE (gateway_revision_id, slot_key),
    UNIQUE (id, gateway_revision_id),
    CHECK ((status = 'active' AND revoked_at IS NULL)
        OR (status IN ('revoked', 'expired') AND revoked_at IS NOT NULL))
);
CREATE INDEX gateway_secret_bindings_by_import
    ON gateway_secret_bindings (import_id, status, id);

CREATE FUNCTION enforce_gateway_secret_binding_scope() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    gateway_revision gateway_revisions%ROWTYPE;
    imported secret_imports%ROWTYPE;
    granted secret_grants%ROWTYPE;
    owned_secret secrets%ROWTYPE;
    selected_version secret_versions%ROWTYPE;
BEGIN
    SELECT * INTO gateway_revision FROM gateway_revisions
      WHERE id = NEW.gateway_revision_id AND gateway_id = NEW.gateway_id;
    SELECT * INTO imported FROM secret_imports WHERE id = NEW.import_id;
    SELECT * INTO granted FROM secret_grants WHERE id = imported.grant_id;
    SELECT * INTO owned_secret FROM secrets WHERE id = granted.secret_id;
    SELECT * INTO selected_version FROM secret_versions
      WHERE id = NEW.secret_version_id AND secret_id = owned_secret.id;
    IF gateway_revision.id IS NULL OR imported.id IS NULL OR granted.id IS NULL
       OR owned_secret.id IS NULL OR selected_version.id IS NULL
       OR imported.target_kind <> 'project'
       OR imported.target_id <> gateway_revision.project_id
       OR imported.status <> 'active' OR granted.status <> 'active'
       OR owned_secret.status <> 'active'
       OR selected_version.revoked_at IS NOT NULL OR selected_version.purged_at IS NOT NULL
       OR NOT (NEW.slot_key = ANY(gateway_revision.secret_slots))
    THEN
        RAISE EXCEPTION 'gateway secret binding is outside the exact declared/imported secret authority'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_gateway_secret_binding_scope() FROM PUBLIC;
CREATE TRIGGER gateway_secret_bindings_exact_scope
BEFORE INSERT ON gateway_secret_bindings
FOR EACH ROW EXECUTE FUNCTION enforce_gateway_secret_binding_scope();

CREATE FUNCTION reject_gateway_secret_binding_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.gateway_id <> OLD.gateway_id
       OR NEW.gateway_revision_id <> OLD.gateway_revision_id
       OR NEW.import_id <> OLD.import_id OR NEW.slot_key <> OLD.slot_key
       OR NEW.secret_version_id <> OLD.secret_version_id
       OR NEW.normalized_hash <> OLD.normalized_hash OR NEW.created_at <> OLD.created_at
    THEN
        RAISE EXCEPTION 'gateway secret binding authority is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER gateway_secret_bindings_immutable
BEFORE UPDATE ON gateway_secret_bindings
FOR EACH ROW EXECUTE FUNCTION reject_gateway_secret_binding_mutation();
CREATE TRIGGER gateway_secret_bindings_no_delete
BEFORE DELETE ON gateway_secret_bindings
FOR EACH ROW EXECUTE FUNCTION reject_brokered_secret_rule_mutation();

-- Inbound rules deliberately have a separate authority parent from outbound
-- agent rules. This prevents a route from borrowing an agent-run lease or a
-- gateway from reusing an outbound origin substitution rule.
CREATE TABLE gateway_brokered_secret_rules (
    id uuid PRIMARY KEY,
    binding_id uuid NOT NULL,
    gateway_revision_id uuid NOT NULL,
    gateway_route_id uuid NOT NULL,
    header_name text NOT NULL CHECK (header_name ~ '^[!#$%&''*+.^_`|~0-9a-z-]{1,64}$'),
    normalized_hash bytea NOT NULL CHECK (octet_length(normalized_hash) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (binding_id, gateway_revision_id)
        REFERENCES gateway_secret_bindings(id, gateway_revision_id),
    FOREIGN KEY (gateway_route_id, gateway_revision_id)
        REFERENCES gateway_routes(id, gateway_revision_id),
    UNIQUE (binding_id),
    UNIQUE (gateway_route_id, header_name)
);
CREATE TRIGGER gateway_brokered_secret_rules_immutable
BEFORE UPDATE OR DELETE ON gateway_brokered_secret_rules
FOR EACH ROW EXECUTE FUNCTION reject_brokered_secret_rule_mutation();

-- The existing gateway runtime session credential is the sole authenticated
-- bearer. This lease adds only an exact version/rule ceiling for one accepted
-- invocation, keeping real secret resolution host-side and value-free.
CREATE TABLE gateway_secret_leases (
    id uuid PRIMARY KEY,
    runtime_session_id uuid NOT NULL,
    invocation_id uuid NOT NULL,
    binding_id uuid NOT NULL,
    secret_version_id uuid NOT NULL REFERENCES secret_versions(id),
    rule_id uuid NOT NULL,
    status text NOT NULL CHECK (status IN ('active', 'revoked', 'expired')),
    issued_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    UNIQUE (runtime_session_id, binding_id),
    UNIQUE (invocation_id, rule_id),
    FOREIGN KEY (runtime_session_id, invocation_id)
        REFERENCES gateway_runtime_authority_sessions(id, invocation_id),
    CHECK (expires_at > issued_at),
    CHECK ((status = 'active' AND revoked_at IS NULL)
        OR (status IN ('revoked', 'expired') AND revoked_at IS NOT NULL))
);
CREATE INDEX gateway_secret_leases_live
    ON gateway_secret_leases (expires_at, id) WHERE status = 'active';

CREATE FUNCTION enforce_gateway_secret_lease_scope() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    session gateway_runtime_authority_sessions%ROWTYPE;
    invocation gateway_invocations%ROWTYPE;
    binding gateway_secret_bindings%ROWTYPE;
    rule gateway_brokered_secret_rules%ROWTYPE;
    imported secret_imports%ROWTYPE;
    granted secret_grants%ROWTYPE;
    owned_secret secrets%ROWTYPE;
    selected_version secret_versions%ROWTYPE;
BEGIN
    SELECT * INTO session FROM gateway_runtime_authority_sessions WHERE id = NEW.runtime_session_id;
    SELECT * INTO invocation FROM gateway_invocations WHERE id = NEW.invocation_id;
    SELECT * INTO binding FROM gateway_secret_bindings WHERE id = NEW.binding_id;
    SELECT * INTO rule FROM gateway_brokered_secret_rules WHERE id = NEW.rule_id;
    SELECT * INTO imported FROM secret_imports WHERE id = binding.import_id;
    SELECT * INTO granted FROM secret_grants WHERE id = imported.grant_id;
    SELECT * INTO owned_secret FROM secrets WHERE id = granted.secret_id;
    SELECT * INTO selected_version FROM secret_versions WHERE id = NEW.secret_version_id AND secret_id = owned_secret.id;
    IF session.id IS NULL OR invocation.id IS NULL OR binding.id IS NULL OR rule.id IS NULL
       OR imported.id IS NULL OR granted.id IS NULL OR owned_secret.id IS NULL OR selected_version.id IS NULL
       OR session.invocation_id <> NEW.invocation_id
       OR session.gateway_id <> invocation.gateway_id
       OR session.gateway_revision_id <> invocation.gateway_revision_id
       OR binding.gateway_id <> invocation.gateway_id
       OR binding.gateway_revision_id <> invocation.gateway_revision_id
       OR rule.binding_id <> NEW.binding_id OR rule.gateway_revision_id <> invocation.gateway_revision_id
       OR rule.gateway_route_id <> invocation.gateway_route_id
       OR NEW.secret_version_id <> binding.secret_version_id
       OR binding.status <> 'active' OR imported.status <> 'active' OR granted.status <> 'active'
       OR owned_secret.status <> 'active' OR selected_version.revoked_at IS NOT NULL
       OR selected_version.purged_at IS NOT NULL
       OR session.status NOT IN ('pending_handoff', 'active')
       OR NEW.expires_at > session.expires_at
    THEN
        RAISE EXCEPTION 'gateway secret lease is outside its exact invocation authority'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_gateway_secret_lease_scope() FROM PUBLIC;
CREATE TRIGGER gateway_secret_leases_exact_scope
BEFORE INSERT ON gateway_secret_leases
FOR EACH ROW EXECUTE FUNCTION enforce_gateway_secret_lease_scope();

CREATE FUNCTION enforce_gateway_secret_lease_lifecycle() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.runtime_session_id <> OLD.runtime_session_id OR NEW.invocation_id <> OLD.invocation_id
       OR NEW.binding_id <> OLD.binding_id OR NEW.secret_version_id <> OLD.secret_version_id
       OR NEW.rule_id <> OLD.rule_id OR NEW.issued_at <> OLD.issued_at OR NEW.expires_at <> OLD.expires_at
    THEN
        RAISE EXCEPTION 'gateway secret lease authority is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.status = NEW.status AND OLD.revoked_at IS DISTINCT FROM NEW.revoked_at THEN
        RAISE EXCEPTION 'gateway secret lease lifecycle metadata is immutable without transition'
            USING ERRCODE = 'integrity_constraint_violation';
    ELSIF OLD.status <> NEW.status AND NOT (OLD.status = 'active' AND NEW.status IN ('revoked', 'expired')) THEN
        RAISE EXCEPTION 'invalid gateway secret lease lifecycle transition'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER gateway_secret_leases_lifecycle
BEFORE UPDATE ON gateway_secret_leases
FOR EACH ROW EXECUTE FUNCTION enforce_gateway_secret_lease_lifecycle();
CREATE TRIGGER gateway_secret_leases_no_delete
BEFORE DELETE ON gateway_secret_leases
FOR EACH ROW EXECUTE FUNCTION reject_brokered_secret_rule_mutation();

ALTER TABLE gateway_secret_bindings ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_secret_bindings FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_secret_bindings_read ON gateway_secret_bindings FOR SELECT TO hephaestus_app
USING (check_permission('user', hephaestus_actor_id(), 'can_read', 'gateway', gateway_id::text) = 1);
CREATE POLICY gateway_secret_bindings_worker ON gateway_secret_bindings TO hephaestus_worker USING (true) WITH CHECK (true);
ALTER TABLE gateway_brokered_secret_rules ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_brokered_secret_rules FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_brokered_secret_rules_worker ON gateway_brokered_secret_rules TO hephaestus_worker USING (true) WITH CHECK (true);
ALTER TABLE gateway_secret_leases ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_secret_leases FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_secret_leases_worker ON gateway_secret_leases TO hephaestus_worker USING (true) WITH CHECK (true);
GRANT SELECT ON gateway_secret_bindings TO hephaestus_app, hephaestus_worker;
GRANT INSERT, UPDATE ON gateway_secret_bindings, gateway_brokered_secret_rules, gateway_secret_leases TO hephaestus_worker;
GRANT SELECT ON gateway_brokered_secret_rules, gateway_secret_leases TO hephaestus_worker;
