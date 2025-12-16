FROM --platform=$BUILDPLATFORM cts/rust-aarch64-linux-gnu:1.87 AS appbuild
ARG TARGETPLATFORM
ARG BUILDPLATFORM
RUN echo "Host platform: $BUILDPLATFORM, image platform: $TARGETPLATFORM"
VOLUME ["/usr/app"]
WORKDIR /usr/app

# make dependency cache
COPY ./.cargo ./.cargo
COPY Cargo.toml ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
ENV CC="aarch64-linux-gnu-gcc"
RUN cargo build --release --target aarch64-unknown-linux-gnu || true

# build actual sources
COPY ./src/ ./src/
ENV CC="aarch64-linux-gnu-gcc"
RUN cargo build --release --target aarch64-unknown-linux-gnu

# extract binary
FROM debian:trixie-slim
COPY --from=appbuild /usr/app/target/aarch64-unknown-linux-gnu/release/docker-stat-prom /usr/local/sbin/docker-stat-prom
CMD ["docker-stat-prom"]
