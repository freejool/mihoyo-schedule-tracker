//! 米游社公开资讯接口抓取 + 正则提取 + SQLite UPSERT 自动校准。
//!
//! 接口：`GET {base}/post/wapi/getNewsList?gids={gids}&page_size=20&type={type}`
//! `type` 依次轮询 `1`（公告）与 `3`（资讯），无需登录 Cookie。
//!
//! 实测（2026-10 数据）：`bbs-api.miyoushe.com` 边缘节点对无签名请求返回 404，
//! `api-takumi.mihoyo.com` 正常返回；因此内置候选域名列表依次尝试。
//! 列表接口的 `post.content` 为空串，时间信息主要来自 `subject` 与 `created_at`。

use std::collections::BTreeMap;
use std::sync::LazyLock;

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveTime, TimeZone};
use regex::Regex;
use reqwest::StatusCode;
use serde_json::Value;
use tracing::{info, warn};

use crate::db::{apply_official_update, insert_sync_log, touch_last_sync_at, OfficialUpdate};
use crate::models::{at_helper, cst, fmt_iso, Game, NewsListResponse, NewsPost, SyncGameResult};
use crate::AppState;

/// 每 6 小时后台轮询 + 前端按钮手动触发，共用的抓取逻辑。
pub const NEWS_TYPES: [(i64, &str); 2] = [(1, "公告"), (3, "资讯")];

// ---------------------------------------------------------------------------
// 正则
// ---------------------------------------------------------------------------

/// 版本号 + “前瞻特别节目”类标题。允许 `「标题」` 等中文间隔。
static RE_LIVE_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)(\d{1,2}\.\d{1,2})\s*版本[^\n]{0,50}?(前瞻特别节目|预览特别节目|特别节目前瞻|前瞻直播)")
        .expect("static regex")
});

/// 命中该词的帖子并非“直播当天”的节目帖（预告 / 回放等）。
static RE_LIVE_NEGATIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(预告|回顾|回放|总结|集锦|花絮|预热|竞猜|问卷|直播预约|抽奖|壁纸|表情包)")
        .expect("static regex")
});

/// 版本开启公告类标题：`X.Y版本…更新说明/更新公告/更新维护/预下载…更新`。
static RE_UPDATE_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)(\d{1,2}\.\d{1,2})\s*版本[^\n]{0,50}?(更新说明|更新公告|更新维护|版本更新)")
        .expect("static regex")
});

/// 命中该词的公告与“版本开启”无关（已知问题 / 活动 / 商城等）。
static RE_UPDATE_NEGATIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(已知问题|问题集中反馈|游戏优化|活动说明|活动跃迁|商城|礼包|频段|编辑器|工具更新|月历|角色|壁纸|表情包|卡池|祈愿)")
        .expect("static regex")
});

/// 预下载 / 更新预告帖：只登记来源链接，不推断开启日期。
static RE_PRELOAD_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)(\d{1,2}\.\d{1,2})\s*版本[^\n]{0,50}?(预下载|更新预告)")
        .expect("static regex")
});

/// 完整日期时间：`2026/10/9 19:30`、`2026年10月9日 19:30`。
static RE_DATE_FULL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<y>20\d{2})\s*[-/年]\s*(?P<m>\d{1,2})\s*[-/月]\s*(?P<d>\d{1,2})\s*日?[^\d]{0,6}(?P<hh>\d{1,2})\s*[:：]\s*(?P<mm>\d{2})")
        .expect("static regex")
});

/// 月日时刻：`10月9日 19:30` / `10月9日19：30`。
static RE_DATE_MD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<m>\d{1,2})\s*月\s*(?P<d>\d{1,2})\s*日[^\d]{0,6}(?P<hh>\d{1,2})\s*[:：]\s*(?P<mm>\d{2})")
        .expect("static regex")
});

