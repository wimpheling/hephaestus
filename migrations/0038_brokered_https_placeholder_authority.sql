-- Immutable, provider-neutral metadata for HTTPS placeholder substitution.
-- No table in this migration can store a secret value or a VM-visible bearer.

CREATE TABLE brokered_secret_rules (
    id uuid PRIMARY KEY,
    binding_id uuid NOT NULL,
    instance_revision_id uuid NOT NULL,
    secret_version_id uuid NOT NULL REFERENCES secret_versions(id),
    direction text NOT NULL CHECK (direction = 'outbound'),
    destination_origin text CHECK (
        destination_origin IS NULL OR destination_origin ~ '^https://[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)*(?::(?:[1-9][0-9]{0,3}|[1-5][0-9]{4}|6[0-4][0-9]{3}|65[0-4][0-9]{2}|655[0-2][0-9]|6553[0-5]))?$'
    ),
    location_kind text NOT NULL CHECK (location_kind IN ('outbound_header_value', 'outbound_header_prefix', 'inbound_gateway_header')),
    header_name text NOT NULL CHECK (header_name ~ '^[!#$%&''*+.^_`|~0-9a-z-]{1,64}$'),
    header_prefix text,
    normalized_hash bytea NOT NULL CHECK (octet_length(normalized_hash) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (binding_id, instance_revision_id)
        REFERENCES agent_secret_bindings(id, instance_revision_id),
    UNIQUE (binding_id),
    UNIQUE (id, binding_id),
    CHECK (destination_origin IS NOT NULL
        AND location_kind IN ('outbound_header_value', 'outbound_header_prefix')),
    CHECK (
        (location_kind = 'outbound_header_prefix'
            AND header_prefix IS NOT NULL
            AND octet_length(header_prefix) <= 256
            AND header_prefix !~ '[\r\n]')
        OR (location_kind <> 'outbound_header_prefix' AND header_prefix IS NULL)
    )
);
CREATE FUNCTION reject_brokered_secret_rule_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'brokered secret rules are immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
CREATE TRIGGER brokered_secret_rules_immutable
BEFORE UPDATE OR DELETE ON brokered_secret_rules
FOR EACH ROW EXECUTE FUNCTION reject_brokered_secret_rule_mutation();

-- A secret lease fixes the selected version and credential for an exact run.
-- This extra immutable snapshot fixes the placeholder rule and exact origin as
-- well, so a later route/binding change cannot broaden an in-flight use.
CREATE TABLE brokered_secret_lease_snapshots (
    id uuid PRIMARY KEY,
    lease_id uuid NOT NULL UNIQUE REFERENCES secret_leases(id),
    runtime_session_id uuid NOT NULL,
    run_id uuid NOT NULL,
    binding_id uuid NOT NULL,
    secret_version_id uuid NOT NULL,
    rule_id uuid NOT NULL,
    destination_origin text NOT NULL,
    location_kind text NOT NULL CHECK (location_kind IN ('outbound_header_value', 'outbound_header_prefix')),
    header_name text NOT NULL CHECK (header_name ~ '^[!#$%&''*+.^_`|~0-9a-z-]{1,64}$'),
    header_prefix text,
    rule_hash bytea NOT NULL CHECK (octet_length(rule_hash) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (runtime_session_id, run_id)
        REFERENCES secret_runtime_sessions(id, run_id),
    FOREIGN KEY (rule_id, binding_id)
        REFERENCES brokered_secret_rules(id, binding_id),
    CHECK (destination_origin ~ '^https://'),
    CHECK (
        (location_kind = 'outbound_header_prefix' AND header_prefix IS NOT NULL)
        OR (location_kind = 'outbound_header_value' AND header_prefix IS NULL)
    )
);
CREATE INDEX brokered_secret_lease_snapshots_by_session
    ON brokered_secret_lease_snapshots (runtime_session_id, created_at DESC);

CREATE FUNCTION enforce_brokered_secret_lease_snapshot() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    lease secret_leases%ROWTYPE;
    rule brokered_secret_rules%ROWTYPE;
BEGIN
    SELECT * INTO lease FROM secret_leases WHERE id = NEW.lease_id;
    SELECT * INTO rule FROM brokered_secret_rules WHERE id = NEW.rule_id;
    IF lease.id IS NULL OR rule.id IS NULL
       OR lease.delivery_mode <> 'brokered'
       OR lease.status <> 'active'
       OR rule.direction <> 'outbound'
       OR NEW.runtime_session_id <> lease.session_id
       OR NEW.run_id <> lease.run_id
       OR NEW.binding_id <> lease.binding_id
       OR NEW.secret_version_id <> lease.secret_version_id
       OR NEW.binding_id <> rule.binding_id
       OR NEW.secret_version_id <> rule.secret_version_id
       OR NEW.destination_origin <> rule.destination_origin
       OR NEW.location_kind <> rule.location_kind
       OR NEW.header_name <> rule.header_name
       OR NEW.header_prefix IS DISTINCT FROM rule.header_prefix
       OR NEW.rule_hash <> rule.normalized_hash
    THEN
        RAISE EXCEPTION 'brokered secret lease snapshot is outside its exact immutable authority'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_brokered_secret_lease_snapshot() FROM PUBLIC;
CREATE TRIGGER brokered_secret_lease_snapshots_exact_authority
BEFORE INSERT ON brokered_secret_lease_snapshots
FOR EACH ROW EXECUTE FUNCTION enforce_brokered_secret_lease_snapshot();
CREATE TRIGGER brokered_secret_lease_snapshots_immutable
BEFORE UPDATE OR DELETE ON brokered_secret_lease_snapshots
FOR EACH ROW EXECUTE FUNCTION reject_brokered_secret_rule_mutation();

-- Value-free evidence only.  The event identifies the immutable placeholder
-- rule but deliberately excludes hostnames beyond the rule snapshot, request
-- payloads, headers, response data, credentials, and secret material.
CREATE TABLE brokered_secret_audit_events (
    id uuid PRIMARY KEY,
    lease_snapshot_id uuid NOT NULL REFERENCES brokered_secret_lease_snapshots(id),
    rule_id uuid NOT NULL,
    runtime_session_id uuid NOT NULL,
    run_id uuid NOT NULL,
    request_id uuid NOT NULL,
    event_kind text NOT NULL CHECK (event_kind IN ('authorization_decision', 'substitution_use')),
    decision text CHECK (decision IN ('allow', 'deny')),
    outcome text CHECK (outcome IN ('succeeded', 'failed', 'cancelled')),
    reason_code text CHECK (reason_code IS NULL OR reason_code ~ '^[a-z][a-z0-9_]{0,63}$'),
    occurred_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (runtime_session_id, run_id)
        REFERENCES secret_runtime_sessions(id, run_id),
    CHECK (
        (event_kind = 'authorization_decision' AND decision IS NOT NULL AND outcome IS NULL)
        OR (event_kind = 'substitution_use' AND decision IS NULL AND outcome IS NOT NULL)
    )
);
CREATE FUNCTION enforce_brokered_secret_audit_ceiling() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    snapshot brokered_secret_lease_snapshots%ROWTYPE;
BEGIN
    SELECT * INTO snapshot FROM brokered_secret_lease_snapshots WHERE id = NEW.lease_snapshot_id;
    IF snapshot.id IS NULL OR NEW.rule_id <> snapshot.rule_id
       OR NEW.runtime_session_id <> snapshot.runtime_session_id
       OR NEW.run_id <> snapshot.run_id THEN
        RAISE EXCEPTION 'brokered secret audit event is outside its immutable lease snapshot'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_brokered_secret_audit_ceiling() FROM PUBLIC;
CREATE TRIGGER brokered_secret_audit_events_exact_ceiling
BEFORE INSERT ON brokered_secret_audit_events
FOR EACH ROW EXECUTE FUNCTION enforce_brokered_secret_audit_ceiling();
CREATE TRIGGER brokered_secret_audit_events_immutable
BEFORE UPDATE OR DELETE ON brokered_secret_audit_events
FOR EACH ROW EXECUTE FUNCTION reject_brokered_secret_rule_mutation();

ALTER TABLE brokered_secret_rules ENABLE ROW LEVEL SECURITY;
ALTER TABLE brokered_secret_rules FORCE ROW LEVEL SECURITY;
CREATE POLICY brokered_secret_rules_read ON brokered_secret_rules FOR SELECT TO hephaestus_app
USING (check_permission('user', hephaestus_actor_id(), 'can_read', 'agent_instance', (
    SELECT revision.instance_id::text FROM agent_instance_revisions AS revision
    WHERE revision.id = brokered_secret_rules.instance_revision_id
)) = 1);
CREATE POLICY brokered_secret_rules_worker ON brokered_secret_rules TO hephaestus_worker USING (true) WITH CHECK (true);

ALTER TABLE brokered_secret_lease_snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE brokered_secret_lease_snapshots FORCE ROW LEVEL SECURITY;
CREATE POLICY brokered_secret_lease_snapshots_worker ON brokered_secret_lease_snapshots TO hephaestus_worker USING (true) WITH CHECK (true);
ALTER TABLE brokered_secret_audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE brokered_secret_audit_events FORCE ROW LEVEL SECURITY;
CREATE POLICY brokered_secret_audit_events_worker ON brokered_secret_audit_events TO hephaestus_worker USING (true) WITH CHECK (true);
GRANT SELECT ON brokered_secret_rules TO hephaestus_app, hephaestus_worker;
GRANT INSERT ON brokered_secret_rules, brokered_secret_lease_snapshots, brokered_secret_audit_events TO hephaestus_worker;
GRANT SELECT ON brokered_secret_lease_snapshots, brokered_secret_audit_events TO hephaestus_worker;
