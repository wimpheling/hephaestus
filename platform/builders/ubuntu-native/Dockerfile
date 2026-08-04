FROM docker.io/library/ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ARG CREATED
ARG REVISION
ARG SOURCE

LABEL org.opencontainers.image.title="Hephaestus Ubuntu native builder" \
      org.opencontainers.image.description="Curated Ubuntu builder for shell and native agent release builds" \
      org.opencontainers.image.source="${SOURCE}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.created="${CREATED}"

ENV DEBIAN_FRONTEND=noninteractive

RUN test -n "$CREATED" && test -n "$REVISION" && test -n "$SOURCE" \
    && apt-get update \
    && apt-get install --yes --no-install-recommends \
        bash \
        build-essential \
        ca-certificates \
        file \
        git \
        make \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace
