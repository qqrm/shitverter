#!/usr/bin/env bash
set -euo pipefail

deployment_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly deployment_dir
readonly compose_file="$deployment_dir/compose.yml"
readonly env_file="$deployment_dir/shitverter.env"
readonly container_name="shitverter"
readonly legacy_container_name="my_shitverter_container"
if [[ $# -ne 1 ]]; then
  echo "usage: $0 ghcr.io/{qqrm,qqrm-labs}/shitverter@sha256:<digest>" >&2
  exit 64
fi

readonly image_reference="$1"
if [[ ! "$image_reference" =~ ^ghcr\.io/(qqrm|qqrm-labs)/shitverter@sha256:[a-f0-9]{64}$ ]]; then
  echo "refusing unexpected image reference: $image_reference" >&2
  exit 64
fi

if [[ ! -f "$compose_file" || ! -f "$env_file" ]]; then
  echo "missing $compose_file or $env_file; bootstrap the VM before deploying" >&2
  exit 66
fi

if [[ -f "$deployment_dir/local-api.migration.json" && ! -f "$deployment_dir/local-api.enabled" ]]; then
  echo "local API migration is unfinished; rerun Deploy with enable_local_api instead of starting a cloud poller" >&2
  exit 69
fi

compose_args=(--project-name shitverter -f "$compose_file")
local_api_enabled=false
if [[ -f "$deployment_dir/local-api.enabled" ]]; then
  local_api_enabled=true
  if [[ ! -f "$deployment_dir/compose.local-api.yml" ]]; then
    echo "local-api.enabled requires compose.local-api.yml" >&2
    exit 66
  fi
  compose_args+=(-f "$deployment_dir/compose.local-api.yml")
  if [[ "$(docker inspect --format '{{.State.Running}}' shitverter-telegram-api 2>/dev/null || true)" != "true" ]]; then
    api_image_file="$deployment_dir/telegram-api-image.env"
    if [[ ! -f "$api_image_file" ]]; then
      echo "start the local Bot API server or run Deploy with enable_local_api first" >&2
      exit 69
    fi
    api_image="$(< "$api_image_file")"
    api_image="${api_image#TELEGRAM_BOT_API_IMAGE=}"
    if [[ ! "$api_image" =~ ^ghcr\.io/(qqrm|qqrm-labs)/shitverter@sha256:[a-f0-9]{64}$ ]]; then
      echo "invalid local Bot API image reference" >&2
      exit 64
    fi
    TELEGRAM_BOT_API_IMAGE="$api_image" docker compose --project-name shitverter-telegram \
      -f "$deployment_dir/compose.telegram-api.yml" config --quiet
    TELEGRAM_BOT_API_IMAGE="$api_image" docker compose --project-name shitverter-telegram \
      -f "$deployment_dir/compose.telegram-api.yml" up --detach --no-build
    python3 "$deployment_dir/enable-local-api.py" --check-api-ready
    if [[ "$(docker inspect --format '{{.State.Running}}' shitverter-telegram-api 2>/dev/null || true)" != "true" ]]; then
      echo "local Bot API server did not start" >&2
      exit 69
    fi
  fi
fi
# Validate configuration before stopping the currently running bot.
SHITVERTER_IMAGE="$image_reference" docker compose "${compose_args[@]}" config --quiet

previous_image="$(docker inspect --format '{{.Config.Image}}' "$container_name" 2>/dev/null || true)"
readonly previous_image
legacy_was_running="$(docker inspect --format '{{.State.Running}}' "$legacy_container_name" 2>/dev/null || true)"
readonly legacy_was_running

if [[ "$local_api_enabled" == "true" && "$legacy_was_running" == "true" ]]; then
  echo "stop the legacy poller and complete the documented logOut migration first" >&2
  exit 69
fi

rollback() {
  if [[ "$legacy_was_running" == "true" ]]; then
    echo "restarting legacy container $legacy_container_name" >&2
    docker start "$legacy_container_name"
  elif [[ -n "$previous_image" ]]; then
    echo "rolling back to $previous_image" >&2
    SHITVERTER_IMAGE="$previous_image" docker compose "${compose_args[@]}" \
      up --detach --force-recreate --remove-orphans
  else
    echo "no previous running deployment is available for rollback" >&2
  fi
}

docker pull "$image_reference"
if [[ "$legacy_was_running" == "true" ]]; then
  echo "stopping legacy container $legacy_container_name"
  docker stop "$legacy_container_name"
fi

if ! SHITVERTER_IMAGE="$image_reference" docker compose "${compose_args[@]}" \
  up --detach --force-recreate --remove-orphans; then
  rollback
  exit 1
fi

for _ in {1..15}; do
  container_state="$(docker inspect --format '{{.State.Running}} {{.State.Restarting}} {{.RestartCount}}' "$container_name" 2>/dev/null || true)"
  if [[ "$container_state" != "true false 0" ]]; then
    echo "new container is not stable: $container_state" >&2
    docker logs --tail 50 "$container_name" >&2 || true
    rollback
    exit 1
  fi
  sleep 1
done

echo "deployed $image_reference"
