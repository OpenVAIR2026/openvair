# community (default) or cloud. The cloud edition is the hosted service's build.
ARG EDITION=community

FROM node:20-bookworm-slim AS web-build
ARG EDITION
ARG HTTP_PROXY
ARG HTTPS_PROXY
ARG NO_PROXY
ENV HTTP_PROXY=$HTTP_PROXY HTTPS_PROXY=$HTTPS_PROXY NO_PROXY=$NO_PROXY
WORKDIR /web
COPY web/package*.json ./
RUN npm ci
COPY web/ ./
RUN case "$EDITION" in community|cloud) ;; *) echo "EDITION must be community or cloud, got: $EDITION" >&2; exit 1 ;; esac \
    && VITE_EDITION="$EDITION" npm run build

FROM rust:1.90-bookworm AS build
ARG EDITION
ARG HTTP_PROXY
ARG HTTPS_PROXY
ARG NO_PROXY
ARG USE_RSPROXY=false
ENV HTTP_PROXY=$HTTP_PROXY HTTPS_PROXY=$HTTPS_PROXY NO_PROXY=$NO_PROXY
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
ENV CARGO_HTTP_MULTIPLEXING=false
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
RUN mkdir -p /usr/local/cargo \
    && if [ "$USE_RSPROXY" = "true" ]; then \
         printf '[source.crates-io]\nreplace-with = "rsproxy"\n\n[source.rsproxy]\nregistry = "sparse+https://rsproxy.cn/index/"\n\n[net]\ngit-fetch-with-cli = true\n\n[http]\nmultiplexing = false\n' > /usr/local/cargo/config.toml; \
       fi \
    && mkdir -p src \
    && printf 'fn main() {}\n' > src/main.rs \
    && cargo fetch --locked
COPY migrations ./migrations
COPY src ./src
RUN case "$EDITION" in \
      community) features="" ;; \
      cloud) features="--features cloud" ;; \
      *) echo "EDITION must be community or cloud, got: $EDITION" >&2; exit 1 ;; \
    esac; \
    build_jobs=$(( $(nproc) / 2 )); \
    if [ "$build_jobs" -lt 1 ]; then build_jobs=1; fi; \
    echo "Building $EDITION edition with $build_jobs Cargo jobs"; \
    cargo build --locked --release --offline -j "$build_jobs" $features

FROM debian:bookworm-slim
ARG HTTP_PROXY
ARG HTTPS_PROXY
ARG NO_PROXY
ENV HTTP_PROXY=$HTTP_PROXY HTTPS_PROXY=$HTTPS_PROXY NO_PROXY=$NO_PROXY
RUN HTTP_PROXY= HTTPS_PROXY= http_proxy= https_proxy= \
    apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --create-home app
WORKDIR /app
COPY --from=build /app/target/release/openvair /usr/local/bin/openvair
COPY --from=build /app/migrations ./migrations
COPY --from=web-build /web/dist ./web/dist
ENV APP_ADDR=0.0.0.0:8080
ENV HTTP_PROXY= HTTPS_PROXY= NO_PROXY=
EXPOSE 8080
USER app
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 CMD curl --fail --silent http://127.0.0.1:8080/healthz || exit 1
CMD ["openvair"]
