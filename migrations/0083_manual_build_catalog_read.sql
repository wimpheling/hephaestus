-- Manual build identity resolution reads the existing platform-owned catalog.
-- The table remains FORCE RLS with its immutable, public-catalog SELECT policy;
-- this grant only permits the application resolver to apply its available,
-- execution-image filter.
GRANT SELECT ON oci_images TO hephaestus_app;
