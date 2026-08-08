-- Repository attachments are trigger-routing records, not repository
-- capability bindings. Preserve that distinction in durable provenance: an
-- attachment's repository/ref identity cannot be retargeted, and every copied
-- trigger repository must match the exact attachment that accepted it.

ALTER TABLE agent_attachments
    ADD CONSTRAINT agent_attachments_id_instance_repository_unique
    UNIQUE (id, instance_id, repository_id);

CREATE FUNCTION reject_attachment_routing_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.id <> NEW.id
       OR OLD.instance_id <> NEW.instance_id
       OR OLD.project_id <> NEW.project_id
       OR OLD.repository_id <> NEW.repository_id
       OR OLD.ref_selector <> NEW.ref_selector
       OR OLD.trigger_policy <> NEW.trigger_policy
       OR OLD.created_by IS DISTINCT FROM NEW.created_by
       OR OLD.created_at <> NEW.created_at
    THEN
        RAISE EXCEPTION 'attachment trigger routing is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER agent_attachments_routing_immutable
BEFORE UPDATE ON agent_attachments
FOR EACH ROW EXECUTE FUNCTION reject_attachment_routing_mutation();

ALTER TABLE run_requests
    ADD CONSTRAINT run_requests_exact_trigger_attachment
    FOREIGN KEY (attachment_id, instance_id, repository_id)
    REFERENCES agent_attachments(id, instance_id, repository_id);

ALTER TABLE deferred_agent_triggers
    ADD CONSTRAINT deferred_triggers_exact_trigger_attachment
    FOREIGN KEY (attachment_id, instance_id, repository_id)
    REFERENCES agent_attachments(id, instance_id, repository_id);

ALTER TABLE run_instance_provenance
    ADD CONSTRAINT run_provenance_exact_trigger_attachment
    FOREIGN KEY (attachment_id, instance_id, target_repository_id)
    REFERENCES agent_attachments(id, instance_id, repository_id);

COMMENT ON COLUMN agent_attachments.repository_id IS
    'Trigger-routing repository only; this column grants no runtime repository authority.';
COMMENT ON COLUMN run_requests.repository_id IS
    'Exact triggering repository provenance; runtime authority comes only from the revision capability snapshot.';
COMMENT ON COLUMN run_instance_provenance.target_repository_id IS
    'Exact triggering repository provenance; this column is not a capability binding.';
