-- Durable release-owned UI handoffs and generation-bound child sessions.
--
-- This draft depends on migrations 0085/0086 (human browser sessions), 0087
-- (installation generations), and 0088 (organization-owned installation
-- targets and the source/target organization invariant). It intentionally
-- contains no application-role verifier or adapter-specific SQL function.
-- Worker writes must insert the child and consume its handoff in one
-- transaction; the lifecycle triggers enforce that ordering.

CREATE TABLE ui_browser_handoffs (
    id uuid PRIMARY KEY,
    handoff_digest bytea NOT NULL UNIQUE
        CHECK (octet_length(handoff_digest) = 32),
    request_id uuid NOT NULL,
    actor_id uuid NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    parent_session_id uuid NOT NULL
        REFERENCES human_browser_sessions(id) ON DELETE RESTRICT,
    installation_id uuid NOT NULL,
    generation_id uuid NOT NULL,
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    route text NOT NULL,
    issued_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (
        octet_length(route) BETWEEN 1 AND 256
        AND route ~ '^[A-Za-z0-9._~-]+(/[A-Za-z0-9._~-]+)*$'
        AND route !~ '(^|/)(\.{1,2})(/|$)'
    ),
    CHECK (expires_at = issued_at + interval '60 seconds'),
    CHECK (
        consumed_at IS NULL
        OR (consumed_at >= issued_at AND consumed_at < expires_at)
    ),
    FOREIGN KEY (generation_id, installation_id)
        REFERENCES ui_installation_generations(id, installation_id)
        ON DELETE RESTRICT
);

CREATE INDEX ui_browser_handoffs_parent
    ON ui_browser_handoffs (parent_session_id, expires_at, id);
CREATE INDEX ui_browser_handoffs_expiry
    ON ui_browser_handoffs (expires_at, id)
    WHERE consumed_at IS NULL;

CREATE TABLE ui_browser_sessions (
    id uuid PRIMARY KEY,
    session_digest bytea NOT NULL UNIQUE
        CHECK (octet_length(session_digest) = 32),
    request_id uuid NOT NULL,
    handoff_id uuid NOT NULL UNIQUE
        REFERENCES ui_browser_handoffs(id) ON DELETE RESTRICT,
    parent_session_id uuid NOT NULL
        REFERENCES human_browser_sessions(id) ON DELETE RESTRICT,
    installation_id uuid NOT NULL,
    generation_id uuid NOT NULL,
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    route text NOT NULL,
    issued_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (
        octet_length(route) BETWEEN 1 AND 256
        AND route ~ '^[A-Za-z0-9._~-]+(/[A-Za-z0-9._~-]+)*$'
        AND route !~ '(^|/)(\.{1,2})(/|$)'
    ),
    CHECK (expires_at > issued_at),
    CHECK (expires_at <= issued_at + interval '12 hours'),
    FOREIGN KEY (generation_id, installation_id)
        REFERENCES ui_installation_generations(id, installation_id)
        ON DELETE RESTRICT
);

CREATE INDEX ui_browser_sessions_parent
    ON ui_browser_sessions (parent_session_id, expires_at, id);
CREATE INDEX ui_browser_sessions_generation
    ON ui_browser_sessions (generation_id, installation_id, expires_at, id);

CREATE FUNCTION enforce_ui_browser_handoff_binding() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    parent_user_id uuid;
    target_organization_id uuid;
BEGIN
    SELECT session.user_id
    INTO parent_user_id
    FROM human_browser_sessions AS session
    WHERE session.id = NEW.parent_session_id;
    IF parent_user_id IS DISTINCT FROM NEW.actor_id THEN
        RAISE EXCEPTION 'UI browser handoff actor does not own parent session'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;

    SELECT CASE
               WHEN installation.scope = 'global'
                   THEN installation.organization_id
               ELSE target_project.organization_id
           END
    INTO target_organization_id
    FROM ui_installations AS installation
    LEFT JOIN repositories AS target_repository
        ON target_repository.id = installation.repository_id
    LEFT JOIN projects AS target_project
        ON target_project.id = COALESCE(
            target_repository.project_id, installation.project_id
        )
    WHERE installation.id = NEW.installation_id;
    IF target_organization_id IS DISTINCT FROM NEW.organization_id THEN
        RAISE EXCEPTION 'UI browser handoff organization is invalid'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_ui_browser_handoff_binding() FROM PUBLIC;

