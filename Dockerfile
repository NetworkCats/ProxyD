# Build stage with Chainguard Rust image for supply chain security
FROM cgr.dev/chainguard/rust:latest-dev AS builder

USER root
RUN apk add --no-cache protobuf-dev

WORKDIR /app

COPY Cargo.toml Cargo.lock* ./
COPY build.rs ./
COPY proto ./proto

RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

COPY src ./src

RUN touch src/main.rs && \
    cargo build --release && \
    cp /app/target/release/proxyd /app/proxyd

# Runtime stage with Chainguard glibc-dynamic for minimal footprint
FROM cgr.dev/chainguard/glibc-dynamic:latest

WORKDIR /app

COPY --from=builder --chown=65532:65532 /app/proxyd /app/proxyd

ENV PROXYD_DATA_DIR=/data
ENV RUST_LOG=proxyd=info

EXPOSE 7891 7892

VOLUME ["/data"]

ENTRYPOINT ["/app/proxyd"]
