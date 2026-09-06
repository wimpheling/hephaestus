# Build scanner tools from released, checksummed source because upstream's
# latest binaries still embed an outdated Go runtime with fixable findings.
FROM docker.io/library/ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90 AS security-tools-build

ARG GO_VERSION=1.26.6
ARG GO_SHA256=708effb774be8237570d0add163225abbdfaf4fca28b2611df167beba4feef89
ARG TRIVY_SOURCE_VERSION=0.74.0
ARG TRIVY_SOURCE_SHA256=04268af574690b84bc3474a5f19e002cd6da3e16899fac9fd39c6e84e7843940
ARG SYFT_SOURCE_VERSION=1.51.1
ARG SYFT_SOURCE_SHA256=da8d83cdca78f2c553e08a5ecb9734016a05adb904168531f582bebfbb9bb2cf

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl tar \
    && rm -rf /var/lib/apt/lists/*

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

FROM docker.io/library/ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ARG CREATED
ARG REVISION
ARG SOURCE
ARG SYFT_VERSION=1.51.1-hephaestus.1
ARG TRIVY_VERSION=0.74.0-hephaestus.1

LABEL org.opencontainers.image.title="Hephaestus OCI verifier operation" \
      org.opencontainers.image.description="Minimal Ubuntu root for isolated repository OCI verification" \
      org.opencontainers.image.source="${SOURCE}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.created="${CREATED}"

# The verifier command and pinned offline vulnerability/SBOM databases are
# installed by the reviewed platform-image release process.  Keep this root
# deliberately free of repository-controlled package managers and runtimes.
COPY --from=security-tools-build /out/trivy /usr/local/bin/trivy
COPY --from=security-tools-build /out/syft /usr/local/bin/syft

RUN test -n "$CREATED" && test -n "$REVISION" && test -n "$SOURCE" \
    && apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates \
        curl \
        jq \
        tar \
        umoci \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir --mode=0700 /var/lib/hephaestus-trivy \
    && TRIVY_CACHE_DIR=/var/lib/hephaestus-trivy trivy image --download-db-only \
    && umoci --version \
    && syft --version | grep --fixed-strings "syft ${SYFT_VERSION}" \
    && trivy --version | grep --fixed-strings "Version: ${TRIVY_VERSION}"

ENV TRIVY_CACHE_DIR=/var/lib/hephaestus-trivy

COPY oci-verify /usr/libexec/hephaestus/oci-verify
RUN chmod 0555 /usr/libexec/hephaestus/oci-verify
