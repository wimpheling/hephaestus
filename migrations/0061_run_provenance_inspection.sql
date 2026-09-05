-- A narrow projection keeps worker-only secret tables and values inaccessible.
-- Both run visibility and secret metadata authority are required for each row.
CREATE FUNCTION inspect_run_https_uses(target_run uuid, after_id uuid, page_size integer)
RETURNS TABLE (
    id uuid, request_id uuid, lease_id uuid, binding_id uuid,
    secret_version_id uuid, rule_id uuid, event_kind text,
    decision text, outcome text, occurred_at timestamptz
)
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
    SELECT audit.id, audit.request_id, snapshot.lease_id, snapshot.binding_id,
           snapshot.secret_version_id, snapshot.rule_id, audit.event_kind,
           audit.decision, audit.outcome, audit.occurred_at
    FROM public.brokered_secret_audit_events audit
    JOIN public.brokered_secret_lease_snapshots snapshot
      ON snapshot.id = audit.lease_snapshot_id AND snapshot.run_id = audit.run_id
    JOIN public.secret_versions version ON version.id = snapshot.secret_version_id
    WHERE audit.run_id = target_run
      AND public.check_permission('user', public.hephaestus_actor_id(),
          'can_read', 'run', target_run::text) = 1
      AND public.check_permission('user', public.hephaestus_actor_id(),
          'inspect_metadata', 'secret', version.secret_id::text) = 1
      AND (after_id IS NULL OR audit.id > after_id)
    ORDER BY audit.id
    LIMIT greatest(1, least(page_size, 201))
$$;
REVOKE ALL ON FUNCTION inspect_run_https_uses(uuid, uuid, integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION inspect_run_https_uses(uuid, uuid, integer) TO hephaestus_app;
