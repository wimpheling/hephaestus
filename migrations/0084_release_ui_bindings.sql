-- The source snapshot is an identity link.  It deliberately stores no UI
-- configuration JSON: the immutable source capture and build request already
-- own that evidence.  Publication code resolves the validated source capture
-- into these release-scoped child rows and immutable artifact/agent IDs.

-- Existing tables need the composite identities used by the exact UI links.
ALTER TABLE releases
    ADD CONSTRAINT releases_id_build_request_unique
    UNIQUE (id, build_request_id);

ALTER TABLE build_request_ui_source_manifests
    ADD CONSTRAINT build_request_ui_source_manifests_build_source_unique
    UNIQUE (build_request_id, source_manifest_revision_id);

ALTER TABLE release_artifacts
    ADD CONSTRAINT release_artifacts_id_release_kind_media_unique
    UNIQUE (id, release_id, kind, media_type);

CREATE TABLE release_ui_source_snapshots (
    release_id uuid PRIMARY KEY,
    build_request_id uuid NOT NULL,
    source_manifest_revision_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (release_id, build_request_id)
        REFERENCES releases(id, build_request_id),
    FOREIGN KEY (build_request_id, source_manifest_revision_id)
        REFERENCES build_request_ui_source_manifests(
            build_request_id, source_manifest_revision_id
        )
);

CREATE TABLE release_ui_descriptors (
    release_id uuid NOT NULL
        REFERENCES release_ui_source_snapshots(release_id),
    ui_key text NOT NULL CHECK (
        ui_key ~ '^[a-z][a-z0-9-]{0,63}$'
    ),
    scope text NOT NULL CHECK (scope IN ('project', 'repository', 'global')),
    -- UiLabel's control, bidi, U+2028, and U+2029 checks remain enforced by
    -- trusted typed publication validation; SQL keeps the bounded nonempty
    -- Unicode length invariant here.
    label text NOT NULL CHECK (char_length(label) BETWEEN 1 AND 80),
    icon text NOT NULL CHECK (icon IN ('app', 'chat', 'code', 'book', 'chart')),
    presentation text NOT NULL CHECK (presentation IN ('iframe', 'full_page')),
    route_base text NOT NULL CHECK (
        octet_length(route_base) BETWEEN 1 AND 256
        AND route_base ~ '^[A-Za-z0-9._~-]+(/[A-Za-z0-9._~-]+)*$'
        AND route_base !~ '(^|/)\.\.?(/|$)'
    ),
    -- Both static and managed content use the same validated relative path.
    entrypoint text NOT NULL CHECK (
        octet_length(entrypoint) BETWEEN 1 AND 256
        AND entrypoint ~ '^[A-Za-z0-9._~-]+(/[A-Za-z0-9._~-]+)*$'
        AND entrypoint !~ '(^|/)\.\.?(/|$)'
    ),
    ui_kit_version integer NOT NULL CHECK (ui_kit_version = 1),
    cache text NOT NULL CHECK (cache = 'no_store'),
    content_kind text NOT NULL CHECK (content_kind IN ('static', 'managed_service')),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (release_id, ui_key),
    UNIQUE (release_id, ui_key, content_kind),
    UNIQUE (release_id, scope, route_base)
);

CREATE TABLE release_ui_static_files (
    release_id uuid NOT NULL,
    ui_key text NOT NULL,
    content_kind text NOT NULL DEFAULT 'static'
        CHECK (content_kind = 'static'),
    route text NOT NULL CHECK (
        octet_length(route) BETWEEN 1 AND 256
        AND route ~ '^[A-Za-z0-9._~-]+(/[A-Za-z0-9._~-]+)*$'
        AND route !~ '(^|/)\.\.?(/|$)'
    ),
    artifact_id uuid NOT NULL,
    artifact_kind text NOT NULL DEFAULT 'file'
        CHECK (artifact_kind = 'file'),
    artifact_media_type text NOT NULL CHECK (artifact_media_type IN (
        'text/html', 'text/css', 'text/javascript', 'application/json',
        'text/plain', 'image/png', 'image/jpeg', 'image/webp', 'image/gif',
        'image/svg+xml', 'application/wasm'
    )),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (release_id, ui_key, route),
    FOREIGN KEY (release_id, ui_key, content_kind)
        REFERENCES release_ui_descriptors(release_id, ui_key, content_kind),
    FOREIGN KEY (artifact_id, release_id, artifact_kind, artifact_media_type)
        REFERENCES release_artifacts(id, release_id, kind, media_type)
);

