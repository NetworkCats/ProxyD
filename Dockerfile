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

# Runtime stage with Chainguard Wolfi base for minimal attack surface
FROM cgr.dev/chainguard/wolfi-base:latest

RUN apk add --no-cache ca-certificates curl && \
    addgroup -S proxyd && adduser -S proxyd -G proxyd

WORKDIR /app

COPY --from=builder /app/proxyd /app/proxyd

RUN mkdir -p /data && chown -R proxyd:proxyd /data /app

USER proxyd

ENV PROXYD_DATA_DIR=/data
ENV RUST_LOG=proxyd=info

EXPOSE 7891 7892

VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:7891/health || exit 1

ENTRYPOINT ["/app/proxyd"]
