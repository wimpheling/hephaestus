-- Hash-only developer personal access tokens. The application can expose
-- safe metadata, while bearer material never enters PostgreSQL.

CREATE FUNCTION pat_git_operations_are_valid(operations text[]) RETURNS boolean
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT cardinality(operations) BETWEEN 1 AND 3
       AND operations <@ ARRAY['discover', 'fetch', 'receive']::text[]
       AND cardinality(operations) = (
           SELECT count(DISTINCT operation)
           FROM unnest(operations) AS operation
       )
$$;

CREATE FUNCTION pat_repository_restrictions_are_valid(
    repositories uuid[]
) RETURNS boolean
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT repositories IS NULL OR (
        cardinality(repositories) BETWEEN 1 AND 128
        AND cardinality(repositories) = (
            SELECT count(DISTINCT repository_id)
            FROM unnest(repositories) AS repository_id
        )
    )
$$;

CREATE TABLE developer_personal_access_tokens (
    id uuid PRIMARY KEY,
    verifier_version smallint NOT NULL CHECK (verifier_version > 0),
    verifier_digest bytea NOT NULL CHECK (octet_length(verifier_digest) = 32),
    owner_user_id uuid NOT NULL REFERENCES users(id),
    label text NOT NULL CHECK (
        length(label) BETWEEN 1 AND 128
        AND label = btrim(label)
        AND label !~ '[[:cntrl:]]'
    ),
    git_operations text[] NOT NULL
        CHECK (pat_git_operations_are_valid(git_operations)),
    repository_restrictions uuid[]
        CHECK (pat_repository_restrictions_are_valid(repository_restrictions)),
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    last_used_at timestamptz,
    creation_request_id uuid NOT NULL,
    revocation_request_id uuid,
    rotated_from_id uuid,
    UNIQUE (id, owner_user_id),
    FOREIGN KEY (rotated_from_id, owner_user_id)
        REFERENCES developer_personal_access_tokens(id, owner_user_id),
    CHECK (expires_at > created_at),
    CHECK (expires_at <= created_at + interval '90 days'),
    CHECK (revoked_at IS NULL OR revoked_at >= created_at),
    CHECK (revoked_at IS NULL OR revoked_at < expires_at),
    CHECK ((revoked_at IS NULL) = (revocation_request_id IS NULL)),
    CHECK (last_used_at IS NULL OR last_used_at >= created_at),
    CHECK (last_used_at IS NULL OR last_used_at < expires_at),
    CHECK (last_used_at IS NULL OR revoked_at IS NULL OR last_used_at <= revoked_at),
    CHECK (rotated_from_id IS NULL OR rotated_from_id <> id)
);
CREATE INDEX developer_pats_by_owner
    ON developer_personal_access_tokens (owner_user_id, created_at DESC, id);
-- One mutation request may mint at most one unrecoverable plaintext value.
-- A retry after a lost response must fail closed instead of issuing a second
-- bearer credential under the same idempotency identity.
CREATE UNIQUE INDEX developer_pats_by_owner_creation_request
    ON developer_personal_access_tokens (
        owner_user_id,
        creation_request_id,
        COALESCE(rotated_from_id, '00000000-0000-0000-0000-000000000000'::uuid)
    );
CREATE INDEX developer_pats_active_expiry
    ON developer_personal_access_tokens (expires_at, id)
    WHERE revoked_at IS NULL;

CREATE TABLE personal_access_token_audit_events (
    id uuid PRIMARY KEY,
    token_id uuid NOT NULL,
    owner_user_id uuid NOT NULL REFERENCES users(id),
    event_type text NOT NULL CHECK (
        event_type IN ('issued', 'rotated', 'revoked', 'authenticated')
    ),
    request_id uuid NOT NULL,
    repository_id uuid,
    git_operation text CHECK (
        git_operation IS NULL OR git_operation IN ('discover', 'fetch', 'receive')
    ),
    related_token_id uuid,
    occurred_at timestamptz NOT NULL,
    FOREIGN KEY (token_id, owner_user_id)
        REFERENCES developer_personal_access_tokens(id, owner_user_id),
    FOREIGN KEY (related_token_id, owner_user_id)
        REFERENCES developer_personal_access_tokens(id, owner_user_id),
    CHECK ((repository_id IS NULL) = (git_operation IS NULL)),
    CHECK (
        (event_type = 'authenticated' AND repository_id IS NOT NULL)
        OR (event_type <> 'authenticated' AND repository_id IS NULL)
    ),
    CHECK (
        (event_type = 'rotated' AND related_token_id IS NOT NULL)
        OR (event_type <> 'rotated' AND related_token_id IS NULL)
    ),
    CHECK (related_token_id IS NULL OR related_token_id <> token_id)
);
CREATE INDEX personal_access_token_audit_by_owner
    ON personal_access_token_audit_events
       (owner_user_id, occurred_at DESC, id);
