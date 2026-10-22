//! SQLite 连接池、自动建表迁移（auto-migration）、基准数据 Seed 与全部 CRUD。
//!
//! * 默认库文件：`/app/data/schedule.db`（`DATABASE_URL=sqlite:///app/data/schedule.db?mode=rwc`）
//! * 启动即开启 `PRAGMA journal_mode=WAL`（后台定时抓取与前端并发读取互不阻塞）
//!   与 `PRAGMA foreign_keys=ON`

use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous};
use sqlx::{Executor, Row, Sqlite, SqlitePool};
use tracing::{info, warn};

use crate::models::{fmt_iso, now_cst, EntryStatus, Game, SyncLog, VersionAnchor};

/// 建表语句：`games` / `version_anchors` / `sync_logs`。
///
/// 相比业务规格额外包含 `games.minor_rollover`（版本号小版本回卷阈值），
/// 用于 42 天级联推算时正确生成 `3.9 → 4.0` / `6.8 → 7.0` 这类大版本跳变。
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS games (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    miyoushe_gid INTEGER NOT NULL,
    miyoushe_slug TEXT NOT NULL,
    default_cycle_days INTEGER NOT NULL DEFAULT 42,
    livestream_offset_days INTEGER NOT NULL DEFAULT 12,
    livestream_default_time TEXT NOT NULL DEFAULT '19:30',
    minor_rollover INTEGER NOT NULL DEFAULT 9,
    last_sync_at TEXT
);

CREATE TABLE IF NOT EXISTS version_anchors (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    game_id TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    version_code TEXT NOT NULL,
    version_title TEXT,
    livestream_at TEXT,
    livestream_status TEXT NOT NULL DEFAULT 'predicted_42d',
    start_at TEXT NOT NULL,
    start_status TEXT NOT NULL DEFAULT 'official_confirmed',
    duration_days INTEGER NOT NULL DEFAULT 42,
    source_post_id TEXT,
    source_url TEXT,
    updated_at TEXT NOT NULL,
    UNIQUE(game_id, version_code)
);

CREATE TABLE IF NOT EXISTS sync_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    game_id TEXT NOT NULL,
    scanned_count INTEGER NOT NULL DEFAULT 0,
    updated_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL,
    detail TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_anchors_game ON version_anchors(game_id, version_code);
CREATE INDEX IF NOT EXISTS idx_sync_logs_created ON sync_logs(created_at DESC);
"#;

/// 首次创建时写入的基准数据（经官方公告核实的 2026-09 / 2026-10 锚点）。
struct SeedAnchor {
    game_id: &'static str,
    version_code: &'static str,
    version_title: &'static str,
    livestream_at: &'static str,
    start_at: &'static str,
    duration_days: i64,
    source_post_id: Option<&'static str>,
    source_url: Option<&'static str>,
}

const SEED_GAMES: &[(&str, &str, i64, &str, i64, i64, &str, i64)] = &[
    // (id, name, gid, slug, default_cycle_days, livestream_offset_days, livestream_default_time, minor_rollover)
    ("ys", "原神", 2, "ys", 42, 12, "20:00", 8),
    ("sr", "崩坏：星穹铁道", 6, "sr", 42, 12, "19:30", 9),
    ("zzz", "绝区零", 8, "zzz", 42, 12, "19:30", 9),
];

const SEED_ANCHORS: &[SeedAnchor] = &[
    SeedAnchor {
        game_id: "zzz",
        version_code: "3.3",
        version_title: "重返天空的旅程",
        livestream_at: "2026-10-09T19:30:00+08:00",
        start_at: "2026-10-21T06:00:00+08:00",
        duration_days: 42,
        source_post_id: Some("78804028"),
        source_url: Some("https://www.miyoushe.com/zzz/article/78804028"),
    },
    SeedAnchor {
        game_id: "ys",
        version_code: "7.1",
        version_title: "往冥府的安魂歌",
        livestream_at: "2026-09-12T20:00:00+08:00",
        start_at: "2026-09-23T06:00:00+08:00",
        duration_days: 42,
        source_post_id: Some("166086"),
        source_url: Some("https://genshin.hoyoverse.com/zh-tw/news/detail/166086"),
    },
    SeedAnchor {
        game_id: "sr",
        version_code: "4.6",
        version_title: "月升之前，与兽共舞",
        livestream_at: "2026-09-20T19:30:00+08:00",
        start_at: "2026-09-28T06:00:00+08:00",
        duration_days: 42,
        source_post_id: None,
        source_url: Some("https://sr.mihoyo.com/"),
    },
];

