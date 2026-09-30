# Purpose

This directory contains the standard OCI image catalog provider. It gives
builders and control-plane callers a PostgreSQL-backed view of immutable image
metadata and the registry evidence needed to decide whether an image is
available.

# Responsibilities

The provider validates stored image keys, references, provenance, roles,
architectures, and availability before returning domain values. It joins
platform image publications to immutable registry references and maps signature,
SBOM, provenance, and scan evidence into the publication state used by callers.

# When

Use the image provider when a builder needs to list or resolve an image, or
when a caller needs the publication and evidence for one image. Give
publication queries an authenticated identity so the authorization transaction
can evaluate the platform image access policy.
