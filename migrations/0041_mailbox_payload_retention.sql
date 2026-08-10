-- Mailbox provenance outlives opaque body bytes.  Bodies are retained for a
-- fixed audit window and are purged only after their sole delivery is
-- terminal; the immutable event/body identity, lengths, digest and retention
-- evidence remain available for inspection and deduplication.
ALTER TABLE mailbox_payloads
    ADD COLUMN body_purged_at timestamptz;

ALTER TABLE mailbox_payloads
    ALTER COLUMN encoded_body DROP NOT NULL;

ALTER TABLE mailbox_payloads
    DROP CONSTRAINT mailbox_payloads_encoded_length_check,
    ADD CONSTRAINT mailbox_payloads_encoded_length_check CHECK (
        (encoded_body IS NOT NULL
            AND body_purged_at IS NULL
            AND encoded_length = octet_length(encoded_body))
        OR (encoded_body IS NULL AND body_purged_at IS NOT NULL)
    );

-- Empty bodies are valid generic envelopes and must retain their exact zero
-- length rather than being confused with an already-purged body.
ALTER TABLE mailbox_payloads
    DROP CONSTRAINT mailbox_payloads_decoded_length_check,
    ADD CONSTRAINT mailbox_payloads_decoded_length_check
        CHECK (decoded_length BETWEEN 0 AND 1048576);

ALTER TABLE mailbox_payloads
    ADD CONSTRAINT mailbox_payloads_purge_evidence CHECK (
        (encoded_body IS NULL) = (body_purged_at IS NOT NULL)
    );

ALTER TABLE mailbox_payloads
    ALTER COLUMN retained_until SET DEFAULT (now() + interval '30 days');
UPDATE mailbox_payloads
SET retained_until = created_at + interval '30 days'
WHERE retained_until IS NULL;
ALTER TABLE mailbox_payloads ALTER COLUMN retained_until SET NOT NULL;

-- Accepted records remain immutable to all ordinary callers.  The narrow
-- worker-only cleanup function below sets this local flag and may only remove
-- bytes; it cannot alter the body identity or integrity evidence.
CREATE FUNCTION reject_mailbox_payload_mutation_except_retention_cleanup()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'UPDATE'
       AND current_setting('hephaestus.mailbox_payload_cleanup', true) = 'on'
       AND OLD.encoded_body IS NOT NULL
       AND NEW.encoded_body IS NULL
       AND OLD.id = NEW.id
       AND OLD.mailbox_id = NEW.mailbox_id
       AND OLD.project_id = NEW.project_id
       AND OLD.encoded_length = NEW.encoded_length
       AND OLD.decoded_length = NEW.decoded_length
       AND OLD.integrity_hash = NEW.integrity_hash
       AND OLD.content_type IS NOT DISTINCT FROM NEW.content_type
       AND OLD.content_encoding IS NOT DISTINCT FROM NEW.content_encoding
       AND OLD.created_at = NEW.created_at
       AND OLD.retained_until = NEW.retained_until
       AND NEW.body_purged_at IS NOT NULL
    THEN
        RETURN NEW;
    END IF;
    -- Retention recalculation is a worker-only policy operation. It never
    -- exposes or changes body bytes and is useful when an operator extends
    -- the audit window before the scheduled cleanup observes expiry.
    IF TG_OP = 'UPDATE'
       AND current_setting('hephaestus.mailbox_payload_retention_recalculation', true) = 'on'
       AND OLD.encoded_body IS NOT DISTINCT FROM NEW.encoded_body
       AND OLD.id = NEW.id
       AND OLD.mailbox_id = NEW.mailbox_id
       AND OLD.project_id = NEW.project_id
       AND OLD.encoded_length = NEW.encoded_length
       AND OLD.decoded_length = NEW.decoded_length
       AND OLD.integrity_hash = NEW.integrity_hash
       AND OLD.content_type IS NOT DISTINCT FROM NEW.content_type
       AND OLD.content_encoding IS NOT DISTINCT FROM NEW.content_encoding
       AND OLD.created_at = NEW.created_at
       AND OLD.body_purged_at IS NOT DISTINCT FROM NEW.body_purged_at
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'accepted mailbox records are immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
REVOKE ALL ON FUNCTION reject_mailbox_payload_mutation_except_retention_cleanup() FROM PUBLIC;
DROP TRIGGER mailbox_payloads_immutable ON mailbox_payloads;
CREATE TRIGGER mailbox_payloads_immutable
BEFORE UPDATE OR DELETE ON mailbox_payloads
FOR EACH ROW EXECUTE FUNCTION reject_mailbox_payload_mutation_except_retention_cleanup();

-- This is intentionally a function rather than a broad UPDATE grant.  The
-- terminal disposition and retention expiry are rechecked in the same locked
-- statement, so an active/reopened delivery can never lose its body.
CREATE FUNCTION purge_expired_mailbox_payloads(batch_limit integer)
RETURNS integer
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    purged integer;
BEGIN
    IF batch_limit < 1 OR batch_limit > 1000 THEN
        RAISE EXCEPTION 'mailbox payload cleanup batch limit is invalid'
            USING ERRCODE = 'check_violation';
    END IF;
    PERFORM set_config('hephaestus.mailbox_payload_cleanup', 'on', true);
    WITH candidates AS (
        SELECT payload.id
        FROM mailbox_payloads AS payload
        JOIN mailbox_events AS event ON event.body_id = payload.id
        JOIN mailbox_deliveries AS delivery ON delivery.event_id = event.id
        WHERE payload.encoded_body IS NOT NULL
          AND payload.retained_until <= now()
          AND delivery.disposition IN ('delivered', 'dead_lettered', 'cancelled')
        ORDER BY payload.retained_until, payload.id
        LIMIT batch_limit
        FOR UPDATE OF payload SKIP LOCKED
    )
    UPDATE mailbox_payloads AS payload
    SET encoded_body = NULL, body_purged_at = now()
    FROM candidates
    WHERE payload.id = candidates.id;
    GET DIAGNOSTICS purged = ROW_COUNT;
    RETURN purged;
END
$$;
REVOKE ALL ON FUNCTION purge_expired_mailbox_payloads(integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION purge_expired_mailbox_payloads(integer) TO hephaestus_worker;
