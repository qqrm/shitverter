#!/usr/bin/env bash
set -euo pipefail

image_name="${IMAGE_NAME:-shitverter:latest}"

# Build a fresh image from the current working tree.
docker build -t "$image_name" .

# Start container with env propagation logic from run.sh.
"$(dirname "$0")/run.sh"
