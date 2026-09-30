# Purpose

`builder-catalog-postgres` implements the PostgreSQL image catalog used by
builders and platform operations. It returns validated OCI image metadata and,
when requested, the exact registry publication and evidence associated with a
platform image.

# Responsibilities

`PgOciImageCatalog` lists, loads, and reference-resolves images from the
catalog, converting every persisted enum, reference, provenance value, and
policy version through domain validation. Publication queries run with the
authenticated actor transaction and require the stored immutable registry
reference to match the requested image reference; required signatures and
other evidence are represented as typed publication state.

# When

Construct `PgOciImageCatalog` with the application `PgPool` and inject it as an
`ImageCatalog` or `RegistryPublicationCatalog`. Use `get_image` or
`find_image_by_reference` for a validated image, then use the publication calls
when a build or platform operation must verify registry availability and
evidence.
