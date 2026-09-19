-- Bind every newly accepted persistent-service invocation to the exact live
-- instance and fencing epoch that admitted it.  Existing terminal history may
-- predate this binding and is intentionally left readable.
ALTER TABLE gateway_service_instances
    ADD CONSTRAINT gateway_service_instances_id_gateway_revision_unique
        UNIQUE (id, gateway_id, revision_id);

ALTER TABLE gateway_invocations
    ADD COLUMN service_instance_id uuid,
    ADD COLUMN service_instance_fencing_token bigint,
    ADD CONSTRAINT gateway_invocations_service_binding_shape_check
        CHECK (
            (service_instance_id IS NULL) =
                (service_instance_fencing_token IS NULL)
            AND (
                service_instance_fencing_token IS NULL
                OR service_instance_fencing_token > 0
            )
        ),
    ADD CONSTRAINT gateway_invocations_service_instance_fk
        FOREIGN KEY (service_instance_id, gateway_id, gateway_revision_id)
        REFERENCES gateway_service_instances (id, gateway_id, revision_id);

-- Invocations accepted before this binding existed cannot be safely routed to
-- a warm instance.  Close them through the existing terminal transition so
-- their host sessions and secret leases are revoked while retaining history.
DO $$
DECLARE invocation_id uuid;
BEGIN
    FOR invocation_id IN
        SELECT invocation.id
          FROM gateway_invocations AS invocation
          JOIN gateway_revisions AS revision
            ON revision.id = invocation.gateway_revision_id
           AND revision.gateway_id = invocation.gateway_id
         WHERE invocation.outcome = 'accepted'
           AND revision.handler_contract = 'http.service.v1'
           AND invocation.service_instance_id IS NULL
    LOOP
        PERFORM gateway_invocation_complete(invocation_id, 'timed_out');
    END LOOP;
END
$$;

CREATE FUNCTION enforce_gateway_invocation_service_binding() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public AS $$
DECLARE handler_contract text;
DECLARE current_fencing_token bigint;
BEGIN
    SELECT revision.handler_contract
      INTO handler_contract
      FROM gateway_revisions AS revision
     WHERE revision.id = NEW.gateway_revision_id
       AND revision.gateway_id = NEW.gateway_id;

    IF handler_contract = 'http.service.v1'
       AND NEW.outcome = 'accepted'
       AND (NEW.service_instance_id IS NULL
            OR NEW.service_instance_fencing_token IS NULL) THEN
        RAISE EXCEPTION
            'accepted persistent-service invocation requires an instance binding'
            USING ERRCODE = 'check_violation';
    END IF;
    IF handler_contract = 'http.service.v1'
       AND NEW.outcome = 'accepted'
       AND NEW.service_instance_id IS NOT NULL THEN
        SELECT instance.fencing_token
          INTO current_fencing_token
          FROM gateway_service_instances AS instance
         WHERE instance.id = NEW.service_instance_id
           AND instance.gateway_id = NEW.gateway_id
           AND instance.revision_id = NEW.gateway_revision_id;
        IF current_fencing_token IS DISTINCT FROM
           NEW.service_instance_fencing_token THEN
            RAISE EXCEPTION
                'gateway invocation service fencing token is stale'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    END IF;
    IF handler_contract IS DISTINCT FROM 'http.service.v1'
       AND (NEW.service_instance_id IS NOT NULL
            OR NEW.service_instance_fencing_token IS NOT NULL) THEN
        RAISE EXCEPTION
            'stateless invocation cannot carry a service instance binding'
            USING ERRCODE = 'check_violation';
    END IF;

    IF TG_OP = 'UPDATE'
       AND (OLD.service_instance_id IS DISTINCT FROM NEW.service_instance_id
            OR OLD.service_instance_fencing_token IS DISTINCT FROM
               NEW.service_instance_fencing_token) THEN
        RAISE EXCEPTION 'gateway invocation service target is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_gateway_invocation_service_binding() FROM PUBLIC;
CREATE TRIGGER gateway_invocations_service_binding_check
BEFORE INSERT OR UPDATE OF service_instance_id, service_instance_fencing_token
ON gateway_invocations
FOR EACH ROW EXECUTE FUNCTION enforce_gateway_invocation_service_binding();

GRANT EXECUTE ON FUNCTION enforce_gateway_invocation_service_binding() TO hephaestus_worker;
