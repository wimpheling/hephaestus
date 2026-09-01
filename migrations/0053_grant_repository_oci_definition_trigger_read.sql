-- The ready-event trigger runs as the authorization owner and resolves the
-- immutable definition from the materialization result. It needs this narrow
-- read grant even though normal application and worker callers use their own
-- RLS-scoped roles.

GRANT SELECT ON repository_oci_image_definitions TO hephaestus_authz_owner;
