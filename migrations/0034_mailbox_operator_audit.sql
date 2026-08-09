-- Operator actions are durable scheduling evidence.  They deliberately retain
-- no envelope, producer, header, trace, or body data.
CREATE TABLE mailbox_operator_audit (
    id uuid PRIMARY KEY,
    mailbox_id uuid NOT NULL REFERENCES mailboxes(id),
    event_id uuid REFERENCES mailbox_events(id),
    actor_id uuid NOT NULL REFERENCES users(id),
    request_id uuid NOT NULL,
    action text NOT NULL CHECK (action IN ('pause', 'resume', 'retry', 'cancel', 'dead_letter')),
    changed boolean NOT NULL,
    occurred_at timestamptz NOT NULL DEFAULT now(),
    CHECK (
        (action IN ('pause', 'resume') AND event_id IS NULL)
        OR (action IN ('retry', 'cancel', 'dead_letter') AND event_id IS NOT NULL)
    )
);
CREATE INDEX mailbox_operator_audit_by_mailbox
    ON mailbox_operator_audit (mailbox_id, occurred_at DESC, id);

ALTER TABLE mailbox_operator_audit ENABLE ROW LEVEL SECURITY;
ALTER TABLE mailbox_operator_audit FORCE ROW LEVEL SECURITY;
CREATE POLICY mailbox_operator_audit_user_inspect ON mailbox_operator_audit
    FOR SELECT TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'agent_instance', (SELECT instance_id::text FROM mailboxes WHERE id = mailbox_id)) = 1);
CREATE POLICY mailbox_operator_audit_worker ON mailbox_operator_audit
    TO hephaestus_worker USING (true) WITH CHECK (true);
GRANT SELECT ON mailbox_operator_audit TO hephaestus_app, hephaestus_worker;
GRANT INSERT ON mailbox_operator_audit TO hephaestus_worker;
