-- The application reader needs only the durable gateway/project relationship
-- to validate an instance scope.  Forced RLS remains the authorization
-- boundary; no gateway or revision configuration columns are exposed.
GRANT SELECT (id, project_id) ON gateways TO hephaestus_app;
