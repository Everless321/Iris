# 多阶段构建：web → rust builder → debian-slim 运行。
# master 用 RustEmbed 把 web/dist 嵌入二进制，所以必须先生成前端产物再编译 Rust。
FROM node:20-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --include=optional --no-fund
COPY web/ ./
RUN npm run build

FROM rust:1.88-bookworm AS builder
WORKDIR /src
COPY . .
# 覆盖 .dockerignore 排除的本地 web/dist，用上一阶段产物
COPY --from=web /src/web/dist ./web/dist
RUN cargo build --release --bin iris-master --bin iris-node

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl jq \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /src/target/release/iris-master /usr/local/bin/iris-master
COPY --from=builder /src/target/release/iris-node /usr/local/bin/iris-node
# install.sh 不再随镜像分发：master /install.sh 已改为 302 redirect 到 GitHub raw
# （见 crates/master/src/api.rs install_script），脚本变更不再需要 rebuild 镜像。
ENV IRIS_CERT_DIR=/data/certs
