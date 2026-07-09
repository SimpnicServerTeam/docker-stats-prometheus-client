FROM --platform=$BUILDPLATFORM cts/rust-aarch64-linux-gnu:1.96 AS chef
ARG TARGETARCH
ARG BUILDPLATFORM
RUN echo "Host platform: $BUILDPLATFORM, target arch: $TARGETARCH"
VOLUME ["/usr/app"]
WORKDIR /usr/app

# create recipe
FROM chef AS planner
COPY ./build.rs ./build.rs
COPY ./src/main.rs ./src/main.rs
COPY Cargo.toml Cargo.lock ./
COPY .cargo/config.toml .cargo/config.toml
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG TARGETARCH
RUN echo "target arch: $TARGETARCH"
COPY --from=planner /usr/app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=target-${TARGETARCH},target=/usr/app/target \
    if [ "$TARGETARCH" = "arm64" ]; then \
      export CC=aarch64-linux-gnu-gcc; \
      cargo chef cook --release --target aarch64-unknown-linux-gnu --recipe-path recipe.json; \
    else \
      cargo chef cook --release --recipe-path recipe.json; \
    fi

# build application
COPY . .
RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=target-${TARGETARCH},target=/usr/app/target \
    if [ "$TARGETARCH" = "arm64" ]; then \
        export CC=aarch64-linux-gnu-gcc; \
        cargo build --release --target aarch64-unknown-linux-gnu && \
        cp target/aarch64-unknown-linux-gnu/release/docker-stat-exporter /usr/app/binary; \
    else \
        cargo build --release && \
        mv target/release/docker-stat-exporter /usr/app/binary; \
    fi

# extract binary
FROM debian:trixie-slim
WORKDIR /usr/local/sbin
COPY --from=builder /usr/app/binary ./docker-stat-exporter
ENTRYPOINT ["docker-stat-exporter"]
