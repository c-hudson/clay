# Clay MUD Client - Build Container
#
# Build the image:
#   docker build -t clay-builder .
#
# Build the binary:
#   docker run --rm -v $(pwd)/output:/output clay-builder
#
# The binary will be in ./output/clay

FROM rust:latest AS builder

# Install system dependencies for GUI and audio
RUN apt-get update && apt-get install -y \
    libasound2-dev \
    libxcb-render0-dev \
    libxcb-shape0-dev \
    libxcb-xfixes0-dev \
    libxcb1-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy source
COPY . .

# Remove Cargo.lock for compatibility
RUN rm -f Cargo.lock

# Build release with GUI and audio
RUN cargo build --release --features remote-gui-audio

# Final stage - just copy the binary
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    libasound2 \
    libxcb-render0 \
    libxcb-shape0 \
    libxcb-xfixes0 \
    libxcb1 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/clay /usr/local/bin/clay

# Default command copies binary to /output if mounted
CMD ["sh", "-c", "if [ -d /output ]; then cp /usr/local/bin/clay /output/ && echo 'Binary copied to /output/clay'; else /usr/local/bin/clay; fi"]
