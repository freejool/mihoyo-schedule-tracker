# mihoyo-schedule-tracker

米哈游核心游戏（《原神》《崩坏：星穹铁道》《绝区零》）**版本开启时间与前瞻特别节目预测日历**。

只聚焦两件事：**前瞻直播时间** 与 **版本开启时间**。后端定时抓取米游社公开 API 写入 SQLite 自动校准，
42 天周期公式向后自动推算，前端支持手动修正覆盖 —— 全部同源部署，天然无浏览器 CORS 问题。

## 特性一览

- 🔄 **后端自动校准**：每 6 小时轮询米游社 `getNewsList`（gids：2=原神 / 6=星铁 / 8=绝区零，type 1 公告 + 3 资讯），正则识别「前瞻特别节目」与「X.Y版本更新说明/更新公告」，UPSERT 写入 SQLite。
- 📐 **42 天周期级联推算**：以最新官方确认 / 手动校准锚点为基准，`start(i) = start(i-1) + duration(i-1)` 向后滚动未来 6 个版本；前瞻时间默认 `start - 12 天`，非周五则对齐到开启周的前一周周五。
- 🟢🔵🟠 **三级状态**：`official_confirmed` 官方定档 / `manual_override` 手动校准 / `predicted_42d` 42 天预测。
- ✏️ **前端一键校准**：改版本号（如 `3.9 → 4.0`）、前瞻时间、开启日期、版本长度（35/42/49 天），保存后全表联动重算；可恢复自动推算。
- 📅 **iCalendar 订阅**：`/calendar.ics?games=ys,sr,zzz&include_predicted=true`，每个版本两个 `VEVENT`（前瞻直播含提前 30 分钟提醒；版本开启 06:00–11:00），iOS / Google Calendar / Outlook 可直接订阅。
- 🐳 **Docker 多阶段构建**：Node 构建前端 → Rust 编译后端（SQLite bundled，免系统依赖）→ Debian slim 运行时。

## 快速开始（Docker）

```bash
cp .env.example .env          # 按需设置 ADMIN_TOKEN（写接口鉴权）
docker compose up -d --build  # 端口 8080
```

- SQLite 数据持久化在宿主机 `./data/`（`schedule.db` + `-wal` / `-shm`），备份只需复制该目录。
- 容器入口脚本会自动修复挂载目录属主（`chown app:app` 后降权运行），无需手动 `chmod`/`chown`；宿主侧 `./data` 内文件属主为 uid 999（app），命令行直接改库需 `sudo chown` 到自己的用户。
- 查看日志：`docker compose logs -f tracker`

## 本地开发

```bash
# 后端（需要 Rust 1.96+）
cp .env.example .env
DATABASE_URL=sqlite:./data/schedule.db?mode=rwc cargo run

# 前端（Vite dev server，/api 已代理到 8080）
cd web && npm install && npm run dev
```

生产模式：`cd web && npm run build` 后由 Axum 直接在 `/` 托管 `web/dist`（SPA fallback 到 index.html）。

## API

| 方法 | 路径 | 说明 | 鉴权 |
|---|---|---|---|
| GET | `/api/schedules` | 三款游戏当前版本 + 未来 6 个版本（含状态、倒计时、原帖链接） | 无 |
| POST | `/api/sync` | 手动触发官方同步，返回扫描/更新数与最新排期 | `ADMIN_TOKEN` |
| POST | `/api/override` | 新增/修改某版本的 `version_code` / `start_at` / `livestream_at` / `duration_days` | `ADMIN_TOKEN` |
| DELETE | `/api/override/{game_id}/{version_code}` | 删除手动校准行，恢复 42 天推算（official 行返回 409） | `ADMIN_TOKEN` |
| GET | `/calendar.ics` | RFC 5545 订阅文件（参数见下） | 无 |
| GET | `/api/health` | 健康检查 | 无 |

`/calendar.ics` 查询参数：

| 参数 | 默认 | 说明 |
|---|---|---|
| `games` | 全部 | 逗号分隔游戏 id，如 `ys,sr,zzz` |
| `include_predicted` | `true` | `false` 时滤除纯推算版本（锚点版本的预测直播仍保留） |
| `versions` | 全部 | 每游戏只输出未来最近 N 个版本（含其前瞻直播）：`versions=1` = 仅下一版本（T+1），`versions=2` = 最近两个 |

## 接入 Notion / 外部日历

- **直接订阅**：复制前端「📅 复制 .ics 订阅链接」弹层里的 URL（可勾选 `仅下一版本（T+1）` 缩小范围），粘贴到 Google/Apple/Outlook 日历的「添加订阅」；Notion Calendar 通过关联 Google 日历间接显示。
- **Notion 数据库（原生，推荐）**：`scripts/notion_sync.py` 读取 `/api/schedules`，把每个游戏未来最近 N 个版本（默认 N=1，T+1）幂等 upsert 到 Notion 数据库，并归档滚动窗口外、由脚本写入过的旧行：

  ```bash
  # 1) Notion 建 Integration 拿到 token；2) 建数据库（属性见脚本头部注释）并共享给该集成
  NOTION_TOKEN=ntn_xxx NOTION_DATABASE_ID=xxx python3 scripts/notion_sync.py   # 实际同步
  python3 scripts/notion_sync.py --dry-run                                     # 先预览将要写入的行
  ```

  脚本零第三方依赖（仅标准库），可 cron 定时执行；`--no-prune` 可关闭旧行归档。

