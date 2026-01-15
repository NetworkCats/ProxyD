# Build stage with Chainguard Rust image for supply chain security
FROM --platform=$BUILDPLATFORM cgr.dev/chainguard/rust:latest-dev AS builder

USER root
RUN apk add --no-cache protobuf-dev

WORKDIR /app

COPY Cargo.toml Cargo.lock* ./
COPY build.rs ./
COPY proto ./proto

ARG TARGETPLATFORM
RUN case "$TARGETPLATFORM" in \
        "linux/amd64") RUST_TARGET="x86_64-unknown-linux-gnu" ;; \
        "linux/arm64") RUST_TARGET="aarch64-unknown-linux-gnu" ;; \
        *) echo "Unsupported platform: $TARGETPLATFORM" && exit 1 ;; \
    esac && \
    rustup target add "$RUST_TARGET" && \
    mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release --target "$RUST_TARGET" && \
    rm -rf src

COPY src ./src

RUN case "$TARGETPLATFORM" in \
        "linux/amd64") RUST_TARGET="x86_64-unknown-linux-gnu" ;; \
        "linux/arm64") RUST_TARGET="aarch64-unknown-linux-gnu" ;; \
    esac && \
    touch src/main.rs && \
    cargo build --release --target "$RUST_TARGET" && \
    mkdir -p /app/output && \
    cp "/app/target/$RUST_TARGET/release/proxyd" /app/output/proxyd

# Runtime stage with Chainguard Wolfi base for minimal attack surface
FROM cgr.dev/chainguard/wolfi-base:latest

RUN apk add --no-cache ca-certificates curl && \
    addgroup -S proxyd && adduser -S proxyd -G proxyd

WORKDIR /app

COPY --from=builder /app/output/proxyd /app/proxyd

RUN mkdir -p /data && chown -R proxyd:proxyd /data /app

USER proxyd

ENV PROXYD_DATA_DIR=/data
ENV RUST_LOG=proxyd=info

EXPOSE 7891 7892

VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:7891/health || exit 1

ENTRYPOINT ["/app/proxyd"]
