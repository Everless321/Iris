# 多阶段构建：builder 编译，debian-slim 运行。一个镜像产出 master + node 两个二进制。
FROM rust:1.88-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release --bin zhuanfa-master --bin zhuanfa-node

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl jq \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /src/target/release/zhuanfa-master /usr/local/bin/zhuanfa-master
COPY --from=builder /src/target/release/zhuanfa-node /usr/local/bin/zhuanfa-node
COPY crates/master/assets/install-node.sh /usr/local/bin/install-node.sh
RUN chmod +x /usr/local/bin/install-node.sh
ENV ZF_CERT_DIR=/data/certs
