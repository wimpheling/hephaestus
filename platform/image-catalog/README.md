# Reviewed platform OCI image catalog

The four initial Ubuntu OCI images are built from the Dockerfiles in the
adjacent `../builders` directory. A release never substitutes a tag for the
immutable OCI digest that the forge registry publishes.

Run the reviewed platform-image release operation on the reviewed
commit. It builds and scans every image for `linux/amd64`, pushes each to the
GitHub Container Registry, attaches provenance and SBOM attestations, and
uploads `platform-image-catalog.json`. Review that artifact alongside its
attestations. The artifact is the exact input to:

```sh
HEPHAESTUS_DATABASE_URL=... \
cargo run -p bootstrap-postgres --bin hephaestus-operator -- \
  provision-image-catalog platform-image-catalog.json
```

Do not commit a manifest with made-up digests, and do not run the provisioning
command until the image digests, scan, and attestations have been reviewed.
The repository deliberately contains the release sources and automation rather
than a fictional production manifest.
