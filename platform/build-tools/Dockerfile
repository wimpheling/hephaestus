# Build scanner tools from released, checksummed source because upstream's
# latest binaries still embed an outdated Go runtime with fixable findings.
FROM registry.fedoraproject.org/fedora-minimal@sha256:8f42d200f04990b41081322d1c260ddf23b124b3b92538665ef4cc3064537249 AS security-tools-build

ARG GO_VERSION=1.26.6
ARG GO_SHA256=708effb774be8237570d0add163225abbdfaf4fca28b2611df167beba4feef89
ARG TRIVY_SOURCE_VERSION=0.74.0
ARG TRIVY_SOURCE_SHA256=04268af574690b84bc3474a5f19e002cd6da3e16899fac9fd39c6e84e7843940
ARG SYFT_SOURCE_VERSION=1.51.1
ARG SYFT_SOURCE_SHA256=da8d83cdca78f2c553e08a5ecb9734016a05adb904168531f582bebfbb9bb2cf

RUN microdnf install --assumeyes ca-certificates curl gzip tar \
    && microdnf clean all

RUN set -eux; \
    curl --fail --location --proto '=https' --tlsv1.2 \
        --output /tmp/go.tar.gz "https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz"; \
    echo "${GO_SHA256}  /tmp/go.tar.gz" | sha256sum --check; \
    tar --extract --gzip --file /tmp/go.tar.gz --directory /opt; \
    curl --fail --location --proto '=https' --tlsv1.2 \
        --output /tmp/trivy.tar.gz "https://github.com/aquasecurity/trivy/archive/refs/tags/v${TRIVY_SOURCE_VERSION}.tar.gz"; \
    echo "${TRIVY_SOURCE_SHA256}  /tmp/trivy.tar.gz" | sha256sum --check; \
    tar --extract --gzip --file /tmp/trivy.tar.gz --directory /tmp; \
    curl --fail --location --proto '=https' --tlsv1.2 \
        --output /tmp/syft.tar.gz "https://github.com/anchore/syft/archive/refs/tags/v${SYFT_SOURCE_VERSION}.tar.gz"; \
    echo "${SYFT_SOURCE_SHA256}  /tmp/syft.tar.gz" | sha256sum --check; \
    tar --extract --gzip --file /tmp/syft.tar.gz --directory /tmp

ENV PATH=/opt/go/bin:${PATH} \
    CGO_ENABLED=0 \
    GOEXPERIMENT=jsonv2

RUN cd "/tmp/trivy-${TRIVY_SOURCE_VERSION}" \
    && go build --mod=readonly --trimpath --buildvcs=false \
        -ldflags='-s -w -X github.com/aquasecurity/trivy/pkg/version/app.ver=0.74.0-hephaestus.1' \
    -o /out/trivy ./cmd/trivy

RUN cd "/tmp/syft-${SYFT_SOURCE_VERSION}" \
    && go build --mod=readonly --trimpath --buildvcs=false \
        -ldflags='-s -w -X main.version=1.51.1-hephaestus.1' \
        -o /out/syft ./cmd/syft

# Administrator-built tool image for explicit local platform-image operations.
# It is never used by normal `cargo dev` startup.
FROM registry.fedoraproject.org/fedora-minimal@sha256:8f42d200f04990b41081322d1c260ddf23b124b3b92538665ef4cc3064537249

ARG SYFT_VERSION=1.51.1-hephaestus.1
ARG TRIVY_VERSION=0.74.0-hephaestus.1
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

COPY --from=security-tools-build /out/trivy /usr/local/bin/trivy
COPY --from=security-tools-build /out/syft /usr/local/bin/syft

RUN set -eux; \
    curl --fail --location --proto '=https' --tlsv1.2 \
        --output /tmp/oras.tar.gz \
        "https://github.com/oras-project/oras/releases/download/v${ORAS_VERSION}/oras_${ORAS_VERSION}_linux_amd64.tar.gz"; \
    echo "${ORAS_SHA256}  /tmp/oras.tar.gz" | sha256sum --check; \
    tar --extract --gzip --file /tmp/oras.tar.gz --directory /usr/local/bin oras; \
    rm --force /tmp/oras.tar.gz; \
    syft --version | grep --fixed-strings "syft ${SYFT_VERSION}"; \
    trivy --version | grep --fixed-strings "Version: ${TRIVY_VERSION}"; \
    oras version | grep --fixed-strings "Version:        ${ORAS_VERSION}"

COPY entrypoint.sh /usr/local/bin/hephaestus-platform-tools
RUN chmod 0555 /usr/local/bin/hephaestus-platform-tools

ENV HOME=/tmp/hephaestus-platform-builder \
    XDG_CACHE_HOME=/tmp/hephaestus-platform-builder/cache \
    TRIVY_CACHE_DIR=/tmp/hephaestus-platform-builder/cache/trivy

ENTRYPOINT ["/usr/local/bin/hephaestus-platform-tools"]
