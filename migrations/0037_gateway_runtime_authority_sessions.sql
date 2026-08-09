-- Gateway invocations are independent workloads. They intentionally use
-- separate snapshot/session tables rather than a synthetic agent run.
CREATE TABLE gateway_authorization_snapshots (
    id uuid PRIMARY KEY,
    invocation_id uuid NOT NULL UNIQUE REFERENCES gateway_invocations(id),
    gateway_id uuid NOT NULL,
    gateway_revision_id uuid NOT NULL,
    authorization_model_version text NOT NULL CHECK (length(authorization_model_version) BETWEEN 1 AND 128),
    normalized_hash bytea NOT NULL CHECK (octet_length(normalized_hash) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (gateway_revision_id, gateway_id) REFERENCES gateway_revisions(id, gateway_id),
    UNIQUE (id, gateway_revision_id)
);

CREATE TABLE gateway_runtime_authority_sessions (
    id uuid PRIMARY KEY,
    snapshot_id uuid NOT NULL,
    invocation_id uuid NOT NULL UNIQUE REFERENCES gateway_invocations(id),
    gateway_id uuid NOT NULL,
    gateway_revision_id uuid NOT NULL,
    identity_hash bytea NOT NULL CHECK (octet_length(identity_hash) = 32),
    snapshot_hash bytea NOT NULL CHECK (octet_length(snapshot_hash) = 32),
    issuance_generation bigint NOT NULL CHECK (issuance_generation > 0),
    credential_hash bytea NOT NULL UNIQUE CHECK (octet_length(credential_hash) = 32),
    status text NOT NULL CHECK (status IN ('pending_handoff', 'active', 'revoked', 'expired')),
    issued_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    acknowledged_at timestamptz,
    revoked_at timestamptz,
    revocation_reason text CHECK (revocation_reason IS NULL OR length(revocation_reason) BETWEEN 1 AND 256),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (snapshot_id, gateway_revision_id) REFERENCES gateway_authorization_snapshots(id, gateway_revision_id),
    FOREIGN KEY (gateway_revision_id, gateway_id) REFERENCES gateway_revisions(id, gateway_id),
    CHECK (expires_at > issued_at),
    CHECK (acknowledged_at IS NULL OR (acknowledged_at >= issued_at AND acknowledged_at < expires_at)),
    CHECK (revoked_at IS NULL OR revoked_at >= issued_at),
    CHECK ((status = 'pending_handoff' AND acknowledged_at IS NULL AND revoked_at IS NULL AND revocation_reason IS NULL)
        OR (status = 'active' AND acknowledged_at IS NOT NULL AND revoked_at IS NULL AND revocation_reason IS NULL)
        OR (status = 'revoked' AND revoked_at IS NOT NULL AND revocation_reason IS NOT NULL)
        OR (status = 'expired' AND revoked_at IS NULL AND revocation_reason IS NULL))
);
CREATE INDEX gateway_runtime_authority_sessions_live ON gateway_runtime_authority_sessions (expires_at, id)
    WHERE status IN ('pending_handoff', 'active');

CREATE FUNCTION enforce_gateway_runtime_session_snapshot() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE snapshot gateway_authorization_snapshots%ROWTYPE;
BEGIN
  SELECT * INTO snapshot FROM gateway_authorization_snapshots WHERE id = NEW.snapshot_id;
  IF snapshot.id IS NULL OR NEW.invocation_id <> snapshot.invocation_id
     OR NEW.gateway_id <> snapshot.gateway_id OR NEW.gateway_revision_id <> snapshot.gateway_revision_id
     OR NEW.snapshot_hash <> snapshot.normalized_hash THEN
    RAISE EXCEPTION 'gateway runtime session does not match exact authorization snapshot' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_gateway_runtime_session_snapshot() FROM PUBLIC;
CREATE TRIGGER gateway_runtime_authority_sessions_exact_snapshot BEFORE INSERT ON gateway_runtime_authority_sessions
  FOR EACH ROW EXECUTE FUNCTION enforce_gateway_runtime_session_snapshot();

-- Reuse the same immutable identity and monotonic lifecycle rules as run
-- sessions, without sharing their run-specific foreign keys.
CREATE FUNCTION enforce_gateway_runtime_session_lifecycle() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.id <> NEW.id OR OLD.snapshot_id <> NEW.snapshot_id OR OLD.invocation_id <> NEW.invocation_id
     OR OLD.gateway_id <> NEW.gateway_id OR OLD.gateway_revision_id <> NEW.gateway_revision_id
     OR OLD.identity_hash <> NEW.identity_hash OR OLD.snapshot_hash <> NEW.snapshot_hash
     OR OLD.issuance_generation <> NEW.issuance_generation OR OLD.credential_hash <> NEW.credential_hash
     OR OLD.issued_at <> NEW.issued_at OR OLD.expires_at <> NEW.expires_at OR OLD.created_at <> NEW.created_at THEN
    RAISE EXCEPTION 'gateway runtime session identity and credential verifier are immutable' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  IF OLD.status = NEW.status THEN
    IF OLD.acknowledged_at IS DISTINCT FROM NEW.acknowledged_at OR OLD.revoked_at IS DISTINCT FROM NEW.revoked_at
       OR OLD.revocation_reason IS DISTINCT FROM NEW.revocation_reason THEN
      RAISE EXCEPTION 'gateway runtime session lifecycle metadata is immutable without transition' USING ERRCODE = 'integrity_constraint_violation';
    END IF;
  ELSIF NOT ((OLD.status = 'pending_handoff' AND NEW.status IN ('active', 'revoked', 'expired'))
       OR (OLD.status = 'active' AND NEW.status IN ('revoked', 'expired'))) THEN
    RAISE EXCEPTION 'invalid gateway runtime session lifecycle transition' USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER gateway_runtime_authority_sessions_lifecycle BEFORE UPDATE ON gateway_runtime_authority_sessions
  FOR EACH ROW EXECUTE FUNCTION enforce_gateway_runtime_session_lifecycle();
CREATE TRIGGER gateway_authorization_snapshots_immutable BEFORE UPDATE OR DELETE ON gateway_authorization_snapshots
  FOR EACH ROW EXECUTE FUNCTION reject_runtime_authority_immutable_record();
CREATE TRIGGER gateway_runtime_authority_sessions_no_delete BEFORE DELETE ON gateway_runtime_authority_sessions
  FOR EACH ROW EXECUTE FUNCTION reject_runtime_authority_immutable_record();

ALTER TABLE gateway_authorization_snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_authorization_snapshots FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_authorization_snapshots_worker ON gateway_authorization_snapshots TO hephaestus_worker USING (true) WITH CHECK (true);
ALTER TABLE gateway_runtime_authority_sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_runtime_authority_sessions FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_runtime_authority_sessions_worker ON gateway_runtime_authority_sessions TO hephaestus_worker USING (true) WITH CHECK (true);
GRANT SELECT, INSERT ON gateway_authorization_snapshots TO hephaestus_worker;
GRANT SELECT, INSERT, UPDATE ON gateway_runtime_authority_sessions TO hephaestus_worker;
