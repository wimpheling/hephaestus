-- Redacted operational telemetry and retention/recovery inspection surfaces.
-- Views use opaque identifiers and aggregate lifecycle facts only; parameters,
-- secret aliases, ciphertext, credentials, and paths are deliberately absent.

-- The application role must never receive access to `secret_versions`, because
-- that table contains encrypted material. This deliberately tiny definer view
-- exposes only lifecycle metadata and performs the same live Mélange decision
-- that protects secret metadata everywhere else.
CREATE VIEW secret_version_metadata
WITH (security_barrier = true) AS
SELECT version.id, version.secret_id, version.sequence, version.status,
       version.created_at, version.revoked_at, version.purged_at
FROM secret_versions AS version
WHERE check_permission(
    hephaestus_subject_type(), hephaestus_actor_id(), 'inspect_metadata',
    'secret', version.secret_id::text
) = 1;

CREATE VIEW release_operation_metrics
WITH (security_invoker = true) AS
SELECT
    (SELECT count(*) FROM build_requests
     WHERE state IN ('queued', 'claimed', 'running')) AS builds_queued,
    (SELECT count(*) FROM build_requests
     WHERE state = 'failed') AS builds_failed,
    (SELECT count(*) FROM releases
     WHERE state = 'published') AS releases_available,
    (SELECT count(*) FROM releases
     WHERE state = 'revoked') AS releases_revoked,
    (SELECT COALESCE(sum(size_bytes), 0)::bigint
     FROM release_artifacts) AS retained_artifact_bytes,
    COALESCE(
        (SELECT extract(epoch FROM avg(completed_at - started_at)) * 1000
         FROM build_executions
         WHERE completed_at IS NOT NULL),
        0
    )::bigint AS average_build_execution_ms;

CREATE VIEW instance_operation_metrics
WITH (security_invoker = true) AS
SELECT
    (SELECT count(*) FROM agent_instances
     WHERE state LIKE 'paused%' OR run_gate_open = false) AS paused_instances,
    (SELECT count(*) FROM agent_updates
     WHERE state NOT IN ('activated', 'rejected')) AS updates_in_progress,
    (SELECT count(*) FROM runs
     WHERE state IN ('queued', 'claimed', 'running')) AS runs_queued,
    COALESCE(
        (SELECT extract(epoch FROM avg(updated_at - created_at)) * 1000
         FROM runs
         WHERE state IN ('succeeded', 'failed', 'cancelled', 'cleaned_up')),
        0
    )::bigint AS average_run_execution_ms,
    (SELECT count(*) FROM agent_instance_volume_leases
     WHERE state <> 'released') AS active_volume_leases,
    (SELECT count(*) FROM outbox
     WHERE published_at IS NULL) AS unpublished_outbox_records,
    COALESCE(
        (SELECT extract(epoch FROM max(now() - occurred_at)) * 1000
         FROM outbox
         WHERE published_at IS NULL),
        0
    )::bigint AS maximum_outbox_lag_ms;

