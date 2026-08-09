-- The edge may retain only request correlation and lifecycle evidence. HTTP
-- headers and bodies, provider credentials, and handler responses never enter
-- this control-plane record.
CREATE TABLE gateway_invocations (
    id uuid PRIMARY KEY,
    gateway_id uuid NOT NULL REFERENCES gateways(id),
    gateway_revision_id uuid NOT NULL,
    gateway_route_id uuid NOT NULL,
    project_id uuid NOT NULL REFERENCES projects(id),
    request_id uuid NOT NULL,
    outcome text NOT NULL CHECK (outcome IN ('accepted', 'completed', 'failed', 'timed_out', 'rejected')),
    accepted_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    UNIQUE (id, gateway_id),
    UNIQUE (request_id),
    FOREIGN KEY (gateway_revision_id, gateway_id)
        REFERENCES gateway_revisions(id, gateway_id),
    FOREIGN KEY (gateway_route_id, gateway_revision_id)
        REFERENCES gateway_routes(id, gateway_revision_id),
    FOREIGN KEY (gateway_id, project_id) REFERENCES gateways(id, project_id),
    CHECK ((outcome = 'accepted') = (completed_at IS NULL))
);
CREATE INDEX gateway_invocations_recent
    ON gateway_invocations (gateway_id, accepted_at DESC, id);

CREATE FUNCTION gateway_invocation_complete(
    p_invocation_id uuid, p_outcome text
) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE changed integer;
BEGIN
  IF p_outcome NOT IN ('completed', 'failed', 'timed_out', 'rejected') THEN
    RAISE EXCEPTION 'invalid gateway invocation terminal outcome' USING ERRCODE = 'check_violation';
  END IF;
  UPDATE gateway_invocations
     SET outcome = p_outcome, completed_at = now()
   WHERE id = p_invocation_id AND outcome = 'accepted';
  GET DIAGNOSTICS changed = ROW_COUNT;
  RETURN changed = 1;
END $$;
REVOKE ALL ON FUNCTION gateway_invocation_complete(uuid, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION gateway_invocation_complete(uuid, text) TO hephaestus_worker;

ALTER TABLE gateway_invocations ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_invocations FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_invocations_read ON gateway_invocations FOR SELECT TO hephaestus_app
  USING (check_permission('user', hephaestus_actor_id(), 'can_read', 'gateway', gateway_id::text) = 1);
CREATE POLICY gateway_invocations_worker ON gateway_invocations TO hephaestus_worker
  USING (true) WITH CHECK (true);
GRANT SELECT ON gateway_invocations TO hephaestus_app, hephaestus_worker;
GRANT INSERT, UPDATE ON gateway_invocations TO hephaestus_worker;

-- The private reconciler and dispatcher run as the worker. Existing gateway
-- RLS remains user-facing; this narrow policy permits reconstruction of active
-- desired routes without granting that worker any user-facing management API.
CREATE POLICY gateways_worker_read ON gateways FOR SELECT TO hephaestus_worker USING (true);
CREATE POLICY gateway_revisions_worker_read ON gateway_revisions FOR SELECT TO hephaestus_worker USING (true);
CREATE POLICY gateway_routes_worker_read ON gateway_routes FOR SELECT TO hephaestus_worker USING (true);
GRANT SELECT ON gateways, gateway_revisions, gateway_routes TO hephaestus_worker;
