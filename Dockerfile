# 前端构建
FROM node:24-bookworm-slim@sha256:cd84903a12dbd26b46f1f3b8144a2568c41c5d37ddd0c7a80a34c7a19786b35f AS web-builder

WORKDIR /build/web

COPY web/package.json web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm \
    npm ci

COPY web/ ./
RUN npm run build


# Rust 构建
FROM rust:1.93-bookworm@sha256:7c4ae649a84014c467d79319bbf17ce2632ae8b8be123ac2fb2ea5be46823f31 AS rust-builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --locked --release \
    && cp target/release/zhuque /tmp/zhuque


# 只用于取得 Bun 可执行文件
FROM oven/bun:1.3.14@sha256:e10577f0db68676a7024391c6e5cb4b879ebd17188ab750cf10024a6d700e5c4 AS bun-runtime


# 运行镜像
FROM node:24-bookworm-slim@sha256:cd84903a12dbd26b46f1f3b8144a2568c41c5d37ddd0c7a80a34c7a19786b35f AS runtime

ARG DEBIAN_FRONTEND=noninteractive

# 系统包跟随 Bookworm 安全更新；语言依赖由 lockfile 固定。
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
        openssh-client \
        python3 \
        python3-pip \
        unzip \
    && ln -s /usr/bin/python3 /usr/local/bin/python \
    && rm -rf /var/lib/apt/lists/*

COPY --from=bun-runtime /usr/local/bin/bun /usr/local/bin/bun
RUN ln -s /usr/local/bin/bun /usr/local/bin/bunx

WORKDIR /app

COPY --from=rust-builder /tmp/zhuque /app/zhuque
COPY --from=web-builder /build/web/dist /app/web/dist

ENV DATA_DIR=/app/data \
    RUST_LOG=info \
    NODE_PATH=/usr/local/lib/node_modules \
    PORT=3000

EXPOSE 3000

CMD ["/app/zhuque"]
