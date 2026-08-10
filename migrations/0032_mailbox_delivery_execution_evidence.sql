-- A mailbox attempt is the durable bridge from accepted work to one normal
-- reusable-instance run. These columns retain exact dispatch evidence, never
-- application payloads or guesses about guest-owned state bytes.

ALTER TABLE run_authorization_snapshots
    ADD CONSTRAINT run_authorization_snapshots_id_run_unique UNIQUE (id, run_id);

ALTER TABLE runs
    ADD COLUMN lease_fencing_token bigint CHECK (lease_fencing_token > 0);

ALTER TABLE mailbox_delivery_attempts
    ADD COLUMN instance_id uuid NOT NULL REFERENCES agent_instances(id),
    ADD COLUMN instance_revision_id uuid NOT NULL,
    ADD COLUMN run_id uuid UNIQUE,
    ADD COLUMN authorization_snapshot_id uuid UNIQUE,
    ADD COLUMN state_volume_id uuid,
    ADD COLUMN lease_id uuid,
    ADD COLUMN lease_fencing_token bigint,
    ADD COLUMN state_access_outcome text NOT NULL CHECK (state_access_outcome IN (
        'no_state', 'completed_access', 'failed_access', 'uncertain_access'
    )),
    ADD CONSTRAINT mailbox_delivery_attempts_revision_fk
        FOREIGN KEY (instance_id, instance_revision_id)
        REFERENCES agent_instance_revisions(instance_id, id),
    ADD CONSTRAINT mailbox_delivery_attempts_run_fk
        FOREIGN KEY (run_id, instance_id, instance_revision_id)
        REFERENCES runs(id, instance_id, instance_revision_id),
    ADD CONSTRAINT mailbox_delivery_attempts_snapshot_fk
        FOREIGN KEY (authorization_snapshot_id, run_id)
        REFERENCES run_authorization_snapshots(id, run_id),
    ADD CONSTRAINT mailbox_delivery_attempts_volume_fk
        FOREIGN KEY (state_volume_id, instance_id)
        REFERENCES agent_instance_state_volumes(id, instance_id),
    ADD CONSTRAINT mailbox_delivery_attempts_lease_fk
        FOREIGN KEY (lease_id) REFERENCES agent_instance_volume_leases(id),
    ADD CONSTRAINT mailbox_delivery_attempts_volume_evidence
        CHECK (
            (state_volume_id IS NULL AND lease_id IS NULL
                AND lease_fencing_token IS NULL)
            OR (state_volume_id IS NOT NULL AND lease_id IS NOT NULL
                AND lease_fencing_token > 0)
        ),
    ADD CONSTRAINT mailbox_delivery_attempts_run_snapshot_evidence
        CHECK (
            (run_id IS NULL AND authorization_snapshot_id IS NULL)
            -- A dispatcher claims the durable run before the runtime-authority
            -- manager can atomically create its snapshot. Only that leased
            -- pre-provisioning state may temporarily lack the snapshot.
            OR (run_id IS NOT NULL AND authorization_snapshot_id IS NOT NULL)
            OR (run_id IS NOT NULL AND authorization_snapshot_id IS NULL
                AND state = 'leased')
        ),
    ADD CONSTRAINT mailbox_delivery_attempts_volume_requires_run
        CHECK (state_volume_id IS NULL OR run_id IS NOT NULL);

CREATE FUNCTION enforce_mailbox_attempt_execution_evidence() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    event_instance_id uuid;
    lease agent_instance_volume_leases%ROWTYPE;
    run_fence bigint;
BEGIN
    SELECT instance_id INTO event_instance_id
    FROM mailbox_events WHERE id = NEW.event_id;
    IF event_instance_id IS NULL OR NEW.instance_id <> event_instance_id THEN
        RAISE EXCEPTION 'mailbox attempt instance must match its accepted event'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NEW.lease_id IS NOT NULL THEN
        SELECT * INTO lease FROM agent_instance_volume_leases WHERE id = NEW.lease_id;
        SELECT lease_fencing_token INTO run_fence FROM runs WHERE id = NEW.run_id;
        IF lease.id IS NULL OR lease.instance_id <> NEW.instance_id
           OR lease.volume_id <> NEW.state_volume_id
           OR lease.fencing_token <> NEW.lease_fencing_token
           OR lease.run_id <> NEW.run_id
           OR run_fence IS DISTINCT FROM NEW.lease_fencing_token THEN
            RAISE EXCEPTION 'mailbox attempt volume evidence is not the exact fenced lease'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_mailbox_attempt_execution_evidence() FROM PUBLIC;
CREATE TRIGGER mailbox_delivery_attempts_exact_execution_evidence
BEFORE INSERT OR UPDATE ON mailbox_delivery_attempts
FOR EACH ROW EXECUTE FUNCTION enforce_mailbox_attempt_execution_evidence();
