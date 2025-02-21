#!/bin/zsh

# Получаем последние изменения с удалённого репозитория "origin"
git fetch origin

# Сбрасываем локальную ветку, устанавливая её равной удалённой ветке "async" из "origin"
git reset --hard origin/async

# Опционально, очищаем неотслеживаемые файлы
git clean -fd

# Останавливаем и удаляем существующий контейнер, если он есть
docker stop my_shitverter_container || true
docker rm my_shitverter_container || true

# Собираем Docker-образ с тегом 'shitverter:latest'
docker build -t shitverter:latest .

# Запускаем новый контейнер, передавая переменную окружения TELOXIDE_TOKEN
docker run -d -e TELOXIDE_TOKEN=$TELEGRAM_API_TOKEN --name my_shitverter_container shitverter:latest
