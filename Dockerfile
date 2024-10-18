# docker image build --tag egoroff/bstore .

# Build service
FROM rust:latest AS rust-build
RUN apt update && apt -y install lld musl-tools cmake
COPY .cargo/ ./.cargo/
COPY bstore/ ./bstore/
COPY client/ ./client/
COPY server/ ./server/
COPY kernel/ ./kernel/
COPY Cargo.toml ./
RUN rustup target add x86_64-unknown-linux-musl && \
    cargo test --workspace --target x86_64-unknown-linux-musl && \
    cargo build --workspace --target x86_64-unknown-linux-musl --release

FROM gcr.io/distroless/static-debian12:latest
ENV BSTORE_PORT=5000 \
    BSTORE_DATA_DIR=/data/data \
    BSTORE_DATA_FILE=bstore.db
COPY --from=rust-build /target/x86_64-unknown-linux-musl/release/bstore /usr/local/bin/bstore
ENTRYPOINT [ "/usr/local/bin/bstore" ]
CMD [ "server" ]
EXPOSE 5000
