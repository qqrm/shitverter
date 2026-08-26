#!/usr/bin/env bash
set -euo pipefail

deployment_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly deployment_dir
readonly compose_file="$deployment_dir/compose.yml"
readonly env_file="$deployment_dir/shitverter.env"
if [[ $# -ne 1 ]]; then
  echo "usage: $0 ghcr.io/qqrm/shitverter@sha256:<digest>" >&2
  exit 64
fi

readonly image_reference="$1"
if [[ ! "$image_reference" =~ ^ghcr\.io/qqrm/shitverter@sha256:[a-f0-9]{64}$ ]]; then
  echo "refusing unexpected image reference: $image_reference" >&2
  exit 64
fi

if [[ ! -f "$compose_file" || ! -f "$env_file" ]]; then
  echo "missing $compose_file or $env_file; bootstrap the VM before deploying" >&2
  exit 66
fi

previous_image="$(docker inspect --format '{{.Config.Image}}' shitverter 2>/dev/null || true)"
readonly previous_image

docker pull "$image_reference"
SHITVERTER_IMAGE="$image_reference" docker compose \
  --project-name shitverter \
  -f "$compose_file" \
  up --detach --force-recreate --remove-orphans

sleep 5
if ! docker inspect --format '{{.State.Running}}' shitverter 2>/dev/null | grep -qx true; then
  if [[ -n "$previous_image" ]]; then
    echo "new container exited; rolling back to $previous_image" >&2
    SHITVERTER_IMAGE="$previous_image" docker compose \
      --project-name shitverter \
      -f "$compose_file" \
      up --detach --force-recreate --remove-orphans
  fi
  exit 1
fi

echo "deployed $image_reference"