-- Consumption is a transition, never an insert state. The child and the
-- consuming update must be committed together by the worker transaction.
CREATE FUNCTION reject_initial_ui_browser_handoff_consumption() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.consumed_at IS NOT NULL THEN
        RAISE EXCEPTION 'UI browser handoff cannot be inserted consumed'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION reject_initial_ui_browser_handoff_consumption() FROM PUBLIC;

-- Handoff identity and issue provenance are immutable. Exactly one transition
-- is permitted: consumed_at NULL -> a timestamp, after a child row already
-- exists for this handoff in the same worker transaction.
CREATE FUNCTION enforce_ui_browser_handoff_lifecycle() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    child_issued_at timestamptz;
BEGIN
    IF OLD.id <> NEW.id
       OR OLD.handoff_digest <> NEW.handoff_digest
       OR OLD.request_id <> NEW.request_id
       OR OLD.actor_id <> NEW.actor_id
       OR OLD.parent_session_id <> NEW.parent_session_id
       OR OLD.installation_id <> NEW.installation_id
       OR OLD.generation_id <> NEW.generation_id
       OR OLD.organization_id <> NEW.organization_id
       OR OLD.route <> NEW.route
       OR OLD.issued_at <> NEW.issued_at
       OR OLD.expires_at <> NEW.expires_at
       OR OLD.created_at <> NEW.created_at
    THEN
        RAISE EXCEPTION 'UI browser handoff identity is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.consumed_at IS NOT NULL
       OR NEW.consumed_at IS NULL
       OR NOT EXISTS (
           SELECT 1
           FROM ui_browser_sessions AS child
           WHERE child.handoff_id = OLD.id
       )
    THEN
        RAISE EXCEPTION 'UI browser handoff consumption is one-time'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;

    SELECT child.issued_at
    INTO child_issued_at
    FROM ui_browser_sessions AS child
    WHERE child.handoff_id = OLD.id;
    IF NEW.consumed_at < child_issued_at
       OR NEW.consumed_at < OLD.issued_at
       OR NEW.consumed_at >= OLD.expires_at
    THEN
        RAISE EXCEPTION 'UI browser handoff consumption timestamp is invalid'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_ui_browser_handoff_lifecycle() FROM PUBLIC;

CREATE FUNCTION enforce_ui_browser_binding() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    handoff_actor_id uuid;
    handoff_parent_session_id uuid;
    handoff_installation_id uuid;
    handoff_generation_id uuid;
    handoff_organization_id uuid;
    handoff_route text;
    handoff_issued_at timestamptz;
    handoff_expires_at timestamptz;
    handoff_consumed_at timestamptz;
    parent_user_id uuid;
    parent_expires_at timestamptz;
    target_organization_id uuid;
BEGIN
    SELECT handoff.actor_id, handoff.parent_session_id,
           handoff.installation_id, handoff.generation_id,
           handoff.organization_id, handoff.route,
           handoff.issued_at, handoff.expires_at, handoff.consumed_at
    INTO handoff_actor_id, handoff_parent_session_id,
         handoff_installation_id, handoff_generation_id,
         handoff_organization_id, handoff_route,
         handoff_issued_at, handoff_expires_at, handoff_consumed_at
    FROM ui_browser_handoffs AS handoff
    WHERE handoff.id = NEW.handoff_id;
    IF handoff_actor_id IS NULL
       OR NEW.parent_session_id <> handoff_parent_session_id
       OR NEW.installation_id <> handoff_installation_id
       OR NEW.generation_id <> handoff_generation_id
       OR NEW.organization_id <> handoff_organization_id
       OR NEW.route <> handoff_route
       OR handoff_consumed_at IS NOT NULL
       OR NEW.issued_at < handoff_issued_at
       OR NEW.issued_at >= handoff_expires_at
    THEN
        RAISE EXCEPTION 'UI browser child binding does not match handoff'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;

    SELECT session.user_id, session.expires_at
    INTO parent_user_id, parent_expires_at
    FROM human_browser_sessions AS session
    WHERE session.id = NEW.parent_session_id;
    IF parent_user_id IS DISTINCT FROM handoff_actor_id
       OR parent_expires_at IS NULL
       OR NEW.expires_at > parent_expires_at
    THEN
        RAISE EXCEPTION 'UI browser child parent binding is invalid'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;

    SELECT CASE
               WHEN installation.scope = 'global'
                   THEN installation.organization_id
               ELSE target_project.organization_id
           END
    INTO target_organization_id
    FROM ui_installations AS installation
    LEFT JOIN repositories AS target_repository
        ON target_repository.id = installation.repository_id
    LEFT JOIN projects AS target_project
        ON target_project.id = COALESCE(
            target_repository.project_id, installation.project_id
        )
    WHERE installation.id = NEW.installation_id;
    IF target_organization_id IS DISTINCT FROM NEW.organization_id THEN
        RAISE EXCEPTION 'UI browser binding organization is invalid'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_ui_browser_binding() FROM PUBLIC;

