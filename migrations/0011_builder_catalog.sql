-- Platform-owned digest-pinned builder image catalog.
-- Entries are installed by a separately reviewed platform provisioning flow;
-- this migration intentionally creates no placeholder or environment-derived
-- images.

CREATE TABLE builder_images (
    id uuid PRIMARY KEY,
    key text NOT NULL UNIQUE CHECK (key ~ '^[a-z0-9][a-z0-9_-]{0,63}$'),
    display_name text NOT NULL CHECK (length(display_name) BETWEEN 1 AND 200),
    image_reference text NOT NULL UNIQUE
        CHECK (image_reference ~ '@sha256:[0-9a-f]{64}$'),
    toolchains jsonb NOT NULL CHECK (jsonb_typeof(toolchains) = 'array'),
    architectures text[] NOT NULL CHECK (cardinality(architectures) > 0),
    preparation_state text NOT NULL CHECK (
        preparation_state IN ('ready', 'preparing', 'failed')
    ),
    availability_state text NOT NULL CHECK (
        availability_state IN ('available', 'unavailable', 'retired')
    ),
    network_ceiling text NOT NULL CHECK (
        network_ceiling IN ('disabled', 'broker_only', 'egress')
    ),
    max_vcpus smallint NOT NULL CHECK (max_vcpus BETWEEN 1 AND 64),
    max_memory_mib integer NOT NULL CHECK (max_memory_mib BETWEEN 128 AND 1048576),
    dependency_policy text NOT NULL CHECK (
        dependency_policy IN (
            'vendored_offline',
            'read_only_platform_cache',
            'constrained_registry_egress'
        )
    ),
    provenance jsonb NOT NULL CHECK (jsonb_typeof(provenance) = 'object'),
    signature_reference text,
    sbom_reference text,
    platform_policy_version text NOT NULL CHECK (length(platform_policy_version) BETWEEN 1 AND 128),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX builder_images_by_availability
    ON builder_images (availability_state, preparation_state, key, id);

ALTER TABLE builder_images ENABLE ROW LEVEL SECURITY;
ALTER TABLE builder_images FORCE ROW LEVEL SECURITY;

-- Catalog metadata is platform-owned and non-tenant-specific. RPC handlers
-- still authorize the caller before invoking this read adapter; RLS prevents
-- accidental writes because no INSERT/UPDATE/DELETE policy is declared.
CREATE POLICY builder_images_select ON builder_images
    FOR SELECT USING (true);