/// 从 SQLite URL 中提取库文件路径（用于自动创建父目录）。
fn sqlite_file_path(url: &str) -> Option<PathBuf> {
    let rest = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))?;
    let path = rest.split('?').next().unwrap_or(rest);
    if path.is_empty() || path == ":memory:" {
        return None;
    }
    Some(PathBuf::from(path))
}

/// 建立连接池；WAL + 外键约束 + busy_timeout。
pub async fn init_pool(database_url: &str) -> anyhow::Result<SqlitePool> {
    if let Some(file) = sqlite_file_path(database_url) {
        if let Some(parent) = file.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)?;
                info!(dir = %parent.display(), "已创建 SQLite 数据目录");
            }
        }
    }

    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(10))
        .pragma("temp_store", "memory");

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(15))
        .connect_with(options)
        .await?;

    info!(url = %database_url, "SQLite 连接池就绪（journal_mode=WAL, foreign_keys=ON）");
    Ok(pool)
}

/// 幂等自动建表 + 轻量升级（为历史库补列）。
pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    for stmt in SCHEMA.split(';') {
        let stmt = stmt.trim();
        if stmt.is_empty() {
            continue;
        }
        sqlx::query(stmt).execute(pool).await?;
    }

    // `minor_rollover` 为后加列：老库需 ALTER。
    let has_col: i64 = sqlx::query("SELECT COUNT(*) AS n FROM pragma_table_info('games') WHERE name = 'minor_rollover'")
        .fetch_one(pool)
        .await?
        .try_get("n")?;
    if has_col == 0 {
        sqlx::query("ALTER TABLE games ADD COLUMN minor_rollover INTEGER NOT NULL DEFAULT 9")
            .execute(pool)
            .await?;
        warn!("games.minor_rollover 缺失，已自动补齐（默认 9：3.9 → 4.0）");
    }

    info!("数据库结构检查完成（CREATE TABLE IF NOT EXISTS）");
    Ok(())
}

/// 首次运行写入基准锚点；已有锚点的游戏不会被覆盖。
pub async fn seed(pool: &SqlitePool) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    for (id, name, gid, slug, cycle, offset, clock, rollover) in SEED_GAMES {
        sqlx::query(
            "INSERT INTO games (id, name, miyoushe_gid, miyoushe_slug, default_cycle_days, livestream_offset_days, livestream_default_time, minor_rollover)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO NOTHING",
        )
        .bind(id)
        .bind(name)
        .bind(gid)
        .bind(slug)
        .bind(cycle)
        .bind(offset)
        .bind(clock)
        .bind(rollover)
        .execute(&mut *tx)
        .await?;
    }

    let now = fmt_iso(now_cst());
    let mut seeded = 0usize;
    for a in SEED_ANCHORS {
        let existing: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM version_anchors WHERE game_id = ?")
                .bind(a.game_id)
                .fetch_one(&mut *tx)
                .await?;
        if existing > 0 {
            continue;
        }
        sqlx::query(
            "INSERT INTO version_anchors
               (game_id, version_code, version_title, livestream_at, livestream_status, start_at, start_status, duration_days, source_post_id, source_url, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(game_id, version_code) DO NOTHING",
        )
        .bind(a.game_id)
        .bind(a.version_code)
        .bind(a.version_title)
        .bind(a.livestream_at)
        .bind(EntryStatus::OfficialConfirmed.as_str())
        .bind(a.start_at)
        .bind(EntryStatus::OfficialConfirmed.as_str())
        .bind(a.duration_days)
        .bind(a.source_post_id)
        .bind(a.source_url)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        seeded += 1;
    }

    tx.commit().await?;
    if seeded > 0 {
        info!(count = seeded, "已写入基准版本锚点（Seed）");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 读取
// ---------------------------------------------------------------------------

fn map_game(row: &SqliteRow) -> Game {
    Game {
        id: row.get("id"),
        name: row.get("name"),
        miyoushe_gid: row.get("miyoushe_gid"),
        miyoushe_slug: row.get("miyoushe_slug"),
        default_cycle_days: row.get("default_cycle_days"),
        livestream_offset_days: row.get("livestream_offset_days"),
        livestream_default_time: row.get("livestream_default_time"),
        minor_rollover: row.get("minor_rollover"),
        last_sync_at: row.get("last_sync_at"),
    }
}

fn map_anchor(row: &SqliteRow) -> VersionAnchor {
    let start_status: String = row.get("start_status");
    let livestream_status: String = row.get("livestream_status");
    VersionAnchor {
        id: row.get("id"),
        game_id: row.get("game_id"),
        version_code: row.get("version_code"),
        version_title: row.get("version_title"),
        livestream_at: row.get("livestream_at"),
        livestream_status: EntryStatus::from_str(&livestream_status).unwrap_or(EntryStatus::Predicted42d),
        start_at: row.get("start_at"),
        start_status: EntryStatus::from_str(&start_status).unwrap_or(EntryStatus::Predicted42d),
        duration_days: row.get("duration_days"),
        source_post_id: row.get("source_post_id"),
        source_url: row.get("source_url"),
        updated_at: row.get("updated_at"),
    }
}

pub async fn list_games(pool: &SqlitePool) -> anyhow::Result<Vec<Game>> {
    let rows = sqlx::query(
        "SELECT id, name, miyoushe_gid, miyoushe_slug, default_cycle_days, livestream_offset_days, livestream_default_time, minor_rollover, last_sync_at
         FROM games ORDER BY miyoushe_gid ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(map_game).collect())
}

pub async fn get_game(pool: &SqlitePool, game_id: &str) -> anyhow::Result<Option<Game>> {
    let row = sqlx::query(
        "SELECT id, name, miyoushe_gid, miyoushe_slug, default_cycle_days, livestream_offset_days, livestream_default_time, minor_rollover, last_sync_at
         FROM games WHERE id = ?",
    )
    .bind(game_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(map_game))
}

pub async fn list_anchors(pool: &SqlitePool) -> anyhow::Result<Vec<VersionAnchor>> {
    let rows = sqlx::query(
        "SELECT id, game_id, version_code, version_title, livestream_at, livestream_status, start_at, start_status, duration_days, source_post_id, source_url, updated_at
         FROM version_anchors ORDER BY game_id ASC, id ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(map_anchor).collect())
}

/// 在任意 executor（连接池 / 事务）上读取单行锚点。
async fn fetch_anchor<'e, E>(
    exec: E,
    game_id: &str,
    version_code: &str,
) -> anyhow::Result<Option<VersionAnchor>>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query(
        "SELECT id, game_id, version_code, version_title, livestream_at, livestream_status, start_at, start_status, duration_days, source_post_id, source_url, updated_at
         FROM version_anchors WHERE game_id = ? AND version_code = ?",
    )
    .bind(game_id)
    .bind(version_code)
    .fetch_optional(exec)
    .await?;
    Ok(row.as_ref().map(map_anchor))
}

