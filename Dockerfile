# Build Stage
FROM clux/muslrust:1.75.0-stable as builder
WORKDIR /home/rust/src

# Cache dependencies
# Copy Cargo.toml and Cargo.lock and create a dummy main file
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build only the dependencies to cache them
RUN cargo build --release --target x86_64-unknown-linux-musl

# Now copy the actual source code
COPY ./ ./

# Touch the main file to update its timestamp
RUN touch src/main.rs

# Build the actual application
RUN cargo build --release --target x86_64-unknown-linux-musl

# Run Stage
FROM alpine:3.19 as runtime

# Install FFmpeg
RUN apk add --no-cache ffmpeg

# Copy the compiled binary from the builder stage
COPY --from=builder /home/rust/src/target/x86_64-unknown-linux-musl/release/converter-bot /usr/local/bin/converter-bot

# Set the working directory and the entrypoint
WORKDIR /usr/local/bin
ENTRYPOINT ["./converter-bot"]
