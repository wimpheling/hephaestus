-- Mailbox delivery changes are published through the existing durable product
-- event journal.  WatchAgentInstance already reauthorizes each delivery, so
-- this redacted invalidation lets authorized clients refresh the mailbox
-- inspection projection without introducing a body-bearing stream.
CREATE FUNCTION mailbox_delivery_application_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    row_value mailbox_deliveries%ROWTYPE;
    occurrence uuid := COALESCE(
        NULLIF(current_setting('hephaestus.occurrence_id', true), '')::uuid,
        gen_random_uuid()
    );
BEGIN
    row_value := CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
    PERFORM append_application_event(
        occurrence,
        'agent_instance',
        row_value.instance_id,
        'agent_instance',
        row_value.instance_id,
        'agent_instance.changed',
        CASE WHEN TG_OP = 'INSERT' THEN 'created' ELSE 'updated' END,
        NULL,
        row_value.project_id,
        NULL
    );
    RETURN row_value;
END
$$;
REVOKE ALL ON FUNCTION mailbox_delivery_application_event() FROM PUBLIC;
CREATE TRIGGER mailbox_delivery_application_event_trigger
AFTER INSERT OR UPDATE OR DELETE ON mailbox_deliveries
FOR EACH ROW EXECUTE FUNCTION mailbox_delivery_application_event();
