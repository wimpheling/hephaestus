-- Gateway mailbox binding mutations are product operations. Capture one
-- committed aggregate event after the binding grant exists so mediated RPCs
-- can return the exact occurrence-scoped receipt.
CREATE OR REPLACE FUNCTION capture_gateway_binding_application_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    gateway_id uuid;
    project_id uuid;
    occurrence uuid := COALESCE(
        NULLIF(current_setting('hephaestus.occurrence_id', true), '')::uuid,
        NULLIF(current_setting('hephaestus.request_id', true), '')::uuid,
        gen_random_uuid()
    );
    change text;
BEGIN
    SELECT binding.gateway_id, binding.project_id
      INTO gateway_id, project_id
      FROM gateway_mailbox_bindings AS binding
     WHERE binding.id = NEW.binding_id;

    change := CASE
        WHEN TG_OP = 'INSERT' THEN 'updated'
        WHEN OLD.status IS DISTINCT FROM NEW.status THEN 'state_changed'
        ELSE NULL
    END;

    IF change IS NOT NULL AND gateway_id IS NOT NULL THEN
        PERFORM append_application_event(
            occurrence, 'project', project_id, 'gateway', gateway_id,
            'gateway.changed', change, NULL, NULL, NULL
        );
    END IF;
    RETURN NEW;
END
$$;

DROP TRIGGER IF EXISTS gateway_mailbox_binding_grant_application_event
    ON gateway_mailbox_binding_grants;
CREATE TRIGGER gateway_mailbox_binding_grant_application_event
AFTER INSERT OR UPDATE OF status ON gateway_mailbox_binding_grants
FOR EACH ROW EXECUTE FUNCTION capture_gateway_binding_application_event();
