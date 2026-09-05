-- Repository image changes only invalidate the project image view. The
-- authoritative, RLS-filtered snapshot remains the ProjectService query.
CREATE FUNCTION capture_repository_oci_image_application_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    occurrence uuid := COALESCE(
        NULLIF(current_setting('hephaestus.request_id', true), '')::uuid,
        gen_random_uuid()
    );
    project uuid := COALESCE(NEW.project_id, OLD.project_id);
    image uuid := COALESCE(NEW.id, OLD.id);
    change text;
    state text;
BEGIN
    change := CASE
        WHEN TG_OP = 'INSERT' THEN 'created'
        WHEN OLD.status IS DISTINCT FROM NEW.status THEN 'state_changed'
        ELSE 'updated'
    END;
    state := CASE COALESCE(NEW.status, OLD.status)
        WHEN 'ready' THEN 'published'
        WHEN 'failed' THEN 'failed'
        WHEN 'retired' THEN 'removed'
        ELSE 'queued'
    END;
    PERFORM append_application_event(
        occurrence, 'project', project, 'project', project,
        'project.changed', change, state, image, NULL
    );
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION capture_repository_oci_image_application_event() FROM PUBLIC;

CREATE TRIGGER repository_oci_image_product_event
AFTER INSERT OR UPDATE OF status ON repository_oci_image_definitions
FOR EACH ROW EXECUTE FUNCTION capture_repository_oci_image_application_event();