CREATE TABLE release_ui_managed_services (
    release_id uuid NOT NULL,
    ui_key text NOT NULL,
    content_kind text NOT NULL DEFAULT 'managed_service'
        CHECK (content_kind = 'managed_service'),
    gateway_name text NOT NULL CHECK (
        gateway_name ~ '^[a-z][a-z0-9_-]{0,63}$'
    ),
    route text NOT NULL CHECK (
        octet_length(route) BETWEEN 2 AND 257
        AND route ~ '^/[A-Za-z0-9._~-]+(/[A-Za-z0-9._~-]+)*$'
        AND route !~ '(^|/)\.\.?(/|$)'
    ),
    release_agent_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (release_id, ui_key),
    FOREIGN KEY (release_id, ui_key, content_kind)
        REFERENCES release_ui_descriptors(release_id, ui_key, content_kind),
    FOREIGN KEY (release_agent_id, release_id)
        REFERENCES release_agents(id, release_id)
);

CREATE TABLE release_ui_api_bindings (
    release_id uuid NOT NULL,
    ui_key text NOT NULL,
    api_key text NOT NULL CHECK (
        api_key ~ '^[a-z][a-z0-9-]{0,63}$'
    ),
    gateway_name text NOT NULL CHECK (
        gateway_name ~ '^[a-z][a-z0-9_-]{0,63}$'
    ),
    method text NOT NULL CHECK (
        method IN ('GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS')
    ),
    route text NOT NULL CHECK (
        octet_length(route) BETWEEN 2 AND 257
        AND route ~ '^/[A-Za-z0-9._~-]+(/[A-Za-z0-9._~-]+)*$'
        AND route !~ '(^|/)\.\.?(/|$)'
    ),
    release_agent_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (release_id, ui_key, api_key),
    FOREIGN KEY (release_id, ui_key)
        REFERENCES release_ui_descriptors(release_id, ui_key),
    FOREIGN KEY (release_agent_id, release_id)
        REFERENCES release_agents(id, release_id)
);

-- The five tables are append-only publication evidence.  A single narrow
-- trigger function is reused only for these five concrete tables; it is not a
-- generic manifest/event framework.
CREATE FUNCTION reject_release_ui_binding_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'published release UI bindings are immutable'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;
REVOKE ALL ON FUNCTION reject_release_ui_binding_mutation() FROM PUBLIC;

-- UI rows are publication inputs and may be inserted only while the parent
-- release is a draft.  The row lock serializes this check with the existing
-- release publication/revocation UPDATE, so a concurrent insert cannot land
-- after the release leaves draft.  This is intentionally a UI-specific
-- guard; existing release-artifact policy behavior is unchanged.
CREATE FUNCTION enforce_release_ui_binding_draft() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    release_state text;
BEGIN
    SELECT state INTO release_state
    FROM releases
    WHERE id = NEW.release_id
    FOR SHARE;
    IF release_state IS DISTINCT FROM 'draft' THEN
        RAISE EXCEPTION 'release UI bindings require a draft release'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_release_ui_binding_draft() FROM PUBLIC;

CREATE TRIGGER release_ui_source_snapshots_require_draft
BEFORE INSERT ON release_ui_source_snapshots
FOR EACH ROW EXECUTE FUNCTION enforce_release_ui_binding_draft();
CREATE TRIGGER release_ui_descriptors_require_draft
BEFORE INSERT ON release_ui_descriptors
FOR EACH ROW EXECUTE FUNCTION enforce_release_ui_binding_draft();
CREATE TRIGGER release_ui_static_files_require_draft
BEFORE INSERT ON release_ui_static_files
FOR EACH ROW EXECUTE FUNCTION enforce_release_ui_binding_draft();
CREATE TRIGGER release_ui_managed_services_require_draft
BEFORE INSERT ON release_ui_managed_services
FOR EACH ROW EXECUTE FUNCTION enforce_release_ui_binding_draft();
CREATE TRIGGER release_ui_api_bindings_require_draft
BEFORE INSERT ON release_ui_api_bindings
FOR EACH ROW EXECUTE FUNCTION enforce_release_ui_binding_draft();

CREATE TRIGGER release_ui_source_snapshots_immutable
BEFORE UPDATE OR DELETE ON release_ui_source_snapshots
FOR EACH ROW EXECUTE FUNCTION reject_release_ui_binding_mutation();
CREATE TRIGGER release_ui_descriptors_immutable
BEFORE UPDATE OR DELETE ON release_ui_descriptors
FOR EACH ROW EXECUTE FUNCTION reject_release_ui_binding_mutation();
CREATE TRIGGER release_ui_static_files_immutable
BEFORE UPDATE OR DELETE ON release_ui_static_files
FOR EACH ROW EXECUTE FUNCTION reject_release_ui_binding_mutation();
CREATE TRIGGER release_ui_managed_services_immutable
BEFORE UPDATE OR DELETE ON release_ui_managed_services
FOR EACH ROW EXECUTE FUNCTION reject_release_ui_binding_mutation();
CREATE TRIGGER release_ui_api_bindings_immutable
BEFORE UPDATE OR DELETE ON release_ui_api_bindings
FOR EACH ROW EXECUTE FUNCTION reject_release_ui_binding_mutation();

