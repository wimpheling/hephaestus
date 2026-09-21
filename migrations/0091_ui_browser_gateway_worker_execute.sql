-- The restricted worker calls migration 0090's security-definer verifier
-- after selecting a digest by safe child-session ID. It receives only the
-- verifier's safe row; no browser secret or parent-session identity crosses
-- the worker boundary.
GRANT EXECUTE ON FUNCTION authenticate_ui_browser_session(
    bytea, uuid, text, text, text
) TO hephaestus_worker;
