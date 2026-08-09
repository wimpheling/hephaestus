-- PostgreSQL is the authority for accepted mailbox work.  JetStream receives
-- only wake-up commands from the existing internal outbox; it is never the
-- source of an event body, delivery disposition, or logical attempt count.

-- The legacy outbox has a closed command classification.  Extend it before
-- any mailbox transaction can enqueue its stable wake-up command.
ALTER TABLE outbox DROP COLUMN message_class;
ALTER TABLE outbox ADD COLUMN message_class text
GENERATED ALWAYS AS (
    CASE
        WHEN subject IN (
            'hephaestus.build.requested.v1',
            'hephaestus.instance.run.requested.v1',
            'hephaestus.run.start',
            'heph.run.command.start.v1',
            'heph.run.command.cancel.v1',
            'hephaestus.control.execute',
            'heph.mailbox.v1.wake',
            'heph.mailbox.v1.dispatch',
            'heph.mailbox.v1.retry',
            'heph.mailbox.v1.cancel',
            'heph.mailbox.v1.recover'
        ) THEN 'internal_command'
        ELSE 'internal_signal'
    END
) STORED;
ALTER TABLE outbox ADD CONSTRAINT outbox_internal_message_class
    CHECK (message_class IN ('internal_command', 'internal_signal'));

CREATE TABLE mailboxes (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id),
    instance_id uuid NOT NULL REFERENCES agent_instances(id),
    state text NOT NULL CHECK (state IN ('active', 'paused', 'removed')),
    removed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (instance_id),
    UNIQUE (id, project_id),
    UNIQUE (id, instance_id),
    FOREIGN KEY (instance_id, project_id)
        REFERENCES agent_instances(id, project_id),
    CHECK (
        (state = 'removed' AND removed_at IS NOT NULL)
        OR (state <> 'removed' AND removed_at IS NULL)
    )
);
CREATE INDEX mailboxes_by_project ON mailboxes (project_id, created_at DESC, id);

-- A body is intentionally opaque to mailbox scheduling.  The immutable
-- encoded bytes, their exact length, and SHA-256 digest are committed with
-- acceptance; decoded byte limits are retained so a decoder cannot silently
-- turn a bounded envelope into an unbounded payload later.
CREATE TABLE mailbox_payloads (
    id uuid PRIMARY KEY,
    mailbox_id uuid NOT NULL REFERENCES mailboxes(id),
    project_id uuid NOT NULL REFERENCES projects(id),
    -- An empty HTTP request body is valid. It remains an opaque, bounded
    -- payload with an exact digest just like a non-empty body.
    encoded_body bytea NOT NULL CHECK (octet_length(encoded_body) BETWEEN 0 AND 1048576),
    encoded_length integer NOT NULL CHECK (encoded_length BETWEEN 0 AND 1048576),
    decoded_length integer NOT NULL CHECK (decoded_length BETWEEN 1 AND 4194304),
    integrity_hash bytea NOT NULL CHECK (octet_length(integrity_hash) = 32),
    content_type text CHECK (length(content_type) BETWEEN 1 AND 256),
    -- Compression is deliberately not accepted in MVP-02. Rejecting it at
    -- the authority boundary avoids a decompression-bomb parser before a
    -- bounded, audited decompressor policy exists.
    content_encoding text CHECK (content_encoding IS NULL OR content_encoding = 'identity'),
    created_at timestamptz NOT NULL DEFAULT now(),
    retained_until timestamptz,
    UNIQUE (id, mailbox_id),
    FOREIGN KEY (mailbox_id, project_id) REFERENCES mailboxes(id, project_id),
    CHECK (encoded_length = octet_length(encoded_body)),
    CHECK (decoded_length = encoded_length)
);
CREATE INDEX mailbox_payloads_retention
    ON mailbox_payloads (retained_until, created_at, id);

CREATE TABLE mailbox_events (
    id uuid PRIMARY KEY,
    mailbox_id uuid NOT NULL REFERENCES mailboxes(id),
    project_id uuid NOT NULL REFERENCES projects(id),
    instance_id uuid NOT NULL REFERENCES agent_instances(id),
    body_id uuid NOT NULL,
    producer_kind text NOT NULL CHECK (
        producer_kind IN ('user', 'gateway', 'repository', 'system', 'external')
    ),
    producer_id text NOT NULL CHECK (length(producer_id) BETWEEN 1 AND 256),
    deduplication_scope text NOT NULL CHECK (length(deduplication_scope) BETWEEN 1 AND 128),
    deduplication_key text NOT NULL CHECK (length(deduplication_key) BETWEEN 1 AND 256),
    method text NOT NULL CHECK (method ~ '^[A-Z]{1,16}$'),
    route text NOT NULL CHECK (length(route) BETWEEN 1 AND 2048),
    selected_headers jsonb NOT NULL DEFAULT '{}'::jsonb,
    content_type text CHECK (length(content_type) BETWEEN 1 AND 256),
    received_at timestamptz NOT NULL,
    trace_context text CHECK (trace_context IS NULL OR length(trace_context) BETWEEN 1 AND 512),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (mailbox_id, deduplication_scope, deduplication_key),
    UNIQUE (id, mailbox_id),
    FOREIGN KEY (mailbox_id, project_id) REFERENCES mailboxes(id, project_id),
    FOREIGN KEY (mailbox_id, instance_id) REFERENCES mailboxes(id, instance_id),
    FOREIGN KEY (body_id, mailbox_id) REFERENCES mailbox_payloads(id, mailbox_id),
    CHECK (jsonb_typeof(selected_headers) = 'object'),
    -- The domain permits 32 values of 1 KiB each. JSON escaping can double
    -- a value containing quotes or backslashes, so retain a conservative 68
    -- KiB database bound rather than rejecting a domain-valid envelope.
    CHECK (octet_length(selected_headers::text) <= 69632)
);
CREATE INDEX mailbox_events_pending_order
    ON mailbox_events (mailbox_id, received_at, id);
