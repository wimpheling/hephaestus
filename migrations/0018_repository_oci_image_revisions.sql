-- Repository-local OCI image definitions are selected from an exact source
-- commit. A repository can therefore retain several auditable immutable
-- definitions for a single user-facing key.

ALTER TABLE repository_oci_image_definitions
    DROP CONSTRAINT repository_oci_image_definitions_project_id_key_key;

CREATE UNIQUE INDEX repository_oci_image_definitions_repository_revision
    ON repository_oci_image_definitions (
        source_repository_id,
        key,
        source_revision,
        context_digest,
        base_image_reference
    );

CREATE INDEX repository_oci_image_definitions_by_repository_key
    ON repository_oci_image_definitions (
        source_repository_id,
        key,
        source_revision,
        status,
        id
    );
