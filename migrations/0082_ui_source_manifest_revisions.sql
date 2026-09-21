-- Durable, repository-scoped capture evidence for the optional release UI
-- declaration.  This is intentionally UI-specific; it is not a generic
-- source-manifest framework.

ALTER TABLE git_receives
    ADD CONSTRAINT git_receives_id_repository_unique
    UNIQUE (id, repository_id);

ALTER TABLE build_requests
    ADD CONSTRAINT build_requests_id_repository_commit_unique
    UNIQUE (id, repository_id, source_commit);

CREATE TABLE ui_source_manifest_revisions (
    id uuid PRIMARY KEY,
    repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    receive_id uuid NOT NULL,
    source_commit text NOT NULL CHECK (
        source_commit ~ '^[0-9a-f]{40}$'
        OR source_commit ~ '^[0-9a-f]{64}$'
    ),
    manifest_path text NOT NULL DEFAULT 'heph.ui.toml'
        CHECK (manifest_path = 'heph.ui.toml'),
    entry_kind text NOT NULL CHECK (
        entry_kind IN ('blob', 'symlink', 'tree', 'gitlink')
    ),
    manifest_oid text NOT NULL CHECK (
        manifest_oid ~ '^[0-9a-f]{40}$'
        OR manifest_oid ~ '^[0-9a-f]{64}$'
    ),
    actual_size_bytes bigint CHECK (actual_size_bytes IS NULL OR actual_size_bytes >= 0),
    source_sha256 bytea CHECK (
        source_sha256 IS NULL OR octet_length(source_sha256) = 32
    ),
    status text NOT NULL CHECK (status IN ('valid', 'invalid')),
    requires_gateways boolean NOT NULL DEFAULT false,
    normalized_ui_config jsonb,
    normalized_ui_hash bytea CHECK (
        normalized_ui_hash IS NULL OR octet_length(normalized_ui_hash) = 32
    ),
    gateway_manifest_oid text CHECK (
        gateway_manifest_oid IS NULL
        OR gateway_manifest_oid ~ '^[0-9a-f]{40}$'
        OR gateway_manifest_oid ~ '^[0-9a-f]{64}$'
    ),
    gateway_actual_size_bytes bigint CHECK (
        gateway_actual_size_bytes IS NULL OR gateway_actual_size_bytes >= 0
    ),
    gateway_source_sha256 bytea CHECK (
        gateway_source_sha256 IS NULL
        OR octet_length(gateway_source_sha256) = 32
    ),
    normalized_gateway_config jsonb,
    normalized_gateway_hash bytea CHECK (
        normalized_gateway_hash IS NULL
        OR octet_length(normalized_gateway_hash) = 32
    ),
    diagnostics jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (repository_id, source_commit),
    UNIQUE (id, repository_id, source_commit, status),
    FOREIGN KEY (receive_id, repository_id)
        REFERENCES git_receives(id, repository_id),
    CHECK (
        jsonb_typeof(diagnostics) = 'array'
        AND jsonb_array_length(diagnostics) <= 64
        AND octet_length(diagnostics::text) <= 32768
    ),
    CHECK (
        normalized_ui_config IS NULL
        OR (
            jsonb_typeof(normalized_ui_config) = 'object'
            AND octet_length(normalized_ui_config::text) <= 1048576
        )
    ),
    CHECK (
        normalized_gateway_config IS NULL
        OR (
            jsonb_typeof(normalized_gateway_config) = 'object'
            AND octet_length(normalized_gateway_config::text) <= 2097152
        )
    ),
    CHECK (
        (status = 'valid'
            AND entry_kind = 'blob'
            AND manifest_oid IS NOT NULL
            AND actual_size_bytes IS NOT NULL
            AND actual_size_bytes <= 262144
            AND source_sha256 IS NOT NULL
            AND normalized_ui_config IS NOT NULL
            AND normalized_ui_hash IS NOT NULL
            AND diagnostics = '[]'::jsonb
            AND (
                (NOT requires_gateways
                    AND gateway_manifest_oid IS NULL
                    AND gateway_actual_size_bytes IS NULL
                    AND gateway_source_sha256 IS NULL
                    AND normalized_gateway_config IS NULL
                    AND normalized_gateway_hash IS NULL)
                OR (
                    requires_gateways
                    AND gateway_manifest_oid IS NOT NULL
                    AND gateway_actual_size_bytes IS NOT NULL
                    AND gateway_actual_size_bytes <= 1048576
                    AND gateway_source_sha256 IS NOT NULL
                    AND normalized_gateway_config IS NOT NULL
                    AND normalized_gateway_hash IS NOT NULL
                )
            )
        )
        OR
        (status = 'invalid'
            AND jsonb_array_length(diagnostics) > 0
            AND normalized_ui_config IS NULL
            AND normalized_ui_hash IS NULL
            AND normalized_gateway_config IS NULL
            AND normalized_gateway_hash IS NULL
        )
    )
);