/// 仅有日期：`2026/10/9`、`2026年10月9日`。
static RE_DATE_BARE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<y>20\d{2})\s*[-/年]\s*(?P<m>\d{1,2})\s*[-/月]\s*(?P<d>\d{1,2})\s*日?")
        .expect("static regex")
});

// ---------------------------------------------------------------------------
// 解析（纯函数，便于测试）
// ---------------------------------------------------------------------------

fn mk_dt(y: i32, m: u32, d: u32, hh: u32, mm: u32) -> Option<DateTime<FixedOffset>> {
    let date = chrono::NaiveDate::from_ymd_opt(y, m, d)?;
    let time = NaiveTime::from_hms_opt(hh, mm, 0)?;
    Some(
        cst()
            .from_local_datetime(&date.and_time(time))
            .single()
            .expect("UTC+8 无夏令时歧义"),
    )
}

/// 在文本片段中提取“最近的一个”日期时间。
fn extract_datetime(text: &str, hint: DateTime<FixedOffset>) -> Option<DateTime<FixedOffset>> {
    if let Some(c) = RE_DATE_FULL.captures(text) {
        let y = c["y"].parse().ok()?;
        let m = c["m"].parse().ok()?;
        let d = c["d"].parse().ok()?;
        let hh = c["hh"].parse().ok()?;
        let mm = c["mm"].parse().ok()?;
        return mk_dt(y, m, d, hh, mm);
    }
    if let Some(c) = RE_DATE_MD.captures(text) {
        let m: u32 = c["m"].parse().ok()?;
        let d: u32 = c["d"].parse().ok()?;
        let hh: u32 = c["hh"].parse().ok()?;
        let mm: u32 = c["mm"].parse().ok()?;
        // 无年份：以帖内时间为基准推断（避免跨年误判为去年）。
        let mut y = hint.year();
        if let Some(dt) = mk_dt(y, m, d, hh, mm) {
            if dt < hint - Duration::days(200) {
                y += 1;
            }
        }
        return mk_dt(y, m, d, hh, mm);
    }
    if let Some(c) = RE_DATE_BARE.captures(text) {
        let y = c["y"].parse().ok()?;
        let m = c["m"].parse().ok()?;
        let d = c["d"].parse().ok()?;
        return mk_dt(y, m, d, 6, 0);
    }
    None
}

/// 提取版本开启时间：优先显式日期（标题或正文），其次“更新公告当天 06:00”。
pub fn detect_start(post: &NewsPost) -> Option<(String, DateTime<FixedOffset>, &'static str)> {
    let hay = post.haystack();
    let caps = RE_UPDATE_TITLE.captures(&hay)?;
    // 干扰帖只按标题判定（已知问题 / 活动 / 商城等）。
    if RE_UPDATE_NEGATIVE.is_match(post.subject.trim()) {
        return None;
    }
    let version = caps[1].to_string();
    let hint = post.created_cst()?;

    // 显式日期优先（如正文“将于11月9日 06:00 开始更新维护”）。
    let m = caps.get(0)?;
    let start_idx = m.start().saturating_sub(60);
    let end_idx = (m.end() + 120).min(hay.len());
    if let Some(dt) = extract_datetime(&hay[start_idx..end_idx], hint) {
        return Some((version, dt, "显式日期"));
    }

    // 无显式日期：更新公告发布日 = 版本开启日，开服时刻 06:00。
    // 实测三家官方均在开服当日 07:00 发布“X.Y版本更新说明/更新公告”。
    let mut date = hint.date_naive();
    if hint.time() < NaiveTime::from_hms_opt(6, 0, 0).expect("06:00 合法") {
        date = date + Duration::days(1); // 深夜帖归属次日
    }
    Some((version, at_helper(date, NaiveTime::from_hms_opt(6, 0, 0).expect("06:00 合法")), "公告当日 06:00"))
}

