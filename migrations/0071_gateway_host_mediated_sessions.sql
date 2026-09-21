-- Gateway sessions can either hand a short-lived bearer to a guest handler or
-- admit one host-mediated service request.  The latter has no guest bearer or
-- bootstrap acknowledgement, so its durable shape must say that explicitly.
ALTER TABLE gateway_runtime_authority_sessions
    ADD COLUMN admission_mode text NOT NULL DEFAULT 'guest_handoff',
    ALTER COLUMN credential_hash DROP NOT NULL;

ALTER TABLE gateway_runtime_authority_sessions
    DROP CONSTRAINT IF EXISTS gateway_runtime_authority_sessions_check;

ALTER TABLE gateway_runtime_authority_sessions
    DROP CONSTRAINT IF EXISTS gateway_runtime_authority_sessions_check3;

ALTER TABLE gateway_runtime_authority_sessions
    ADD CONSTRAINT gateway_runtime_authority_sessions_admission_mode_check
        CHECK (admission_mode IN ('guest_handoff', 'host_mediated')),
    ADD CONSTRAINT gateway_runtime_authority_sessions_mode_shape_check
        CHECK (
            (admission_mode = 'guest_handoff' AND credential_hash IS NOT NULL)
            OR (admission_mode = 'host_mediated' AND credential_hash IS NULL)
        ),
    ADD CONSTRAINT gateway_runtime_authority_sessions_lifecycle_shape_check
        CHECK (
            (status = 'pending_handoff'
                AND admission_mode = 'guest_handoff'
                AND acknowledged_at IS NULL
                AND revoked_at IS NULL
                AND revocation_reason IS NULL)
            OR (status = 'active'
                AND revoked_at IS NULL
                AND revocation_reason IS NULL
                AND (
                    (admission_mode = 'guest_handoff' AND acknowledged_at IS NOT NULL)
                    OR (admission_mode = 'host_mediated' AND acknowledged_at IS NULL)
                ))
            OR (status = 'revoked'
                AND revoked_at IS NOT NULL
                AND revocation_reason IS NOT NULL)
            OR (status = 'expired'
                AND revoked_at IS NULL
                AND revocation_reason IS NULL)
        );

CREATE OR REPLACE FUNCTION enforce_gateway_runtime_session_lifecycle() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.id <> NEW.id OR OLD.snapshot_id <> NEW.snapshot_id OR OLD.invocation_id <> NEW.invocation_id
     OR OLD.gateway_id <> NEW.gateway_id OR OLD.gateway_revision_id <> NEW.gateway_revision_id
     OR OLD.identity_hash <> NEW.identity_hash OR OLD.snapshot_hash <> NEW.snapshot_hash
     OR OLD.issuance_generation <> NEW.issuance_generation
     OR OLD.credential_hash IS DISTINCT FROM NEW.credential_hash
     OR OLD.admission_mode <> NEW.admission_mode
     OR OLD.issued_at <> NEW.issued_at OR OLD.expires_at <> NEW.expires_at OR OLD.created_at <> NEW.created_at THEN
    RAISE EXCEPTION 'gateway runtime session identity, mode, and credential verifier are immutable' USING ERRCODE = 'integrity_constraint_violation';
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
