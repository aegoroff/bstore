# docker image build --tag egoroff/bstore .

# Build service
FROM rust:latest as rust-build
RUN apt update && apt -y install lld musl-tools
COPY .cargo/ ./.cargo/
COPY bstore/ ./bstore/
COPY client/ ./client/
COPY server/ ./server/
COPY kernel/ ./kernel/
COPY Cargo.toml ./
RUN rustup target add x86_64-unknown-linux-musl
RUN cargo test --workspace --target x86_64-unknown-linux-musl
RUN cargo build --workspace --target x86_64-unknown-linux-musl --release

FROM gcr.io/distroless/static-debian12:latest
ENV BSTORE_PORT=5000
ENV BSTORE_DATA_DIR=/data/data
ENV BSTORE_DATA_FILE=bstore.db
COPY --from=rust-build /target/x86_64-unknown-linux-musl/release/bstore /usr/local/bin/bstore
ENTRYPOINT [ "/usr/local/bin/bstore" ]
CMD [ "server" ]
EXPOSE 5000
