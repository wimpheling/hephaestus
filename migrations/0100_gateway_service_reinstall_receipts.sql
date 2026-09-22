-- Service gateway reinstall commands update desired_service_revision_id. Keep
-- that mediated mutation receipt-producing when the gateway row already
-- exists. Migration 0073 retained the desired-revision trigger but replaced
-- the earlier gateway_install replay branch.
CREATE OR REPLACE FUNCTION capture_gateway_application_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    occurrence uuid := COALESCE(
        NULLIF(current_setting('hephaestus.occurrence_id', true), '')::uuid,
        NULLIF(current_setting('hephaestus.request_id', true), '')::uuid,
        gen_random_uuid()
    );
    change text;
BEGIN
    change := CASE
        WHEN TG_OP = 'INSERT' THEN 'created'
        WHEN OLD.lifecycle IS DISTINCT FROM NEW.lifecycle THEN 'state_changed'
        WHEN OLD.active_revision_id IS DISTINCT FROM NEW.active_revision_id THEN 'updated'
        WHEN OLD.desired_service_revision_id IS DISTINCT FROM NEW.desired_service_revision_id THEN 'updated'
        WHEN current_setting('hephaestus.gateway_install', true) = 'true'
            THEN 'updated'
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

DROP TRIGGER IF EXISTS gateway_product_event ON gateways;
CREATE TRIGGER gateway_product_event
AFTER INSERT OR UPDATE OF lifecycle, active_revision_id, desired_service_revision_id ON gateways
FOR EACH ROW EXECUTE FUNCTION capture_gateway_application_event();
