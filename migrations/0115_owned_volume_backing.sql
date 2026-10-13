-- Sealed backing metadata only. No migration creates physical ownership proof.
-- Exact FK requires a candidate key without weakening the frozen 110 identity.
ALTER TABLE volume_creation_seals ADD CONSTRAINT volume_creation_seal_backing_key UNIQUE(id,volume_id,project_id);
CREATE TABLE volume_owned_backing_purposes (
 volume_id uuid PRIMARY KEY REFERENCES agent_instance_state_volumes(id),
 seal_id uuid NOT NULL UNIQUE REFERENCES volume_creation_seals(id),
 project_id uuid NOT NULL REFERENCES projects(id),
 host_id text NOT NULL CHECK(octet_length(host_id) BETWEEN 1 AND 128),
 root_path text NOT NULL CHECK(octet_length(root_path) BETWEEN 1 AND 4096 AND left(root_path,1)='/'),
 owner_namespace uuid NOT NULL CHECK(owner_namespace<>'00000000-0000-0000-0000-000000000000'),
 namespace_name text NOT NULL,
 canonical_path text NOT NULL,
 birth_generation bigint NOT NULL CHECK(birth_generation=1),
 canonical_bytes bytea NOT NULL,
 purpose_hash bytea NOT NULL CHECK(purpose_hash=sha256(canonical_bytes)),
 UNIQUE(volume_id,purpose_hash),
 UNIQUE(volume_id,seal_id,project_id),
 FOREIGN KEY(seal_id,volume_id,project_id) REFERENCES volume_creation_seals(id,volume_id,project_id)
);
CREATE TABLE volume_owned_provisioning_operations (
 id uuid PRIMARY KEY CHECK(id<>'00000000-0000-0000-0000-000000000000'),
 volume_id uuid NOT NULL REFERENCES volume_owned_backing_purposes(volume_id),
 purpose_hash bytea NOT NULL,
 expected_generation bigint NOT NULL CHECK(expected_generation>=0),
 generation bigint NOT NULL CHECK(generation=expected_generation+1),
 actor_id uuid NOT NULL REFERENCES users(id),
 request_id uuid NOT NULL CHECK(request_id<>'00000000-0000-0000-0000-000000000000'),
 event_id uuid NOT NULL REFERENCES application_events(id),
 UNIQUE(volume_id,generation), UNIQUE(id,volume_id,generation),
 FOREIGN KEY(volume_id,purpose_hash) REFERENCES volume_owned_backing_purposes(volume_id,purpose_hash)
);
CREATE TABLE volume_owned_backing_observations (
 id uuid PRIMARY KEY CHECK(id<>'00000000-0000-0000-0000-000000000000'),
 operation_id uuid NOT NULL,
 volume_id uuid NOT NULL,
 generation bigint NOT NULL,
 birth_observation_id uuid NOT NULL,
 phase text NOT NULL CHECK(phase IN ('recorded','format_intent','failed_before_format','ready','uncertain')),
 proof_json jsonb NOT NULL CHECK(jsonb_typeof(proof_json)='object'),
 evidence_hash bytea NOT NULL CHECK(evidence_hash=sha256(convert_to(proof_json::text,'UTF8'))),
 event_id uuid NOT NULL REFERENCES application_events(id),
 created_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(id,volume_id),
 FOREIGN KEY(operation_id,volume_id,generation) REFERENCES volume_owned_provisioning_operations(id,volume_id,generation),
 FOREIGN KEY(birth_observation_id,volume_id) REFERENCES volume_owned_backing_observations(id,volume_id) DEFERRABLE INITIALLY DEFERRED,
 CHECK(proof_json->>'id'=id::text AND (proof_json->>'generation')::bigint=generation AND proof_json->>'phase'=phase)
);
CREATE TABLE volume_owned_transition_proofs (
 id uuid PRIMARY KEY,
 volume_id uuid NOT NULL REFERENCES volume_owned_backing_purposes(volume_id),
 operation_id uuid NOT NULL,
 generation bigint NOT NULL CHECK(generation>0),
 previous_generation bigint NOT NULL CHECK(previous_generation>=0),
 mode text NOT NULL CHECK(mode IN ('claim','recorded','format_intent','failed_before_format','ready','uncertain')),
 transaction_id bigint NOT NULL DEFAULT txid_current(),
 created_at timestamptz NOT NULL DEFAULT now(),
 FOREIGN KEY(operation_id,volume_id,generation) REFERENCES volume_owned_provisioning_operations(id,volume_id,generation) DEFERRABLE INITIALLY DEFERRED
);

