FROM docker.io/library/ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ARG CREATED
ARG REVISION
ARG SOURCE

LABEL org.opencontainers.image.title="Hephaestus Rust Ubuntu builder" \
      org.opencontainers.image.description="Curated Ubuntu builder with Rust and Cargo 1.88.0" \
      org.opencontainers.image.source="${SOURCE}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.created="${CREATED}"

ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo \
    PATH=/opt/cargo/bin:${PATH}

RUN test -n "$CREATED" && test -n "$REVISION" && test -n "$SOURCE" \
    && apt-get update \
    && apt-get install --yes --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        file \
        git \
        pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && curl --fail --location --proto '=https' --tlsv1.2 \
        --output /tmp/rust.tar.xz \
        https://static.rust-lang.org/dist/2025-06-26/rust-1.88.0-x86_64-unknown-linux-gnu.tar.xz \
    && echo '7b5437c1d18a174faae253a18eac22c32288dccfc09ff78d5ee99b7467e21bca  /tmp/rust.tar.xz' | sha256sum --check \
    && mkdir /tmp/rust \
    && tar --extract --xz --file /tmp/rust.tar.xz --directory /tmp/rust --strip-components=1 \
    && /tmp/rust/install.sh --prefix=/opt/rust --without=rust-docs \
    && ln --symbolic /opt/rust/bin/cargo /usr/local/bin/cargo \
    && ln --symbolic /opt/rust/bin/rustc /usr/local/bin/rustc \
    && ln --symbolic /opt/rust/bin/rustfmt /usr/local/bin/rustfmt \
    && rm -rf /tmp/rust /tmp/rust.tar.xz \
    && test "$(rustc --version)" = 'rustc 1.88.0 (6b00bc388 2025-06-23)' \
    && test "$(cargo --version)" = 'cargo 1.88.0 (873a06493 2025-05-10)'

WORKDIR /workspace
