#!/usr/bin/env bash
set -euo pipefail

deployment_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly deployment_dir
readonly compose_file="$deployment_dir/compose.yml"
readonly env_file="$deployment_dir/shitverter.env"
readonly container_name="shitverter"
readonly legacy_container_name="my_shitverter_container"
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

previous_image="$(docker inspect --format '{{.Config.Image}}' "$container_name" 2>/dev/null || true)"
readonly previous_image
legacy_was_running="$(docker inspect --format '{{.State.Running}}' "$legacy_container_name" 2>/dev/null || true)"
readonly legacy_was_running

rollback() {
  if [[ -n "$previous_image" ]]; then
    echo "rolling back to $previous_image" >&2
    SHITVERTER_IMAGE="$previous_image" docker compose \
      --project-name shitverter \
      -f "$compose_file" \
      up --detach --force-recreate --remove-orphans
  elif [[ "$legacy_was_running" == "true" ]]; then
    echo "restarting legacy container $legacy_container_name" >&2
    docker start "$legacy_container_name"
  else
    echo "no previous running deployment is available for rollback" >&2
  fi
}

docker pull "$image_reference"
if [[ "$legacy_was_running" == "true" ]]; then
  echo "stopping legacy container $legacy_container_name"
  docker stop "$legacy_container_name"
fi

if ! SHITVERTER_IMAGE="$image_reference" docker compose \
  --project-name shitverter \
  -f "$compose_file" \
  up --detach --force-recreate --remove-orphans; then
  rollback
  exit 1
fi

sleep 5
if ! docker inspect --format '{{.State.Running}}' "$container_name" 2>/dev/null | grep -qx true; then
  echo "new container exited" >&2
  rollback
  exit 1
fi

echo "deployed $image_reference"
