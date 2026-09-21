-- Project readers may inspect only aggregate loss metadata.  The worker-owned
-- policy and forced RLS from migration 0078 remain the append/maintenance
-- boundary; this application policy is project-scoped and payload-free.
CREATE POLICY gateway_service_log_usage_read ON gateway_service_log_project_usage
    FOR SELECT TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_read', 'project', project_id::text) = 1);

REVOKE SELECT ON gateway_service_log_project_usage FROM hephaestus_app;
GRANT SELECT (project_id, storage_dropped_chunks, storage_dropped_bytes)
    ON gateway_service_log_project_usage TO hephaestus_app;
