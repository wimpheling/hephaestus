-- Durable project-owned OCI builder definitions.
--
-- This migration records source metadata and the result of an external,
-- policy-controlled OCI preparation workflow. It deliberately does not run a
-- container builder, resolve tags, or create image digests.

CREATE TABLE project_builder_definitions (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    source_repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE RESTRICT,
    key text NOT NULL CHECK (key ~ '^[a-z0-9][a-z0-9_-]{0,63}$'),
    display_name text NOT NULL CHECK (length(display_name) BETWEEN 1 AND 200),
    source_revision text NOT NULL CHECK (source_revision ~ '^[0-9a-f]{40}([0-9a-f]{24})?$'),
    dockerfile_path text NOT NULL CHECK (
        length(dockerfile_path) BETWEEN 1 AND 1024
        AND dockerfile_path !~ '(^/|\\|[[:cntrl:]]|//|(^|/)\.\.?(/|$))'
    ),
    context_path text NOT NULL CHECK (
        length(context_path) BETWEEN 1 AND 1024
        AND (
            context_path = '.'
            OR context_path !~ '(^/|\\|[[:cntrl:]]|//|(^|/)\.\.?(/|$))'
        )
    ),
    context_digest text NOT NULL CHECK (context_digest ~ '^sha256:[0-9a-f]{64}$'),
    approved_base_image_reference text NOT NULL CHECK (
        approved_base_image_reference ~ '^.+@sha256:[0-9a-f]{64}$'
    ),
    status text NOT NULL CHECK (
        status IN ('draft', 'preparing', 'ready', 'failed', 'retired')
    ),
    oci_image_reference text CHECK (
        oci_image_reference IS NULL
        OR oci_image_reference ~ '^.+@sha256:[0-9a-f]{64}$'
    ),
    oci_image_digest text CHECK (
        oci_image_digest IS NULL OR oci_image_digest ~ '^sha256:[0-9a-f]{64}$'
    ),
    provenance jsonb CHECK (
        provenance IS NULL OR jsonb_typeof(provenance) = 'object'
    ),
    failure_reason text CHECK (
        failure_reason IS NULL OR length(failure_reason) BETWEEN 1 AND 2048
    ),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, key),
    CHECK (
        (oci_image_reference IS NULL AND oci_image_digest IS NULL AND provenance IS NULL)
        OR (oci_image_reference IS NOT NULL AND oci_image_digest IS NOT NULL
            AND provenance IS NOT NULL)
    ),
    CHECK (
        oci_image_reference IS NULL
        OR split_part(oci_image_reference, '@', 2) = oci_image_digest
    ),
    CHECK (
        (status = 'ready'
            AND oci_image_reference IS NOT NULL
            AND oci_image_digest IS NOT NULL
            AND provenance IS NOT NULL
            AND failure_reason IS NULL)
        OR (status = 'failed'
            AND oci_image_reference IS NULL
            AND oci_image_digest IS NULL
            AND provenance IS NULL
            AND failure_reason IS NOT NULL)
        OR (status IN ('draft', 'preparing')
            AND oci_image_reference IS NULL
            AND oci_image_digest IS NULL
            AND provenance IS NULL
            AND failure_reason IS NULL)
        OR (status = 'retired')
    )
);

CREATE INDEX project_builder_definitions_by_project
    ON project_builder_definitions (project_id, status, key, id);

ALTER TABLE project_builder_definitions ENABLE ROW LEVEL SECURITY;
ALTER TABLE project_builder_definitions FORCE ROW LEVEL SECURITY;

CREATE POLICY project_builder_definitions_select
    ON project_builder_definitions FOR SELECT
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'project', project_id::text) = 1);

CREATE POLICY project_builder_definitions_insert
    ON project_builder_definitions FOR INSERT
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_write',
        'project', project_id::text) = 1);

CREATE POLICY project_builder_definitions_update
    ON project_builder_definitions FOR UPDATE
    USING (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'project', project_id::text) = 1)
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'project', project_id::text) = 1);

GRANT SELECT, INSERT, UPDATE ON project_builder_definitions
    TO hephaestus_app, hephaestus_worker;

-- Enforce that the source repository belongs to the same project and that the
-- selected base is an exact ready/available platform catalog entry. The
-- security-definer function is intentionally the only cross-table policy
-- check; callers still need project permission through RLS.
CREATE FUNCTION validate_project_builder_definition() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET row_security = off
AS $$
DECLARE
    repository_project_id uuid;
    catalog_reference text;
    catalog_preparation text;
    catalog_availability text;
    base_changed boolean := false;
BEGIN
    SELECT repository.project_id
      INTO repository_project_id
      FROM repositories AS repository
     WHERE repository.id = NEW.source_repository_id;
    IF repository_project_id IS NULL OR repository_project_id <> NEW.project_id THEN
        RAISE EXCEPTION 'project builder source repository must belong to its project';
    END IF;

    IF TG_OP = 'INSERT' THEN
        base_changed := true;
        SELECT image.image_reference, image.preparation_state, image.availability_state
          INTO catalog_reference, catalog_preparation, catalog_availability
          FROM builder_images AS image
         WHERE image.image_reference = NEW.approved_base_image_reference;
    ELSIF NEW.approved_base_image_reference IS DISTINCT FROM OLD.approved_base_image_reference THEN
        base_changed := true;
        SELECT image.image_reference, image.preparation_state, image.availability_state
          INTO catalog_reference, catalog_preparation, catalog_availability
          FROM builder_images AS image
         WHERE image.image_reference = NEW.approved_base_image_reference;
    END IF;
    IF base_changed THEN
        IF catalog_reference IS NULL
           OR catalog_preparation <> 'ready'
           OR catalog_availability <> 'available' THEN
            RAISE EXCEPTION 'project builder base image is not an approved available platform image';
        END IF;
    END IF;

    IF TG_OP = 'UPDATE' THEN
        IF OLD.status <> 'draft'
           AND (NEW.project_id, NEW.source_repository_id, NEW.key,
                NEW.display_name, NEW.source_revision, NEW.dockerfile_path,
                NEW.context_path, NEW.context_digest,
                NEW.approved_base_image_reference)
               IS DISTINCT FROM
               (OLD.project_id, OLD.source_repository_id, OLD.key,
                OLD.display_name, OLD.source_revision, OLD.dockerfile_path,
                OLD.context_path, OLD.context_digest,
                OLD.approved_base_image_reference) THEN
            RAISE EXCEPTION 'project builder source metadata is immutable after draft';
        END IF;
        IF OLD.status IN ('ready', 'retired')
           AND (NEW.oci_image_reference, NEW.oci_image_digest, NEW.provenance)
               IS DISTINCT FROM
               (OLD.oci_image_reference, OLD.oci_image_digest, OLD.provenance) THEN
            RAISE EXCEPTION 'project builder OCI output provenance is immutable';
        END IF;
        IF OLD.status = 'retired' AND NEW.status <> 'retired' THEN
            RAISE EXCEPTION 'retired project builders cannot be reactivated';
        END IF;
    END IF;
    RETURN NEW;
END
$$;

ALTER FUNCTION validate_project_builder_definition() OWNER TO hephaestus_authz_owner;
GRANT EXECUTE ON FUNCTION validate_project_builder_definition() TO hephaestus_app, hephaestus_worker;
GRANT SELECT ON repositories, builder_images TO hephaestus_authz_owner;

CREATE TRIGGER validate_project_builder_definition
BEFORE INSERT OR UPDATE ON project_builder_definitions
FOR EACH ROW EXECUTE FUNCTION validate_project_builder_definition();
