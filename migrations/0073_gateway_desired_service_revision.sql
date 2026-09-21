-- A service revision is declared before it is admitted as the serving
-- revision. Stateless gateway mutations retain their immediate activation
-- semantics; service readiness will promote this candidate later.
ALTER TABLE gateways
    ADD COLUMN desired_service_revision_id uuid;

ALTER TABLE gateways
    ADD CONSTRAINT gateways_desired_service_revision_fk
        FOREIGN KEY (desired_service_revision_id, id)
        REFERENCES gateway_revisions(id, gateway_id)
        DEFERRABLE INITIALLY DEFERRED;

CREATE FUNCTION enforce_gateway_desired_service_revision() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public AS $$
BEGIN
  IF NEW.desired_service_revision_id IS NOT NULL AND NOT EXISTS (
      SELECT 1
        FROM gateway_revisions revision
       WHERE revision.id = NEW.desired_service_revision_id
         AND revision.gateway_id = NEW.id
         AND revision.handler_contract = 'http.service.v1'
  ) THEN
    RAISE EXCEPTION 'gateway desired revision must be an HTTP service revision'
      USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION enforce_gateway_desired_service_revision() FROM PUBLIC;
CREATE TRIGGER gateways_desired_service_revision_check
BEFORE INSERT OR UPDATE OF desired_service_revision_id ON gateways
FOR EACH ROW EXECUTE FUNCTION enforce_gateway_desired_service_revision();

-- Existing service rows have never passed a readiness gate. Preserve their
-- immutable declaration as desired state and fail closed until supervision
-- proves a serving VM is ready.
UPDATE gateways AS gateway
   SET desired_service_revision_id = gateway.active_revision_id,
       active_revision_id = NULL,
       updated_at = now()
 WHERE gateway.active_revision_id IS NOT NULL
   AND EXISTS (
       SELECT 1
         FROM gateway_revisions revision
        WHERE revision.id = gateway.active_revision_id
          AND revision.gateway_id = gateway.id
          AND revision.handler_contract = 'http.service.v1'
   );

CREATE OR REPLACE FUNCTION capture_gateway_application_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    occurrence uuid := COALESCE(
        NULLIF(current_setting('hephaestus.occurrence_id', true), '')::uuid,
        gen_random_uuid()
    );
    change text;
BEGIN
    change := CASE
        WHEN TG_OP = 'INSERT' THEN 'created'
        WHEN OLD.lifecycle IS DISTINCT FROM NEW.lifecycle THEN 'state_changed'
        WHEN OLD.active_revision_id IS DISTINCT FROM NEW.active_revision_id THEN 'updated'
        WHEN OLD.desired_service_revision_id IS DISTINCT FROM NEW.desired_service_revision_id THEN 'updated'
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
DROP TRIGGER IF EXISTS gateway_product_event ON gateways;
CREATE TRIGGER gateway_product_event
AFTER INSERT OR UPDATE OF lifecycle, active_revision_id, desired_service_revision_id ON gateways
FOR EACH ROW EXECUTE FUNCTION capture_gateway_application_event();

GRANT EXECUTE ON FUNCTION enforce_gateway_desired_service_revision() TO hephaestus_worker;
GRANT UPDATE (active_revision_id, desired_service_revision_id, updated_at) ON gateways
    TO hephaestus_worker;
