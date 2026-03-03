#!/usr/bin/env bash
set -euo pipefail

container_name="${CONTAINER_NAME:-my_shitverter_container}"
image_name="${IMAGE_NAME:-shitverter:latest}"
env_file="${ENV_FILE:-.env}"

# Stop/remove previous container if it exists.
docker stop "$container_name" >/dev/null 2>&1 || true
docker rm "$container_name" >/dev/null 2>&1 || true

run_args=(--detach --name "$container_name")

# Prefer loading variables from .env (if present).
if [[ -f "$env_file" ]]; then
  run_args+=(--env-file "$env_file")
fi

# Fallback: support externally exported variable name used in old scripts.
if [[ -n "${TELEGRAM_API_TOKEN:-}" && -z "${TELOXIDE_TOKEN:-}" ]]; then
  run_args+=(-e "TELOXIDE_TOKEN=$TELEGRAM_API_TOKEN")
fi

# Pass through explicit TELOXIDE_TOKEN from environment when provided.
if [[ -n "${TELOXIDE_TOKEN:-}" ]]; then
  run_args+=(-e "TELOXIDE_TOKEN=$TELOXIDE_TOKEN")
fi

docker run "${run_args[@]}" "$image_name"
