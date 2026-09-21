-- Preserve the verified runtime session and originating attachment for Git
-- receives. Trigger routing uses this provenance to suppress only the
-- attachment that produced the runtime push.

ALTER TABLE git_receives
    ADD COLUMN runtime_session_id uuid,
    ADD COLUMN runtime_attachment_id uuid;

ALTER TABLE git_receives
    ADD CONSTRAINT git_receives_runtime_provenance_shape CHECK (
        runtime_attachment_id IS NULL OR runtime_session_id IS NOT NULL
    ),
    ADD CONSTRAINT git_receives_runtime_session_fk
        FOREIGN KEY (runtime_session_id)
        REFERENCES runtime_authority_sessions(id),
    ADD CONSTRAINT git_receives_runtime_attachment_fk
        FOREIGN KEY (runtime_attachment_id)
        REFERENCES agent_attachments(id);

-- The application role may resolve only the optional attachment bound to one
-- exact runtime session, run, instance, revision, authorization snapshot, and
-- Git repository capability. It cannot enumerate runtime sessions. The
-- lookup intentionally does not require a live status: Git authentication
-- already checked it before the atomic receive, and accepted publication must
-- remain durable if cleanup revokes the session before post-receive capture.
CREATE FUNCTION resolve_runtime_receive_attachment(
    p_runtime_session_id uuid,
    p_repository_id uuid
)
RETURNS TABLE (attachment_id uuid)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, public
STABLE
AS $$
    SELECT session.attachment_id
    FROM runtime_authority_sessions AS session
    JOIN run_authorization_snapshots AS auth_snapshot
      ON auth_snapshot.id = session.snapshot_id
     AND auth_snapshot.run_id = session.run_id
     AND auth_snapshot.instance_id = session.instance_id
     AND auth_snapshot.instance_revision_id = session.instance_revision_id
    JOIN run_git_authority_snapshots AS git
      ON git.snapshot_id = session.snapshot_id
     AND git.instance_revision_id = session.instance_revision_id
     AND git.repository_id = p_repository_id
    LEFT JOIN run_instance_provenance AS provenance
      ON provenance.run_id = session.run_id
     AND provenance.instance_id = session.instance_id
     AND provenance.instance_revision_id = session.instance_revision_id
     AND provenance.attachment_id = session.attachment_id
    LEFT JOIN agent_attachments AS attachment
      ON attachment.id = session.attachment_id
     AND attachment.instance_id = session.instance_id
    WHERE session.id = p_runtime_session_id
      AND (
          session.attachment_id IS NULL
          OR (
              provenance.run_id IS NOT NULL
              AND attachment.id IS NOT NULL
          )
      )
$$;

REVOKE ALL ON FUNCTION resolve_runtime_receive_attachment(uuid, uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION resolve_runtime_receive_attachment(uuid, uuid)
    TO hephaestus_app, hephaestus_worker;
