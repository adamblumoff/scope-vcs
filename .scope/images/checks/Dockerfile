# syntax=docker/dockerfile:1.7

FROM --platform=linux/amd64 rust:1.94.0-bookworm@sha256:4673f78db88b71f09d5451bbc404734807918161241215ba0a50bbbe9b448117 AS scope-runtime-builder

WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY api api
COPY worker worker
COPY runner-runtime runner-runtime
RUN cargo build --locked --release -p scope-runner-runtime

FROM --platform=linux/amd64 rust:1.94.0-bookworm@sha256:4673f78db88b71f09d5451bbc404734807918161241215ba0a50bbbe9b448117

ARG NODE_VERSION=24.18.0
ARG NODE_SHA256=55aa7153f9d88f28d765fcdad5ae6945b5c0f98a36881703817e4c450fa76742
ARG PNPM_VERSION=11.1.0
ARG POSTGRES_VERSION=15.18-0+deb12u1
ARG PLAYWRIGHT_VERSION=1.61.1
ARG DEBIAN_SNAPSHOT=20260518T000000Z

ENV DEBIAN_FRONTEND=noninteractive \
    PLAYWRIGHT_BROWSERS_PATH=/root/.cache/ms-playwright \
    RUSTUP_TOOLCHAIN=1.94.0-x86_64-unknown-linux-gnu

COPY --from=scope-runtime-builder /build/target/release/scope-runner-runtime /scope/bin/scope-runner-runtime

RUN sed -i \
        -e "s|http://deb.debian.org/debian-security|https://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}|" \
        -e "s|http://deb.debian.org/debian|https://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}|" \
        /etc/apt/sources.list.d/debian.sources \
    && printf '%s\n' 'Acquire::Check-Valid-Until "false";' > /etc/apt/apt.conf.d/99snapshot \
    && apt-get update \
    && apt-get install --yes --no-install-recommends \
        iproute2 \
        "postgresql-15=${POSTGRES_VERSION}" \
        "postgresql-client-15=${POSTGRES_VERSION}" \
    && rm -rf /var/lib/apt/lists/*

RUN curl --fail --silent --show-error --retry 3 --retry-all-errors --retry-delay 2 \
        "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
        --output /tmp/node.tar.xz \
    && echo "${NODE_SHA256}  /tmp/node.tar.xz" | sha256sum --check --strict \
    && tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 \
    && rm /tmp/node.tar.xz \
    && corepack enable \
    && corepack prepare "pnpm@${PNPM_VERSION}" --activate

RUN rustup component add \
        --toolchain "${RUSTUP_TOOLCHAIN}" \
        rustfmt \
        clippy \
    && npm install --global "playwright@${PLAYWRIGHT_VERSION}" \
    && playwright install --with-deps chromium \
    && npm cache clean --force \
    && rm -rf /var/lib/apt/lists/* /tmp/*

ARG PLAYWRIGHT_CHROMIUM_VERSION=149.0.7827.55
ARG PLAYWRIGHT_CHROMIUM_REVISION=1228

RUN test "$(rustc --version | cut -d' ' -f2)" = "1.94.0" \
    && test -x /scope/bin/scope-runner-runtime \
    && test "$(node --version)" = "v${NODE_VERSION}" \
    && test "$(pnpm --version)" = "${PNPM_VERSION}" \
    && test "$(psql --version | cut -d' ' -f3)" = "15.18" \
    && test "$(playwright --version)" = "Version ${PLAYWRIGHT_VERSION}" \
    && test -x "${PLAYWRIGHT_BROWSERS_PATH}/chromium-${PLAYWRIGHT_CHROMIUM_REVISION}/chrome-linux64/chrome" \
    && "${PLAYWRIGHT_BROWSERS_PATH}/chromium-${PLAYWRIGHT_CHROMIUM_REVISION}/chrome-linux64/chrome" --version \
        | grep --quiet "^Google Chrome for Testing ${PLAYWRIGHT_CHROMIUM_VERSION} *$"

WORKDIR /workspace
