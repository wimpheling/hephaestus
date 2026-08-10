-- Gateway mutations invalidate project-scoped management views through the
-- existing durable product-event outbox. The event is intentionally only an
-- invalidation: bodies, headers, provider configuration, and secret material
-- remain behind the authorized gateway snapshot RPC.
ALTER TABLE application_events DROP CONSTRAINT application_events_aggregate_type_check;
ALTER TABLE application_events ADD CONSTRAINT application_events_aggregate_type_check CHECK (
    aggregate_type IN ('identity_profile', 'identity_organizations', 'organization', 'project', 'repository',
        'repository_ref', 'build', 'release', 'agent_instance', 'run', 'review', 'secret_metadata',
        'secret_grant', 'secret_import', 'agent_secret_binding', 'artifact', 'registry_publication', 'gateway')
);

ALTER TABLE application_events DROP CONSTRAINT application_events_event_type_check;
ALTER TABLE application_events ADD CONSTRAINT application_events_event_type_check CHECK (
    (aggregate_type, event_type) IN (
        ('identity_profile', 'identity.profile_changed'), ('identity_organizations', 'identity.organizations_changed'),
        ('organization', 'organization.changed'), ('project', 'project.changed'), ('repository', 'repository.changed'),
        ('repository_ref', 'repository.ref_changed'), ('build', 'build.changed'), ('release', 'release.changed'),
        ('agent_instance', 'agent_instance.changed'), ('run', 'run.changed'), ('review', 'review.changed'),
        ('secret_metadata', 'secret_metadata.changed'), ('secret_grant', 'secret_grant.changed'),
        ('secret_import', 'secret_import.changed'), ('agent_secret_binding', 'agent_secret_binding.changed'),
        ('artifact', 'artifact.changed'), ('registry_publication', 'registry.publication_changed'),
        ('gateway', 'gateway.changed')
    )
);

CREATE FUNCTION capture_gateway_application_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    occurrence uuid := COALESCE(
        NULLIF(current_setting('hephaestus.request_id', true), '')::uuid,
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

CREATE TRIGGER gateway_product_event
AFTER INSERT OR UPDATE OF lifecycle, active_revision_id ON gateways
FOR EACH ROW EXECUTE FUNCTION capture_gateway_application_event();