CREATE FUNCTION owned_backing_actor_transition(old_volume agent_instance_state_volumes,new_volume agent_instance_state_volumes) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public AS $$
 SELECT old_volume.creation_seal_id IS NOT NULL AND EXISTS(
  SELECT 1 FROM volume_owned_transition_proofs proof JOIN volume_owned_backing_purposes purpose USING(volume_id)
  WHERE proof.volume_id=old_volume.id AND proof.transaction_id=txid_current() AND proof.mode='claim'
    AND proof.previous_generation=old_volume.provisioning_generation
    AND proof.generation=new_volume.provisioning_generation
    AND proof.generation=old_volume.provisioning_generation+1
    AND new_volume.host_id=purpose.host_id AND new_volume.host_path=purpose.canonical_path
    AND new_volume.provisioning_state=old_volume.provisioning_state AND new_volume.state=old_volume.state)
$$;
REVOKE ALL ON FUNCTION owned_backing_actor_transition(agent_instance_state_volumes,agent_instance_state_volumes) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION owned_backing_actor_transition(agent_instance_state_volumes,agent_instance_state_volumes) TO hephaestus_app,hephaestus_worker;

CREATE FUNCTION guard_owned_backing_purpose() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE seal volume_creation_seals%ROWTYPE; volume agent_instance_state_volumes%ROWTYPE; expected bytea;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'owned backing purpose is immutable' USING ERRCODE='integrity_constraint_violation'; END IF;
 SELECT * INTO seal FROM volume_creation_seals WHERE id=NEW.seal_id AND volume_id=NEW.volume_id AND project_id=NEW.project_id;
 SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=NEW.volume_id;
 IF seal.id IS NULL OR COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app'
  OR seal.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
  OR check_permission('user',hephaestus_actor_id(),'can_manage','project',NEW.project_id::text)<>1
  OR volume.creation_seal_id IS DISTINCT FROM seal.id OR volume.host_id IS NOT NULL
  OR volume.provisioning_generation<>0 OR volume.provisioning_state<>'reserved' OR volume.state<>'uninitialized'
  OR NEW.root_path ~ '(^|/)\.\.?(/|$)|//'
  OR NEW.root_path ~ '[[:cntrl:]]' OR NEW.host_id ~ '[[:cntrl:]]' OR (NEW.root_path<>'/' AND right(NEW.root_path,1)='/')
  OR NEW.namespace_name<>'.owned-'||NEW.volume_id::text||'-'||NEW.seal_id::text
  OR NEW.canonical_path<>rtrim(NEW.root_path,'/')||'/'||NEW.volume_id::text||'.raw' THEN
  RAISE EXCEPTION 'backing purpose needs original sealed actor, new resource, exact provider namespace' USING ERRCODE='integrity_constraint_violation';
 END IF;
 expected:=convert_to('heph-owned-backing-purpose-v1','UTF8')||decode('00','hex')
  ||int2send(octet_length(seal.canonical_bytes)::smallint)||seal.canonical_bytes
  ||uuid_send(seal.id)||seal.registration_hash||uuid_send(seal.first_request_id)||uuid_send(seal.event_id)
  ||int8send(seal.event_cursor)||int8send(seal.event_aggregate_version)
  ||int2send(octet_length(convert_to(NEW.host_id,'UTF8'))::smallint)||convert_to(NEW.host_id,'UTF8')
  ||int2send(octet_length(convert_to(NEW.root_path,'UTF8'))::smallint)||convert_to(NEW.root_path,'UTF8')
  ||uuid_send(NEW.owner_namespace)||int8send(NEW.birth_generation);
 IF NEW.canonical_bytes<>expected THEN RAISE EXCEPTION 'backing purpose differs from original full sealed receipt' USING ERRCODE='integrity_constraint_violation'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER owned_backing_purpose_guard BEFORE INSERT OR UPDATE OR DELETE ON volume_owned_backing_purposes FOR EACH ROW EXECUTE FUNCTION guard_owned_backing_purpose();

