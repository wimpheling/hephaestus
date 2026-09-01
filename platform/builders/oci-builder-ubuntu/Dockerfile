FROM docker.io/library/ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ARG CREATED
ARG REVISION
ARG SOURCE

LABEL org.opencontainers.image.title="Hephaestus OCI builder operation" \
      org.opencontainers.image.description="Minimal Ubuntu root for isolated repository OCI layout construction" \
      org.opencontainers.image.source="${SOURCE}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.created="${CREATED}"

# This root is a platform operation image. It is never exposed as a project
# Dockerfile base or ordinary execution image; Buildah needs only its local
# storage and the read-only inputs mounted by the one-shot microVM.
RUN test -n "$CREATED" && test -n "$REVISION" && test -n "$SOURCE" \
    && apt-get update \
    && apt-get install --yes --no-install-recommends \
        buildah \
        ca-certificates \
        fuse-overlayfs \
        uidmap \
    && rm -rf /var/lib/apt/lists/* \
    # `run-local.sh` adds the fixed UID 10001 account after materialization.
    # Buildah resolves the executing account by numeric UID, so reserve both
    # forms without pre-creating that account in this immutable operation root.
    && printf '%s\n' 'heph-agent:100000:65536' '10001:100000:65536' > /etc/subuid \
    && printf '%s\n' 'heph-agent:100000:65536' '10001:100000:65536' > /etc/subgid \
    && buildah --version

COPY oci-build /usr/libexec/hephaestus/oci-build
RUN chmod 0555 /usr/libexec/hephaestus/oci-build
