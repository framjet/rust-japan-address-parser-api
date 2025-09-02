# syntax=docker/dockerfile:1.6
FROM rust:1.89-alpine AS builder

ARG TARGETARCH
WORKDIR /app

# Build deps for Rust + musl + OpenSSL (static)
RUN apk add --no-cache \
    alpine-sdk \
    build-base \
    musl-dev \
    pkgconfig \
    openssl-dev \
    openssl-libs-static

# Map BUILD arch -> Rust musl target
RUN case "$TARGETARCH" in \
      amd64)  echo x86_64-unknown-linux-musl  > /tmp/rust-target ;; \
      arm64)  echo aarch64-unknown-linux-musl > /tmp/rust-target ;; \
      *) echo "Unsupported arch: $TARGETARCH" && exit 1 ;; \
    esac

RUN cat /tmp/rust-target

# Add the Rust target
RUN rustup target add $(cat /tmp/rust-target)

# Fully static linking
ENV RUSTFLAGS="-C target-feature=+crt-static"

# Copy manifests and prebuild deps for layer caching
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo "fn main() {}" > src/main.rs
RUN cargo build --release --target $(cat /tmp/rust-target)
RUN rm -rf src

# Now the real source
COPY src ./src
RUN touch ./src/main.rs && \
    cargo build --release --target $(cat /tmp/rust-target) && \
    cp target/$(cat /tmp/rust-target)/release/rust-japan-address-parser-api /usr/local/bin/japi

# ---- final image ----
FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=builder /usr/local/bin/japi /usr/local/bin/japi

EXPOSE 3000
ENV RUST_LOG=info
ENV HOST=0.0.0.0
ENV PORT=3000
ENTRYPOINT ["/usr/local/bin/japi"]
