-- Platform-owned immutable OCI image catalog. An image is an artifact, not a
-- build or guest-runtime class. Execution-specific policy belongs to the
-- build or guest contract that selects this catalog key.

CREATE TABLE oci_images (
    id uuid PRIMARY KEY,
    key text NOT NULL UNIQUE CHECK (key ~ '^[a-z0-9][a-z0-9_-]{0,63}$'),
    display_name text NOT NULL CHECK (length(display_name) BETWEEN 1 AND 200),
    image_reference text NOT NULL UNIQUE
        CHECK (image_reference ~ '@sha256:[0-9a-f]{64}$'),
    toolchains jsonb NOT NULL CHECK (jsonb_typeof(toolchains) = 'array'),
    architectures text[] NOT NULL CHECK (cardinality(architectures) > 0),
    availability_state text NOT NULL CHECK (
        availability_state IN ('available', 'unavailable', 'retired')
    ),
    provenance jsonb NOT NULL CHECK (jsonb_typeof(provenance) = 'object'),
    signature_reference text,
    sbom_reference text,
    platform_policy_version text NOT NULL CHECK (
        length(platform_policy_version) BETWEEN 1 AND 128
    ),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX oci_images_by_availability
    ON oci_images (availability_state, key, id);

ALTER TABLE oci_images ENABLE ROW LEVEL SECURITY;
ALTER TABLE oci_images FORCE ROW LEVEL SECURITY;

-- Catalog metadata is platform-owned and non-tenant-specific. RPC handlers
-- authorize callers separately; RLS prevents accidental mutations because no
-- INSERT, UPDATE, or DELETE policy exists for application roles.
CREATE POLICY oci_images_select ON oci_images
    FOR SELECT USING (true);
