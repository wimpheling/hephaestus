-- OCI images used to construct or verify other OCI images are trusted platform
-- operations, not tenant-selectable execution roots. Keeping the role on the
-- immutable catalog record lets every selection path fail closed.

ALTER TABLE oci_images
    ADD COLUMN role text NOT NULL DEFAULT 'execution' CHECK (
        role IN ('execution', 'platform_operation')
    );

CREATE INDEX oci_images_by_role_availability
    ON oci_images (role, availability_state, key, id);
