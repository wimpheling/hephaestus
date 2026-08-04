-- Forge-owned OCI registry control plane. Zot owns OCI bytes and graph state;
-- PostgreSQL owns only durable authorization, verification and lifecycle data.

CREATE TABLE registry_namespaces (
    id uuid PRIMARY KEY,
    repository_path text NOT NULL UNIQUE CHECK (
        length(repository_path) BETWEEN 1 AND 255
        AND repository_path = lower(repository_path)
        AND repository_path !~ '[[:space:]@:]'
    ),
    owner_kind text NOT NULL CHECK (
        owner_kind IN ('platform_builder', 'repository_builder', 'release_agent')
    ),
    platform_builder_key text,
    owner_id uuid,
    project_id uuid REFERENCES projects(id) ON DELETE RESTRICT,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (
        (owner_kind = 'platform_builder'
            AND platform_builder_key ~ '^[a-z0-9][a-z0-9_-]{0,63}$'
            AND owner_id IS NULL AND project_id IS NULL
            AND repository_path = 'platform/builders/' || platform_builder_key)
        OR
        (owner_kind = 'repository_builder'
            AND platform_builder_key IS NULL AND owner_id IS NOT NULL
            AND project_id IS NOT NULL
            AND repository_path = 'projects/' || project_id::text
                || '/repository-builders/' || owner_id::text)
        OR
        (owner_kind = 'release_agent'
            AND platform_builder_key IS NULL AND owner_id IS NOT NULL
            AND project_id IS NOT NULL
            AND repository_path = 'projects/' || project_id::text
                || '/release-agents/' || owner_id::text)
    )
);
CREATE UNIQUE INDEX registry_platform_namespace_owner
    ON registry_namespaces (platform_builder_key)
    WHERE owner_kind = 'platform_builder';
CREATE UNIQUE INDEX registry_resource_namespace_owner
    ON registry_namespaces (owner_kind, owner_id)
    WHERE owner_kind IN ('repository_builder', 'release_agent');
CREATE INDEX registry_namespaces_by_project
    ON registry_namespaces (project_id, owner_kind, owner_id);

CREATE TABLE registry_publications (
    id uuid PRIMARY KEY,
    namespace_id uuid NOT NULL REFERENCES registry_namespaces(id) ON DELETE RESTRICT,
    owner_kind text NOT NULL CHECK (
        owner_kind IN ('platform_builder', 'repository_builder', 'release_agent')
    ),
    platform_builder_key text,
    owner_id uuid,
    project_id uuid REFERENCES projects(id) ON DELETE RESTRICT,
    registry_authority text NOT NULL CHECK (
        length(registry_authority) BETWEEN 1 AND 253
        AND registry_authority = lower(registry_authority)
        AND registry_authority !~ '[[:space:]@/]'
    ),
    expected_digest text NOT NULL CHECK (expected_digest ~ '^sha256:[0-9a-f]{64}$'),
    expected_media_type text NOT NULL CHECK (
        expected_media_type IN (
            'application/vnd.oci.image.manifest.v1+json',
            'application/vnd.oci.image.index.v1+json'
        )
    ),
    expected_size bigint NOT NULL CHECK (expected_size > 0),
    policy_version text NOT NULL CHECK (
        length(policy_version) BETWEEN 1 AND 128
        AND policy_version = btrim(policy_version)
        AND policy_version !~ '[[:space:]]'
    ),
    signature_required boolean NOT NULL DEFAULT false,
    state text NOT NULL DEFAULT 'pending' CHECK (
        state IN ('pending', 'publishing', 'verified', 'approved', 'retired', 'missing')
    ),
    verified_at timestamptz,
    approved_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (namespace_id, registry_authority, expected_digest, policy_version),
    CHECK (state = 'retired' OR (state IN ('verified', 'approved', 'missing')) = (verified_at IS NOT NULL)),
    CHECK (state = 'retired' OR (state IN ('approved', 'missing')) = (approved_at IS NOT NULL)),
    CHECK (
        (owner_kind = 'platform_builder' AND platform_builder_key IS NOT NULL
            AND owner_id IS NULL AND project_id IS NULL)
        OR (owner_kind IN ('repository_builder', 'release_agent')
            AND platform_builder_key IS NULL AND owner_id IS NOT NULL AND project_id IS NOT NULL)
    )
);
CREATE INDEX registry_publications_by_namespace
    ON registry_publications (namespace_id, state, created_at, id);
