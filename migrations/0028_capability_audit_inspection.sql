-- Immutable, redacted audit evidence for privileged runtime capability calls.
-- The event stores only opaque identifiers, closed vocabularies, and bounded
-- machine reason codes. Request payloads, paths, credentials, secret values,
-- and provider responses have no durable column in this boundary.

ALTER TABLE runtime_authority_sessions
    ADD CONSTRAINT runtime_authority_sessions_id_snapshot_unique
    UNIQUE (id, snapshot_id);

CREATE TABLE capability_audit_events (
    id uuid PRIMARY KEY,
    runtime_session_id uuid NOT NULL,
    snapshot_id uuid NOT NULL,
    binding_id uuid NOT NULL,
    run_id uuid NOT NULL,
    instance_id uuid NOT NULL,
    instance_revision_id uuid NOT NULL,
    slot_key text NOT NULL CHECK (slot_key ~ '^[a-z][a-z0-9_-]{0,63}$'),
    resource_kind text NOT NULL CHECK (
        resource_kind IN (
            'repository', 'project', 'agent_instance', 'gateway', 'run',
            'state_volume'
        )
    ),
    resource_id uuid NOT NULL,
    grantor_id uuid NOT NULL REFERENCES users(id),
    request_id uuid NOT NULL,
    event_kind text NOT NULL CHECK (
        event_kind IN ('authorization_decision', 'capability_use')
    ),
    operation text NOT NULL,
    decision text CHECK (decision IN ('allow', 'deny')),
    outcome text CHECK (outcome IN ('succeeded', 'failed')),
    reason_code text CHECK (
        reason_code IS NULL
        OR reason_code ~ '^[a-z][a-z0-9_]{0,63}$'
    ),
    authorization_model_version text NOT NULL
        CHECK (length(authorization_model_version) BETWEEN 1 AND 128),
    occurred_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (runtime_session_id, snapshot_id)
        REFERENCES runtime_authority_sessions(id, snapshot_id),
    FOREIGN KEY (snapshot_id, binding_id)
        REFERENCES run_authorization_snapshot_bindings(snapshot_id, binding_id),
    CHECK (
        (event_kind = 'authorization_decision'
            AND decision IS NOT NULL AND outcome IS NULL)
        OR (event_kind = 'capability_use'
            AND decision IS NULL AND outcome IS NOT NULL)
    )
);
CREATE INDEX capability_audit_events_by_session
    ON capability_audit_events
       (runtime_session_id, occurred_at DESC, id DESC);
CREATE INDEX capability_audit_events_by_request
    ON capability_audit_events
       (request_id, occurred_at, id);

CREATE FUNCTION enforce_capability_audit_ceiling() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    stored_operations text[];
    stored_model_version text;
    stored_run_id uuid;
    stored_instance_id uuid;
    stored_instance_revision_id uuid;
    stored_slot_key text;
    stored_resource_kind text;
    stored_resource_id uuid;
    stored_grantor_id uuid;
BEGIN
    SELECT binding.granted_operations, snapshot.authorization_model_version,
           session.run_id, session.instance_id, session.instance_revision_id,
           binding.slot_key, binding.resource_kind, binding.resource_id,
           source_binding.created_by
    INTO stored_operations, stored_model_version, stored_run_id,
         stored_instance_id, stored_instance_revision_id, stored_slot_key,
         stored_resource_kind, stored_resource_id, stored_grantor_id
    FROM run_authorization_snapshot_bindings AS binding
    JOIN run_authorization_snapshots AS snapshot
      ON snapshot.id = binding.snapshot_id
    JOIN runtime_authority_sessions AS session
      ON session.snapshot_id = snapshot.id
     AND session.id = NEW.runtime_session_id
    JOIN agent_capability_bindings AS source_binding
      ON source_binding.id = binding.binding_id
     AND source_binding.instance_revision_id = binding.instance_revision_id
    WHERE binding.snapshot_id = NEW.snapshot_id
      AND binding.binding_id = NEW.binding_id;

    IF stored_operations IS NULL
       OR NOT NEW.operation = ANY(stored_operations)
       OR NEW.authorization_model_version <> stored_model_version
    THEN
        RAISE EXCEPTION 'capability audit event is outside the immutable snapshot ceiling'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    NEW.run_id := stored_run_id;
    NEW.instance_id := stored_instance_id;
    NEW.instance_revision_id := stored_instance_revision_id;
    NEW.slot_key := stored_slot_key;
    NEW.resource_kind := stored_resource_kind;
    NEW.resource_id := stored_resource_id;
    NEW.grantor_id := stored_grantor_id;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_capability_audit_ceiling() FROM PUBLIC;
