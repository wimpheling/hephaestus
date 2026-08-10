-- Gateway lifecycle receipts are keyed by the mediator idempotency occurrence,
-- just like every other product mutation. The original gateway trigger used
-- request_id instead, making a committed transition impossible to acknowledge
-- through the reauthorizing product-event watch.
CREATE OR REPLACE FUNCTION capture_gateway_application_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    occurrence uuid := COALESCE(
        NULLIF(current_setting('hephaestus.occurrence_id', true), '')::uuid,
        gen_random_uuid()
    );
    change text;
BEGIN
    change := CASE
        WHEN TG_OP = 'INSERT' THEN 'created'
        WHEN OLD.lifecycle IS DISTINCT FROM NEW.lifecycle THEN 'state_changed'
        WHEN OLD.active_revision_id IS DISTINCT FROM NEW.active_revision_id THEN 'updated'
        ELSE NULL
    END;
    IF change IS NOT NULL THEN
        PERFORM append_application_event(
            occurrence, 'project', NEW.project_id, 'gateway', NEW.id,
            'gateway.changed', change,
            CASE NEW.lifecycle
                WHEN 'enabled' THEN 'active'
                WHEN 'paused' THEN 'paused_activation_recovery'
                WHEN 'removed' THEN 'removed'
            END,
            NULL, NULL
        );
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION capture_gateway_application_event() FROM PUBLIC;