pub async fn get_anchor(
    pool: &SqlitePool,
    game_id: &str,
    version_code: &str,
) -> anyhow::Result<Option<VersionAnchor>> {
    fetch_anchor(pool, game_id, version_code).await
}

// ---------------------------------------------------------------------------
// 写入
// ---------------------------------------------------------------------------

/// 完整写入一行锚点（`ON CONFLICT(game_id, version_code) DO UPDATE`）。
#[allow(clippy::too_many_arguments)]
pub async fn upsert_anchor(
    pool: &SqlitePool,
    game_id: &str,
    version_code: &str,
    version_title: Option<&str>,
    livestream_at: Option<&str>,
    livestream_status: EntryStatus,
    start_at: &str,
    start_status: EntryStatus,
    duration_days: i64,
    source_post_id: Option<&str>,
    source_url: Option<&str>,
) -> anyhow::Result<()> {
    let now = fmt_iso(now_cst());
    sqlx::query(
        "INSERT INTO version_anchors
           (game_id, version_code, version_title, livestream_at, livestream_status, start_at, start_status, duration_days, source_post_id, source_url, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(game_id, version_code) DO UPDATE SET
           version_title     = COALESCE(excluded.version_title, version_anchors.version_title),
           livestream_at     = COALESCE(excluded.livestream_at, version_anchors.livestream_at),
           livestream_status = excluded.livestream_status,
           start_at          = excluded.start_at,
           start_status      = excluded.start_status,
           duration_days     = excluded.duration_days,
           source_post_id    = COALESCE(excluded.source_post_id, version_anchors.source_post_id),
           source_url        = COALESCE(excluded.source_url, version_anchors.source_url),
           updated_at        = excluded.updated_at",
    )
    .bind(game_id)
    .bind(version_code)
    .bind(version_title)
    .bind(livestream_at)
    .bind(livestream_status.as_str())
    .bind(start_at)
    .bind(start_status.as_str())
    .bind(duration_days)
    .bind(source_post_id)
    .bind(source_url)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// 官方抓取结果的增量更新意图（字段为 `None` 表示本次未识别到）。
#[derive(Debug, Default, Clone)]
pub struct OfficialUpdate {
    pub version_code: String,
    pub version_title: Option<String>,
    pub livestream_at: Option<String>,
    pub start_at: Option<String>,
    pub duration_days: Option<i64>,
    pub source_post_id: Option<String>,
    pub source_url: Option<String>,
}

/// 一次官方合并的结果。
#[derive(Debug, Default, Clone)]
pub struct MergeOutcome {
    /// 是否有字段发生实质变化（用于 `updated_count`）。
    pub changed: bool,
    pub log: Vec<String>,
}

/// 合并官方抓取结果。策略：
/// * `manual_override` 的字段永不被官方数据覆盖（仅补写来源链接与未锁定字段）；
/// * 新版本节点必须带有可解析的开启时间才会落库，否则仅登记来源链接。
pub async fn apply_official_update(
    pool: &SqlitePool,
    game: &Game,
    upd: &OfficialUpdate,
) -> anyhow::Result<MergeOutcome> {
    let existing = get_anchor(pool, &game.id, &upd.version_code).await?;
    let mut out = MergeOutcome::default();

    match existing {
        None => {
            let Some(start_at) = upd.start_at.as_deref() else {
                out.log
                    .push(format!("版本 {} 尚无官方开启时间，仅登记来源链接", upd.version_code));
                return Ok(out);
            };
            let livestream_status = if upd.livestream_at.is_some() {
                EntryStatus::OfficialConfirmed
            } else {
                EntryStatus::Predicted42d
            };
            upsert_anchor(
                pool,
                &game.id,
                &upd.version_code,
                upd.version_title.as_deref(),
                upd.livestream_at.as_deref(),
                livestream_status,
                start_at,
                EntryStatus::OfficialConfirmed,
                upd.duration_days.unwrap_or(game.default_cycle_days),
                upd.source_post_id.as_deref(),
                upd.source_url.as_deref(),
            )
            .await?;
            out.changed = true;
            out.log
                .push(format!("新建锚点 {}（official_confirmed，开启 {start_at}）", upd.version_code));
        }
        Some(cur) => {
            let start_locked = cur.start_status == EntryStatus::ManualOverride;
            let live_locked = cur.livestream_status == EntryStatus::ManualOverride;

            let start_at = if start_locked { None } else { upd.start_at.clone() };
            let livestream_at = if live_locked { None } else { upd.livestream_at.clone() };
            let duration = if start_locked { None } else { upd.duration_days };
            let title = if cur.version_title.is_none() || !start_locked {
                upd.version_title.clone()
            } else {
                None
            };

            out.changed = start_at.is_some() || livestream_at.is_some() || title.is_some();

            upsert_anchor(
                pool,
                &game.id,
                &upd.version_code,
                title.as_deref(),
                livestream_at.as_deref(),
                if livestream_at.is_some() {
                    EntryStatus::OfficialConfirmed
                } else {
                    cur.livestream_status
                },
                start_at.as_deref().unwrap_or(&cur.start_at),
                if start_at.is_some() {
                    EntryStatus::OfficialConfirmed
                } else if start_locked {
                    EntryStatus::ManualOverride
                } else {
                    cur.start_status
                },
                duration.unwrap_or(cur.duration_days),
                upd.source_post_id.as_deref(),
                upd.source_url.as_deref(),
            )
            .await?;

            if start_locked && upd.start_at.is_some() {
                out.log
                    .push(format!("版本 {} 开启时间已手动校准锁定，保留现值", upd.version_code));
            }
            if live_locked && upd.livestream_at.is_some() {
                out.log
                    .push(format!("版本 {} 前瞻时间已手动校准锁定，保留现值", upd.version_code));
            }
            if let Some(s) = &start_at {
                out.log.push(format!("更新版本 {} 开启时间 → {}", upd.version_code, s));
            }
            if let Some(l) = &livestream_at {
                out.log.push(format!("更新版本 {} 前瞻直播 → {}", upd.version_code, l));
            }
            if !out.changed {
                out.log.push(format!("版本 {} 无新增官方信息", upd.version_code));
            }
        }
    }

    Ok(out)
}

/// 手动校准：新增或整行改写，两个字段状态均置为 `manual_override`。
#[allow(clippy::too_many_arguments)]
pub async fn write_manual_override(
    pool: &SqlitePool,
    game: &Game,
    from_code: &str,
    to_code: &str,
    version_title: Option<&str>,
    start_at: &str,
    livestream_at: Option<&str>,
    duration_days: i64,
) -> anyhow::Result<()> {
    let now = fmt_iso(now_cst());
    let mut tx = pool.begin().await?;

    let previous = fetch_anchor(&mut *tx, &game.id, from_code).await?;
    let (source_post_id, source_url, prev_title) = previous
        .as_ref()
        .map(|p| (p.source_post_id.clone(), p.source_url.clone(), p.version_title.clone()))
        .unwrap_or((None, None, None));

    if from_code != to_code {
        sqlx::query("DELETE FROM version_anchors WHERE game_id = ? AND version_code = ?")
            .bind(&game.id)
            .bind(from_code)
            .execute(&mut *tx)
            .await?;
    }

    sqlx::query(
        "INSERT INTO version_anchors
           (game_id, version_code, version_title, livestream_at, livestream_status, start_at, start_status, duration_days, source_post_id, source_url, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(game_id, version_code) DO UPDATE SET
           version_title     = excluded.version_title,
           livestream_at     = excluded.livestream_at,
           livestream_status = excluded.livestream_status,
           start_at          = excluded.start_at,
           start_status      = excluded.start_status,
           duration_days     = excluded.duration_days,
           updated_at        = excluded.updated_at",
    )
    .bind(&game.id)
    .bind(to_code)
    .bind(version_title.or(prev_title.as_deref()))
    .bind(livestream_at)
    .bind(EntryStatus::ManualOverride.as_str())
    .bind(start_at)
    .bind(EntryStatus::ManualOverride.as_str())
    .bind(duration_days)
    .bind(source_post_id)
    .bind(source_url)
    .bind(&now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// `DELETE` 手动校准行，使其回到 42 天自动推算。
pub async fn delete_anchor(pool: &SqlitePool, game_id: &str, version_code: &str) -> anyhow::Result<u64> {
    let res = sqlx::query("DELETE FROM version_anchors WHERE game_id = ? AND version_code = ?")
        .bind(game_id)
        .bind(version_code)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

pub async fn insert_sync_log(
    pool: &SqlitePool,
    game_id: Option<&str>,
    scanned_count: i64,
    updated_count: i64,
    status: &str,
    detail: Option<&str>,
) -> anyhow::Result<()> {
    let now = fmt_iso(now_cst());
    sqlx::query(
        "INSERT INTO sync_logs (game_id, scanned_count, updated_count, status, detail, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(game_id.unwrap_or("all"))
    .bind(scanned_count)
    .bind(updated_count)
    .bind(status)
    .bind(detail)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// 最近一条**汇总**同步日志（`game_id = 'all'`，展示给前端的全局状态）。
pub async fn latest_summary_sync_log(pool: &SqlitePool) -> anyhow::Result<Option<SyncLog>> {
    latest_sync_log_where(pool, Some("all")).await
}

async fn latest_sync_log_where(pool: &SqlitePool, game_id: Option<&str>) -> anyhow::Result<Option<SyncLog>> {
    let row = if let Some(gid) = game_id {
        sqlx::query(
            "SELECT id, game_id, scanned_count, updated_count, status, detail, created_at
             FROM sync_logs WHERE game_id = ? ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(gid)
        .fetch_optional(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT id, game_id, scanned_count, updated_count, status, detail, created_at
             FROM sync_logs ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await?
    };
    Ok(row.map(|r| SyncLog {
        id: r.get("id"),
        game_id: Some(r.get::<String, _>("game_id")),
        scanned_count: r.get("scanned_count"),
        updated_count: r.get("updated_count"),
        status: r.get("status"),
        detail: r.get("detail"),
        created_at: r.get("created_at"),
    }))
}

pub async fn touch_last_sync_at(pool: &SqlitePool, game_id: &str) -> anyhow::Result<()> {
    let now = fmt_iso(now_cst());
    sqlx::query("UPDATE games SET last_sync_at = ? WHERE id = ?")
        .bind(&now)
        .bind(game_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_path_extraction_ignores_query_params() {
        assert_eq!(
            sqlite_file_path("sqlite:///app/data/schedule.db?mode=rwc").unwrap(),
            PathBuf::from("/app/data/schedule.db")
        );
        assert_eq!(
            sqlite_file_path("sqlite:./data/schedule.db").unwrap(),
            PathBuf::from("./data/schedule.db")
        );
        assert!(sqlite_file_path("sqlite::memory:").is_none());
    }

    #[test]
    fn schema_contains_all_three_tables() {
        for t in ["CREATE TABLE IF NOT EXISTS games", "version_anchors", "sync_logs"] {
            assert!(SCHEMA.contains(t), "缺少 {t}");
        }
    }
}