CREATE INDEX registry_publications_by_digest
    ON registry_publications (expected_digest, state, id);

-- These descriptors are evidence read back from Zot, not a second OCI graph.
CREATE TABLE registry_publication_platforms (
    publication_id uuid NOT NULL REFERENCES registry_publications(id) ON DELETE RESTRICT,
    digest text NOT NULL CHECK (digest ~ '^sha256:[0-9a-f]{64}$'),
    size bigint NOT NULL CHECK (size > 0),
    media_type text NOT NULL CHECK (media_type = 'application/vnd.oci.image.manifest.v1+json'),
    operating_system text NOT NULL CHECK (operating_system ~ '^[a-z0-9][a-z0-9_.-]{0,63}$'),
    architecture text NOT NULL CHECK (architecture ~ '^[a-z0-9][a-z0-9_.-]{0,63}$'),
    variant text CHECK (variant ~ '^[a-z0-9][a-z0-9_.-]{0,63}$'),
    PRIMARY KEY (publication_id, digest)
);
CREATE TABLE registry_publication_evidence (
    publication_id uuid NOT NULL REFERENCES registry_publications(id) ON DELETE RESTRICT,
    kind text NOT NULL CHECK (kind IN ('sbom', 'provenance', 'scan', 'signature')),
    subject_digest text NOT NULL CHECK (subject_digest ~ '^sha256:[0-9a-f]{64}$'),
    digest text NOT NULL CHECK (digest ~ '^sha256:[0-9a-f]{64}$'),
    size bigint NOT NULL CHECK (size > 0),
    media_type text NOT NULL CHECK (
        length(media_type) BETWEEN 1 AND 255 AND media_type ~ '^application/[a-z0-9.+-]+$'
    ),
    artifact_type text NOT NULL CHECK (
        length(artifact_type) BETWEEN 1 AND 255 AND artifact_type ~ '^application/[a-z0-9.+-]+$'
    ),
    PRIMARY KEY (publication_id, kind),
    UNIQUE (publication_id, digest)
);

-- Notification bodies are intentionally not retained. The source transport
-- validates them first; this table provides bounded at-least-once processing.
CREATE TABLE registry_notification_inbox (
    id uuid PRIMARY KEY,
    event_key text NOT NULL UNIQUE CHECK (length(event_key) BETWEEN 1 AND 200),
    repository_path text NOT NULL CHECK (length(repository_path) BETWEEN 1 AND 255),
    action text NOT NULL CHECK (action IN ('push', 'pull', 'delete')),
    target_digest text CHECK (target_digest ~ '^sha256:[0-9a-f]{64}$'),
    target_media_type text CHECK (length(target_media_type) BETWEEN 1 AND 255),
    target_size bigint CHECK (target_size >= 0),
    event_occurred_at timestamptz NOT NULL,
    payload_sha256 bytea NOT NULL CHECK (octet_length(payload_sha256) = 32),
    state text NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'claimed', 'processed', 'rejected')),
    claim_token uuid,
    lease_expires_at timestamptz,
    failure_code text CHECK (failure_code ~ '^[a-z][a-z0-9_]{0,63}$'),
    received_at timestamptz NOT NULL DEFAULT now(),
    processed_at timestamptz,
    CHECK (
        (state = 'pending' AND claim_token IS NULL AND lease_expires_at IS NULL
            AND processed_at IS NULL AND failure_code IS NULL)
        OR (state = 'claimed' AND claim_token IS NOT NULL AND lease_expires_at IS NOT NULL
            AND processed_at IS NULL AND failure_code IS NULL)
        OR (state = 'processed' AND claim_token IS NULL AND lease_expires_at IS NULL
            AND processed_at IS NOT NULL AND failure_code IS NULL)
        OR (state = 'rejected' AND claim_token IS NULL AND lease_expires_at IS NULL
            AND processed_at IS NOT NULL AND failure_code IS NOT NULL)
    )
);
CREATE INDEX registry_notification_inbox_claimable
    ON registry_notification_inbox (received_at, id)
    WHERE state IN ('pending', 'claimed');