CREATE FUNCTION reject_ui_browser_handoff_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'UI browser handoff records are immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
REVOKE ALL ON FUNCTION reject_ui_browser_handoff_mutation() FROM PUBLIC;

CREATE FUNCTION reject_ui_browser_session_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'UI browser session records are immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
REVOKE ALL ON FUNCTION reject_ui_browser_session_mutation() FROM PUBLIC;

-- A child cannot be committed without its handoff being consumed. This is
-- deferred because the safe worker transaction inserts the child first, then
-- marks the handoff consumed; the handoff trigger enforces the reverse
-- direction at the update point.
CREATE FUNCTION enforce_ui_browser_child_consumed() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM ui_browser_handoffs AS handoff
        WHERE handoff.id = NEW.handoff_id
          AND handoff.consumed_at IS NOT NULL
    ) THEN
        RAISE EXCEPTION 'UI browser child requires consumed handoff'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_ui_browser_child_consumed() FROM PUBLIC;

CREATE TRIGGER ui_browser_handoff_lifecycle
BEFORE UPDATE ON ui_browser_handoffs
FOR EACH ROW EXECUTE FUNCTION enforce_ui_browser_handoff_lifecycle();
CREATE TRIGGER ui_browser_handoff_initial_state
BEFORE INSERT ON ui_browser_handoffs
FOR EACH ROW EXECUTE FUNCTION reject_initial_ui_browser_handoff_consumption();
CREATE TRIGGER ui_browser_handoff_binding
BEFORE INSERT ON ui_browser_handoffs
FOR EACH ROW EXECUTE FUNCTION enforce_ui_browser_handoff_binding();
CREATE TRIGGER ui_browser_handoff_delete_forbidden
BEFORE DELETE ON ui_browser_handoffs
FOR EACH ROW EXECUTE FUNCTION reject_ui_browser_handoff_mutation();
CREATE TRIGGER ui_browser_session_binding
BEFORE INSERT ON ui_browser_sessions
FOR EACH ROW EXECUTE FUNCTION enforce_ui_browser_binding();
CREATE TRIGGER ui_browser_session_mutation_forbidden
BEFORE UPDATE OR DELETE ON ui_browser_sessions
FOR EACH ROW EXECUTE FUNCTION reject_ui_browser_session_mutation();
CREATE CONSTRAINT TRIGGER ui_browser_child_requires_consumed_handoff
AFTER INSERT ON ui_browser_sessions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_ui_browser_child_consumed();

ALTER TABLE ui_browser_handoffs ENABLE ROW LEVEL SECURITY;
ALTER TABLE ui_browser_handoffs FORCE ROW LEVEL SECURITY;
ALTER TABLE ui_browser_sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE ui_browser_sessions FORCE ROW LEVEL SECURITY;

CREATE POLICY ui_browser_handoffs_worker
    ON ui_browser_handoffs FOR ALL TO hephaestus_worker
    USING (true) WITH CHECK (true);
CREATE POLICY ui_browser_sessions_worker
    ON ui_browser_sessions FOR ALL TO hephaestus_worker
    USING (true) WITH CHECK (true);

REVOKE ALL ON ui_browser_handoffs, ui_browser_sessions
    FROM PUBLIC, hephaestus_app, hephaestus_worker;
GRANT SELECT, INSERT, UPDATE ON ui_browser_handoffs TO hephaestus_worker;
GRANT SELECT, INSERT ON ui_browser_sessions TO hephaestus_worker;