CREATE INDEX personal_access_token_audit_by_token
    ON personal_access_token_audit_events (token_id, occurred_at, id);

CREATE FUNCTION enforce_developer_pat_lifecycle() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.id <> NEW.id
       OR OLD.verifier_version <> NEW.verifier_version
       OR OLD.verifier_digest <> NEW.verifier_digest
       OR OLD.owner_user_id <> NEW.owner_user_id
       OR OLD.label <> NEW.label
       OR OLD.git_operations <> NEW.git_operations
       OR OLD.repository_restrictions IS DISTINCT FROM NEW.repository_restrictions
       OR OLD.created_at <> NEW.created_at
       OR OLD.expires_at <> NEW.expires_at
       OR OLD.creation_request_id <> NEW.creation_request_id
       OR OLD.rotated_from_id IS DISTINCT FROM NEW.rotated_from_id
    THEN
        RAISE EXCEPTION 'personal access token authority is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;

    IF OLD.revoked_at IS NOT NULL
       AND (
           OLD.revoked_at IS DISTINCT FROM NEW.revoked_at
           OR OLD.revocation_request_id IS DISTINCT FROM NEW.revocation_request_id
       )
    THEN
        RAISE EXCEPTION 'personal access token revocation is irreversible'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.last_used_at IS NOT NULL
       AND NEW.last_used_at IS DISTINCT FROM OLD.last_used_at
       AND (NEW.last_used_at IS NULL OR NEW.last_used_at < OLD.last_used_at)
    THEN
        RAISE EXCEPTION 'personal access token last-used time is monotonic'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER developer_personal_access_tokens_lifecycle
BEFORE UPDATE ON developer_personal_access_tokens
FOR EACH ROW EXECUTE FUNCTION enforce_developer_pat_lifecycle();

CREATE FUNCTION reject_personal_access_token_delete() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'personal access token records are retained for audit'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
CREATE TRIGGER developer_personal_access_tokens_no_delete
BEFORE DELETE ON developer_personal_access_tokens
FOR EACH ROW EXECUTE FUNCTION reject_personal_access_token_delete();
CREATE TRIGGER personal_access_token_audit_events_immutable
BEFORE UPDATE OR DELETE ON personal_access_token_audit_events
FOR EACH ROW EXECUTE FUNCTION reject_personal_access_token_delete();

GRANT SELECT, INSERT, UPDATE ON developer_personal_access_tokens
    TO hephaestus_app, hephaestus_worker;
GRANT SELECT, INSERT ON personal_access_token_audit_events
    TO hephaestus_app, hephaestus_worker;

ALTER TABLE developer_personal_access_tokens ENABLE ROW LEVEL SECURITY;
ALTER TABLE developer_personal_access_tokens FORCE ROW LEVEL SECURITY;
CREATE POLICY developer_pats_owner_select
    ON developer_personal_access_tokens FOR SELECT TO hephaestus_app
    USING (owner_user_id::text = hephaestus_actor_id());
CREATE POLICY developer_pats_owner_insert
    ON developer_personal_access_tokens FOR INSERT TO hephaestus_app
    WITH CHECK (owner_user_id::text = hephaestus_actor_id());
CREATE POLICY developer_pats_owner_update
    ON developer_personal_access_tokens FOR UPDATE TO hephaestus_app
    USING (owner_user_id::text = hephaestus_actor_id())
    WITH CHECK (owner_user_id::text = hephaestus_actor_id());
CREATE POLICY developer_pats_worker
    ON developer_personal_access_tokens TO hephaestus_worker
    USING (true) WITH CHECK (true);

ALTER TABLE personal_access_token_audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE personal_access_token_audit_events FORCE ROW LEVEL SECURITY;
CREATE POLICY personal_access_token_audit_owner_select
    ON personal_access_token_audit_events FOR SELECT TO hephaestus_app
    USING (owner_user_id::text = hephaestus_actor_id());
CREATE POLICY personal_access_token_audit_owner_insert
    ON personal_access_token_audit_events FOR INSERT TO hephaestus_app
    WITH CHECK (owner_user_id::text = hephaestus_actor_id());
CREATE POLICY personal_access_token_audit_worker
    ON personal_access_token_audit_events TO hephaestus_worker
    USING (true) WITH CHECK (true);