CREATE FUNCTION owned_backing_identity(proof jsonb) RETURNS jsonb
LANGUAGE sql IMMUTABLE SET search_path=pg_catalog,public AS $$
 SELECT jsonb_build_array(proof->'root_device',proof->'root_inode',proof->'namespace_device',proof->'namespace_inode',proof->'owner_uid',proof->'backing_device',proof->'backing_inode',proof->'birth_record_hash',proof->'inode_record_hash')
$$;
CREATE FUNCTION guard_owned_transition_proof() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE volume agent_instance_state_volumes%ROWTYPE; purpose volume_owned_backing_purposes%ROWTYPE; seal volume_creation_seals%ROWTYPE; caller text;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'owned transition proofs are append-only' USING ERRCODE='integrity_constraint_violation'; END IF;
 SELECT * INTO volume FROM agent_instance_state_volumes WHERE id=NEW.volume_id;
 SELECT * INTO purpose FROM volume_owned_backing_purposes WHERE volume_id=NEW.volume_id;
 SELECT * INTO seal FROM volume_creation_seals WHERE id=volume.creation_seal_id;
 caller:=COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text);
 IF purpose.volume_id IS NULL OR NEW.transaction_id<>txid_current() OR volume.state<>'uninitialized'
  OR NEW.previous_generation<>volume.provisioning_generation THEN
  RAISE EXCEPTION 'owned transition targets a different current resource fence' USING ERRCODE='integrity_constraint_violation';
 END IF;
 IF NEW.mode='claim' THEN
  IF caller<>'hephaestus_app' OR seal.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
   OR check_permission('user',hephaestus_actor_id(),'can_manage','project',volume.project_id::text)<>1
   OR NEW.generation<>volume.provisioning_generation+1
   OR EXISTS(SELECT 1 FROM volume_owned_provisioning_operations operation WHERE operation.volume_id=NEW.volume_id
       AND NOT EXISTS(SELECT 1 FROM volume_owned_backing_observations observation WHERE observation.operation_id=operation.id AND observation.phase='failed_before_format')) THEN
   RAISE EXCEPTION 'original live actor and closed failed-before-format lineage required for new operation' USING ERRCODE='integrity_constraint_violation';
  END IF;
 ELSE
  IF caller<>'hephaestus_worker' OR NEW.generation<>volume.provisioning_generation
   OR NOT EXISTS(SELECT 1 FROM volume_owned_provisioning_operations operation WHERE operation.id=NEW.operation_id AND operation.volume_id=NEW.volume_id AND operation.generation=NEW.generation)
   OR EXISTS(SELECT 1 FROM volume_owned_backing_observations observation WHERE observation.operation_id=NEW.operation_id AND observation.phase IN ('failed_before_format','ready')) THEN
   RAISE EXCEPTION 'worker observation requires the exact active original operation' USING ERRCODE='integrity_constraint_violation';
  END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER owned_transition_proof_guard BEFORE INSERT OR UPDATE OR DELETE ON volume_owned_transition_proofs FOR EACH ROW EXECUTE FUNCTION guard_owned_transition_proof();

