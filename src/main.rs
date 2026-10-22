//! 米哈游版本前瞻日历 —— Axum 服务入口。
//!
//! * SQLite 自动建表 / Seed 初始化
//! * 后台定时轮询米游社官方公告（`SYNC_INTERVAL_HOURS`，默认 6 小时）
//! * REST API + iCalendar 订阅 + Svelte 前端静态托管（同源部署）

mod db;
mod engine;
mod ics;
mod models;
mod scraper;

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Serialize;
use sqlx::SqlitePool;
use tokio::time::MissedTickBehavior;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{error, info};

use crate::models::{
    fmt_iso, now_cst, EntryStatus, IcsQuery, OverrideRequest, OverrideResponse, SchedulesResponse,
    SyncResponse, VersionCode,
};

/// 共享应用状态。
pub struct AppState {
    pub pool: SqlitePool,
    pub http: reqwest::Client,
    pub admin_token: Option<String>,
    pub api_bases: Vec<String>,
    pub dist_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
struct Health {
    ok: bool,
    name: &'static str,
    version: &'static str,
    tz: &'static str,
}

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

struct Config {
    database_url: String,
    port: u16,
    sync_interval_hours: u64,
    sync_on_start: bool,
    admin_token: Option<String>,
    api_bases: Vec<String>,
    dist_dir: PathBuf,
}

impl Config {
    fn from_env() -> Config {
        let env = |key: &str, default: &str| std::env::var(key).unwrap_or_else(|_| default.to_string());

        let sync_interval_hours = env("SYNC_INTERVAL_HOURS", "6").parse::<u64>().unwrap_or(6).max(1);
        let sync_on_start = env("SYNC_ON_START", "true").to_ascii_lowercase() == "true";
        let port = env("PORT", "8080").parse::<u16>().unwrap_or(8080);

        let api_bases = env(
            "MIYOUSHE_API_BASES",
            "https://bbs-api.miyoushe.com,https://api-takumi.mihoyo.com,https://bbs-api-static.miyoushe.com",
        )
        .split(',')
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();

        Config {
            database_url: env("DATABASE_URL", "sqlite:///app/data/schedule.db?mode=rwc"),
            port,
            sync_interval_hours,
            sync_on_start,
            admin_token: std::env::var("ADMIN_TOKEN").ok().filter(|t| !t.is_empty()),
            api_bases,
            dist_dir: PathBuf::from(env("WEB_DIST", "web/dist")),
        }
    }
}

// ---------------------------------------------------------------------------
// 错误类型
// ---------------------------------------------------------------------------

enum AppError {
    BadRequest(String),
    Unauthorized(&'static str),
    NotFound(String),
    Conflict(String),
    Internal(anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            AppError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m.to_string()),
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, m.clone()),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m.clone()),
            AppError::Internal(e) => {
                error!("内部错误：{e:#}");
                (StatusCode::INTERNAL_SERVER_ERROR, "服务器内部错误".to_string())
            }
        };
        (status, Json(models::ErrorBody { error: msg })).into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Internal(e)
    }
}

// ---------------------------------------------------------------------------
// 授权
// ---------------------------------------------------------------------------

