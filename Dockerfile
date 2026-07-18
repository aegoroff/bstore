# Multi-arch image: linux/amd64 + linux/arm64 via buildx (see docker.sh).
# Rust compile on BUILDPLATFORM; final stage matches TARGETPLATFORM.

# --- amd64: native musl build on Alpine ---
FROM --platform=$BUILDPLATFORM rust:alpine AS rust-build-amd64
WORKDIR /bstore
RUN apk add musl-dev lld cmake make curl ca-certificates && update-ca-certificates
COPY .cargo/ ./.cargo/
COPY bstore/ ./bstore/
COPY client/ ./client/
COPY server/ ./server/
COPY kernel/ ./kernel/
COPY Cargo.toml Cargo.lock ./
RUN rustup target add x86_64-unknown-linux-musl && \
    cargo build -p bstore --release --target x86_64-unknown-linux-musl --locked && \
    mkdir -p /out && \
    cp target/x86_64-unknown-linux-musl/release/bstore /out/bstore

# --- arm64: cross-compile with rust-musl-cross ---
FROM --platform=$BUILDPLATFORM ghcr.io/rust-cross/rust-musl-cross:aarch64-musl AS rust-build-arm64
RUN rustup toolchain uninstall stable-x86_64-unknown-linux-gnu && \
    rustup toolchain install stable-x86_64-unknown-linux-gnu && \
    rustup update stable && \
    rustup target add aarch64-unknown-linux-musl
WORKDIR /bstore
COPY .cargo/ ./.cargo/
COPY bstore/ ./bstore/
COPY client/ ./client/
COPY server/ ./server/
COPY kernel/ ./kernel/
COPY Cargo.toml Cargo.lock ./
RUN cargo build -p bstore --release --target aarch64-unknown-linux-musl --locked && \
    mkdir -p /out && \
    cp target/aarch64-unknown-linux-musl/release/bstore /out/bstore

ARG TARGETARCH
FROM rust-build-${TARGETARCH} AS rust-build

FROM gcr.io/distroless/static-debian13:latest
ENV BSTORE_PORT=5000 \
    BSTORE_DATA_DIR=/data/data \
    BSTORE_DATA_FILE=bstore.db
COPY --from=rust-build /out/bstore /usr/local/bin/bstore
ENTRYPOINT [ "/usr/local/bin/bstore" ]
CMD [ "server" ]
EXPOSE 5000