/// 提取前瞻直播时间：显式时刻 > 直播窗口规则。
pub fn detect_livestream(post: &NewsPost, game: &Game) -> Option<(String, DateTime<FixedOffset>, &'static str)> {
    let hay = post.haystack();
    let caps = RE_LIVE_TITLE.captures(&hay)?;
    if RE_LIVE_NEGATIVE.is_match(&hay) {
        return None;
    }
    let version = caps[1].to_string();
    let hint = post.created_cst()?;

    let m = caps.get(0)?;
    let start_idx = m.start().saturating_sub(60);
    let end_idx = (m.end() + 120).min(hay.len());
    let near = &hay[start_idx..end_idx];

    if let Some(dt) = extract_datetime(near, hint) {
        return Some((version, dt, "显式时刻"));
    }

    // 直播窗口规则：官方“前瞻特别节目”帖在开播期间发布（实测 20:20 发布、19:30 开播），
    // 因此帖内日期 = 直播日期，时刻取该游戏默认开播时刻。
    let clock = game.livestream_time();
    let window_start = clock - Duration::minutes(30);
    let window_end = clock + Duration::hours(3);
    if hint.time() >= window_start && hint.time() <= window_end {
        return Some((version, at_helper(hint.date_naive(), clock), "直播窗口"));
    }
    None
}

/// 预下载 / 更新预告帖 → 仅登记来源（不推断日期）。
pub fn detect_preload(post: &NewsPost) -> Option<String> {
    let caps = RE_PRELOAD_TITLE.captures(post.subject.trim())?;
    if RE_UPDATE_NEGATIVE.is_match(post.subject.trim()) {
        return None;
    }
    Some(caps[1].to_string())
}

// ---------------------------------------------------------------------------
// 抓取与落库
// ---------------------------------------------------------------------------

/// 单游戏同步结果（写入 `sync_logs` 与 `games.last_sync_at`），即 REST DTO。
pub type GameSyncOutcome = SyncGameResult;

/// 展开 reqwest 错误链为单行文本（Display 只输出最外层；根因藏在 source 链里，
/// 例如连接层拒绝 https 时真正的错误是 `invalid URL, scheme is not http`）。
fn fmt_reqwest_err(e: &reqwest::Error) -> String {
    let mut out = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        out.push_str(" → ");
        out.push_str(&s.to_string());
        src = s.source();
    }
    out
}

