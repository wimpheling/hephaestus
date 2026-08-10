-- Immutable dispatch-time capability snapshots and hash-only generic runtime
-- sessions. Plaintext bearer material exists only in the trusted host handoff
-- adapter and never enters PostgreSQL.

ALTER TABLE runs
    ADD CONSTRAINT runs_exact_revision_unique
    UNIQUE (id, instance_id, instance_revision_id);

CREATE TABLE run_authorization_snapshots (
    id uuid PRIMARY KEY,
    run_id uuid NOT NULL,
    instance_id uuid NOT NULL,
    instance_revision_id uuid NOT NULL,
    authorization_model_version text NOT NULL
        CHECK (length(authorization_model_version) BETWEEN 1 AND 128),
    normalized_hash bytea NOT NULL CHECK (octet_length(normalized_hash) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (run_id, instance_id, instance_revision_id)
        REFERENCES runs(id, instance_id, instance_revision_id),
    UNIQUE (run_id),
    UNIQUE (id, instance_revision_id)
);
CREATE INDEX run_authorization_snapshots_by_revision
    ON run_authorization_snapshots (instance_revision_id, created_at DESC, id);

CREATE TABLE run_authorization_snapshot_bindings (
    snapshot_id uuid NOT NULL,
    instance_revision_id uuid NOT NULL,
    ordinal integer NOT NULL CHECK (ordinal >= 0),
    binding_id uuid NOT NULL,
    binding_hash bytea NOT NULL CHECK (octet_length(binding_hash) = 32),
    slot_key text NOT NULL CHECK (slot_key ~ '^[a-z][a-z0-9_-]{0,63}$'),
    resource_kind text NOT NULL CHECK (
        resource_kind IN (
            'repository', 'project', 'agent_instance', 'gateway', 'run',
            'state_volume'
        )
    ),
    resource_id uuid NOT NULL,
    granted_operations text[] NOT NULL CHECK (
        cardinality(granted_operations) BETWEEN 1 AND 32
    ),
    PRIMARY KEY (snapshot_id, ordinal),
    UNIQUE (snapshot_id, binding_id),
    UNIQUE (snapshot_id, slot_key),
    FOREIGN KEY (snapshot_id, instance_revision_id)
        REFERENCES run_authorization_snapshots(id, instance_revision_id),
    FOREIGN KEY (binding_id, instance_revision_id)
        REFERENCES agent_capability_bindings(id, instance_revision_id),
    CHECK (capability_operations_are_unique(granted_operations)),
    CHECK (capability_operations_are_legal(resource_kind, granted_operations))
);
CREATE INDEX run_authorization_snapshot_bindings_by_resource
    ON run_authorization_snapshot_bindings
       (resource_kind, resource_id, snapshot_id, ordinal);

CREATE FUNCTION enforce_snapshot_binding_copy() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    stored_binding agent_capability_bindings%ROWTYPE;
BEGIN
    SELECT * INTO stored_binding
    FROM agent_capability_bindings
    WHERE id = NEW.binding_id
      AND instance_revision_id = NEW.instance_revision_id;

    IF stored_binding.id IS NULL
       OR NEW.slot_key <> stored_binding.slot_key
       OR NEW.resource_kind <> stored_binding.resource_kind
       OR NEW.resource_id <> stored_binding.resource_id
       OR NEW.granted_operations <> stored_binding.granted_operations
       OR NEW.binding_hash <> stored_binding.normalized_hash
    THEN
        RAISE EXCEPTION 'authorization snapshot binding does not match immutable binding'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_snapshot_binding_copy() FROM PUBLIC;
CREATE TRIGGER run_authorization_snapshot_bindings_exact_copy
BEFORE INSERT ON run_authorization_snapshot_bindings
FOR EACH ROW EXECUTE FUNCTION enforce_snapshot_binding_copy();

CREATE TABLE runtime_authority_sessions (
    id uuid PRIMARY KEY,
    snapshot_id uuid NOT NULL,
    run_id uuid NOT NULL,
    instance_id uuid NOT NULL,
    instance_revision_id uuid NOT NULL,
    attachment_id uuid,
    identity_hash bytea NOT NULL CHECK (octet_length(identity_hash) = 32),
    snapshot_hash bytea NOT NULL CHECK (octet_length(snapshot_hash) = 32),
    issuance_generation bigint NOT NULL CHECK (issuance_generation > 0),
    credential_hash bytea NOT NULL UNIQUE CHECK (octet_length(credential_hash) = 32),
    status text NOT NULL CHECK (
        status IN ('pending_handoff', 'active', 'revoked', 'expired')
    ),
    issued_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    acknowledged_at timestamptz,
    revoked_at timestamptz,
    revocation_reason text CHECK (
        revocation_reason IS NULL
        OR length(revocation_reason) BETWEEN 1 AND 256
    ),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (snapshot_id, instance_revision_id)
        REFERENCES run_authorization_snapshots(id, instance_revision_id),
    FOREIGN KEY (run_id, instance_id, instance_revision_id)
        REFERENCES runs(id, instance_id, instance_revision_id),
    FOREIGN KEY (attachment_id, instance_id)
        REFERENCES agent_attachments(id, instance_id),
    UNIQUE (run_id),
    UNIQUE (snapshot_id),
    CHECK (expires_at > issued_at),
    CHECK (
        acknowledged_at IS NULL
        OR (acknowledged_at >= issued_at AND acknowledged_at < expires_at)
    ),
    CHECK (revoked_at IS NULL OR revoked_at >= issued_at),
    CHECK (
        (status = 'pending_handoff'
            AND acknowledged_at IS NULL
            AND revoked_at IS NULL
            AND revocation_reason IS NULL)
        OR (status = 'active'
            AND acknowledged_at IS NOT NULL
            AND revoked_at IS NULL
            AND revocation_reason IS NULL)
        OR (status = 'revoked'
            AND revoked_at IS NOT NULL
            AND revocation_reason IS NOT NULL)
        OR (status = 'expired'
            AND revoked_at IS NULL
            AND revocation_reason IS NULL)
    )
);
CREATE INDEX runtime_authority_sessions_live
    ON runtime_authority_sessions (expires_at, id)
    WHERE status IN ('pending_handoff', 'active');

CREATE FUNCTION enforce_runtime_session_snapshot() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    stored_snapshot run_authorization_snapshots%ROWTYPE;
    stored_run runs%ROWTYPE;
BEGIN
    SELECT * INTO stored_snapshot
    FROM run_authorization_snapshots
    WHERE id = NEW.snapshot_id;
    SELECT * INTO stored_run FROM runs WHERE id = NEW.run_id;

    IF stored_snapshot.id IS NULL
       OR stored_run.id IS NULL
       OR NEW.run_id <> stored_snapshot.run_id
       OR NEW.instance_id <> stored_snapshot.instance_id
       OR NEW.instance_revision_id <> stored_snapshot.instance_revision_id
       OR NEW.attachment_id IS DISTINCT FROM stored_run.attachment_id
       OR NEW.snapshot_hash <> stored_snapshot.normalized_hash
    THEN
        RAISE EXCEPTION 'runtime session does not match exact authorization snapshot'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_runtime_session_snapshot() FROM PUBLIC;
CREATE TRIGGER runtime_authority_sessions_exact_snapshot
BEFORE INSERT ON runtime_authority_sessions
FOR EACH ROW EXECUTE FUNCTION enforce_runtime_session_snapshot();

CREATE FUNCTION enforce_runtime_session_lifecycle() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.id <> NEW.id
       OR OLD.snapshot_id <> NEW.snapshot_id
       OR OLD.run_id <> NEW.run_id
       OR OLD.instance_id <> NEW.instance_id
       OR OLD.instance_revision_id <> NEW.instance_revision_id
       OR OLD.attachment_id IS DISTINCT FROM NEW.attachment_id
       OR OLD.identity_hash <> NEW.identity_hash
       OR OLD.snapshot_hash <> NEW.snapshot_hash
       OR OLD.issuance_generation <> NEW.issuance_generation
       OR OLD.credential_hash <> NEW.credential_hash
       OR OLD.issued_at <> NEW.issued_at
       OR OLD.expires_at <> NEW.expires_at
       OR OLD.created_at <> NEW.created_at
    THEN
        RAISE EXCEPTION 'runtime session identity and credential verifier are immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;

    IF OLD.status = NEW.status THEN
        IF OLD.acknowledged_at IS DISTINCT FROM NEW.acknowledged_at
           OR OLD.revoked_at IS DISTINCT FROM NEW.revoked_at
           OR OLD.revocation_reason IS DISTINCT FROM NEW.revocation_reason
        THEN
            RAISE EXCEPTION 'runtime session lifecycle metadata is immutable without transition'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    ELSIF NOT (
        (OLD.status = 'pending_handoff'
            AND NEW.status IN ('active', 'revoked', 'expired'))
        OR (OLD.status = 'active' AND NEW.status IN ('revoked', 'expired'))
    ) THEN
        RAISE EXCEPTION 'invalid runtime session lifecycle transition'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER runtime_authority_sessions_lifecycle
BEFORE UPDATE ON runtime_authority_sessions
FOR EACH ROW EXECUTE FUNCTION enforce_runtime_session_lifecycle();

CREATE FUNCTION reject_runtime_authority_immutable_record() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'runtime authority snapshot records are immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
CREATE TRIGGER run_authorization_snapshots_immutable
BEFORE UPDATE OR DELETE ON run_authorization_snapshots
FOR EACH ROW EXECUTE FUNCTION reject_runtime_authority_immutable_record();
CREATE TRIGGER run_authorization_snapshot_bindings_immutable
BEFORE UPDATE OR DELETE ON run_authorization_snapshot_bindings
FOR EACH ROW EXECUTE FUNCTION reject_runtime_authority_immutable_record();
CREATE TRIGGER runtime_authority_sessions_no_delete
BEFORE DELETE ON runtime_authority_sessions
FOR EACH ROW EXECUTE FUNCTION reject_runtime_authority_immutable_record();

GRANT SELECT ON run_authorization_snapshots,
    run_authorization_snapshot_bindings TO hephaestus_app, hephaestus_worker;
GRANT INSERT ON run_authorization_snapshots,
    run_authorization_snapshot_bindings TO hephaestus_worker;
GRANT SELECT, INSERT, UPDATE ON runtime_authority_sessions TO hephaestus_worker;

ALTER TABLE run_authorization_snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_authorization_snapshots FORCE ROW LEVEL SECURITY;
CREATE POLICY run_authorization_snapshots_user_select
    ON run_authorization_snapshots FOR SELECT TO hephaestus_app
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read', 'run', run_id::text
    ) = 1);
