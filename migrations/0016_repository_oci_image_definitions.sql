-- Durable definitions for immutable OCI images produced from repository source.
--
-- A definition records the exact source tree and base image that a worker must
-- use to produce an image. It is not an execution class: a ready image may be
-- selected by any build or guest contract permitted by normal project policy.
-- The worker alone records the immutable output digest and its provenance.

CREATE TABLE repository_oci_image_definitions (
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
    base_image_reference text NOT NULL CHECK (
        base_image_reference ~ '^.+@sha256:[0-9a-f]{64}$'
    ),
    status text NOT NULL CHECK (
        status IN ('draft', 'producing', 'ready', 'failed', 'retired')
    ),
    image_reference text CHECK (
        image_reference IS NULL
        OR image_reference ~ '^.+@sha256:[0-9a-f]{64}$'
    ),
    image_digest text CHECK (
        image_digest IS NULL OR image_digest ~ '^sha256:[0-9a-f]{64}$'
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
        (image_reference IS NULL AND image_digest IS NULL AND provenance IS NULL)
        OR (image_reference IS NOT NULL AND image_digest IS NOT NULL
            AND provenance IS NOT NULL)
    ),
    CHECK (
        image_reference IS NULL
        OR split_part(image_reference, '@', 2) = image_digest
    ),
    CHECK (
        (status = 'ready'
            AND image_reference IS NOT NULL
            AND image_digest IS NOT NULL
            AND provenance IS NOT NULL
            AND failure_reason IS NULL)
        OR (status = 'failed'
            AND image_reference IS NULL
            AND image_digest IS NULL
            AND provenance IS NULL
            AND failure_reason IS NOT NULL)
        OR (status IN ('draft', 'producing')
            AND image_reference IS NULL
            AND image_digest IS NULL
            AND provenance IS NULL
            AND failure_reason IS NULL)
        OR (status = 'retired')
    )
);

CREATE INDEX repository_oci_image_definitions_by_project
    ON repository_oci_image_definitions (project_id, status, key, id);

-- Extend the Mélange tuple projection only after the resource table exists.
-- `melange_base_tuples` is the generated, forward-reference-free base view.
CREATE OR REPLACE VIEW melange_tuples (
    subject_type, subject_id, relation, object_type, object_id
) AS
SELECT subject_type, subject_id, relation, object_type, object_id
FROM melange_base_tuples
UNION ALL
SELECT
    'project', image.project_id::text, 'project',
    'repository_oci_image', image.id::text
FROM repository_oci_image_definitions AS image;

ALTER TABLE repository_oci_image_definitions ENABLE ROW LEVEL SECURITY;
ALTER TABLE repository_oci_image_definitions FORCE ROW LEVEL SECURITY;

CREATE POLICY repository_oci_image_definitions_select
    ON repository_oci_image_definitions FOR SELECT
    USING (check_permission('user', hephaestus_actor_id(), 'can_read',
        'repository_oci_image', id::text) = 1);

CREATE POLICY repository_oci_image_definitions_insert
    ON repository_oci_image_definitions FOR INSERT
    -- The image tuple does not exist until this row is accepted, so creation
    -- is authorized by the explicit Mélange project `can_write` relation;
    -- every subsequent image operation uses the direct image object.
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_write',
        'project', project_id::text) = 1);

CREATE POLICY repository_oci_image_definitions_update
    ON repository_oci_image_definitions FOR UPDATE
    USING (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'repository_oci_image', id::text) = 1)
    WITH CHECK (check_permission('user', hephaestus_actor_id(), 'can_manage',
        'repository_oci_image', id::text) = 1);

GRANT SELECT, INSERT, UPDATE ON repository_oci_image_definitions
    TO hephaestus_app, hephaestus_worker;

-- Enforce that source and output provenance are immutable. The only catalog
-- coupling is that the exact base reference must be an available OCI image at
-- definition time; execution permissions are deliberately not image fields.
CREATE FUNCTION validate_repository_oci_image_definition() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET row_security = off
AS $$
DECLARE
    repository_project_id uuid;
    catalog_reference text;
    catalog_availability text;
    base_changed boolean := false;
BEGIN
    SELECT repository.project_id
      INTO repository_project_id
      FROM repositories AS repository
     WHERE repository.id = NEW.source_repository_id;
    IF repository_project_id IS NULL OR repository_project_id <> NEW.project_id THEN
        RAISE EXCEPTION 'repository OCI image source repository must belong to its project';
    END IF;

    IF TG_OP = 'INSERT'
       OR NEW.base_image_reference IS DISTINCT FROM OLD.base_image_reference THEN
        base_changed := true;
        SELECT image.image_reference, image.availability_state
          INTO catalog_reference, catalog_availability
          FROM oci_images AS image
         WHERE image.image_reference = NEW.base_image_reference;
    END IF;
    IF base_changed
       AND (catalog_reference IS NULL OR catalog_availability <> 'available') THEN
        RAISE EXCEPTION 'repository OCI image base is not an available catalog image';
    END IF;

    IF TG_OP = 'UPDATE' THEN
        IF OLD.status <> 'draft'
           AND (NEW.project_id, NEW.source_repository_id, NEW.key,
                NEW.display_name, NEW.source_revision, NEW.dockerfile_path,
                NEW.context_path, NEW.context_digest, NEW.base_image_reference)
               IS DISTINCT FROM
               (OLD.project_id, OLD.source_repository_id, OLD.key,
                OLD.display_name, OLD.source_revision, OLD.dockerfile_path,
                OLD.context_path, OLD.context_digest, OLD.base_image_reference) THEN
            RAISE EXCEPTION 'repository OCI image source metadata is immutable after draft';
        END IF;
        IF OLD.status IN ('ready', 'retired')
           AND (NEW.image_reference, NEW.image_digest, NEW.provenance)
               IS DISTINCT FROM
               (OLD.image_reference, OLD.image_digest, OLD.provenance) THEN
            RAISE EXCEPTION 'repository OCI image output provenance is immutable';
        END IF;
        IF OLD.status = 'retired' AND NEW.status <> 'retired' THEN
            RAISE EXCEPTION 'retired repository OCI images cannot be reactivated';
        END IF;
    END IF;
    RETURN NEW;
END
$$;

ALTER FUNCTION validate_repository_oci_image_definition() OWNER TO hephaestus_authz_owner;
GRANT EXECUTE ON FUNCTION validate_repository_oci_image_definition()
    TO hephaestus_app, hephaestus_worker;
GRANT SELECT ON repositories, oci_images TO hephaestus_authz_owner;

CREATE TRIGGER validate_repository_oci_image_definition
BEFORE INSERT OR UPDATE ON repository_oci_image_definitions
FOR EACH ROW EXECUTE FUNCTION validate_repository_oci_image_definition();
