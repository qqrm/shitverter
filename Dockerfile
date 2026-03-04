# syntax=docker/dockerfile:1
#
# Multi-stage build:
# - build-env: Rust toolchain + musl C toolchain (not shipped to prod)
# - builder:   compiles the release binary for musl
# - runtime:   minimal Alpine + ffmpeg + the binary
#
# To pin the toolchain for reproducibility:
#   docker build --build-arg MUSLRUST_TAG=1.93.1-stable-2026-02-23 -t shitverter:latest .

ARG MUSLRUST_TAG=stable

FROM clux/muslrust:${MUSLRUST_TAG} AS build-env
WORKDIR /app

FROM build-env AS builder

# Strip symbols at compile time to reduce binary size.
ENV RUSTFLAGS="-C strip=symbols"

# ---- dependency cache (keeps rebuilds fast) ----
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && printf "fn main() {}\n" > src/main.rs
RUN cargo build --release --locked --target x86_64-unknown-linux-musl

# ---- build the real application ----
RUN rm -f src/main.rs
COPY src ./src
RUN cargo build --release --locked --target x86_64-unknown-linux-musl

# ---- production runtime ----
FROM alpine:3.23.3 AS runtime

# ffmpeg is required for video conversion; ca-certificates is needed for HTTPS (Telegram API).
RUN apk add --no-cache ffmpeg ca-certificates \
  && addgroup -S app \
  && adduser -S -G app app \
  && mkdir -p /tmp \
  && chown -R app:app /tmp

COPY --from=builder --chown=app:app /app/target/x86_64-unknown-linux-musl/release/converter-bot /usr/local/bin/converter-bot

USER app
WORKDIR /tmp
ENV RUST_LOG=info
ENTRYPOINT ["/usr/local/bin/converter-bot"]