/// 依次尝试候选 API 域名；成功即停。返回 `(base, body)`。
async fn fetch_news_list(
    state: &AppState,
    base: &str,
    gids: i64,
    news_type: i64,
) -> Result<Value, String> {
    let url = format!("{base}/post/wapi/getNewsList?gids={gids}&page_size=20&type={news_type}");
    let resp = state
        .http
        .get(&url)
        .header(reqwest::header::REFERER, format!("https://www.miyoushe.com/"))
        .send()
        .await
        .map_err(|e| format!("请求失败：{}", fmt_reqwest_err(&e)))?;

    if resp.status() != StatusCode::OK {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    let text = resp.text().await.map_err(|e| format!("读响应失败：{e}"))?;
    serde_json::from_str::<Value>(&text).map_err(|e| format!("JSON 解析失败：{e}（body 前 120 字符：{}）", &text[..text.len().min(120)]))
}

/// 在候选域名中抓取一个 `type` 的列表，返回新闻条目（任一域名成功即可）。
async fn fetch_type_once(state: &AppState, game: &Game, news_type: i64) -> Result<Vec<NewsPost>, String> {
    let mut last_err = String::new();
    for base in &state.api_bases {
        match fetch_news_list(state, base, game.miyoushe_gid, news_type).await {
            Ok(json) => {
                let parsed: NewsListResponse = serde_json::from_value(json)
                    .map_err(|e| format!("payload 结构不匹配：{e}"))?;
                if parsed.retcode != 0 {
                    return Err(format!("retcode={} message={:?}", parsed.retcode, parsed.message));
                }
                let posts: Vec<NewsPost> = parsed
                    .data
                    .map(|d| d.list.into_iter().map(|it| it.post).collect())
                    .unwrap_or_default();
                return Ok(posts);
            }
            Err(e) => {
                last_err = format!("{base} → {e}");
                warn!(base = %base, game = %game.id, type = news_type, "列表拉取失败");
            }
        }
    }
    Err(if last_err.is_empty() { "所有 API 域名均不可达".into() } else { last_err })
}

/// 同步单款游戏：扫描公告 + 资讯两个 type，正则识别后 UPSERT。
pub async fn sync_game(state: &AppState, game: &Game) -> GameSyncOutcome {
    let mut scanned = 0i64;
    let mut updated = 0i64;
    let mut detail_lines: Vec<String> = Vec::new();
    let mut any_ok = false;
    let mut first_err: Option<String> = None;

    // 按版本号聚合，同一版本只合并一次。
    let mut updates: BTreeMap<String, OfficialUpdate> = BTreeMap::new();

    for (t, label) in NEWS_TYPES {
        match fetch_type_once(state, game, t).await {
            Ok(posts) => {
                any_ok = true;
                scanned += posts.len() as i64;
                detail_lines.push(format!("{label} × {} 篇", posts.len()));
                for post in posts {
                    let source_post_id = if post.post_id.is_empty() { None } else { Some(post.post_id.clone()) };
                    let source_url = source_post_id
                        .as_ref()
                        .map(|id| format!("https://www.miyoushe.com/{}/article/{id}", game.miyoushe_slug));

                    if let Some((ver, dt, how)) = detect_livestream(&post, game) {
                        let e = updates.entry(ver.clone()).or_insert_with(|| OfficialUpdate {
                            version_code: ver.clone(),
                            ..Default::default()
                        });
                        if e.livestream_at.is_none() {
                            e.livestream_at = Some(fmt_iso(dt));
                            e.version_title = extract_title(&post);
                            e.source_post_id.clone_from(&source_post_id);
                            e.source_url.clone_from(&source_url);
                            detail_lines.push(format!("{} 前瞻直播 → {how} {}", ver, fmt_iso(dt)));
                        }
                    }
                    if let Some((ver, dt, how)) = detect_start(&post) {
                        let e = updates.entry(ver.clone()).or_insert_with(|| OfficialUpdate {
                            version_code: ver.clone(),
                            ..Default::default()
                        });
                        if e.start_at.is_none() {
                            e.start_at = Some(fmt_iso(dt));
                            e.version_title = e.version_title.clone().or_else(|| extract_title(&post));
                            e.source_post_id.clone_from(&source_post_id);
                            e.source_url.clone_from(&source_url);
                            detail_lines.push(format!("{} 开启时间 → {how} {}", ver, fmt_iso(dt)));
                        }
                    }
                    if let Some(ver) = detect_preload(&post) {
                        let e = updates.entry(ver.clone()).or_insert_with(|| OfficialUpdate {
                            version_code: ver.clone(),
                            ..Default::default()
                        });
                        e.source_post_id.clone_from(&source_post_id);
                        e.source_url.clone_from(&source_url);
                    }
                }
            }
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e.clone());
                }
                warn!(game = %game.id, type = t, "同步失败：{e}");
            }
        }
    }

    for upd in updates.values() {
        match apply_official_update(&state.pool, game, upd).await {
            Ok(outcome) => {
                if outcome.changed {
                    updated += 1;
                }
                detail_lines.extend(outcome.log);
            }
            Err(e) => {
                warn!(game = %game.id, version = %upd.version_code, "写入失败：{e:#}");
                if first_err.is_none() {
                    first_err = Some(format!("写入 {} 失败：{e}", upd.version_code));
                }
            }
        }
    }

    // 只要有任一 type 成功拉到官方数据即视为本轮同步“成功”，
    // 部分域名/type 的失败会记录在 detail 中（全失败才标 error）。
    let status = if any_ok { "success" } else { "error" };
    let detail = Some(if any_ok {
        format!(
            "{}；命中 {} 个版本节点；{}",
            detail_lines.join("；"),
            updated,
            first_err.unwrap_or_else(|| "无异常".into())
        )
    } else {
        format!("全部接口不可达：{}", first_err.unwrap_or_default())
    });

    if any_ok {
        if let Err(e) = touch_last_sync_at(&state.pool, &game.id).await {
            warn!(game = %game.id, "更新 last_sync_at 失败：{e:#}");
        }
    }

    let _ = insert_sync_log(
        &state.pool,
        Some(&game.id),
        scanned,
        updated,
        status,
        detail.as_deref(),
    )
    .await;

    GameSyncOutcome {
        game_id: game.id.clone(),
        scanned_count: scanned,
        updated_count: updated,
        status: status.to_string(),
        detail,
    }
}