CREATE FUNCTION validate_registry_namespace_immutability() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF (NEW.repository_path, NEW.owner_kind, NEW.platform_builder_key, NEW.owner_id, NEW.project_id)
       IS DISTINCT FROM
       (OLD.repository_path, OLD.owner_kind, OLD.platform_builder_key, OLD.owner_id, OLD.project_id) THEN
        RAISE EXCEPTION 'registry namespace ownership is immutable';
    END IF;
    NEW.updated_at := now();
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION validate_registry_namespace_immutability() FROM PUBLIC;
CREATE TRIGGER registry_namespace_immutable
BEFORE UPDATE ON registry_namespaces FOR EACH ROW
EXECUTE FUNCTION validate_registry_namespace_immutability();

CREATE FUNCTION validate_registry_publication() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public
SET row_security = off AS $$
DECLARE
    v_namespace registry_namespaces%ROWTYPE;
    v_sbom_count integer;
    v_provenance_count integer;
    v_scan_count integer;
    v_signature_count integer;
    v_platform_count integer;
BEGIN
    SELECT * INTO v_namespace FROM registry_namespaces WHERE id = NEW.namespace_id;
    IF v_namespace.id IS NULL
       OR (NEW.owner_kind, NEW.platform_builder_key, NEW.owner_id, NEW.project_id)
          IS DISTINCT FROM
          (v_namespace.owner_kind, v_namespace.platform_builder_key, v_namespace.owner_id, v_namespace.project_id) THEN
        RAISE EXCEPTION 'registry publication owner does not match namespace ownership';
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF NEW.state <> 'pending' OR NEW.verified_at IS NOT NULL OR NEW.approved_at IS NOT NULL THEN
            RAISE EXCEPTION 'registry publication must begin pending';
        END IF;
        RETURN NEW;
    END IF;
    IF (NEW.namespace_id, NEW.owner_kind, NEW.platform_builder_key, NEW.owner_id, NEW.project_id,
        NEW.registry_authority, NEW.expected_digest,
        NEW.expected_media_type, NEW.expected_size, NEW.policy_version,
        NEW.signature_required) IS DISTINCT FROM
       (OLD.namespace_id, OLD.owner_kind, OLD.platform_builder_key, OLD.owner_id, OLD.project_id,
        OLD.registry_authority, OLD.expected_digest,
        OLD.expected_media_type, OLD.expected_size, OLD.policy_version,
        OLD.signature_required) THEN
        RAISE EXCEPTION 'registry publication identity is immutable';
    END IF;
    IF NOT (CASE OLD.state
        WHEN 'pending' THEN NEW.state IN ('pending', 'publishing', 'verified', 'retired')
        WHEN 'publishing' THEN NEW.state IN ('publishing', 'pending', 'verified', 'retired')
        WHEN 'verified' THEN NEW.state IN ('verified', 'approved')
        WHEN 'approved' THEN NEW.state IN ('approved', 'missing', 'retired')
        WHEN 'missing' THEN NEW.state IN ('missing', 'approved', 'retired')
        WHEN 'retired' THEN NEW.state = 'retired'
        ELSE false
    END) THEN
        RAISE EXCEPTION 'illegal registry publication transition % -> %', OLD.state, NEW.state;
    END IF;
    IF NEW.state IN ('verified', 'approved', 'missing')
       OR (NEW.state = 'retired' AND NEW.verified_at IS NOT NULL) THEN
        SELECT count(*) FILTER (WHERE kind = 'sbom'),
               count(*) FILTER (WHERE kind = 'provenance'),
               count(*) FILTER (WHERE kind = 'scan'),
               count(*) FILTER (WHERE kind = 'signature')
          INTO v_sbom_count, v_provenance_count, v_scan_count, v_signature_count
          FROM registry_publication_evidence
         WHERE publication_id = NEW.id AND subject_digest = NEW.expected_digest;
        SELECT count(*) INTO v_platform_count FROM registry_publication_platforms
         WHERE publication_id = NEW.id;
        IF v_platform_count = 0 OR v_sbom_count <> 1 OR v_provenance_count <> 1 OR v_scan_count <> 1
           OR (NEW.signature_required AND v_signature_count <> 1) THEN
            RAISE EXCEPTION 'verified registry publication lacks required evidence';
        END IF;
    END IF;
    IF OLD.verified_at IS NOT NULL
       AND NEW.verified_at IS DISTINCT FROM OLD.verified_at THEN
        RAISE EXCEPTION 'registry verification timestamp is immutable';
    END IF;
    IF OLD.approved_at IS NOT NULL
       AND NEW.approved_at IS DISTINCT FROM OLD.approved_at THEN
        RAISE EXCEPTION 'registry approval timestamp is immutable';
    END IF;
    NEW.updated_at := now();
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION validate_registry_publication() FROM PUBLIC;
ALTER FUNCTION validate_registry_publication() OWNER TO hephaestus_authz_owner;
GRANT SELECT ON registry_namespaces, registry_publication_evidence,
    registry_publication_platforms TO hephaestus_authz_owner;
