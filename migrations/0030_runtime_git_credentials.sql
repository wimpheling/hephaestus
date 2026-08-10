-- Hash-only, non-renewable Git credentials bound to exact runtime sessions and
-- immutable dispatch-time Git snapshots.

CREATE TABLE runtime_git_credentials (
    runtime_session_id uuid PRIMARY KEY,
    authorization_snapshot_id uuid NOT NULL,
    binding_id uuid NOT NULL,
    repository_id uuid NOT NULL,
    scope_hash bytea NOT NULL CHECK (octet_length(scope_hash) = 32),
    issuance_generation bigint NOT NULL CHECK (issuance_generation > 0),
    credential_hash bytea NOT NULL UNIQUE CHECK (octet_length(credential_hash) = 32),
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (runtime_session_id) REFERENCES runtime_authority_sessions(id),
    FOREIGN KEY (authorization_snapshot_id) REFERENCES run_git_authority_snapshots(snapshot_id),
    FOREIGN KEY (authorization_snapshot_id, binding_id)
        REFERENCES run_authorization_snapshot_bindings(snapshot_id, binding_id),
    FOREIGN KEY (repository_id) REFERENCES repositories(id)
);

CREATE FUNCTION enforce_runtime_git_credential_binding() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    session runtime_authority_sessions%ROWTYPE;
    git_snapshot run_git_authority_snapshots%ROWTYPE;
BEGIN
    SELECT * INTO session FROM runtime_authority_sessions
    WHERE id = NEW.runtime_session_id;
    SELECT * INTO git_snapshot FROM run_git_authority_snapshots
    WHERE snapshot_id = NEW.authorization_snapshot_id;

    IF session.id IS NULL OR git_snapshot.snapshot_id IS NULL
       OR session.status <> 'pending_handoff'
       OR NEW.authorization_snapshot_id <> session.snapshot_id
       OR NEW.binding_id <> git_snapshot.binding_id
       OR NEW.repository_id <> git_snapshot.repository_id
       OR NEW.scope_hash <> git_snapshot.normalized_hash
       OR NEW.issuance_generation <> session.issuance_generation
       OR NEW.expires_at <> session.expires_at
    THEN
        RAISE EXCEPTION 'runtime Git credential does not match its exact durable session and scope'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_runtime_git_credential_binding() FROM PUBLIC;
CREATE TRIGGER runtime_git_credentials_exact_binding
BEFORE INSERT ON runtime_git_credentials
FOR EACH ROW EXECUTE FUNCTION enforce_runtime_git_credential_binding();
CREATE TRIGGER runtime_git_credentials_immutable
BEFORE UPDATE OR DELETE ON runtime_git_credentials
FOR EACH ROW EXECUTE FUNCTION reject_runtime_authority_immutable_record();

GRANT SELECT, INSERT ON runtime_git_credentials TO hephaestus_worker;
ALTER TABLE runtime_git_credentials ENABLE ROW LEVEL SECURITY;
ALTER TABLE runtime_git_credentials FORCE ROW LEVEL SECURITY;
CREATE POLICY runtime_git_credentials_worker
    ON runtime_git_credentials TO hephaestus_worker
    USING (true) WITH CHECK (true);

-- The application role can verify only one presented credential against one
-- exact repository operation. It cannot list verifier material.
CREATE FUNCTION authenticate_runtime_git_credential(
    p_credential_hash bytea,
    p_repository_id uuid,
    p_operation text
)
RETURNS TABLE (
    runtime_session_id uuid,
    authorization_snapshot_id uuid,
    binding_id uuid,
    repository_id uuid,
    scope_hash bytea,
    issuance_generation bigint,
    expires_at timestamptz,
    grammar_version smallint,
    git_operations text[],
    ref_globs text[],
    changed_path_globs text[],
    branch_update_policy text,
    branch_create boolean,
    branch_delete boolean,
    tag_create boolean,
    tag_update boolean,
    tag_delete boolean,
    other_create boolean,
    other_update boolean,
    other_delete boolean,
    request_bytes bigint,
    pack_bytes bigint,
    object_count integer,
    ref_updates integer,
    expected_parent text,
    evaluated_at timestamptz
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, public
STABLE
AS $$
    SELECT credential.runtime_session_id,
           credential.authorization_snapshot_id,
           credential.binding_id,
           credential.repository_id,
           credential.scope_hash,
           credential.issuance_generation,
           credential.expires_at,
           git_snapshot.grammar_version,
           git_snapshot.git_operations,
           git_snapshot.ref_globs,
           git_snapshot.changed_path_globs,
           git_snapshot.branch_update_policy,
           git_snapshot.branch_create,
           git_snapshot.branch_delete,
           git_snapshot.tag_create,
           git_snapshot.tag_update,
           git_snapshot.tag_delete,
           git_snapshot.other_create,
           git_snapshot.other_update,
           git_snapshot.other_delete,
           git_snapshot.request_bytes,
           git_snapshot.pack_bytes,
           git_snapshot.object_count,
           git_snapshot.ref_updates,
           git_snapshot.expected_parent,
           now()
    FROM runtime_git_credentials AS credential
    JOIN runtime_authority_sessions AS session
      ON session.id = credential.runtime_session_id
     AND session.snapshot_id = credential.authorization_snapshot_id
    JOIN run_git_authority_snapshots AS git_snapshot
      ON git_snapshot.snapshot_id = credential.authorization_snapshot_id
     AND git_snapshot.binding_id = credential.binding_id
     AND git_snapshot.repository_id = credential.repository_id
     AND git_snapshot.normalized_hash = credential.scope_hash
    JOIN run_authorization_snapshot_bindings AS generic_binding
      ON generic_binding.snapshot_id = credential.authorization_snapshot_id
     AND generic_binding.binding_id = credential.binding_id
     AND generic_binding.resource_kind = 'repository'
     AND generic_binding.resource_id = credential.repository_id
    WHERE credential.credential_hash = p_credential_hash
      AND credential.repository_id = p_repository_id
      AND p_operation = ANY(git_snapshot.git_operations)
      AND session.status = 'active'
      AND session.expires_at = credential.expires_at
      AND session.expires_at > now()
      AND NOT EXISTS (
          SELECT 1
          FROM unnest(generic_binding.granted_operations) AS granted(operation)
          WHERE check_permission(
              'agent_instance', session.instance_id::text,
              'agent_' || granted.operation,
              'repository', credential.repository_id::text
          ) <> 1
      )
$$;
REVOKE ALL ON FUNCTION authenticate_runtime_git_credential(bytea, uuid, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION authenticate_runtime_git_credential(bytea, uuid, text)
    TO hephaestus_app, hephaestus_worker;

COMMENT ON TABLE runtime_git_credentials IS
    'One immutable hash-only Git bearer verifier per exact generic runtime session.';