/// 同步全部游戏；成功后写入一条汇总日志。
pub async fn sync_all(state: &AppState) -> Vec<GameSyncOutcome> {
    let games = match crate::db::list_games(&state.pool).await {
        Ok(g) => g,
        Err(e) => {
            warn!("读取游戏列表失败：{e:#}");
            return Vec::new();
        }
    };

    let mut outcomes = Vec::with_capacity(games.len());
    let mut total_scanned = 0i64;
    let mut total_updated = 0i64;
    let mut all_ok = true;

    for game in &games {
        let out = sync_game(state, game).await;
        total_scanned += out.scanned_count;
        total_updated += out.updated_count;
        if out.status != "success" {
            all_ok = false;
        }
        info!(
            game = %out.game_id,
            scanned = out.scanned_count,
            updated = out.updated_count,
            status = out.status,
            "官方同步完成"
        );
        outcomes.push(out);
    }

    let summary_status = if all_ok { "success" } else { "error" };
    let summary_detail = Some(format!(
        "扫描 {} 篇公告/资讯，命中并更新 {} 个版本节点",
        total_scanned, total_updated
    ));
    let _ = insert_sync_log(
        &state.pool,
        None,
        total_scanned,
        total_updated,
        summary_status,
        summary_detail.as_deref(),
    )
    .await;
    info!(
        scanned = total_scanned,
        updated = total_updated,
        status = summary_status,
        "官方同步汇总"
    );
    outcomes
}

