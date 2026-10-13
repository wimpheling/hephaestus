-- Metadata-only creation lineage. No historical row is adopted or backfilled.
ALTER TABLE agent_instance_state_volumes
    ADD COLUMN creation_seal_id uuid,
    ADD COLUMN creation_operation_id uuid,
    ADD CONSTRAINT volume_creation_pair CHECK ((creation_seal_id IS NULL) = (creation_operation_id IS NULL));

CREATE TABLE volume_creation_seals (
    id uuid PRIMARY KEY CHECK (id <> '00000000-0000-0000-0000-000000000000'),
    volume_id uuid NOT NULL UNIQUE,
    project_id uuid NOT NULL,
    filesystem_uuid uuid NOT NULL,
    capacity_bytes bigint NOT NULL,
    scope_id uuid NOT NULL CHECK (scope_id <> '00000000-0000-0000-0000-000000000000'),
    operation_id uuid NOT NULL UNIQUE CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'),
    command_id uuid NOT NULL CHECK (command_id <> '00000000-0000-0000-0000-000000000000'),
    effect_id uuid NOT NULL CHECK (effect_id <> '00000000-0000-0000-0000-000000000000'),
    attempt_id uuid NOT NULL CHECK (attempt_id <> '00000000-0000-0000-0000-000000000000'),
    actor_id uuid NOT NULL REFERENCES users(id),
    origin_request_id uuid NOT NULL CHECK (origin_request_id <> '00000000-0000-0000-0000-000000000000'),
    resource_version bigint NOT NULL CHECK (resource_version > 0),
    generation bigint NOT NULL CHECK (generation > 0),
    input_hash bytea NOT NULL CHECK (octet_length(input_hash)=32),
    intent_hash bytea NOT NULL CHECK (octet_length(intent_hash)=32),
    canonical_bytes bytea NOT NULL,
    registration_hash bytea NOT NULL CHECK (registration_hash=sha256(canonical_bytes)),
    first_request_id uuid NOT NULL CHECK (first_request_id <> '00000000-0000-0000-0000-000000000000'),
    event_id uuid NOT NULL REFERENCES application_events(id),
    event_cursor bigint NOT NULL CHECK (event_cursor>0),
    event_aggregate_version bigint NOT NULL CHECK (event_aggregate_version>0),
    UNIQUE(id,volume_id,project_id,operation_id),
    FOREIGN KEY(volume_id,project_id) REFERENCES agent_instance_state_volumes(id,project_id) DEFERRABLE INITIALLY DEFERRED,
    CHECK(canonical_bytes=convert_to('heph-owned-volume-registration-v1','UTF8') || decode('00','hex')
      || uuid_send(volume_id) || uuid_send(project_id) || uuid_send(filesystem_uuid)
      || uuid_send(scope_id) || uuid_send(operation_id) || uuid_send(command_id) || uuid_send(effect_id)
      || uuid_send(attempt_id) || uuid_send(actor_id) || uuid_send(origin_request_id)
      || int8send(capacity_bytes) || int8send(resource_version) || int8send(generation) || input_hash || intent_hash)
);
ALTER TABLE agent_instance_state_volumes ADD CONSTRAINT volume_birth_seal
    FOREIGN KEY(creation_seal_id,id,project_id,creation_operation_id)
    REFERENCES volume_creation_seals(id,volume_id,project_id,operation_id) DEFERRABLE INITIALLY DEFERRED;

