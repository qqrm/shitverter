# syntax=docker/dockerfile:1
FROM alpine:3.24.2 AS builder

RUN apk add --no-cache build-base cmake git gperf linux-headers openssl-dev zlib-dev

# Official Bot API 10.3 source and its pinned TDLib submodule.
ARG TELEGRAM_BOT_API_COMMIT=e3e9dd8e5b3d7ab8537cd5a10dc31d5ffa8f82d1
ARG BUILD_JOBS=2
WORKDIR /src
RUN git init . \
  && git remote add origin https://github.com/tdlib/telegram-bot-api.git \
  && git fetch --depth 1 origin "$TELEGRAM_BOT_API_COMMIT" \
  && git checkout --detach FETCH_HEAD \
  && git submodule update --init --recursive --depth 1 \
  && cmake -S . -B /build -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr/local \
  && cmake --build /build --target install --parallel "$BUILD_JOBS" \
  && strip /usr/local/bin/telegram-bot-api

FROM alpine:3.24.2 AS runtime
RUN apk add --no-cache ca-certificates libstdc++ libatomic openssl zlib \
  && addgroup -g 1000 app \
  && adduser -D -H -u 1000 -G app app \
  && mkdir -p /var/lib/telegram-bot-api/tmp /var/lib/shitverter-work \
  && chown -R 1000:1000 /var/lib/telegram-bot-api /var/lib/shitverter-work \
  && chmod 700 /var/lib/telegram-bot-api /var/lib/shitverter-work
COPY --from=builder /usr/local/bin/telegram-bot-api /usr/local/bin/telegram-bot-api
USER 1000:1000
WORKDIR /var/lib/telegram-bot-api
ENTRYPOINT ["/usr/local/bin/telegram-bot-api"]
CMD ["--local", "--http-port=8081", "--dir=/var/lib/telegram-bot-api", "--temp-dir=/var/lib/telegram-bot-api/tmp"]