CREATE INDEX mailbox_events_by_project ON mailbox_events (project_id, received_at DESC, id);

CREATE TABLE mailbox_deliveries (
    event_id uuid PRIMARY KEY REFERENCES mailbox_events(id),
    mailbox_id uuid NOT NULL REFERENCES mailboxes(id),
    project_id uuid NOT NULL REFERENCES projects(id),
    instance_id uuid NOT NULL REFERENCES agent_instances(id),
    disposition text NOT NULL CHECK (disposition IN (
        'pending', 'eligible', 'leased', 'running', 'delivered', 'retryable',
        'denied', 'dead_lettered', 'cancelled'
    )),
    logical_attempt_count integer NOT NULL DEFAULT 0
        CHECK (logical_attempt_count BETWEEN 0 AND 100),
    next_eligible_at timestamptz,
    dispatch_sequence bigint CHECK (dispatch_sequence > 0),
    denial_code text CHECK (denial_code IS NULL OR denial_code ~ '^[a-z][a-z0-9_]{0,63}$'),
    terminal_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (event_id, mailbox_id) REFERENCES mailbox_events(id, mailbox_id),
    FOREIGN KEY (mailbox_id, project_id) REFERENCES mailboxes(id, project_id),
    FOREIGN KEY (mailbox_id, instance_id) REFERENCES mailboxes(id, instance_id),
    CHECK (
        (disposition IN ('delivered', 'denied', 'dead_lettered', 'cancelled'))
        = (terminal_at IS NOT NULL)
    ),
    CHECK ((disposition = 'retryable') = (next_eligible_at IS NOT NULL)),
    CHECK ((disposition = 'denied') = (denial_code IS NOT NULL))
);
CREATE INDEX mailbox_deliveries_eligible
    ON mailbox_deliveries (next_eligible_at, event_id)
    WHERE disposition IN ('pending', 'eligible', 'retryable');
CREATE UNIQUE INDEX mailbox_deliveries_dispatch_sequence
    ON mailbox_deliveries (instance_id, dispatch_sequence)
    WHERE dispatch_sequence IS NOT NULL;

CREATE TABLE mailbox_delivery_attempts (
    id uuid PRIMARY KEY,
    event_id uuid NOT NULL REFERENCES mailbox_events(id),
    mailbox_id uuid NOT NULL REFERENCES mailboxes(id),
    attempt_number integer NOT NULL CHECK (attempt_number BETWEEN 1 AND 100),
    state text NOT NULL CHECK (state IN ('leased', 'running', 'completed', 'failed', 'uncertain')),
    command_id uuid NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    UNIQUE (event_id, attempt_number),
    FOREIGN KEY (event_id, mailbox_id) REFERENCES mailbox_events(id, mailbox_id),
    CHECK ((state IN ('completed', 'failed', 'uncertain')) = (completed_at IS NOT NULL))
);
CREATE INDEX mailbox_delivery_attempts_by_mailbox
    ON mailbox_delivery_attempts (mailbox_id, created_at DESC, id);

-- Acceptance has one atomic durable effect: the immutable event, its initial
-- delivery record, and a versioned wake-up command.  Repeated publication
-- with the same declared deduplication key hits the event uniqueness boundary
-- and therefore cannot create another logical wake-up command.
CREATE FUNCTION initialize_mailbox_event_delivery() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
BEGIN
    INSERT INTO mailbox_deliveries (
        event_id, mailbox_id, project_id, instance_id, disposition
    ) VALUES (
        NEW.id, NEW.mailbox_id, NEW.project_id, NEW.instance_id, 'pending'
    );
    INSERT INTO outbox (
        id, aggregate_type, aggregate_id, subject, event_type, payload,
        occurred_at
    ) VALUES (
        NEW.id, 'mailbox_event', NEW.id, 'heph.mailbox.v1.wake',
        'mailbox.wake.v1', jsonb_build_object(
            'schema_version', 1,
            'command_kind', 'wake',
            'operation_id', NEW.id,
            'mailbox_event_id', NEW.id,
            'mailbox_body_id', NEW.body_id
        ), NEW.received_at
    );
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION initialize_mailbox_event_delivery() FROM PUBLIC;
CREATE TRIGGER mailbox_events_initialize_delivery
AFTER INSERT ON mailbox_events
FOR EACH ROW EXECUTE FUNCTION initialize_mailbox_event_delivery();

