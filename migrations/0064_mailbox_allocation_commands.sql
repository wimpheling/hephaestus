-- Instance-scoped mailbox allocation is a user mutation.  Keep its retry
-- identity separate from mailbox state so a later retry returns the same
-- mailbox and receipt after another operation touches the instance.
CREATE TABLE mailbox_allocation_commands (
    command_key bytea PRIMARY KEY CHECK (octet_length(command_key) = 32),
    operation text NOT NULL CHECK (operation = 'create_mailbox'),
    project_id uuid NOT NULL REFERENCES projects(id),
    instance_id uuid NOT NULL REFERENCES agent_instances(id),
    actor_id uuid NOT NULL REFERENCES users(id),
    mailbox_id uuid REFERENCES mailboxes(id),
    request_id uuid,
    completed_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE mailbox_allocation_commands ENABLE ROW LEVEL SECURITY;
ALTER TABLE mailbox_allocation_commands FORCE ROW LEVEL SECURITY;
CREATE POLICY mailbox_allocation_commands_manage ON mailbox_allocation_commands
    FOR ALL TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'agent_instance', instance_id::text) = 1
        AND check_permission('user', hephaestus_actor_id(), 'can_manage',
        'project', project_id::text) = 1)
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'agent_instance', instance_id::text) = 1
        AND check_permission('user', hephaestus_actor_id(), 'can_manage',
        'project', project_id::text) = 1);
CREATE POLICY mailbox_allocation_commands_worker ON mailbox_allocation_commands
    TO hephaestus_worker USING (true) WITH CHECK (true);

GRANT SELECT, INSERT, UPDATE ON mailbox_allocation_commands TO hephaestus_app, hephaestus_worker;

-- Allocation has no body-bearing event of its own.  Touching the mailbox
-- emits the instance-scoped redacted product event needed for mutation
-- receipts and projections while preserving mailbox ownership in this table.
CREATE FUNCTION mailbox_allocation_application_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    occurrence uuid := COALESCE(
        NULLIF(current_setting('hephaestus.occurrence_id', true), '')::uuid,
        gen_random_uuid()
    );
BEGIN
    IF current_setting('hephaestus.mailbox_allocation', true) IS DISTINCT FROM 'true' THEN
        RETURN NEW;
    END IF;
    PERFORM append_application_event(
        occurrence, 'agent_instance', NEW.instance_id,
        'agent_instance', NEW.instance_id, 'agent_instance.changed',
        CASE WHEN TG_OP = 'INSERT' THEN 'created' ELSE 'updated' END,
        NULL, NEW.project_id, NULL
    );
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION mailbox_allocation_application_event() FROM PUBLIC;
CREATE TRIGGER mailbox_allocation_application_event_trigger
AFTER INSERT OR UPDATE ON mailboxes
FOR EACH ROW EXECUTE FUNCTION mailbox_allocation_application_event();