CREATE POLICY run_authorization_snapshots_worker
    ON run_authorization_snapshots TO hephaestus_worker
    USING (true) WITH CHECK (true);

ALTER TABLE run_authorization_snapshot_bindings ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_authorization_snapshot_bindings FORCE ROW LEVEL SECURITY;
CREATE POLICY run_authorization_snapshot_bindings_user_select
    ON run_authorization_snapshot_bindings FOR SELECT TO hephaestus_app
    USING (EXISTS (
        SELECT 1 FROM run_authorization_snapshots AS snapshot
        WHERE snapshot.id = snapshot_id
          AND check_permission(
              'user', hephaestus_actor_id(), 'can_read',
              'run', snapshot.run_id::text
          ) = 1
    ));
CREATE POLICY run_authorization_snapshot_bindings_worker
    ON run_authorization_snapshot_bindings TO hephaestus_worker
    USING (true) WITH CHECK (true);

ALTER TABLE runtime_authority_sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE runtime_authority_sessions FORCE ROW LEVEL SECURITY;
CREATE POLICY runtime_authority_sessions_worker
    ON runtime_authority_sessions TO hephaestus_worker
    USING (true) WITH CHECK (true);

-- Credential verifiers are authentication material and cannot be listed by
-- the application role. This function reveals only safe exact-session claims.
CREATE FUNCTION authenticate_runtime_authority(p_credential_hash bytea)
RETURNS TABLE (
    session_id uuid,
    snapshot_id uuid,
    run_id uuid,
    instance_id uuid,
    instance_revision_id uuid,
    attachment_id uuid,
    issuance_generation bigint,
    expires_at timestamptz
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, public
STABLE
AS $$
    SELECT session.id, session.snapshot_id, session.run_id,
           session.instance_id, session.instance_revision_id,
           session.attachment_id, session.issuance_generation,
           session.expires_at
    FROM runtime_authority_sessions AS session
    WHERE session.credential_hash = p_credential_hash
      AND session.status = 'active'
      AND session.expires_at > now()
$$;
REVOKE ALL ON FUNCTION authenticate_runtime_authority(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION authenticate_runtime_authority(bytea) TO hephaestus_app;