CREATE FUNCTION guard_owned_volume_change() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE proof volume_owned_transition_proofs%ROWTYPE; purpose volume_owned_backing_purposes%ROWTYPE;
BEGIN
 IF OLD.creation_seal_id IS NULL THEN RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END; END IF;
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'sealed volume lineage requires retained tombstones' USING ERRCODE='integrity_constraint_violation'; END IF;
 IF OLD.state<>'uninitialized' AND NEW.state='uninitialized' THEN RAISE EXCEPTION 'owned backing cannot return to creation' USING ERRCODE='integrity_constraint_violation'; END IF;
 IF (NEW.host_id,NEW.host_path,NEW.provisioning_generation,NEW.provisioning_state) IS NOT DISTINCT FROM
    (OLD.host_id,OLD.host_path,OLD.provisioning_generation,OLD.provisioning_state)
    AND NOT(OLD.state='uninitialized' AND NEW.state<>'uninitialized') THEN RETURN NEW; END IF;
 SELECT * INTO purpose FROM volume_owned_backing_purposes WHERE volume_id=OLD.id;
 SELECT * INTO proof FROM volume_owned_transition_proofs WHERE volume_id=OLD.id AND transaction_id=txid_current()
   AND previous_generation=OLD.provisioning_generation AND generation=NEW.provisioning_generation
   AND id::text=current_setting('hephaestus.occurrence_id',true);
 IF proof.id IS NULL OR NEW.host_id IS DISTINCT FROM purpose.host_id OR NEW.host_path IS DISTINCT FROM purpose.canonical_path
  OR NEW.filesystem_uuid IS DISTINCT FROM OLD.filesystem_uuid OR NEW.capacity_bytes IS DISTINCT FROM OLD.capacity_bytes
  OR (OLD.host_id IS NOT NULL AND (NEW.host_id,NEW.host_path) IS DISTINCT FROM (OLD.host_id,OLD.host_path)) THEN
  RAISE EXCEPTION 'sealed resource mutation requires exact same-transaction owned proof' USING ERRCODE='integrity_constraint_violation';
 END IF;
 IF proof.mode='claim' THEN
  IF NOT owned_backing_actor_transition(OLD,NEW) THEN RAISE EXCEPTION 'invalid owned actor claim' USING ERRCODE='integrity_constraint_violation'; END IF;
 ELSE
  IF NEW.provisioning_generation<>OLD.provisioning_generation OR NEW.provisioning_state<>(CASE proof.mode
      WHEN 'recorded' THEN 'creating' WHEN 'format_intent' THEN 'formatting' WHEN 'ready' THEN 'ready' ELSE 'uncertain' END)
    OR NEW.state<>(CASE WHEN proof.mode='ready' THEN 'ready' ELSE 'uninitialized' END) THEN
   RAISE EXCEPTION 'owned observation does not match metadata phase' USING ERRCODE='integrity_constraint_violation';
  END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER owned_backing_change_guard BEFORE UPDATE OR DELETE ON agent_instance_state_volumes FOR EACH ROW EXECUTE FUNCTION guard_owned_volume_change();
CREATE FUNCTION guard_owned_operation() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE purpose volume_owned_backing_purposes%ROWTYPE;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'owned operations are append-only' USING ERRCODE='integrity_constraint_violation'; END IF;
 SELECT * INTO purpose FROM volume_owned_backing_purposes WHERE volume_id=NEW.volume_id;
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_app'
  OR NEW.actor_id::text IS DISTINCT FROM hephaestus_actor_id()
  OR NEW.request_id::text IS DISTINCT FROM current_setting('hephaestus.request_id',true)
  OR NEW.purpose_hash<>purpose.purpose_hash
  OR NOT EXISTS(SELECT 1 FROM volume_owned_transition_proofs proof WHERE proof.id=NEW.id AND proof.operation_id=NEW.id AND proof.volume_id=NEW.volume_id AND proof.mode='claim' AND proof.generation=NEW.generation AND proof.previous_generation=NEW.expected_generation AND proof.transaction_id=txid_current())
  OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes volume WHERE volume.id=NEW.volume_id AND volume.provisioning_generation=NEW.generation AND volume.host_id=purpose.host_id AND volume.host_path=purpose.canonical_path)
  OR NOT EXISTS(SELECT 1 FROM application_events event WHERE event.id=NEW.event_id AND event.occurrence_id=NEW.id AND event.scope_kind='project' AND event.scope_id=purpose.project_id AND event.aggregate_type='project' AND event.aggregate_id=purpose.project_id AND event.event_type='project.changed' AND event.change_kind='updated' AND event.actor_id=NEW.actor_id AND event.request_id=NEW.request_id)
  OR NOT EXISTS(SELECT 1 FROM authorization_audit_events audit WHERE audit.actor_id=NEW.actor_id AND audit.request_id=NEW.request_id AND audit.object_type='project' AND audit.object_id=purpose.project_id AND audit.permission='can_manage' AND audit.decision='allow') THEN
  RAISE EXCEPTION 'owned operation requires exact actor audit, transition and committed event' USING ERRCODE='integrity_constraint_violation';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER owned_operation_guard BEFORE INSERT OR UPDATE OR DELETE ON volume_owned_provisioning_operations FOR EACH ROW EXECUTE FUNCTION guard_owned_operation();

