#!/bin/zsh

# Fetch the latest changes from the remote repository
git fetch origin

# Reset local main branch to match the remote main branch, discarding local changes
git reset --hard origin/main

# Optionally, clean up untracked files
git clean -fd

# Use Docker Compose to rebuild and restart the service
docker-compose down
docker-compose up --build -d
