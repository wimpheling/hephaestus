-- Immutable, worker-owned self-revocation idempotency. A row is retained for
-- every exact actor-derived command, including a no-op for an absent, expired,
-- or already-revoked session. PostgreSQL stores only the SID digest.
-- The explicit RESTRICT foreign keys retain these audit parents while ledger
-- rows are retained; deletion is an intentional lifecycle operation, not a
-- path that silently erases revocation evidence.
ALTER TABLE human_browser_sessions
    ADD CONSTRAINT human_browser_sessions_id_user_sid_key
        UNIQUE (id, user_id, sid_digest);

CREATE TABLE human_browser_session_revocations (
    revocation_idempotency_id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    sid_digest bytea NOT NULL CHECK (octet_length(sid_digest) = 32),
    request_id uuid NOT NULL,
    matched_session_id uuid,
    changed boolean NOT NULL,
    CHECK (NOT changed OR matched_session_id IS NOT NULL),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (matched_session_id, user_id, sid_digest)
        REFERENCES human_browser_sessions(id, user_id, sid_digest)
        ON DELETE RESTRICT
);

-- Revocation receipts are immutable audit bindings. A retry reads the row;
-- it never rewrites the SID, actor, request, or outcome.
CREATE FUNCTION reject_human_browser_session_revocation_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'human browser session revocation records are immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
REVOKE ALL ON FUNCTION reject_human_browser_session_revocation_mutation() FROM PUBLIC;
CREATE TRIGGER human_browser_session_revocations_immutable
BEFORE UPDATE OR DELETE ON human_browser_session_revocations
FOR EACH ROW EXECUTE FUNCTION reject_human_browser_session_revocation_mutation();

ALTER TABLE human_browser_session_revocations ENABLE ROW LEVEL SECURITY;
ALTER TABLE human_browser_session_revocations FORCE ROW LEVEL SECURITY;
REVOKE ALL ON human_browser_session_revocations FROM PUBLIC, hephaestus_app,
    hephaestus_worker;
GRANT SELECT, INSERT ON human_browser_session_revocations TO hephaestus_worker;
CREATE POLICY human_browser_session_revocations_worker
    ON human_browser_session_revocations TO hephaestus_worker
    USING (true) WITH CHECK (true);
