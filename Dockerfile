# syntax=docker/dockerfile:1.7
#
# Built on Linux. The media plane links libwebrtc through the LiveKit SDK, which
# does not link on Windows/MSVC — the deployment target is Linux and this is the
# canonical build.

FROM rust:1.95-bookworm AS chef-base
WORKDIR /workspace
RUN cargo install cargo-chef --locked

FROM chef-base AS planner
COPY . /workspace
RUN cargo chef prepare --recipe-path recipe.json

FROM chef-base AS deps
COPY --from=planner /workspace/recipe.json recipe.json
RUN --mount=type=cache,id=sonari-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=sonari-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=sonari-target,target=/workspace/target \
    cargo chef cook --release --recipe-path recipe.json

FROM chef-base AS builder
COPY . /workspace
RUN --mount=type=cache,id=sonari-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=sonari-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=sonari-target,target=/workspace/target \
    cargo build --release -p app \
    && mkdir -p /artifacts \
    && cp /workspace/target/release/app /artifacts/sonari

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libstdc++6 libglib2.0-0 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /workspace/crates/platform/postgres/migrations /workspace/crates/platform/postgres/migrations
COPY --from=builder /artifacts/sonari /usr/local/bin/sonari
ENTRYPOINT ["/usr/local/bin/sonari"]