CREATE VIEW secret_operation_metrics
WITH (security_barrier = true) AS
WITH visible_secrets AS MATERIALIZED (
    SELECT id
    FROM secrets
    WHERE check_permission(
        hephaestus_subject_type(), hephaestus_actor_id(), 'inspect_metadata',
        'secret', id::text
    ) = 1
)
SELECT
    (SELECT count(*) FROM secrets
     WHERE id IN (SELECT id FROM visible_secrets)
       AND status = 'active') AS active_secrets,
    (SELECT count(*) FROM secret_versions
     WHERE secret_id IN (SELECT id FROM visible_secrets)) AS retained_versions,
    COALESCE(
        (SELECT extract(epoch FROM max(now() - created_at))
         FROM secret_versions
         WHERE secret_id IN (SELECT id FROM visible_secrets)),
        0
    )::bigint AS oldest_version_age_seconds,
    (SELECT count(*) FROM secret_versions
     WHERE secret_id IN (SELECT id FROM visible_secrets)
       AND sequence > 1) AS rotations,
    (SELECT count(*)
     FROM secret_leases AS lease
     JOIN secret_versions AS version ON version.id = lease.secret_version_id
     WHERE version.secret_id IN (SELECT id FROM visible_secrets)
       AND lease.status = 'active'
       AND lease.expires_at > now()) AS active_leases,
    (SELECT count(*) FROM secret_audit_events
     WHERE secret_id IN (SELECT id FROM visible_secrets)
       AND (decision = 'deny' OR outcome LIKE '%denied%')) AS denied_resolutions,
    (SELECT count(*) FROM secret_audit_events
     WHERE secret_id IN (SELECT id FROM visible_secrets)
       AND operation = 'use_brokered') AS broker_operations,
    (SELECT count(DISTINCT runtime_run_id) FROM secret_audit_events
     WHERE secret_id IN (SELECT id FROM visible_secrets)
       AND operation = 'receive_raw') AS raw_delivery_runs,
    (SELECT count(DISTINCT mount.run_id)
     FROM secret_runtime_mounts AS mount
     JOIN run_secret_provenance AS provenance ON provenance.run_id = mount.run_id
     WHERE provenance.secret_id IN (SELECT id FROM visible_secrets)
       AND mount.state = 'materialized') AS active_raw_mounts;

-- Operator inspection is read-only and retains exact historical provenance.
CREATE VIEW release_provenance_inspection
WITH (security_invoker = true) AS
SELECT
    release.id AS release_id,
    release.state AS release_state,
    release.repository_id,
    release.source_commit,
    release.build_request_id,
    release.manifest_hash,
    instance.id AS instance_id,
    instance.active_revision_id,
    instance.state AS instance_state,
    instance.run_gate_open,
    revision.id AS revision_id,
    attachment.id AS attachment_id,
    attachment.repository_id AS target_repository_id,
    attachment.enabled AS attachment_enabled,
    attachment.removed_at AS attachment_removed_at,
    update_record.id AS update_id,
    update_record.state AS update_state,
    run.id AS run_id,
    run.state AS run_state,
    run.instance_revision_id AS run_revision_id,
    run.release_id AS run_release_id,
    run.lease_id
FROM releases AS release
LEFT JOIN release_agents AS release_agent
  ON release_agent.release_id = release.id
LEFT JOIN agent_instance_revisions AS revision
  ON revision.release_agent_id = release_agent.id
LEFT JOIN agent_instances AS instance ON instance.id = revision.instance_id
LEFT JOIN agent_attachments AS attachment ON attachment.instance_id = instance.id
LEFT JOIN agent_updates AS update_record ON update_record.instance_id = instance.id
LEFT JOIN runs AS run ON run.instance_id = instance.id;

GRANT SELECT ON secret_version_metadata, release_operation_metrics,
    instance_operation_metrics, secret_operation_metrics,
    release_provenance_inspection
    TO hephaestus_app, hephaestus_worker;

-- Every lifecycle table emits only its kind and opaque row identifier. Browser
-- subscribers must reauthorize and re-read through RLS after each wakeup.
CREATE TRIGGER builds_ui_wakeup
AFTER INSERT OR UPDATE ON build_requests
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER release_artifacts_ui_wakeup
AFTER INSERT OR UPDATE ON release_artifacts
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER release_agents_ui_wakeup
AFTER INSERT OR UPDATE ON release_agents
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER instance_revisions_ui_wakeup
AFTER INSERT ON agent_instance_revisions
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER secret_records_ui_wakeup
AFTER INSERT OR UPDATE ON secrets
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER secret_grants_ui_wakeup
AFTER INSERT OR UPDATE ON secret_grants
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER secret_imports_ui_wakeup
AFTER INSERT OR UPDATE ON secret_imports
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER secret_bindings_ui_wakeup
AFTER INSERT OR UPDATE ON agent_secret_bindings
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER secret_versions_ui_wakeup
AFTER INSERT OR UPDATE ON secret_versions
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
CREATE TRIGGER secret_leases_ui_wakeup
AFTER INSERT OR UPDATE ON secret_leases
FOR EACH ROW EXECUTE FUNCTION notify_ui_wakeup();
