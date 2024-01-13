#!/bin/zsh

docker rm my_shitverter_container

docker run -d -e TELOXIDE_TOKEN$=TELEGRAM_API_TOKEN --name my_shitverter_container shitverter:latest