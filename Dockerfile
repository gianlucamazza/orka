# syntax=docker/dockerfile:1

# --- Base: shared stage with cargo-chef + build deps ---
FROM rust:1.93-slim-bookworm AS chef
RUN cargo install cargo-chef
RUN apt-get update && apt-get install -y pkg-config libssl-dev curl g++ && rm -rf /var/lib/apt/lists/*
WORKDIR /app

# --- Planner: generate dependency recipe ---
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# --- Cook: build release dependencies (cached) ---
FROM chef AS cook
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo chef cook --release --recipe-path recipe.json

# --- Cook dev: build debug dependencies (cached) ---
FROM chef AS cook-dev
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo chef cook --recipe-path recipe.json

# --- Builder: compile workspace crates (release) ---
FROM cook AS builder
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release --bin orka-server

# --- Builder dev: compile workspace crates (debug) ---
FROM cook-dev AS builder-dev
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo install cargo-watch
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --bin orka-server

# --- Dev runtime: source + cargo-watch for hot-reload ---
FROM chef AS dev
COPY --from=builder-dev /usr/local/cargo/bin/cargo-watch /usr/local/cargo/bin/cargo-watch
COPY . .
CMD ["cargo-watch", "-x", "run -p orka-server"]

# --- Prod runtime ---
FROM debian:bookworm-slim

ARG BUILD_DATE
ARG VCS_REF
ARG VERSION

LABEL org.opencontainers.image.created="${BUILD_DATE}" \
      org.opencontainers.image.revision="${VCS_REF}" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.title="orka-server" \
      org.opencontainers.image.source="https://github.com/gianlucamazza/orka"

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 libstdc++6 curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -s /usr/sbin/nologin orka

COPY --from=builder /app/target/release/orka-server /usr/local/bin/orka-server

RUN mkdir -p /workspaces && chown orka:orka /workspaces

USER orka

EXPOSE 8080 8081

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://localhost:8080/health/live || exit 1

STOPSIGNAL SIGTERM

ENTRYPOINT ["orka-server"]