CREATE TRIGGER registry_publication_lifecycle
BEFORE INSERT OR UPDATE ON registry_publications FOR EACH ROW
EXECUTE FUNCTION validate_registry_publication();

CREATE FUNCTION validate_registry_evidence_immutability() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, public
SET row_security = off AS $$
DECLARE
    v_state text;
    v_expected_digest text;
BEGIN
    SELECT state INTO v_state FROM registry_publications WHERE id = COALESCE(NEW.publication_id, OLD.publication_id);
    IF v_state NOT IN ('pending', 'publishing') THEN
        RAISE EXCEPTION 'registry verification evidence is immutable';
    END IF;
    IF TG_OP <> 'INSERT' THEN
        RAISE EXCEPTION 'registry verification evidence cannot be changed';
    END IF;
    IF TG_TABLE_NAME = 'registry_publication_evidence' THEN
        SELECT expected_digest INTO v_expected_digest FROM registry_publications WHERE id = NEW.publication_id;
        IF NEW.subject_digest <> v_expected_digest THEN
            RAISE EXCEPTION 'registry evidence subject must match the expected manifest digest';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION validate_registry_evidence_immutability() FROM PUBLIC;
ALTER FUNCTION validate_registry_evidence_immutability() OWNER TO hephaestus_authz_owner;
GRANT SELECT ON registry_publications TO hephaestus_authz_owner;
CREATE TRIGGER registry_platform_evidence_immutable
BEFORE INSERT OR UPDATE OR DELETE ON registry_publication_platforms FOR EACH ROW
EXECUTE FUNCTION validate_registry_evidence_immutability();
CREATE TRIGGER registry_referrer_evidence_immutable
BEFORE INSERT OR UPDATE OR DELETE ON registry_publication_evidence FOR EACH ROW
EXECUTE FUNCTION validate_registry_evidence_immutability();

-- Registry lifecycle changes receive a normal product event only after the
-- transaction commits. Raw Zot notifications never mutate publications.
ALTER TABLE application_event_scopes DROP CONSTRAINT application_event_scopes_scope_kind_check;
ALTER TABLE application_event_scopes ADD CONSTRAINT application_event_scopes_scope_kind_check
    CHECK (scope_kind IN ('identity', 'organization', 'project', 'repository', 'run', 'agent_instance', 'registry'));
ALTER TABLE application_events DROP CONSTRAINT application_events_aggregate_type_check;
ALTER TABLE application_events ADD CONSTRAINT application_events_aggregate_type_check CHECK (
    aggregate_type IN ('identity_profile', 'identity_organizations', 'organization', 'project', 'repository',
        'repository_ref', 'build', 'release', 'agent_instance', 'run', 'review', 'secret_metadata',
        'secret_grant', 'secret_import', 'agent_secret_binding', 'artifact', 'registry_publication')
);
ALTER TABLE application_events DROP CONSTRAINT application_events_check1;
ALTER TABLE application_events DROP CONSTRAINT application_events_event_type_check;
ALTER TABLE application_events ADD CONSTRAINT application_events_event_type_check CHECK (
    (aggregate_type, event_type) IN (
        ('identity_profile', 'identity.profile_changed'), ('identity_organizations', 'identity.organizations_changed'),
        ('organization', 'organization.changed'), ('project', 'project.changed'), ('repository', 'repository.changed'),
        ('repository_ref', 'repository.ref_changed'), ('build', 'build.changed'), ('release', 'release.changed'),
        ('agent_instance', 'agent_instance.changed'), ('run', 'run.changed'), ('review', 'review.changed'),
        ('secret_metadata', 'secret_metadata.changed'), ('secret_grant', 'secret_grant.changed'),
        ('secret_import', 'secret_import.changed'), ('agent_secret_binding', 'agent_secret_binding.changed'),
        ('artifact', 'artifact.changed'), ('registry_publication', 'registry.publication_changed')
    )
);
CREATE FUNCTION capture_registry_publication_event() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
BEGIN
    IF OLD.state IS DISTINCT FROM NEW.state THEN
        PERFORM append_application_event(
            gen_random_uuid(),
            CASE WHEN NEW.project_id IS NULL THEN 'registry' ELSE 'project' END,
            COALESCE(NEW.project_id, NEW.id),
            'registry_publication', NEW.id,
            'registry.publication_changed', 'state_changed',
            CASE NEW.state
                WHEN 'verified' THEN 'active'
                WHEN 'approved' THEN 'published'
                WHEN 'retired' THEN 'revoked'
                WHEN 'missing' THEN 'failed'
                WHEN 'publishing' THEN 'running'
                ELSE 'pending'
            END,
            NULL, NULL
        );
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION capture_registry_publication_event() FROM PUBLIC;
CREATE TRIGGER registry_publication_product_event
AFTER UPDATE OF state ON registry_publications FOR EACH ROW
EXECUTE FUNCTION capture_registry_publication_event();