`POST /api/override` 请求示例：

```json
{
  "game_id": "ys",
  "version_code": "7.2",
  "new_version_code": "8.0",
  "start_at": "2026-11-04T06:00",
  "livestream_at": "2026-10-23T20:00",
  "duration_days": 42
}
```

`start_at` / `livestream_at` 接受 `YYYY-MM-DD`（分别按 06:00 / 游戏默认开播时刻）、`YYYY-MM-DDTHH:MM[:SS]` 或带偏移的 RFC3339，naive 值一律按 UTC+8 解释。

## 数据库设计要点

```text
games              游戏基础配置：gid、slug、42 天周期、前瞻提前天数（12）、默认开播时刻（原神 20:00 / 星铁、绝区零 19:30）
version_anchors    已确认/已校准版本锚点：start_at + start_status、livestream_at + livestream_status、duration_days、官方原帖
sync_logs          官方同步日志（扫描数 / 更新数 / success|error）
```

- 启动自动执行 `CREATE TABLE IF NOT EXISTS` 三表建表 + 幂等 Seed（2026-09/10 官方核实锚点：原神 7.1、星铁 4.6、绝区零 3.3），已有锚点的游戏不会被覆盖。
- **校准优先级**：`official_confirmed > manual_override > predicted_42d`。官方同步永不覆盖 `manual_override` 字段（仅补写来源链接与未锁定字段）。
- `PRAGMA journal_mode=WAL` + `foreign_keys=ON`，后台抓取与前端并发读互不阻塞。

## 同步识别规则（实测验证）

米游社列表接口的 `content` 字段为空，时间信息来自 `subject` 与 `created_at`（2026-10 实测数据）：

| 帖子模式 | 推导规则 | 实测样例 |
|---|---|---|
| `X.Y版本更新说明/更新公告` | 帖子发布日 = 版本开启日，06:00 开服（三家官方均于开服当日 07:00 发布） | zzz「3.2版本…更新公告」09-09 07:00 → 09-09 06:00 |
| `X.Y版本…前瞻特别节目`（排除预告/回放） | 帖子发布于「开播窗口」内（开播前 30 分 ~ 开播后 3 小时）→ 当日默认开播时刻 | zzz 3.3 节目帖 10-09 20:20 → 10-09 19:30 |
| 正文含显式日期（`10月9日 19:30` / `2026/10/9`） | 直接采用，优先级最高 | — |
| `X.Y版本预下载/更新预告` | 只登记来源链接，不推断日期（预下载距开服 2~4 天不等） | sr「4.6版本预下载&更新预告」 |

> `bbs-api.miyoushe.com` 边缘节点对无签名请求返回 404，故内置候选域名依次尝试
> （`MIYOUSHE_API_BASES` 可配置，默认含 `api-takumi.mihoyo.com`）。

## 环境变量

| 变量 | 默认值 | 说明 |
|---|---|---|
| `PORT` | `8080` | HTTP 端口 |
| `DATABASE_URL` | `sqlite:///app/data/schedule.db?mode=rwc` | SQLite 连接串 |
| `SYNC_INTERVAL_HOURS` | `6` | 后台轮询间隔（小时） |
| `SYNC_ON_START` | `true` | 启动 5 秒后立即同步一次；`false` 则启动不同步，仅按 `SYNC_INTERVAL_HOURS` 周期轮询 |
| `ADMIN_TOKEN` | 空 | 写接口 Bearer Token（空 = 不鉴权） |
| `MIYOUSHE_API_BASES` | 见上 | 候选 API 域名（逗号分隔） |
| `TZ` | `Asia/Shanghai` | 容器时区 |
| `WEB_DIST` | `web/dist` | 前端静态目录（Docker 内固定） |

## 技术栈

- **后端**：Rust · Axum · SQLx/SQLite（bundled，WAL）· tokio · reqwest(rustls) · chrono · regex · tower-http（静态托管 / CORS / Gzip）
- **前端**：Svelte 5（Runes）· Vite · TypeScript · Tailwind CSS
- **部署**：多阶段 Dockerfile（node:24-alpine → rust:1.96-slim-bookworm → debian:bookworm-slim）+ docker-compose

## 目录结构

```text
├── Cargo.toml
├── Dockerfile
├── docker-compose.yml
├── .env.example
├── src/
│   ├── main.rs      # Axum 路由、静态服务、SQLite 初始化与后台定时任务
│   ├── db.rs        # 连接池、建表迁移、Seed、CRUD
│   ├── models.rs    # 行模型 / DTO / 时间工具
│   ├── engine.rs    # 42 天级联推算 + 日期对齐
│   ├── scraper.rs   # 米游社 API 抓取、正则提取、UPSERT
│   └── ics.rs       # RFC 5545 iCalendar 生成器
└── web/             # Svelte 5 + Vite + Tailwind 前端
```
