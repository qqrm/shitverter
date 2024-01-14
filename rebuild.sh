#!/bin/zsh

# Fetch the latest changes from the remote repository
git fetch origin

# Reset local main branch to match the remote main branch, discarding local changes
git reset --hard origin/main

# Optionally, clean up untracked files
git clean -fd

# Stop and remove the existing container if it exists
docker stop my_shitverter_container || true
docker rm my_shitverter_container || true

# Build the Docker image with the tag 'shitverter:latest'
docker build -t shitverter:latest .

# Run the new container here
docker run -d -e TELOXIDE_TOKEN=$TELEGRAM_API_TOKEN --name my_shitverter_container shitverter:latest