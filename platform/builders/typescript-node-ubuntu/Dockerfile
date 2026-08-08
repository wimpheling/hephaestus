FROM docker.io/library/ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ARG CREATED
ARG REVISION
ARG SOURCE

LABEL org.opencontainers.image.title="Hephaestus TypeScript Node Ubuntu builder" \
      org.opencontainers.image.description="Curated Ubuntu builder with Node 24.19.0 LTS, pnpm, and TypeScript" \
      org.opencontainers.image.source="${SOURCE}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.created="${CREATED}"

ENV DEBIAN_FRONTEND=noninteractive \
    PATH=/opt/node/bin:${PATH}

# npm is used only by this trusted construction stage. The shipped Node agent
# toolchain uses pnpm; removing npm also removes its archive/network dependency
# graph from the release image.
RUN test -n "$CREATED" && test -n "$REVISION" && test -n "$SOURCE" \
    && apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl file git xz-utils \
    && rm -rf /var/lib/apt/lists/* \
    && curl --fail --location --proto '=https' --tlsv1.2 \
        --output /tmp/node.tar.xz \
        https://nodejs.org/dist/v24.19.0/node-v24.19.0-linux-x64.tar.xz \
    && echo '14b342e71204f811bde6153be8e04b62aef63c236fef92b55f9c83154b409647  /tmp/node.tar.xz' | sha256sum --check \
    && mkdir /opt/node \
    && tar --extract --xz --file /tmp/node.tar.xz --directory /opt/node --strip-components=1 \
    && rm /tmp/node.tar.xz \
    && npm install --global --no-audit --no-fund \
        pnpm@11.20.0 \
        typescript@5.9.3 \
    && rm /opt/node/bin/npm /opt/node/bin/npx \
    && rm -rf /opt/node/lib/node_modules/npm \
    && test "$(node --version)" = 'v24.19.0' \
    && test "$(pnpm --version)" = '11.20.0' \
    && test "$(tsc --version)" = 'Version 5.9.3'

WORKDIR /workspace