ALTER TABLE registry_namespaces ENABLE ROW LEVEL SECURITY;
ALTER TABLE registry_namespaces FORCE ROW LEVEL SECURITY;
ALTER TABLE registry_publications ENABLE ROW LEVEL SECURITY;
ALTER TABLE registry_publications FORCE ROW LEVEL SECURITY;
ALTER TABLE registry_publication_platforms ENABLE ROW LEVEL SECURITY;
ALTER TABLE registry_publication_platforms FORCE ROW LEVEL SECURITY;
ALTER TABLE registry_publication_evidence ENABLE ROW LEVEL SECURITY;
ALTER TABLE registry_publication_evidence FORCE ROW LEVEL SECURITY;
ALTER TABLE registry_notification_inbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE registry_notification_inbox FORCE ROW LEVEL SECURITY;
CREATE POLICY registry_namespaces_select ON registry_namespaces FOR SELECT USING (
    project_id IS NOT NULL
    AND check_permission('user', hephaestus_actor_id(), 'can_read', 'project', project_id::text) = 1
);
-- Platform-builder metadata is catalog data, not tenant OCI authorization.
-- It is visible only to an authenticated app actor; registry pull grants stay
-- at the separate token-authorization boundary.
CREATE POLICY registry_publications_select ON registry_publications FOR SELECT USING (
    EXISTS (SELECT 1 FROM registry_namespaces namespace WHERE namespace.id = namespace_id
        AND namespace.project_id IS NOT NULL
        AND check_permission('user', hephaestus_actor_id(), 'can_read', 'project', namespace.project_id::text) = 1)
);
CREATE POLICY registry_platforms_select ON registry_publication_platforms FOR SELECT USING (
    EXISTS (SELECT 1 FROM registry_publications publication WHERE publication.id = publication_id)
);
CREATE POLICY registry_evidence_select ON registry_publication_evidence FOR SELECT USING (
    EXISTS (SELECT 1 FROM registry_publications publication WHERE publication.id = publication_id)
);
CREATE POLICY registry_namespaces_worker ON registry_namespaces FOR ALL TO hephaestus_worker USING (true) WITH CHECK (true);
CREATE POLICY registry_publications_worker ON registry_publications FOR ALL TO hephaestus_worker USING (true) WITH CHECK (true);
CREATE POLICY registry_platforms_worker ON registry_publication_platforms FOR ALL TO hephaestus_worker USING (true) WITH CHECK (true);
CREATE POLICY registry_evidence_worker ON registry_publication_evidence FOR ALL TO hephaestus_worker USING (true) WITH CHECK (true);
CREATE POLICY registry_inbox_worker ON registry_notification_inbox FOR ALL TO hephaestus_worker USING (true) WITH CHECK (true);
GRANT SELECT ON registry_namespaces, registry_publications, registry_publication_platforms,
    registry_publication_evidence TO hephaestus_app;
GRANT SELECT, INSERT, UPDATE ON registry_namespaces, registry_publications,
    registry_publication_platforms, registry_publication_evidence, registry_notification_inbox TO hephaestus_worker;