CREATE FUNCTION guard_volume_creation_birth() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
 IF TG_OP='UPDATE' THEN
   IF NEW.creation_seal_id IS DISTINCT FROM OLD.creation_seal_id
      OR NEW.creation_operation_id IS DISTINCT FROM OLD.creation_operation_id THEN
     RAISE EXCEPTION 'volume birth seal is immutable; independent metadata cannot be adopted' USING ERRCODE='integrity_constraint_violation';
   END IF;
 ELSIF NEW.creation_seal_id IS NOT NULL THEN
   -- SECURITY DEFINER changes current_user; inspect the active caller role.
   IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app' OR NEW.instance_id IS NOT NULL
      OR NEW.creation_operation_id::text IS DISTINCT FROM current_setting('hephaestus.occurrence_id',true)
      OR check_permission('user',hephaestus_actor_id(),'can_manage','project',NEW.project_id::text)<>1
      OR EXISTS(SELECT 1 FROM application_events WHERE occurrence_id=NEW.creation_operation_id) THEN
     RAISE EXCEPTION 'sealed metadata requires a fresh authorized creation occurrence' USING ERRCODE='integrity_constraint_violation';
   END IF;
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_volume_creation_birth() FROM PUBLIC;
CREATE TRIGGER volume_creation_birth BEFORE INSERT OR UPDATE ON agent_instance_state_volumes
FOR EACH ROW EXECUTE FUNCTION guard_volume_creation_birth();

CREATE FUNCTION guard_volume_creation_seal() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
 IF TG_OP<>'INSERT' THEN
   RAISE EXCEPTION 'volume creation lineage is append-only' USING ERRCODE='integrity_constraint_violation';
 END IF;
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app'
    OR NEW.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
    OR NEW.first_request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
    OR check_permission('user',hephaestus_actor_id(),'can_manage','project',NEW.project_id::text)<>1
    OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes volume WHERE volume.id=NEW.volume_id
       AND volume.project_id=NEW.project_id AND volume.instance_id IS NULL
       AND volume.creation_seal_id=NEW.id AND volume.creation_operation_id=NEW.operation_id
       AND volume.capacity_bytes=NEW.capacity_bytes AND volume.filesystem_uuid=NEW.filesystem_uuid)
    OR NOT EXISTS(SELECT 1 FROM application_events event WHERE event.id=NEW.event_id
       AND event.occurrence_id=NEW.operation_id AND event.scope_kind='project' AND event.scope_id=NEW.project_id
       AND event.aggregate_type='project' AND event.aggregate_id=NEW.project_id
       AND event.event_type='project.changed' AND event.change_kind='created'
       AND event.actor_id=NEW.actor_id AND event.request_id=NEW.first_request_id
       AND event.cursor=NEW.event_cursor AND event.aggregate_version=NEW.event_aggregate_version)
    OR NOT EXISTS(SELECT 1 FROM authorization_audit_events audit WHERE audit.actor_id=NEW.actor_id
       AND audit.request_id=NEW.first_request_id AND audit.object_type='project' AND audit.object_id=NEW.project_id
       AND audit.permission='can_manage' AND audit.decision='allow') THEN
   RAISE EXCEPTION 'seal requires exact original metadata, live actor, audit, and committed creation event' USING ERRCODE='integrity_constraint_violation';
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION guard_volume_creation_seal() FROM PUBLIC;
CREATE TRIGGER volume_creation_seal_guard BEFORE INSERT OR UPDATE OR DELETE ON volume_creation_seals
FOR EACH ROW EXECUTE FUNCTION guard_volume_creation_seal();
ALTER TABLE volume_creation_seals ENABLE ROW LEVEL SECURITY;
ALTER TABLE volume_creation_seals FORCE ROW LEVEL SECURITY;
CREATE POLICY volume_creation_seal_read ON volume_creation_seals FOR SELECT TO hephaestus_app
USING(check_permission('user',hephaestus_actor_id(),'can_manage','state_volume',volume_id::text)=1
   OR check_permission('user',hephaestus_actor_id(),'can_manage','project',project_id::text)=1);
CREATE POLICY volume_creation_seal_insert ON volume_creation_seals FOR INSERT TO hephaestus_app
WITH CHECK(actor_id::text=hephaestus_actor_id()
 AND first_request_id::text=current_setting('hephaestus.request_id',true)
 AND check_permission('user',hephaestus_actor_id(),'can_manage','project',project_id::text)=1);
CREATE POLICY volume_creation_seal_worker_read ON volume_creation_seals FOR SELECT TO hephaestus_worker USING(true);
GRANT SELECT,INSERT ON volume_creation_seals TO hephaestus_app;
GRANT SELECT ON volume_creation_seals TO hephaestus_worker;
