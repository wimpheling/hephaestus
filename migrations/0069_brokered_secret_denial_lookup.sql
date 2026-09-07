-- Return only the exact immutable runtime authority needed to record a
-- brokered HTTPS denial after the application role has lost live access.
CREATE FUNCTION lookup_brokered_secret_denial_context(
    p_credential_hash bytea,
    p_run_id uuid,
    p_slot_key text,
    p_rule_id uuid,
    p_destination_origin text
)
RETURNS TABLE (
    session_id uuid,
    run_id uuid,
    instance_id uuid,
    instance_revision_id uuid,
    attachment_id uuid,
    phase text,
    expires_at timestamptz,
    lease_id uuid,
    secret_version_id uuid,
    destinations text[]
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
    SELECT session.id,
           session.run_id,
           session.instance_id,
           session.instance_revision_id,
           session.attachment_id,
           session.phase,
           session.expires_at,
           lease.id,
           lease.secret_version_id,
           lease.destinations
      FROM public.secret_runtime_sessions AS session
      JOIN public.secret_leases AS lease
        ON lease.session_id = session.id
       AND lease.run_id = session.run_id
      JOIN public.brokered_secret_lease_snapshots AS snapshot
        ON snapshot.lease_id = lease.id
       AND snapshot.runtime_session_id = session.id
       AND snapshot.run_id = session.run_id
       AND snapshot.binding_id = lease.binding_id
       AND snapshot.secret_version_id = lease.secret_version_id
     WHERE session.runtime_credential_hash = p_credential_hash
       AND session.run_id = p_run_id
       AND lease.slot_key = p_slot_key
       AND lease.delivery_mode = 'brokered'
       AND lease.status <> 'released'
       AND snapshot.rule_id = p_rule_id
       AND snapshot.destination_origin = p_destination_origin
     LIMIT 1
$$;
REVOKE ALL ON FUNCTION lookup_brokered_secret_denial_context(bytea, uuid, text, uuid, text)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION lookup_brokered_secret_denial_context(bytea, uuid, text, uuid, text)
    TO hephaestus_app, hephaestus_worker;
