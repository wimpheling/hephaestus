-- Durable, revision-scoped service failure information and retry backoff.
-- Failure details stay bounded and redacted; raw provider logs never enter the
-- gateway database.
ALTER TABLE gateway_service_instances
    ADD COLUMN failure_code text,
    ADD COLUMN failed_at timestamptz,
    ADD COLUMN exit_code integer,
    ADD COLUMN exit_signal integer,
    ADD CONSTRAINT gateway_service_instance_failure_shape_check
        CHECK ((failure_code IS NULL) = (failed_at IS NULL)),
    ADD CONSTRAINT gateway_service_instance_failure_code_check
        CHECK (
            failure_code IS NULL
            OR failure_code IN (
                'preparation', 'startup', 'readiness', 'health',
                'unexpected_exit', 'cleanup'
            )
        ),
    ADD CONSTRAINT gateway_service_instance_exit_shape_check
        CHECK (NOT (exit_code IS NOT NULL AND exit_signal IS NOT NULL)),
    ADD CONSTRAINT gateway_service_instance_exit_bounds_check
        CHECK (
            (exit_code IS NULL OR exit_code BETWEEN 0 AND 255)
            AND (exit_signal IS NULL OR exit_signal BETWEEN 1 AND 255)
        ),
    ADD CONSTRAINT gateway_service_instance_exit_failure_check
        CHECK (
            (failure_code = 'unexpected_exit') IS TRUE
            OR (exit_code IS NULL AND exit_signal IS NULL)
        );

CREATE FUNCTION enforce_gateway_service_instance_failure() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public AS $$
BEGIN
    IF OLD.failure_code IS NOT NULL
       AND (
           NEW.failure_code IS DISTINCT FROM OLD.failure_code
           OR NEW.failed_at IS DISTINCT FROM OLD.failed_at
           OR NEW.exit_code IS DISTINCT FROM OLD.exit_code
           OR NEW.exit_signal IS DISTINCT FROM OLD.exit_signal
       ) THEN
        RAISE EXCEPTION 'gateway service failure metadata is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_gateway_service_instance_failure() FROM PUBLIC;
CREATE TRIGGER gateway_service_instance_failure_check
BEFORE UPDATE OF failure_code, failed_at, exit_code, exit_signal
ON gateway_service_instances
FOR EACH ROW EXECUTE FUNCTION enforce_gateway_service_instance_failure();

CREATE TABLE gateway_service_retry_state (
    gateway_id uuid NOT NULL REFERENCES gateways(id),
    revision_id uuid NOT NULL,
    failure_streak integer NOT NULL DEFAULT 0
        CHECK (failure_streak BETWEEN 0 AND 32),
    next_retry_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (gateway_id, revision_id),
    FOREIGN KEY (revision_id, gateway_id)
        REFERENCES gateway_revisions(id, gateway_id),
    CHECK ((failure_streak = 0) = (next_retry_at IS NULL))
);

CREATE FUNCTION enforce_gateway_service_retry_revision() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM gateway_revisions AS revision
         WHERE revision.id = NEW.revision_id
           AND revision.gateway_id = NEW.gateway_id
           AND revision.handler_contract = 'http.service.v1'
    ) THEN
        RAISE EXCEPTION 'gateway service retry state requires an HTTP service revision'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_gateway_service_retry_revision() FROM PUBLIC;
CREATE TRIGGER gateway_service_retry_revision_check
BEFORE INSERT OR UPDATE OF gateway_id, revision_id ON gateway_service_retry_state
FOR EACH ROW EXECUTE FUNCTION enforce_gateway_service_retry_revision();

ALTER TABLE gateway_service_retry_state ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_service_retry_state FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_service_retry_state_read ON gateway_service_retry_state
    FOR SELECT TO hephaestus_app USING (EXISTS (
        SELECT 1
          FROM gateways AS gateway
         WHERE gateway.id = gateway_service_retry_state.gateway_id
           AND check_permission(
               'user', hephaestus_actor_id(), 'can_read',
               'project', gateway.project_id::text
           ) = 1
    ));
CREATE POLICY gateway_service_retry_state_worker ON gateway_service_retry_state
    TO hephaestus_worker USING (true) WITH CHECK (true);

GRANT SELECT ON gateway_service_retry_state TO hephaestus_app;
GRANT SELECT, INSERT, UPDATE ON gateway_service_retry_state TO hephaestus_worker;
GRANT EXECUTE ON FUNCTION enforce_gateway_service_retry_revision() TO hephaestus_worker;
GRANT EXECUTE ON FUNCTION enforce_gateway_service_instance_failure() TO hephaestus_worker;
