# Build Stage
FROM clux/muslrust:latest as builder
WORKDIR /home/rust/src

# Copy the source code and Cargo files
COPY ./ ./

# Compile the application with musl target
RUN cargo build --release --target x86_64-unknown-linux-musl

# Run Stage
FROM alpine:latest as runtime

# Install FFmpeg
RUN apk add --no-cache ffmpeg

# Copy the compiled binary from the builder stage
COPY --from=builder /home/rust/src/target/x86_64-unknown-linux-musl/release/converter-bot /usr/local/bin/converter-bot

# Set the working directory and the entrypoint
WORKDIR /usr/local/bin
ENTRYPOINT ["./converter-bot"]
