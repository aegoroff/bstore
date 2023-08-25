# docker image build --tag egoroff/bstore .

# Build service
FROM rust:bullseye as rust-build
RUN apt update && apt -y install lld
COPY .cargo/ ./.cargo/
COPY bstore/ ./bstore/
COPY client/ ./client/
COPY server/ ./server/
COPY kernel/ ./kernel/
COPY Cargo.toml ./
RUN cargo test --workspace
RUN cargo build --workspace --release

FROM gcr.io/distroless/cc-debian11:latest
ENV BSTORE_PORT=5000
ENV BSTORE_DATA_DIR=/data/data
ENV BSTORE_DATA_FILE=bstore.db
COPY --from=rust-build /target/release/bstore /usr/local/bin/bstore
ENTRYPOINT [ "/usr/local/bin/bstore" ]
CMD [ "server" ]
EXPOSE 5000