CREATE FUNCTION guard_owned_observation() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
DECLARE purpose volume_owned_backing_purposes%ROWTYPE; seal volume_creation_seals%ROWTYPE; original volume_owned_backing_observations%ROWTYPE; proof jsonb; filesystem jsonb; expected_state text;
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'owned observations are append-only' USING ERRCODE='integrity_constraint_violation'; END IF;
 SELECT * INTO purpose FROM volume_owned_backing_purposes WHERE volume_id=NEW.volume_id;
 SELECT * INTO seal FROM volume_creation_seals WHERE id=purpose.seal_id;
 SELECT * INTO original FROM volume_owned_backing_observations WHERE volume_id=NEW.volume_id AND birth_observation_id=id LIMIT 1;
 proof:=NEW.proof_json; filesystem:=proof->'filesystem';
 IF NOT (proof ?& ARRAY['id','purpose_hash','generation','phase','root_device','root_inode','namespace_device','namespace_inode','owner_uid','backing_device','backing_inode','birth_record_hash','inode_record_hash','journal_hash','never_format_started','capacity_bytes','filesystem'])
  OR (SELECT count(*) FROM jsonb_object_keys(proof))<>17
  OR jsonb_typeof(proof->'never_format_started')<>'boolean'
  OR jsonb_typeof(proof->'purpose_hash')<>'array' OR jsonb_typeof(proof->'birth_record_hash')<>'array'
  OR jsonb_typeof(proof->'inode_record_hash')<>'array' OR jsonb_typeof(proof->'journal_hash')<>'array'
  OR EXISTS(SELECT 1 FROM jsonb_array_elements((proof->'birth_record_hash')||(proof->'inode_record_hash')||(proof->'journal_hash')) byte WHERE jsonb_typeof(byte)<>'number' OR byte::text !~ '^[0-9]+$' OR byte::text::integer NOT BETWEEN 0 AND 255)
  OR EXISTS(SELECT 1 FROM unnest(ARRAY['generation','root_device','root_inode','namespace_device','namespace_inode','owner_uid','backing_device','backing_inode','capacity_bytes']) field WHERE jsonb_typeof(proof->field)<>'number' OR proof->>field !~ '^[0-9]+$') THEN
  RAISE EXCEPTION 'owned backing observation has malformed bounded wire evidence' USING ERRCODE='integrity_constraint_violation';
 END IF;
 expected_state:=CASE NEW.phase WHEN 'recorded' THEN 'creating' WHEN 'format_intent' THEN 'formatting' WHEN 'ready' THEN 'ready' ELSE 'uncertain' END;
 IF COALESCE(NULLIF(current_setting('role',true),'none'),session_user::text)<>'hephaestus_worker'
  OR NOT EXISTS(SELECT 1 FROM volume_owned_transition_proofs transition WHERE transition.id=NEW.id AND transition.operation_id=NEW.operation_id AND transition.volume_id=NEW.volume_id AND transition.generation=NEW.generation AND transition.mode=NEW.phase AND transition.transaction_id=txid_current())
  OR NOT EXISTS(SELECT 1 FROM agent_instance_state_volumes volume WHERE volume.id=NEW.volume_id AND volume.provisioning_generation=NEW.generation AND volume.provisioning_state=expected_state AND volume.host_id=purpose.host_id AND volume.host_path=purpose.canonical_path)
  OR NOT EXISTS(SELECT 1 FROM application_events event WHERE event.id=NEW.event_id AND event.occurrence_id=NEW.id AND event.scope_kind='project' AND event.scope_id=purpose.project_id AND event.aggregate_type='project' AND event.aggregate_id=purpose.project_id AND event.event_type='project.changed')
  OR proof->'purpose_hash'<>to_jsonb(ARRAY(SELECT get_byte(purpose.purpose_hash,n) FROM generate_series(0,31) n))
  OR (proof->>'capacity_bytes')::bigint<>seal.capacity_bytes
  OR (proof->>'root_inode')::bigint<=0 OR (proof->>'namespace_inode')::bigint<=0 OR (proof->>'backing_inode')::bigint<=0
  OR (proof->>'root_device')::bigint<0 OR (proof->>'namespace_device')::bigint<0 OR (proof->>'backing_device')::bigint<0
  OR (proof->>'owner_uid')::bigint NOT BETWEEN 0 AND 4294967295
  OR jsonb_array_length(proof->'birth_record_hash')<>32 OR jsonb_array_length(proof->'inode_record_hash')<>32 OR jsonb_array_length(proof->'journal_hash')<>32
  OR ((NEW.phase IN ('recorded','failed_before_format')) IS DISTINCT FROM (proof->>'never_format_started')::boolean) THEN
  RAISE EXCEPTION 'owned worker observation differs from exact current purpose or proof' USING ERRCODE='integrity_constraint_violation';
 END IF;
 IF original.id IS NULL THEN
  IF NEW.birth_observation_id<>NEW.id THEN RAISE EXCEPTION 'first observation must establish its own exact journal birth' USING ERRCODE='integrity_constraint_violation'; END IF;
 ELSE
  IF NEW.birth_observation_id<>original.id OR owned_backing_identity(proof)<>owned_backing_identity(original.proof_json) THEN
   RAISE EXCEPTION 'physical root namespace or recorded backing inode differs from original birth' USING ERRCODE='integrity_constraint_violation';
  END IF;
 END IF;
 IF NEW.phase IN ('recorded','failed_before_format') AND EXISTS(SELECT 1 FROM volume_owned_backing_observations observation WHERE observation.volume_id=NEW.volume_id AND observation.phase IN ('format_intent','ready')) THEN
  RAISE EXCEPTION 'format intent can never become never-formatted proof' USING ERRCODE='integrity_constraint_violation';
 END IF;
 IF NEW.phase='ready' THEN
  IF jsonb_typeof(filesystem)<>'object' OR NOT(filesystem ?& ARRAY['filesystem_uuid','block_size','block_count','clean']) OR (SELECT count(*) FROM jsonb_object_keys(filesystem))<>4 OR jsonb_typeof(filesystem->'clean')<>'boolean' OR NOT COALESCE((filesystem->>'clean')::boolean,false)
    OR jsonb_typeof(filesystem->'filesystem_uuid')<>'string' OR jsonb_typeof(filesystem->'block_size')<>'number' OR jsonb_typeof(filesystem->'block_count')<>'number'
    OR filesystem->>'block_count' !~ '^[1-9][0-9]*$'
    OR filesystem->>'filesystem_uuid'<>seal.filesystem_uuid::text
    OR (filesystem->>'block_size')::bigint<>4096
    OR (filesystem->>'block_count')::numeric*4096<>seal.capacity_bytes THEN
   RAISE EXCEPTION 'Ready requires exact initial clean filesystem birth geometry' USING ERRCODE='integrity_constraint_violation';
  END IF;
 ELSIF filesystem IS DISTINCT FROM 'null'::jsonb THEN
  RAISE EXCEPTION 'non-Ready observation cannot claim filesystem birth' USING ERRCODE='integrity_constraint_violation';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER owned_observation_guard BEFORE INSERT OR UPDATE OR DELETE ON volume_owned_backing_observations FOR EACH ROW EXECUTE FUNCTION guard_owned_observation();

