-- Verification workers do not mutate the immutable build request or its draft
-- release. Their durable result must nevertheless invalidate the authorized
-- build projection so connected clients learn a match or mismatch without
-- polling.
CREATE FUNCTION capture_build_verification_application_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_row jsonb := CASE WHEN TG_OP = 'DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
    v_change text := CASE TG_OP
        WHEN 'INSERT' THEN 'created' WHEN 'UPDATE' THEN 'updated' ELSE 'removed'
    END;
    v_occurrence uuid := COALESCE(
        NULLIF(current_setting('hephaestus.occurrence_id', true), '')::uuid,
        gen_random_uuid()
    );
    v_build uuid := (v_row ->> 'build_request_id')::uuid;
    v_repository uuid;
BEGIN
    IF TG_OP = 'UPDATE'
       AND to_jsonb(OLD) ->> 'state' IS DISTINCT FROM to_jsonb(NEW) ->> 'state' THEN
        v_change := 'state_changed';
    END IF;
    SELECT repository_id INTO v_repository FROM build_requests WHERE id = v_build;
    IF v_repository IS NULL THEN
        RAISE EXCEPTION 'verification references missing build request %', v_build;
    END IF;
    PERFORM append_application_event(
        v_occurrence, 'repository', v_repository, 'build', v_build,
        'build.changed', v_change, NULL, v_repository
    );
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END
$$;
REVOKE ALL ON FUNCTION capture_build_verification_application_event() FROM PUBLIC;

CREATE TRIGGER build_verifications_application_event
AFTER INSERT OR UPDATE OR DELETE ON build_verifications
FOR EACH ROW EXECUTE FUNCTION capture_build_verification_application_event();
