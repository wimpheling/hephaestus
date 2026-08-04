-- Repository-local OCI builder revisions.
--
-- A key is selected from the repository's exact source commit, so a project
-- may retain several auditable revisions for the same key. The old project
-- key uniqueness incorrectly collapsed those immutable revisions.

ALTER TABLE project_builder_definitions
    DROP CONSTRAINT project_builder_definitions_project_id_key_key;

CREATE UNIQUE INDEX project_builder_definitions_repository_revision
    ON project_builder_definitions (
        source_repository_id,
        key,
        source_revision,
        context_digest,
        approved_base_image_reference
    );

CREATE INDEX project_builder_definitions_by_repository_key
    ON project_builder_definitions (
        source_repository_id,
        key,
        source_revision,
        status,
        id
    );
