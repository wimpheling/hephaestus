-- Durable ownership and fencing for long-lived gateway service launch attempts.
-- A non-cleaned row reserves one exact gateway/revision pair until its
-- provider and materializer resources have been cleaned successfully.
CREATE TABLE gateway_service_instances (
    id uuid PRIMARY KEY,
    gateway_id uuid NOT NULL REFERENCES gateways(id),
    revision_id uuid NOT NULL,
    owner_host_id text NOT NULL,
    owner_uuid uuid NOT NULL,
    fencing_token bigint NOT NULL CHECK (fencing_token > 0),
    vm_id text NOT NULL,
    state text NOT NULL CHECK (
        state IN ('provisioning', 'starting', 'ready', 'draining',
                  'stopping', 'failed', 'cleaned')
    ),
    lease_expires_at timestamptz NOT NULL,
    heartbeat_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    cleaned_at timestamptz,
    FOREIGN KEY (revision_id, gateway_id)
        REFERENCES gateway_revisions(id, gateway_id),
    CHECK (octet_length(owner_host_id) BETWEEN 1 AND 200),
    CHECK (owner_host_id !~ '[[:space:][:cntrl:]]'),
    CHECK (owner_uuid <> '00000000-0000-0000-0000-000000000000'),
    CHECK (vm_id = 'gateway-service-' || id::text),
    CHECK (lease_expires_at > heartbeat_at),
    CHECK ((state = 'cleaned') = (cleaned_at IS NOT NULL))
);

CREATE UNIQUE INDEX gateway_service_instances_live_pair
    ON gateway_service_instances (gateway_id, revision_id)
    WHERE state <> 'cleaned';

CREATE INDEX gateway_service_instances_expired_by_host
    ON gateway_service_instances (owner_host_id, lease_expires_at, gateway_id, id)
    WHERE state <> 'cleaned';

CREATE FUNCTION enforce_gateway_service_instance_revision() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM gateway_revisions revision
         WHERE revision.id = NEW.revision_id
           AND revision.gateway_id = NEW.gateway_id
           AND revision.handler_contract = 'http.service.v1'
    ) THEN
        RAISE EXCEPTION 'gateway service instance requires an HTTP service revision'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_gateway_service_instance_revision() FROM PUBLIC;
CREATE TRIGGER gateway_service_instance_revision_check
BEFORE INSERT OR UPDATE OF gateway_id, revision_id ON gateway_service_instances
FOR EACH ROW EXECUTE FUNCTION enforce_gateway_service_instance_revision();

CREATE FUNCTION enforce_gateway_service_instance_ownership() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public AS $$
BEGIN
    IF NEW.id <> OLD.id
       OR NEW.gateway_id <> OLD.gateway_id
       OR NEW.revision_id <> OLD.revision_id
       OR NEW.owner_host_id <> OLD.owner_host_id
       OR NEW.vm_id <> OLD.vm_id
       OR NEW.created_at <> OLD.created_at THEN
        RAISE EXCEPTION 'gateway service instance identity is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NEW.state = 'cleaned' AND OLD.state = 'cleaned' THEN
        RAISE EXCEPTION 'cleaned gateway service instance is terminal'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NEW.owner_uuid <> OLD.owner_uuid THEN
        IF OLD.state = 'cleaned'
           OR OLD.lease_expires_at > clock_timestamp()
           OR NEW.fencing_token <> OLD.fencing_token + 1
           OR NEW.state <> 'stopping' THEN
            RAISE EXCEPTION 'gateway service ownership fencing transition is invalid'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    ELSIF NEW.fencing_token <> OLD.fencing_token THEN
        IF OLD.state = 'cleaned'
           OR OLD.lease_expires_at > clock_timestamp()
           OR NEW.fencing_token <> OLD.fencing_token + 1
           OR NEW.state <> 'stopping' THEN
            RAISE EXCEPTION 'gateway service ownership fencing transition is invalid'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
    END IF;
    IF NEW.heartbeat_at < OLD.heartbeat_at
       OR NEW.lease_expires_at < OLD.lease_expires_at THEN
        RAISE EXCEPTION 'gateway service lease timestamps cannot move backward'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NOT (
        NEW.state = OLD.state
        OR (OLD.state = 'provisioning' AND NEW.state IN ('starting', 'failed', 'stopping'))
        OR (OLD.state = 'starting' AND NEW.state IN ('ready', 'failed', 'stopping'))
        OR (OLD.state = 'ready' AND NEW.state IN ('draining', 'failed', 'stopping'))
        OR (OLD.state = 'draining' AND NEW.state IN ('stopping', 'failed'))
        OR (OLD.state = 'stopping' AND NEW.state = 'cleaned')
        OR (OLD.state = 'failed' AND NEW.state = 'stopping')
    ) THEN
        RAISE EXCEPTION 'gateway service lifecycle transition is invalid'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_gateway_service_instance_ownership() FROM PUBLIC;
CREATE TRIGGER gateway_service_instance_ownership_check
BEFORE UPDATE ON gateway_service_instances
FOR EACH ROW EXECUTE FUNCTION enforce_gateway_service_instance_ownership();

ALTER TABLE gateway_service_instances ENABLE ROW LEVEL SECURITY;
ALTER TABLE gateway_service_instances FORCE ROW LEVEL SECURITY;
CREATE POLICY gateway_service_instances_read ON gateway_service_instances
    FOR SELECT TO hephaestus_app
    USING (EXISTS (
        SELECT 1
          FROM gateways gateway
         WHERE gateway.id = gateway_service_instances.gateway_id
           AND check_permission(
               'user', hephaestus_actor_id(), 'can_read',
               'project', gateway.project_id::text
           ) = 1
    ));
CREATE POLICY gateway_service_instances_worker ON gateway_service_instances
    TO hephaestus_worker USING (true) WITH CHECK (true);

GRANT SELECT ON gateway_service_instances TO hephaestus_app;
GRANT SELECT, INSERT, UPDATE ON gateway_service_instances TO hephaestus_worker;
GRANT EXECUTE ON FUNCTION enforce_gateway_service_instance_revision() TO hephaestus_worker;
GRANT EXECUTE ON FUNCTION enforce_gateway_service_instance_ownership() TO hephaestus_worker;