CREATE TABLE build_request_ui_source_manifests (
    build_request_id uuid PRIMARY KEY,
    repository_id uuid NOT NULL,
    source_commit text NOT NULL CHECK (
        source_commit ~ '^[0-9a-f]{40}$'
        OR source_commit ~ '^[0-9a-f]{64}$'
    ),
    source_manifest_revision_id uuid NOT NULL,
    source_status text NOT NULL DEFAULT 'valid'
        CHECK (source_status = 'valid'),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (build_request_id, repository_id, source_commit)
        REFERENCES build_requests(id, repository_id, source_commit),
    FOREIGN KEY (
        source_manifest_revision_id, repository_id, source_commit, source_status
    ) REFERENCES ui_source_manifest_revisions(
        id, repository_id, source_commit, status
    )
);

CREATE FUNCTION reject_ui_source_manifest_mutation() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'UI source manifest capture records are immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
REVOKE ALL ON FUNCTION reject_ui_source_manifest_mutation() FROM PUBLIC;

CREATE TRIGGER ui_source_manifest_revisions_immutable
BEFORE UPDATE OR DELETE ON ui_source_manifest_revisions
FOR EACH ROW EXECUTE FUNCTION reject_ui_source_manifest_mutation();

CREATE TRIGGER build_request_ui_source_manifests_immutable
BEFORE UPDATE OR DELETE ON build_request_ui_source_manifests
FOR EACH ROW EXECUTE FUNCTION reject_ui_source_manifest_mutation();

ALTER TABLE ui_source_manifest_revisions ENABLE ROW LEVEL SECURITY;
ALTER TABLE ui_source_manifest_revisions FORCE ROW LEVEL SECURITY;
ALTER TABLE build_request_ui_source_manifests ENABLE ROW LEVEL SECURITY;
ALTER TABLE build_request_ui_source_manifests FORCE ROW LEVEL SECURITY;

CREATE POLICY ui_source_manifest_revisions_worker
    ON ui_source_manifest_revisions FOR ALL TO hephaestus_worker
    USING (true) WITH CHECK (true);
CREATE POLICY ui_source_manifest_revisions_select
    ON ui_source_manifest_revisions FOR SELECT TO hephaestus_app
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'repository', repository_id::text
    ) = 1);
CREATE POLICY ui_source_manifest_revisions_insert
    ON ui_source_manifest_revisions FOR INSERT TO hephaestus_app
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_write',
        'repository', repository_id::text
    ) = 1);

CREATE POLICY build_request_ui_source_manifests_worker
    ON build_request_ui_source_manifests FOR ALL TO hephaestus_worker
    USING (true) WITH CHECK (true);
CREATE POLICY build_request_ui_source_manifests_select
    ON build_request_ui_source_manifests FOR SELECT TO hephaestus_app
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read',
        'build', build_request_id::text
    ) = 1);
CREATE POLICY build_request_ui_source_manifests_insert
    ON build_request_ui_source_manifests FOR INSERT TO hephaestus_app
    WITH CHECK (
        check_permission(
            'user', hephaestus_actor_id(), 'can_execute',
            'build', build_request_id::text
        ) = 1
        AND check_permission(
            'user', hephaestus_actor_id(), 'can_write',
            'repository', repository_id::text
        ) = 1
    );

REVOKE ALL ON ui_source_manifest_revisions,
    build_request_ui_source_manifests FROM PUBLIC;
REVOKE UPDATE, DELETE ON ui_source_manifest_revisions,
    build_request_ui_source_manifests FROM hephaestus_app, hephaestus_worker;
GRANT SELECT, INSERT ON ui_source_manifest_revisions,
    build_request_ui_source_manifests TO hephaestus_app, hephaestus_worker;
