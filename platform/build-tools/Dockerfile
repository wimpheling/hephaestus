# Administrator-built tool image for explicit local platform-image operations.
# It is never used by normal `cargo dev` startup.
FROM registry.fedoraproject.org/fedora-minimal@sha256:8f42d200f04990b41081322d1c260ddf23b124b3b92538665ef4cc3064537249

ARG SYFT_VERSION=1.50.0
ARG SYFT_SHA256=bf7b29ff57f06da30918266a0e1c2885a8f99784798d1bdb1628886aa015d788
ARG TRIVY_VERSION=0.73.0
ARG TRIVY_SHA256=2edd39da482bb4e9831962487b68f68e3928ec3137794757f54d00383d79547b
ARG ORAS_VERSION=1.3.3
ARG ORAS_SHA256=9ce999f8d2de03fc03968b29d743077a58783e545e5eaa53917ca177352d0e59

RUN microdnf install --assumeyes \
        buildah \
        ca-certificates \
        curl \
        findutils \
        gzip \
        jq \
        shadow-utils \
        skopeo \
        tar \
    && microdnf clean all \
    && buildah --version | grep --fixed-strings 'buildah version 1.43.2' \
    && skopeo --version | grep --fixed-strings 'skopeo version 1.22.2' \
    && jq --version | grep --fixed-strings 'jq-1.8.1'

RUN set -eux; \
    curl --fail --location --proto '=https' --tlsv1.2 \
        --output /tmp/syft.tar.gz \
        "https://github.com/anchore/syft/releases/download/v${SYFT_VERSION}/syft_${SYFT_VERSION}_linux_amd64.tar.gz"; \
    echo "${SYFT_SHA256}  /tmp/syft.tar.gz" | sha256sum --check; \
    tar --extract --gzip --file /tmp/syft.tar.gz --directory /usr/local/bin syft; \
    curl --fail --location --proto '=https' --tlsv1.2 \
        --output /tmp/trivy.tar.gz \
        "https://github.com/aquasecurity/trivy/releases/download/v${TRIVY_VERSION}/trivy_${TRIVY_VERSION}_Linux-64bit.tar.gz"; \
    echo "${TRIVY_SHA256}  /tmp/trivy.tar.gz" | sha256sum --check; \
    tar --extract --gzip --file /tmp/trivy.tar.gz --directory /usr/local/bin trivy; \
    curl --fail --location --proto '=https' --tlsv1.2 \
        --output /tmp/oras.tar.gz \
        "https://github.com/oras-project/oras/releases/download/v${ORAS_VERSION}/oras_${ORAS_VERSION}_linux_amd64.tar.gz"; \
    echo "${ORAS_SHA256}  /tmp/oras.tar.gz" | sha256sum --check; \
    tar --extract --gzip --file /tmp/oras.tar.gz --directory /usr/local/bin oras; \
    rm --force /tmp/syft.tar.gz /tmp/trivy.tar.gz /tmp/oras.tar.gz; \
    syft --version | grep --fixed-strings "syft ${SYFT_VERSION}"; \
    trivy --version | grep --fixed-strings "Version: ${TRIVY_VERSION}"; \
    oras version | grep --fixed-strings "Version:        ${ORAS_VERSION}"

COPY entrypoint.sh /usr/local/bin/hephaestus-platform-tools
RUN chmod 0555 /usr/local/bin/hephaestus-platform-tools

ENV HOME=/tmp/hephaestus-platform-builder \
    XDG_CACHE_HOME=/tmp/hephaestus-platform-builder/cache \
    TRIVY_CACHE_DIR=/tmp/hephaestus-platform-builder/cache/trivy

ENTRYPOINT ["/usr/local/bin/hephaestus-platform-tools"]