fn check_auth(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    if let Some(token) = &state.admin_token {
        let expected = format!("Bearer {token}");
        let provided = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim());
        if provided != Some(expected.as_str()) {
            return Err(AppError::Unauthorized("需要有效的 Authorization: Bearer <token>"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 处理器
// ---------------------------------------------------------------------------

async fn build_schedules(state: &AppState) -> anyhow::Result<SchedulesResponse> {
    let games = db::list_games(&state.pool).await?;
    let anchors = db::list_anchors(&state.pool).await?;
    let sync = db::latest_summary_sync_log(&state.pool).await?;
    let now = now_cst();
    let schedules = engine::build_all(&games, &anchors, now);
    let next_up = engine::next_up(&schedules);
    Ok(SchedulesResponse {
        server_time: fmt_iso(now),
        tz: "Asia/Shanghai".to_string(),
        games: schedules,
        sync,
        next_up,
    })
}

async fn handle_health(State(state): State<Arc<AppState>>) -> Result<Json<Health>, AppError> {
    let _ = &state;
    Ok(Json(Health {
        ok: true,
        name: "mihoyo-schedule-tracker",
        version: env!("CARGO_PKG_VERSION"),
        tz: "Asia/Shanghai",
    }))
}

async fn handle_schedules(State(state): State<Arc<AppState>>) -> Result<Json<SchedulesResponse>, AppError> {
    Ok(Json(build_schedules(&state).await?))
}

async fn handle_sync(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<SyncResponse>, AppError> {
    check_auth(&state, &headers)?;
    let results = scraper::sync_all(&state).await;
    let scanned_count = results.iter().map(|r| r.scanned_count).sum();
    let updated_count = results.iter().map(|r| r.updated_count).sum();
    let schedules = build_schedules(&state).await?;
    Ok(Json(SyncResponse {
        ok: true,
        scanned_count,
        updated_count,
        results,
        schedules,
    }))
}

async fn handle_override(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<OverrideRequest>,
) -> Result<Json<OverrideResponse>, AppError> {
    check_auth(&state, &headers)?;

    if VersionCode::parse(&req.version_code).is_none() {
        return Err(AppError::BadRequest(format!("version_code 格式应为 X.Y：{}", req.version_code)));
    }
    let to_code = req.new_version_code.clone().unwrap_or_else(|| req.version_code.clone());
    if VersionCode::parse(&to_code).is_none() {
        return Err(AppError::BadRequest(format!("new_version_code 格式应为 X.Y：{to_code}")));
    }

    let game = db::get_game(&state.pool, &req.game_id)
        .await?
        .ok_or_else(|| AppError::BadRequest(format!("未知游戏：{}", req.game_id)))?;

    let existing = db::get_anchor(&state.pool, &game.id, &req.version_code).await?;

    // 版本开启时间：请求体优先，其次沿用已有锚点，都没有则报错。
    let start_at = match &req.start_at {
        Some(s) => models::parse_flexible(s, chrono::NaiveTime::from_hms_opt(6, 0, 0).expect("06:00 合法"))
            .map_err(|e| AppError::BadRequest(format!("start_at 无法解析：{e}")))?,
        None => existing
            .as_ref()
            .map(|a| a.start_dt())
            .ok_or_else(|| AppError::BadRequest("缺少 start_at（且该版本尚无锚点）".to_string()))?,
    };

    let livestream_at = match &req.livestream_at {
        Some(s) => Some(
            models::parse_flexible(s, game.livestream_time())
                .map_err(|e| AppError::BadRequest(format!("livestream_at 无法解析：{e}")))?,
        ),
        None => existing.as_ref().and_then(|a| a.livestream_dt()),
    };

    let duration = req
        .duration_days
        .unwrap_or(existing.as_ref().map(|a| a.duration_days).unwrap_or(game.default_cycle_days));
    if !(14..=98).contains(&duration) {
        return Err(AppError::BadRequest("duration_days 应在 14–98 天之间".to_string()));
    }

    db::write_manual_override(
        &state.pool,
        &game,
        &req.version_code,
        &to_code,
        req.version_title.as_deref(),
        &fmt_iso(start_at),
        livestream_at.as_ref().map(|d| fmt_iso(*d)).as_deref(),
        duration,
    )
    .await?;

    info!(game = %game.id, version = %to_code, "手动校准已写入 version_anchors");
    Ok(Json(OverrideResponse {
        ok: true,
        deleted: false,
        schedules: build_schedules(&state).await?,
    }))
}

async fn handle_override_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((game_id, version_code)): Path<(String, String)>,
) -> Result<Json<OverrideResponse>, AppError> {
    check_auth(&state, &headers)?;

    let anchor = db::get_anchor(&state.pool, &game_id, &version_code)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("未找到 {game_id} {version_code} 的锚点记录")))?;

    if anchor.start_status != EntryStatus::ManualOverride && anchor.livestream_status != EntryStatus::ManualOverride {
        return Err(AppError::Conflict(
            "该记录由官方同步维护（official_confirmed），不可直接删除；请先手动校准再删除".to_string(),
        ));
    }

    db::delete_anchor(&state.pool, &game_id, &version_code).await?;
    info!(game = %game_id, version = %version_code, "已删除手动校准锚点，恢复 42 天自动推算");
    Ok(Json(OverrideResponse {
        ok: true,
        deleted: true,
        schedules: build_schedules(&state).await?,
    }))
}

async fn handle_calendar(
    State(state): State<Arc<AppState>>,
    Query(q): Query<IcsQuery>,
) -> Result<Response, AppError> {
    let games = db::list_games(&state.pool).await?;
    let anchors = db::list_anchors(&state.pool).await?;
    let now = now_cst();
    let schedules = engine::build_all(&games, &anchors, now);
    let body = ics::build_calendar(&schedules, &q);

    Ok(Response::builder()
        .header(header::CONTENT_TYPE, ics::content_type())
        .header(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("inline; filename=\"mihoyo-schedule.ics\""),
        )
        .body(axum::body::Body::from(body))
        .expect("valid response"))
}

async fn api_not_found(Path(path): Path<String>) -> AppError {
    AppError::NotFound(format!("接口不存在：/api/{path}"))
}

// ---------------------------------------------------------------------------
// 后台定时同步
// ---------------------------------------------------------------------------

/// 周期性执行 `job`：先消费 `tokio::time::interval` 首个立即返回的 tick，
/// 确保首个完整周期满后才执行任务——否则 `SYNC_ON_START=false` 时仍会在启动瞬间同步一次。
async fn run_periodic<F, Fut>(mut interval: tokio::time::Interval, mut job: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    interval.tick().await;
    loop {
        interval.tick().await;
        job().await;
    }
}

async fn run_background_sync(state: Arc<AppState>, interval_hours: u64, sync_on_start: bool) {
    if sync_on_start {
        tokio::time::sleep(Duration::from_secs(5)).await;
        info!("启动后首次官方同步");
        scraper::sync_all(&state).await;
    }

    let state = state.clone();
    let interval = tokio::time::interval(Duration::from_secs(interval_hours.saturating_mul(3600)));
    run_periodic(interval, move || {
        let state = Arc::clone(&state);
        async move {
            info!("后台定时同步开始（每 {interval_hours} 小时）");
            scraper::sync_all(&state).await;
        }
    })
    .await;
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,mihoyo_schedule_tracker=info")),
        )
        .with_target(false)
        .init();

    let config = Config::from_env();
    info!(port = config.port, dist = %config.dist_dir.display(), "mihoyo-schedule-tracker 启动");

    // SQLite：WAL + 外键 + 自动建表 + Seed
    let pool = db::init_pool(&config.database_url).await?;
    db::migrate(&pool).await?;
    db::seed(&pool).await?;

    let http = reqwest::Client::builder()
        .user_agent(
            "Mozilla/5.0 (compatible; mihoyo-schedule-tracker/0.1; +https://github.com/; AppleWebKit/537.36)",
        )
        .timeout(Duration::from_secs(20))
        .gzip(true)
        .build()?;

    let state = Arc::new(AppState {
        pool,
        http,
        admin_token: config.admin_token.clone(),
        api_bases: config.api_bases.clone(),
        dist_dir: config.dist_dir.clone(),
    });

    // 后台定时任务：每 6 小时抓取一次官方公告
    let bg = state.clone();
    tokio::spawn(run_background_sync(bg, config.sync_interval_hours, config.sync_on_start));

    // 静态资源：Svelte 构建产物（web/dist），SPA fallback 到 index.html
    let index = config.dist_dir.join("index.html");
    let serve_dir = ServeDir::new(&config.dist_dir).fallback(ServeFile::new(index));

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/health", get(handle_health))
        .route("/api/schedules", get(handle_schedules))
        .route("/api/sync", post(handle_sync))
        .route("/api/override", post(handle_override))
        .route("/api/override/{game_id}/{version_code}", delete(handle_override_delete))
        .route("/calendar.ics", get(handle_calendar))
        .route("/api/{*rest}", get(api_not_found))
        .fallback_service(serve_dir)
        .with_state(state)
        .layer(cors)
        .layer(CompressionLayer::new());

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", config.port)).await?;
    info!(addr = %listener.local_addr()?, "HTTP 服务已监听");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            info!("收到退出信号，正在关闭…");
        })
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// `tokio::time::interval` 的首个 tick 立即返回：若循环直接消费它，
    /// 启动瞬间就会执行任务（违背 `SYNC_ON_START=false` 语义）。
    /// `run_periodic` 必须先消费首个 tick，首个完整周期满后才执行首次任务。
    #[tokio::test(start_paused = true)]
    async fn periodic_loop_skips_immediate_first_tick() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        let interval = tokio::time::interval(Duration::from_secs(1));
        let handle = tokio::spawn(async move {
            run_periodic(interval, move || {
                let c = Arc::clone(&c);
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                }
            })
            .await;
        });

        tokio::time::advance(Duration::from_millis(500)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0, "首个周期未满（0.5s < 1s），不得执行任务");

        tokio::time::advance(Duration::from_millis(600)).await;
        tokio::task::yield_now().await; // 让被定时器唤醒的任务先执行
        assert_eq!(calls.load(Ordering::SeqCst), 1, "满一个周期（1.1s）后应执行首次任务");

        handle.abort();
    }
}