/// 从标题中截取版本名称（「…」内文案）。
fn extract_title(post: &NewsPost) -> Option<String> {
    let s = post.subject.trim();
    let start = s.find('「')?;
    let end = s[start..].find('」')? + start;
    let inner_start = start + '「'.len_utf8();
    if end > inner_start {
        Some(s[inner_start..end].to_string())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 测试：使用 2026-10 实测抓取的官方帖子样本
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::parse_flexible;

    fn game(id: &str, clock: &str) -> Game {
        Game {
            id: id.to_string(),
            name: id.to_string(),
            miyoushe_gid: 2,
            miyoushe_slug: id.to_string(),
            default_cycle_days: 42,
            livestream_offset_days: 12,
            livestream_default_time: clock.to_string(),
            minor_rollover: 9,
            last_sync_at: None,
        }
    }

    fn post(subject: &str, created_at_iso: &str, content: &str) -> NewsPost {
        let created_at = parse_flexible(created_at_iso, NaiveTime::from_hms_opt(6, 0, 0).unwrap())
            .unwrap()
            .timestamp();
        NewsPost {
            post_id: "test".into(),
            subject: subject.into(),
            content: content.into(),
            structured_content: String::new(),
            created_at,
        }
    }

    #[test]
    fn zzz_3_3_livestream_confirmed_via_broadcast_window() {
        // 实测：节目帖 2026-10-09 20:20 发布（19:30 开播），命中“直播窗口”规则。
        let g = game("zzz", "19:30");
        let p = post("《绝区零》3.3版本「重返天空的旅程」前瞻特别节目", "2026-10-09T20:20:11+08:00", "");
        let hit = detect_livestream(&p, &g).expect("应命中");
        assert_eq!(hit.0, "3.3");
        assert_eq!(fmt_iso(hit.1), "2026-10-09T19:30:00+08:00");
        assert_eq!(hit.2, "直播窗口");
    }

    #[test]
    fn preview_post_is_rejected() {
        // 预告帖发布于直播前 4 天 12:00，既无显式时刻也不在窗口内 → 不确认。
        let g = game("zzz", "19:30");
        let p = post("3.3版本「重返天空的旅程」前瞻特别节目预告", "2026-10-05T12:00:19+08:00", "");
        assert!(detect_livestream(&p, &g).is_none());
    }

    #[test]
    fn explicit_datetime_wins_over_window() {
        let g = game("ys", "20:00");
        let p = post(
            "《原神》7.2版本「灰烬与明天的巡礼」前瞻特别节目",
            "2026-10-22T12:00:00+08:00",
            "本期特别节目将于2026年10月23日 20:00 播出",
        );
        let hit = detect_livestream(&p, &g).expect("应命中");
        assert_eq!(fmt_iso(hit.1), "2026-10-23T20:00:00+08:00");
        assert_eq!(hit.2, "显式时刻");
    }

    #[test]
    fn update_announcement_pins_start_at_0600() {
        // 实测：绝区零 3.2 更新公告发布于开服当日 07:00。
        let p = post("3.2版本「她与她的隐秘往事」更新公告", "2026-09-09T07:00:10+08:00", "");
        let hit = detect_start(&p).expect("应命中");
        assert_eq!(hit.0, "3.2");
        assert_eq!(fmt_iso(hit.1), "2026-09-09T06:00:00+08:00");
        assert_eq!(hit.2, "公告当日 06:00");
    }

    #[test]
    fn sr_update_note_pins_start_at_0600() {
        let p = post("4.6版本更新说明", "2026-09-28T07:00:00+08:00", "");
        let hit = detect_start(&p).expect("应命中");
        assert_eq!(fmt_iso(hit.1), "2026-09-28T06:00:00+08:00");
    }

    #[test]
    fn known_issue_post_is_rejected() {
        let p = post("4.6版本游戏优化及已知问题说明（10/14更新）", "2026-09-28T15:08:00+08:00", "");
        assert!(detect_start(&p).is_none());
    }

    #[test]
    fn shop_post_is_rejected() {
        let p = post("3.2版本「商城」上新说明", "2026-09-05T00:00:00+08:00", "");
        assert!(detect_start(&p).is_none());
    }

    #[test]
    fn explicit_maintenance_date_in_body_is_used() {
        let p = post(
            "4.7版本更新说明",
            "2026-11-09T07:00:00+08:00",
            "亲爱的开拓者：4.7版本将于11月9日 06:00 开始更新维护",
        );
        let hit = detect_start(&p).expect("应命中");
        assert_eq!(fmt_iso(hit.1), "2026-11-09T06:00:00+08:00");
        assert_eq!(hit.2, "显式日期");
    }

    #[test]
    fn preload_post_registers_version_without_date() {
        let p = post("4.6版本预下载&更新预告", "2026-09-24T14:00:00+08:00", "");
        assert_eq!(detect_preload(&p).as_deref(), Some("4.6"));
    }

    #[test]
    fn year_inference_keeps_announcement_in_current_year() {
        // 12月25日的公告：月日无年份，推断为公告当年。
        let p = post("4.0版本更新说明", "2026-12-25T07:00:00+08:00", "将于12月25日 06:00 更新");
        let hit = detect_start(&p).expect("应命中");
        assert_eq!(fmt_iso(hit.1), "2026-12-25T06:00:00+08:00");
    }

    #[test]
    fn title_extraction_grabs_quoted_name() {
        let p = post("《绝区零》3.3版本「重返天空的旅程」前瞻特别节目", "2026-10-09T20:20:11+08:00", "");
        assert_eq!(extract_title(&p).as_deref(), Some("重返天空的旅程"));
    }
}
