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

# Пересобрать проект с нуля и запустить контейнер
rebuild:
    IMAGE_NAME={{image_name}} CONTAINER_NAME={{container_name}} ./rebuild.sh

# Запустить контейнер (без пересборки)
run:
    IMAGE_NAME={{image_name}} CONTAINER_NAME={{container_name}} ./run.sh