CREATE FUNCTION reject_mailbox_immutable_record_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'accepted mailbox records are immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
CREATE TRIGGER mailbox_payloads_immutable
BEFORE UPDATE OR DELETE ON mailbox_payloads
FOR EACH ROW EXECUTE FUNCTION reject_mailbox_immutable_record_mutation();
CREATE TRIGGER mailbox_events_immutable
BEFORE UPDATE OR DELETE ON mailbox_events
FOR EACH ROW EXECUTE FUNCTION reject_mailbox_immutable_record_mutation();

-- Mailboxes remain tombstone-safe.  A removed instance can retain its
-- immutable accepted work and audit provenance, but no new acceptance occurs.
CREATE FUNCTION enforce_mailbox_event_acceptance() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    mailbox mailboxes%ROWTYPE;
BEGIN
    SELECT * INTO mailbox FROM mailboxes WHERE id = NEW.mailbox_id;
    IF mailbox.id IS NULL OR mailbox.state <> 'active'
       OR NEW.project_id <> mailbox.project_id
       OR NEW.instance_id <> mailbox.instance_id THEN
        RAISE EXCEPTION 'mailbox is unavailable for event acceptance'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_mailbox_event_acceptance() FROM PUBLIC;
CREATE TRIGGER mailbox_events_acceptance_integrity
BEFORE INSERT ON mailbox_events
FOR EACH ROW EXECUTE FUNCTION enforce_mailbox_event_acceptance();

-- RLS routes each user action through the parent instance relation.  The
-- concrete mailbox object is owned by exactly one instance, so this preserves
-- publish/inspect/retry/recover boundaries without creating a second, parallel
-- authorization object model.  Workers perform consume and recovery state
-- transitions under their dedicated database role.
ALTER TABLE mailboxes ENABLE ROW LEVEL SECURITY;
ALTER TABLE mailboxes FORCE ROW LEVEL SECURITY;
CREATE POLICY mailboxes_user_inspect ON mailboxes FOR SELECT TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'agent_instance', instance_id::text) = 1);
CREATE POLICY mailboxes_worker ON mailboxes TO hephaestus_worker
    USING (true) WITH CHECK (true);

ALTER TABLE mailbox_payloads ENABLE ROW LEVEL SECURITY;
ALTER TABLE mailbox_payloads FORCE ROW LEVEL SECURITY;
CREATE POLICY mailbox_payloads_user_inspect ON mailbox_payloads FOR SELECT TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'agent_instance', (SELECT instance_id::text FROM mailboxes WHERE id = mailbox_id)) = 1);
CREATE POLICY mailbox_payloads_worker ON mailbox_payloads TO hephaestus_worker
    USING (true) WITH CHECK (true);

ALTER TABLE mailbox_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE mailbox_events FORCE ROW LEVEL SECURITY;
CREATE POLICY mailbox_events_user_inspect ON mailbox_events FOR SELECT TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'agent_instance', instance_id::text) = 1);
CREATE POLICY mailbox_events_user_publish ON mailbox_events FOR INSERT TO hephaestus_app
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_execute',
        'agent_instance', instance_id::text) = 1);
CREATE POLICY mailbox_events_worker ON mailbox_events TO hephaestus_worker
    USING (true) WITH CHECK (true);

ALTER TABLE mailbox_deliveries ENABLE ROW LEVEL SECURITY;
ALTER TABLE mailbox_deliveries FORCE ROW LEVEL SECURITY;
CREATE POLICY mailbox_deliveries_user_inspect ON mailbox_deliveries FOR SELECT TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'agent_instance', instance_id::text) = 1);
CREATE POLICY mailbox_deliveries_user_recover ON mailbox_deliveries FOR UPDATE TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_recover',
        'agent_instance', instance_id::text) = 1)
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_recover',
        'agent_instance', instance_id::text) = 1);
CREATE POLICY mailbox_deliveries_worker ON mailbox_deliveries TO hephaestus_worker
    USING (true) WITH CHECK (true);

ALTER TABLE mailbox_delivery_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE mailbox_delivery_attempts FORCE ROW LEVEL SECURITY;
CREATE POLICY mailbox_delivery_attempts_user_inspect ON mailbox_delivery_attempts FOR SELECT TO hephaestus_app
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'agent_instance', (SELECT instance_id::text FROM mailboxes WHERE id = mailbox_id)) = 1);
CREATE POLICY mailbox_delivery_attempts_worker ON mailbox_delivery_attempts TO hephaestus_worker
    USING (true) WITH CHECK (true);

GRANT SELECT ON mailboxes, mailbox_payloads, mailbox_events, mailbox_deliveries,
    mailbox_delivery_attempts TO hephaestus_app, hephaestus_worker;
GRANT INSERT ON mailbox_events TO hephaestus_app;
GRANT INSERT, UPDATE ON mailboxes, mailbox_payloads, mailbox_events,
    mailbox_deliveries, mailbox_delivery_attempts TO hephaestus_worker;
