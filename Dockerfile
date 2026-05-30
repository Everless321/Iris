# 多阶段构建：builder 编译，debian-slim 运行。一个镜像产出 master + node 两个二进制。
FROM rust:1.88-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release --bin zhuanfa-master --bin zhuanfa-node

FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /src/target/release/zhuanfa-master /usr/local/bin/zhuanfa-master
COPY --from=builder /src/target/release/zhuanfa-node /usr/local/bin/zhuanfa-node
ENV ZF_CERT_DIR=/data/certs
