# syntax=docker/dockerfile:1

###############################################################################
# Stage 1 —— 前端构建（Node 24 + Vite + Svelte 5）
###############################################################################
FROM node:24-alpine AS web

WORKDIR /build/web

# 先拷贝依赖清单以复用 npm 缓存层
COPY web/package.json web/package-lock.json* ./
# 覆盖用户级 npm allow-scripts 策略：确保 esbuild 等依赖的 postinstall 正常执行
ENV NPM_CONFIG_IGNORE_SCRIPTS=false
RUN npm install --no-audit --no-fund

COPY web/ ./
RUN npm run build && ls -la dist/index.html

###############################################################################
# Stage 2 —— Rust 编译（rust 1.96，sqlite 采用 bundled 编译，无需系统 libsqlite3）
###############################################################################
FROM rust:1.96-slim-bookworm AS builder

# libsqlite3-sys bundled 需要 C 编译器
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

COPY Cargo.toml Cargo.lock* ./
COPY src ./src

# 关闭增量编译/调试符号，减小镜像体积
RUN cargo build --release --locked \
    && ls -lh /build/target/release/mihoyo-schedule-tracker

###############################################################################
# Stage 3 —— 运行时（Debian slim + ca-certificates + tzdata）
###############################################################################
FROM debian:bookworm-slim

ENV TZ=Asia/Shanghai \
    PORT=8080 \
    DATABASE_URL=sqlite:///app/data/schedule.db?mode=rwc \
    SYNC_INTERVAL_HOURS=6 \
    SYNC_ON_START=true

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /app app

WORKDIR /app

# 前端静态产物 + 后端二进制
COPY --from=web /build/web/dist /app/web/dist
COPY --from=builder /build/target/release/mihoyo-schedule-tracker /app/mihoyo-schedule-tracker

# SQLite 数据目录：由宿主机 ./data 挂载（schedule.db / -wal / -shm）
RUN mkdir -p /app/data && chown -R app:app /app

USER app

EXPOSE 8080

HEALTHCHECK --interval=60s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/api/health >/dev/null || exit 1

CMD ["/app/mihoyo-schedule-tracker"]
