# Переменные
container_name := "my_shitverter_container"
image_name := "shitverter:latest"

# Показать список доступных команд
default:
    @just --list

# Остановить контейнер, если он запущен
stop-container:
    docker stop {{container_name}} || true
    docker rm {{container_name}} || true

# Обновить код из репозитория
update-code:
    # Получить последние изменения с удалённого репозитория
    git fetch origin
    # Сбросить локальную ветку
    git reset --hard
    # Очистить неотслеживаемые файлы
    git clean -fd

# Пересобрать проект с нуля
rebuild: update-code stop-container
    # Собрать Docker-образ
    docker build -t {{image_name}} .
    # Запустить новый контейнер с токеном Telegram API
    docker run -d -e TELOXIDE_TOKEN=$TELEGRAM_API_TOKEN --name {{container_name}} {{image_name}}

# Запустить контейнер (без пересборки)
run: stop-container
    # Запустить контейнер с токеном Telegram API
    docker run -d -e TELOXIDE_TOKEN=$TELEGRAM_API_TOKEN --name {{container_name}} {{image_name}} 