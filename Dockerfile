# Builder stage
FROM rust:latest as builder

WORKDIR /usr/src/converter-bot

# Copy the Cargo.toml and Cargo.lock.
COPY ./Cargo.toml ./Cargo.lock ./

# Copy the source code.
COPY ./src ./src

# Build the application.
# This will only recompile if the source code has changed.
RUN cargo build --release

# Runtime stage
FROM debian:buster-slim

# Install FFmpeg.
RUN apt-get update && apt-get install -y ffmpeg && rm -rf /var/lib/apt/lists/*

# Copy the binary from the builder stage.
COPY --from=builder /usr/src/converter-bot/target/release/converter-bot /converter-bot

# Set the startup command.
CMD ["/converter-bot"]
