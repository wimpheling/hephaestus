-- The preparation-history trigger runs as the authorization owner so it can
-- record a transition atomically with the worker-owned queue mutation. Grant
-- only that trigger owner INSERT; application callers retain read-only access
-- through the existing RLS policy.

GRANT INSERT ON repository_oci_image_preparation_events TO hephaestus_authz_owner;