CREATE FUNCTION verify_owned_transition_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
 IF NEW.mode='claim' THEN
  IF NOT EXISTS(SELECT 1 FROM volume_owned_provisioning_operations operation WHERE operation.id=NEW.id AND operation.volume_id=NEW.volume_id AND operation.generation=NEW.generation) THEN
   RAISE EXCEPTION 'owned claim mutation lacks its committed operation receipt' USING ERRCODE='integrity_constraint_violation';
  END IF;
 ELSE
  IF NOT EXISTS(SELECT 1 FROM volume_owned_backing_observations observation WHERE observation.id=NEW.id AND observation.operation_id=NEW.operation_id AND observation.volume_id=NEW.volume_id AND observation.phase=NEW.mode AND observation.generation=NEW.generation) THEN
   RAISE EXCEPTION 'owned worker mutation lacks its committed exact observation' USING ERRCODE='integrity_constraint_violation';
  END IF;
 END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER owned_transition_commit AFTER INSERT ON volume_owned_transition_proofs DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_owned_transition_commit();

-- Preserve every 103 rule. Only its actor handle/fence restriction gains this
-- exact same-transaction sealed-claim exception; standalone generic claims fail.
CREATE OR REPLACE FUNCTION enforce_private_volume_intent() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF NEW.instance_id IS NULL AND
           (NEW.filesystem_uuid IS NULL OR NEW.filesystem_uuid = '00000000-0000-0000-0000-000000000000'::uuid
            OR NEW.capacity_bytes < 16777216 OR NEW.capacity_bytes % 4096 <> 0) THEN
            RAISE EXCEPTION 'standalone volume needs a valid filesystem UUID and aligned provider capacity'
                USING ERRCODE = 'integrity_constraint_violation';
        END IF;
        IF current_user = 'hephaestus_app' AND
           (NEW.host_id IS NOT NULL OR NEW.provisioning_state <> 'reserved'
            OR NEW.provisioning_generation <> 0 OR NEW.state <> 'uninitialized') THEN
            RAISE EXCEPTION 'actor registration cannot assign provider handles or provisioning progress'
                USING ERRCODE = 'insufficient_privilege';
        END IF;
        RETURN NEW;
    END IF;
    IF (OLD.instance_id IS NULL OR OLD.host_id IS NOT NULL) AND
       (NEW.capacity_bytes IS DISTINCT FROM OLD.capacity_bytes
        OR NEW.filesystem_uuid IS DISTINCT FROM OLD.filesystem_uuid) THEN
        RAISE EXCEPTION 'reserved volume capacity and filesystem UUID are immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.filesystem_uuid IS NOT NULL AND NEW.filesystem_uuid IS DISTINCT FROM OLD.filesystem_uuid THEN
        RAISE EXCEPTION 'filesystem UUID cannot change after reservation'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.host_id IS NOT NULL AND
       (NEW.host_id IS DISTINCT FROM OLD.host_id OR NEW.host_path IS DISTINCT FROM OLD.host_path) THEN
        RAISE EXCEPTION 'reserved provider handles are immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF current_user = 'hephaestus_app' AND
       NOT owned_backing_actor_transition(OLD,NEW) AND
       (NEW.host_id IS DISTINCT FROM OLD.host_id OR NEW.host_path IS DISTINCT FROM OLD.host_path
        OR NEW.provisioning_state IS DISTINCT FROM OLD.provisioning_state
        OR NEW.provisioning_generation IS DISTINCT FROM OLD.provisioning_generation
        OR NEW.capacity_bytes IS DISTINCT FROM OLD.capacity_bytes
        OR NEW.filesystem_uuid IS DISTINCT FROM OLD.filesystem_uuid) THEN
        RAISE EXCEPTION 'provider reservation and progress require a trusted worker'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    IF NEW.provisioning_generation <> OLD.provisioning_generation
       AND (NEW.provisioning_generation <> OLD.provisioning_generation + 1
            OR NEW.provisioning_state <> OLD.provisioning_state
            OR OLD.provisioning_state = 'ready') THEN
        RAISE EXCEPTION 'invalid provisioning claim generation'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NEW.provisioning_state IS DISTINCT FROM OLD.provisioning_state AND
       (NEW.provisioning_generation <> OLD.provisioning_generation
        OR NOT ((OLD.provisioning_state IN ('reserved','uncertain') AND NEW.provisioning_state IN ('creating','ready','uncertain'))
             OR (OLD.provisioning_state = 'creating' AND NEW.provisioning_state IN ('formatting','ready','uncertain'))
             OR (OLD.provisioning_state = 'formatting' AND NEW.provisioning_state IN ('creating','ready','uncertain')))) THEN
        RAISE EXCEPTION 'invalid provisioning progress transition'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END $$;
