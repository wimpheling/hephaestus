-- Terminal host-mediated gateway requests must release both their durable
-- session and any exact inbound secret leases. Stateless guest handoffs retain
-- their existing completion semantics until their normal session expiry.
CREATE OR REPLACE FUNCTION gateway_invocation_complete(
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
  IF changed = 1 THEN
    UPDATE gateway_runtime_authority_sessions AS session
       SET status = 'revoked', revoked_at = now(),
           revocation_reason = 'invocation_' || p_outcome,
           updated_at = now()
     WHERE session.invocation_id = p_invocation_id
       AND session.admission_mode = 'host_mediated'
       AND session.status IN ('pending_handoff', 'active');

    UPDATE gateway_secret_leases AS lease
       SET status = 'revoked', revoked_at = now()
     WHERE lease.invocation_id = p_invocation_id
       AND lease.status = 'active'
       AND EXISTS (
           SELECT 1
             FROM gateway_runtime_authority_sessions AS session
            WHERE session.id = lease.runtime_session_id
              AND session.invocation_id = p_invocation_id
              AND session.admission_mode = 'host_mediated'
       );
  END IF;
  RETURN changed = 1;
END $$;