-- The publication resolver must still check that a static descriptor's exact
-- entrypoint has exactly one matching static-file route.  That relationship
-- is intentionally validated in the same publication transaction: encoding
-- it with cyclic row triggers would add ordering-dependent database logic,
-- while the constraints above already bound both rows to one release/UI and
-- one content kind.

ALTER TABLE release_ui_source_snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE release_ui_source_snapshots FORCE ROW LEVEL SECURITY;
ALTER TABLE release_ui_descriptors ENABLE ROW LEVEL SECURITY;
ALTER TABLE release_ui_descriptors FORCE ROW LEVEL SECURITY;
ALTER TABLE release_ui_static_files ENABLE ROW LEVEL SECURITY;
ALTER TABLE release_ui_static_files FORCE ROW LEVEL SECURITY;
ALTER TABLE release_ui_managed_services ENABLE ROW LEVEL SECURITY;
ALTER TABLE release_ui_managed_services FORCE ROW LEVEL SECURITY;
ALTER TABLE release_ui_api_bindings ENABLE ROW LEVEL SECURITY;
ALTER TABLE release_ui_api_bindings FORCE ROW LEVEL SECURITY;

CREATE POLICY release_ui_source_snapshots_worker
    ON release_ui_source_snapshots FOR ALL TO hephaestus_worker
    USING (true) WITH CHECK (true);
CREATE POLICY release_ui_descriptors_worker
    ON release_ui_descriptors FOR ALL TO hephaestus_worker
    USING (true) WITH CHECK (true);
CREATE POLICY release_ui_static_files_worker
    ON release_ui_static_files FOR ALL TO hephaestus_worker
    USING (true) WITH CHECK (true);
CREATE POLICY release_ui_managed_services_worker
    ON release_ui_managed_services FOR ALL TO hephaestus_worker
    USING (true) WITH CHECK (true);
CREATE POLICY release_ui_api_bindings_worker
    ON release_ui_api_bindings FOR ALL TO hephaestus_worker
    USING (true) WITH CHECK (true);

CREATE POLICY release_ui_source_snapshots_select
    ON release_ui_source_snapshots FOR SELECT TO hephaestus_app
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read', 'release', release_id::text
    ) = 1);
CREATE POLICY release_ui_descriptors_select
    ON release_ui_descriptors FOR SELECT TO hephaestus_app
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read', 'release', release_id::text
    ) = 1);
CREATE POLICY release_ui_static_files_select
    ON release_ui_static_files FOR SELECT TO hephaestus_app
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read', 'release', release_id::text
    ) = 1);
CREATE POLICY release_ui_managed_services_select
    ON release_ui_managed_services FOR SELECT TO hephaestus_app
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read', 'release', release_id::text
    ) = 1);
CREATE POLICY release_ui_api_bindings_select
    ON release_ui_api_bindings FOR SELECT TO hephaestus_app
    USING (check_permission(
        'user', hephaestus_actor_id(), 'can_read', 'release', release_id::text
    ) = 1);

CREATE POLICY release_ui_source_snapshots_insert
    ON release_ui_source_snapshots FOR INSERT TO hephaestus_app
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_publish', 'release', release_id::text
    ) = 1);
CREATE POLICY release_ui_descriptors_insert
    ON release_ui_descriptors FOR INSERT TO hephaestus_app
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_publish', 'release', release_id::text
    ) = 1);
CREATE POLICY release_ui_static_files_insert
    ON release_ui_static_files FOR INSERT TO hephaestus_app
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_publish', 'release', release_id::text
    ) = 1);
CREATE POLICY release_ui_managed_services_insert
    ON release_ui_managed_services FOR INSERT TO hephaestus_app
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_publish', 'release', release_id::text
    ) = 1);
CREATE POLICY release_ui_api_bindings_insert
    ON release_ui_api_bindings FOR INSERT TO hephaestus_app
    WITH CHECK (check_permission(
        'user', hephaestus_actor_id(), 'can_publish', 'release', release_id::text
    ) = 1);

REVOKE ALL ON release_ui_source_snapshots,
    release_ui_descriptors,
    release_ui_static_files,
    release_ui_managed_services,
    release_ui_api_bindings FROM PUBLIC;
REVOKE UPDATE, DELETE ON release_ui_source_snapshots,
    release_ui_descriptors,
    release_ui_static_files,
    release_ui_managed_services,
    release_ui_api_bindings FROM hephaestus_app, hephaestus_worker;
GRANT SELECT, INSERT ON release_ui_source_snapshots,
    release_ui_descriptors,
    release_ui_static_files,
    release_ui_managed_services,
    release_ui_api_bindings TO hephaestus_app, hephaestus_worker;