ALTER TABLE volume_owned_backing_purposes ENABLE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_backing_purposes FORCE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_provisioning_operations ENABLE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_provisioning_operations FORCE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_backing_observations ENABLE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_backing_observations FORCE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_transition_proofs ENABLE ROW LEVEL SECURITY;
ALTER TABLE volume_owned_transition_proofs FORCE ROW LEVEL SECURITY;
CREATE POLICY owned_purpose_actor_read ON volume_owned_backing_purposes FOR SELECT TO hephaestus_app USING(check_permission('user',hephaestus_actor_id(),'can_manage','project',project_id::text)=1);
CREATE POLICY owned_purpose_actor_insert ON volume_owned_backing_purposes FOR INSERT TO hephaestus_app WITH CHECK(check_permission('user',hephaestus_actor_id(),'can_manage','project',project_id::text)=1);
CREATE POLICY owned_purpose_worker_read ON volume_owned_backing_purposes FOR SELECT TO hephaestus_worker USING(true);
CREATE POLICY owned_operation_actor_read ON volume_owned_provisioning_operations FOR SELECT TO hephaestus_app USING(check_permission('user',hephaestus_actor_id(),'can_manage','state_volume',volume_id::text)=1);
CREATE POLICY owned_operation_actor_insert ON volume_owned_provisioning_operations FOR INSERT TO hephaestus_app WITH CHECK(actor_id::text=hephaestus_actor_id());
CREATE POLICY owned_operation_worker_read ON volume_owned_provisioning_operations FOR SELECT TO hephaestus_worker USING(true);
CREATE POLICY owned_observation_actor_read ON volume_owned_backing_observations FOR SELECT TO hephaestus_app USING(check_permission('user',hephaestus_actor_id(),'can_manage','state_volume',volume_id::text)=1);
CREATE POLICY owned_observation_worker_read ON volume_owned_backing_observations FOR SELECT TO hephaestus_worker USING(true);
CREATE POLICY owned_observation_worker_insert ON volume_owned_backing_observations FOR INSERT TO hephaestus_worker WITH CHECK(true);
CREATE POLICY owned_transition_actor_read ON volume_owned_transition_proofs FOR SELECT TO hephaestus_app USING(check_permission('user',hephaestus_actor_id(),'can_manage','state_volume',volume_id::text)=1);
CREATE POLICY owned_transition_actor_insert ON volume_owned_transition_proofs FOR INSERT TO hephaestus_app WITH CHECK(mode='claim' AND check_permission('user',hephaestus_actor_id(),'can_manage','state_volume',volume_id::text)=1);
CREATE POLICY owned_transition_worker_read ON volume_owned_transition_proofs FOR SELECT TO hephaestus_worker USING(true);
CREATE POLICY owned_transition_worker_insert ON volume_owned_transition_proofs FOR INSERT TO hephaestus_worker WITH CHECK(mode<>'claim');
GRANT SELECT,INSERT ON volume_owned_backing_purposes,volume_owned_provisioning_operations TO hephaestus_app;
GRANT SELECT ON volume_owned_backing_purposes,volume_owned_provisioning_operations TO hephaestus_worker;
GRANT SELECT ON volume_owned_backing_observations TO hephaestus_app;
GRANT SELECT,INSERT ON volume_owned_backing_observations TO hephaestus_worker;
GRANT SELECT,INSERT ON volume_owned_transition_proofs TO hephaestus_app,hephaestus_worker;
REVOKE ALL ON FUNCTION guard_owned_backing_purpose(),guard_owned_transition_proof(),guard_owned_volume_change(),guard_owned_operation(),guard_owned_observation(),verify_owned_transition_commit(),owned_backing_identity(jsonb) FROM PUBLIC;

CREATE FUNCTION verify_owned_purpose_commit() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog,public AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM volume_owned_provisioning_operations operation JOIN agent_instance_state_volumes volume ON volume.id=operation.volume_id
   WHERE operation.volume_id=NEW.volume_id AND operation.expected_generation=0 AND operation.generation=NEW.birth_generation
   AND volume.host_id=NEW.host_id AND volume.host_path=NEW.canonical_path AND volume.creation_seal_id=NEW.seal_id) THEN
  RAISE EXCEPTION 'owned backing purpose lacks atomic first admitted operation' USING ERRCODE='integrity_constraint_violation';
 END IF;
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION verify_owned_purpose_commit() FROM PUBLIC;
CREATE CONSTRAINT TRIGGER owned_purpose_commit AFTER INSERT ON volume_owned_backing_purposes DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_owned_purpose_commit();

-- Unpublished 111 compatibility foundation. This predicate conveys no actor
-- permission or physical proof. 112 replaces it with the permanent birth fence.
CREATE FUNCTION owned_birth_is_live(p_volume uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog,public AS $$ SELECT true $$;
REVOKE ALL ON FUNCTION owned_birth_is_live(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION owned_birth_is_live(uuid) TO hephaestus_app,hephaestus_worker;
