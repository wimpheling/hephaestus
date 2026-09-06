-- A build contract may pin one ready project-owned OCI image in addition to a
-- reviewed catalog image. The immutable reference remains the execution
-- authority; the source column records which resource supplied that reference
-- for audit and release provenance.

ALTER TABLE build_request_images
    DROP CONSTRAINT build_request_images_image_id_fkey,
    ALTER COLUMN image_id DROP NOT NULL,
    ADD COLUMN repository_oci_image_id uuid
        REFERENCES repository_oci_image_definitions(id),
    ADD CONSTRAINT build_request_images_exactly_one_image_source
        CHECK (num_nonnulls(image_id, repository_oci_image_id) = 1);

CREATE INDEX build_request_images_by_repository_oci_image
    ON build_request_images (repository_oci_image_id)
    WHERE repository_oci_image_id IS NOT NULL;
