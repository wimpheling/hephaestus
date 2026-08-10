FROM docker.io/library/ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ARG CREATED
ARG REVISION
ARG SOURCE

LABEL org.opencontainers.image.title="Hephaestus Python Ubuntu builder" \
      org.opencontainers.image.description="Curated Ubuntu builder with CPython 3.13.5 and pip 25.1.1" \
      org.opencontainers.image.source="${SOURCE}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.created="${CREATED}"

ENV DEBIAN_FRONTEND=noninteractive \
    PATH=/opt/python/bin:${PATH}

RUN test -n "$CREATED" && test -n "$REVISION" && test -n "$SOURCE" \
    && apt-get update \
    && apt-get install --yes --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        file \
        git \
        libbz2-dev \
        libffi-dev \
        libgdbm-dev \
        liblzma-dev \
        libncursesw5-dev \
        libreadline-dev \
        libsqlite3-dev \
        libssl-dev \
        zlib1g-dev \
    && rm -rf /var/lib/apt/lists/* \
    && curl --fail --location --proto '=https' --tlsv1.2 \
        --output /tmp/python.tgz \
        https://www.python.org/ftp/python/3.13.5/Python-3.13.5.tgz \
    && echo 'e6190f52699b534ee203d9f417bdbca05a92f23e35c19c691a50ed2942835385  /tmp/python.tgz' | sha256sum --check \
    && mkdir /tmp/python \
    && tar --extract --gzip --file /tmp/python.tgz --directory /tmp/python --strip-components=1 \
    && cd /tmp/python \
    && ./configure --prefix=/opt/python --with-ensurepip=install \
    && make --jobs="$(nproc)" \
    && make install \
    && /opt/python/bin/python3 -m pip install --no-cache-dir pip==25.1.1 \
    && cd / \
    && rm -rf /tmp/python /tmp/python.tgz \
    && ln --symbolic /opt/python/bin/python3 /usr/local/bin/python3 \
    && ln --symbolic /opt/python/bin/pip3 /usr/local/bin/pip3 \
    && test "$(python3 --version)" = 'Python 3.13.5' \
    && test "$(python3 -m pip --version | awk '{print $2}')" = '25.1.1'

WORKDIR /workspace