CREATE TRIGGER capability_audit_events_exact_ceiling
BEFORE INSERT ON capability_audit_events
FOR EACH ROW EXECUTE FUNCTION enforce_capability_audit_ceiling();

CREATE FUNCTION reject_capability_audit_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'capability audit events are immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
CREATE TRIGGER capability_audit_events_immutable
BEFORE UPDATE OR DELETE ON capability_audit_events
FOR EACH ROW EXECUTE FUNCTION reject_capability_audit_mutation();

ALTER TABLE capability_audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE capability_audit_events FORCE ROW LEVEL SECURITY;
CREATE POLICY capability_audit_events_user_select
    ON capability_audit_events FOR SELECT TO hephaestus_app
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read', 'run', run_id::text
    ) = 1);
CREATE POLICY capability_audit_events_worker
    ON capability_audit_events TO hephaestus_worker
    USING (true) WITH CHECK (true);

-- The application inspection surface excludes verifier hashes and all guest
-- bootstrap metadata by construction. Security-invoker preserves event RLS.
CREATE VIEW capability_audit_inspection
WITH (security_invoker = true) AS
SELECT
    event.id,
    event.run_id,
    event.instance_id,
    event.instance_revision_id,
    event.runtime_session_id,
    event.snapshot_id,
    event.binding_id,
    event.slot_key,
    event.resource_kind,
    event.resource_id,
    event.grantor_id,
    event.operation,
    event.request_id,
    event.event_kind,
    event.decision,
    event.outcome,
    event.reason_code,
    event.authorization_model_version,
    event.occurred_at
FROM capability_audit_events AS event;

REVOKE ALL ON capability_audit_events, capability_audit_inspection FROM PUBLIC;
GRANT SELECT ON capability_audit_events, capability_audit_inspection
    TO hephaestus_app, hephaestus_worker;
GRANT INSERT ON capability_audit_events TO hephaestus_worker;

-- Runtime credential verifiers and bootstrap state remain worker-only. This
-- function exposes only bounded lifecycle claims after reauthorizing the
-- current user against the parent instance.
CREATE FUNCTION inspect_runtime_authority_sessions(
    p_instance_id uuid,
    p_limit integer DEFAULT 100
) RETURNS TABLE (
    id uuid,
    snapshot_id uuid,
    run_id uuid,
    instance_revision_id uuid,
    status text,
    issued_at timestamptz,
    expires_at timestamptz,
    acknowledged_at timestamptz,
    revoked_at timestamptz,
    revocation_reason text
)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET row_security = off
STABLE
AS $$
BEGIN
    IF p_limit < 1 OR p_limit > 200
       OR check_permission(
            'user', hephaestus_actor_id(), 'can_read',
            'agent_instance', p_instance_id::text
          ) <> 1
    THEN
        RETURN;
    END IF;
    RETURN QUERY
    SELECT session.id, session.snapshot_id, session.run_id,
           session.instance_revision_id, session.status,
           session.issued_at, session.expires_at, session.acknowledged_at,
           session.revoked_at, session.revocation_reason
    FROM runtime_authority_sessions AS session
    WHERE session.instance_id = p_instance_id
    ORDER BY session.issued_at DESC, session.id DESC
    LIMIT p_limit;
END
$$;
REVOKE ALL ON FUNCTION inspect_runtime_authority_sessions(uuid, integer)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION inspect_runtime_authority_sessions(uuid, integer)
    TO hephaestus_app;
