-- Redacted append-only evidence for release-owned UI request decisions.
--
-- This stream is not a product-event journal and has no owner aggregate or
-- outbox side effect. It deliberately has no parent SID, credential digest,
-- path, query, header, body, or provider-response column. Anonymous denials
-- leave all safe subject/target references NULL.

CREATE TABLE public.ui_request_audit_events (
    id uuid PRIMARY KEY,
    request_id uuid NOT NULL,
    surface text NOT NULL CHECK (
        surface IN (
            'handoff_issue', 'handoff_exchange', 'bootstrap', 'content', 'static',
            'managed', 'api', 'embed'
        )
    ),
    decision text NOT NULL CHECK (decision IN ('allowed', 'denied')),
    outcome text NOT NULL CHECK (
        outcome IN ('succeeded', 'failed', 'not_attempted')
    ),
    reason_code text NOT NULL CHECK (
        reason_code IN (
            'none', 'unauthenticated', 'unauthorized', 'invalid_input',
            'invalid_route', 'expired', 'revoked', 'generation_mismatch',
            'not_found', 'unavailable', 'upstream_failure'
        )
    ),
    actor_id uuid REFERENCES users(id) ON DELETE RESTRICT,
    organization_id uuid REFERENCES organizations(id) ON DELETE RESTRICT,
    installation_id uuid REFERENCES ui_installations(id) ON DELETE RESTRICT,
    generation_id uuid REFERENCES ui_installation_generations(id) ON DELETE RESTRICT,
    child_session_id uuid REFERENCES ui_browser_sessions(id) ON DELETE RESTRICT,
    gateway_id uuid REFERENCES gateways(id) ON DELETE RESTRICT,
    gateway_revision_id uuid REFERENCES gateway_revisions(id) ON DELETE RESTRICT,
    occurred_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (
        (decision = 'denied' AND outcome = 'not_attempted')
        OR (decision = 'allowed' AND outcome IN ('succeeded', 'failed'))
    ),
    CHECK (generation_id IS NULL OR installation_id IS NOT NULL),
    CHECK (
        (actor_id IS NULL AND organization_id IS NULL
            AND installation_id IS NULL AND generation_id IS NULL)
        OR (actor_id IS NOT NULL AND organization_id IS NULL
            AND installation_id IS NULL AND generation_id IS NULL)
        OR (actor_id IS NOT NULL AND organization_id IS NOT NULL
            AND installation_id IS NOT NULL AND generation_id IS NOT NULL)
    ),
    CHECK ((gateway_id IS NULL) = (gateway_revision_id IS NULL)),
    CHECK (
        child_session_id IS NULL
        OR (installation_id IS NOT NULL AND generation_id IS NOT NULL)
    )
);

CREATE INDEX ui_request_audit_events_by_request
    ON public.ui_request_audit_events (request_id, occurred_at, id);
CREATE INDEX ui_request_audit_events_by_organization
    ON public.ui_request_audit_events (organization_id, occurred_at DESC, id DESC)
    WHERE organization_id IS NOT NULL;

-- Safe context references are accepted only when they describe one exact
-- installation owner, generation, and child session. Unverified caller
-- values must be omitted by the typed port and therefore cannot become
-- durable verified context through a direct worker insert.
CREATE FUNCTION enforce_ui_request_audit_context() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET row_security = off
AS $$
DECLARE
    installation_organization uuid;
    generation_installation uuid;
    child_installation uuid;
    child_generation uuid;
    child_organization uuid;
    child_actor uuid;
    gateway_binding_exists boolean;
BEGIN
    IF NEW.gateway_id IS NOT NULL THEN
        SELECT EXISTS (
            SELECT 1
            FROM public.ui_installation_bindings AS binding
            WHERE binding.generation_id = NEW.generation_id
              AND binding.gateway_id = NEW.gateway_id
              AND binding.gateway_revision_id = NEW.gateway_revision_id
        ) INTO gateway_binding_exists;
        IF NOT gateway_binding_exists THEN
            RAISE EXCEPTION 'UI audit gateway context is not an immutable generation binding'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    END IF;

    IF NEW.installation_id IS NOT NULL THEN
        SELECT CASE
                   WHEN installation.scope = 'global'
                   THEN installation.organization_id
                   ELSE project.organization_id
               END
        INTO installation_organization
        FROM public.ui_installations AS installation
        LEFT JOIN public.projects AS project ON project.id = installation.project_id
        WHERE installation.id = NEW.installation_id;
        IF installation_organization IS NULL THEN
            RAISE EXCEPTION 'UI audit installation context is unknown'
                USING ERRCODE = 'foreign_key_violation';
        END IF;
        IF NEW.organization_id IS NOT NULL
           AND NEW.organization_id <> installation_organization
        THEN
            RAISE EXCEPTION 'UI audit organization context does not match installation'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    END IF;

    IF NEW.generation_id IS NOT NULL THEN
        SELECT generation.installation_id
        INTO generation_installation
        FROM public.ui_installation_generations AS generation
        WHERE generation.id = NEW.generation_id;
        IF generation_installation IS NULL
           OR generation_installation <> NEW.installation_id
        THEN
            RAISE EXCEPTION 'UI audit generation context does not match installation'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    END IF;

    IF NEW.child_session_id IS NOT NULL THEN
        SELECT session.installation_id, session.generation_id,
               session.organization_id, parent.user_id
        INTO child_installation, child_generation, child_organization, child_actor
        FROM public.ui_browser_sessions AS session
        JOIN public.human_browser_sessions AS parent
          ON parent.id = session.parent_session_id
        WHERE session.id = NEW.child_session_id;
        IF child_installation IS NULL
           OR child_installation <> NEW.installation_id
           OR child_generation <> NEW.generation_id
           OR (
               NEW.organization_id IS NOT NULL
               AND child_organization <> NEW.organization_id
           )
           OR child_actor <> NEW.actor_id
           OR NEW.actor_id IS NULL
           OR NEW.organization_id IS NULL
           OR NEW.installation_id IS NULL
           OR NEW.generation_id IS NULL
        THEN
            RAISE EXCEPTION 'UI audit child context does not match installation'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    END IF;

    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_ui_request_audit_context() FROM PUBLIC;

CREATE TRIGGER ui_request_audit_context
BEFORE INSERT ON public.ui_request_audit_events
FOR EACH ROW EXECUTE FUNCTION enforce_ui_request_audit_context();

CREATE FUNCTION reject_ui_request_audit_mutation() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    RAISE EXCEPTION 'UI request audit events are immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
REVOKE ALL ON FUNCTION reject_ui_request_audit_mutation() FROM PUBLIC;

CREATE TRIGGER ui_request_audit_events_immutable
BEFORE UPDATE OR DELETE ON public.ui_request_audit_events
FOR EACH ROW EXECUTE FUNCTION reject_ui_request_audit_mutation();

ALTER TABLE public.ui_request_audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.ui_request_audit_events FORCE ROW LEVEL SECURITY;
CREATE POLICY ui_request_audit_events_worker
    ON public.ui_request_audit_events FOR INSERT TO hephaestus_worker
    WITH CHECK (true);

-- No application-role write, credential, or direct-table read grant is made.
-- There is currently no UI audit inspection surface; any future inspection
-- must be a separate security-definer, organization-scoped read contract.
REVOKE ALL ON public.ui_request_audit_events
    FROM PUBLIC, hephaestus_app, hephaestus_worker;
GRANT INSERT ON public.ui_request_audit_events TO hephaestus_worker;
