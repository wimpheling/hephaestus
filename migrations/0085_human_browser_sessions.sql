-- Durable human browser sessions. The browser and mediator carry a random
-- UUID SID; PostgreSQL stores only its domain-separated verifier digest.
CREATE TABLE human_browser_sessions (
    id uuid PRIMARY KEY,
    sid_digest bytea NOT NULL UNIQUE CHECK (octet_length(sid_digest) = 32),
    creation_idempotency_id uuid NOT NULL UNIQUE,
    creation_request_id uuid NOT NULL,
    identity_binding_digest bytea NOT NULL
        CHECK (octet_length(identity_binding_digest) = 32),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    issued_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    revocation_reason text CHECK (
        revocation_reason IS NULL
        OR revocation_reason IN ('logout', 'administrative', 'security')
    ),
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (expires_at > issued_at),
    CHECK (expires_at <= issued_at + interval '24 hours'),
    CHECK (revoked_at IS NULL OR revoked_at >= issued_at),
    CHECK ((revoked_at IS NULL) = (revocation_reason IS NULL))
);

CREATE INDEX human_browser_sessions_active_by_user
    ON human_browser_sessions (user_id, expires_at, id)
    WHERE revoked_at IS NULL;
CREATE INDEX human_browser_sessions_expiry
    ON human_browser_sessions (expires_at, id)
    WHERE revoked_at IS NULL;

-- Session identity and expiry are immutable. A session may be closed once;
-- it can never be reopened or have its verifier replaced.
CREATE FUNCTION enforce_human_browser_session_lifecycle() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.id <> NEW.id
       OR OLD.sid_digest <> NEW.sid_digest
       OR OLD.creation_idempotency_id <> NEW.creation_idempotency_id
       OR OLD.creation_request_id <> NEW.creation_request_id
       OR OLD.identity_binding_digest <> NEW.identity_binding_digest
       OR OLD.user_id <> NEW.user_id
       OR OLD.issued_at <> NEW.issued_at
       OR OLD.expires_at <> NEW.expires_at
       OR OLD.created_at <> NEW.created_at
    THEN
        RAISE EXCEPTION 'human browser session identity is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.revoked_at IS NOT NULL
       AND (NEW.revoked_at IS DISTINCT FROM OLD.revoked_at
            OR NEW.revocation_reason IS DISTINCT FROM OLD.revocation_reason)
    THEN
        RAISE EXCEPTION 'human browser session revocation is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_human_browser_session_lifecycle() FROM PUBLIC;
CREATE TRIGGER human_browser_session_lifecycle
BEFORE UPDATE ON human_browser_sessions
FOR EACH ROW EXECUTE FUNCTION enforce_human_browser_session_lifecycle();

ALTER TABLE human_browser_sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE human_browser_sessions FORCE ROW LEVEL SECURITY;
REVOKE ALL ON human_browser_sessions FROM PUBLIC, hephaestus_app, hephaestus_worker;
GRANT SELECT, INSERT, UPDATE ON human_browser_sessions TO hephaestus_worker;
CREATE POLICY human_browser_sessions_worker
    ON human_browser_sessions TO hephaestus_worker
    USING (true) WITH CHECK (true);

-- Authentication exposes only safe metadata. It does not expose the session
-- table or permit application-role enumeration of session verifiers.
CREATE FUNCTION authenticate_human_browser_session(
    p_sid_digest bytea,
    p_user_id uuid
) RETURNS TABLE (
    session_id uuid,
    user_id uuid,
    issued_at timestamptz,
    expires_at timestamptz
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
STABLE
AS $$
    WITH auth_instant AS MATERIALIZED (SELECT statement_timestamp() AS instant)
    SELECT session.id, session.user_id, session.issued_at, session.expires_at
    FROM public.human_browser_sessions AS session
    JOIN public.users ON public.users.id = session.user_id
    CROSS JOIN auth_instant
    WHERE octet_length(p_sid_digest) = 32
      AND session.sid_digest = p_sid_digest
      AND session.user_id = p_user_id
      AND session.revoked_at IS NULL
      AND session.issued_at <= auth_instant.instant
      AND session.expires_at > auth_instant.instant
      AND public.users.status = 'active'
$$;
REVOKE ALL ON FUNCTION authenticate_human_browser_session(bytea, uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION authenticate_human_browser_session(bytea, uuid)
    TO hephaestus_app, hephaestus_worker;

-- No ad hoc session event or direct legacy outbox row is created here. The
-- adapter must use an approved existing append_application_event contract for
-- lifecycle auditing; this migration deliberately exposes no SID material.
