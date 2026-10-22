//! 数据结构定义：SQLite 行模型、REST DTO、米游社官方接口载荷。
//!
//! 时间统一以 **UTC+8（北京时间）** 的 `DateTime<FixedOffset>` 参与运算，
//! 落库与出参统一为 ISO 8601 字符串（例：`2026-10-21T06:00:00+08:00`）。

use std::str::FromStr;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};

/// 全系统时区：UTC+8 北京时间。
pub fn cst() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).expect("UTC+8 offset is always valid")
}

/// 当前北京时间。
pub fn now_cst() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&cst())
}

/// ISO 8601（秒级）序列化，保持 `+08:00` 偏移。
pub fn fmt_iso(dt: DateTime<FixedOffset>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// 中文星期（周一 … 周日）。
pub fn weekday_zh(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "周一",
        Weekday::Tue => "周二",
        Weekday::Wed => "周三",
        Weekday::Thu => "周四",
        Weekday::Fri => "周五",
        Weekday::Sat => "周六",
        Weekday::Sun => "周日",
    }
}

/// 宽松地解析前端 / 环境变量传入的时间字符串，全部按 UTC+8 解释：
///
/// * `2026-11-04T06:00` / `2026-11-04T06:00:00` → 当日该时刻（+08:00）
/// * `2026-11-04T06:00:00+08:00` / `...Z` → 保留原偏移后换算为 +08:00
/// * `2026-11-04` → 当日 `fallback_time`
pub fn parse_flexible(input: &str, fallback_time: NaiveTime) -> anyhow::Result<DateTime<FixedOffset>> {
    let s = input.trim();
    if s.is_empty() {
        anyhow::bail!("时间字符串为空");
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&cst()));
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(cst().from_local_datetime(&naive).single().expect("UTC+8 无夏令时歧义"));
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(cst()
            .from_local_datetime(&date.and_time(fallback_time))
            .single()
            .expect("UTC+8 无夏令时歧义"));
    }
    anyhow::bail!("无法解析时间：{s}（支持 YYYY-MM-DD / YYYY-MM-DDTHH:MM[:SS] / RFC3339）")
}

/// `'19:30'` / `'20:00'` → `NaiveTime`，非法值回退到 19:30。
pub fn parse_clock(s: &str) -> NaiveTime {
    NaiveTime::parse_from_str(s.trim(), "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(s.trim(), "%H:%M:%S"))
        .unwrap_or_else(|_| NaiveTime::from_hms_opt(19, 30, 0).expect("19:30 合法"))
}

/// 在指定日期上取某个时刻（UTC+8），供抓取 / 推算共用。
pub fn at_helper(date: chrono::NaiveDate, time: NaiveTime) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(8 * 3600)
        .expect("UTC+8 offset is always valid")
        .from_local_datetime(&date.and_time(time))
        .single()
        .expect("UTC+8 无夏令时歧义")
}

/// 版本号 `major.minor`：用于排序与级联递增。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionCode {
    pub major: u32,
    pub minor: u32,
}

impl VersionCode {
    pub fn parse(code: &str) -> Option<Self> {
        let mut it = code.trim().split('.');
        let major = it.next()?.trim().parse().ok()?;
        let minor = it.next().unwrap_or("0").trim().parse().unwrap_or(0);
        Some(Self { major, minor })
    }

    pub fn render(self) -> String {
        format!("{}.{}", self.major, self.minor)
    }

    /// 下一版本号；`minor_rollover` 为可允许的最大小版本号（默认 9，即 `3.9 → 4.0`）。
    pub fn next(self, minor_rollover: u32) -> Self {
        if self.minor >= minor_rollover {
            Self { major: self.major + 1, minor: 0 }
        } else {
            Self { major: self.major, minor: self.minor + 1 }
        }
    }
}

impl PartialOrd for VersionCode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VersionCode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.major, self.minor).cmp(&(other.major, other.minor))
    }
}

/// 排期状态：三级数据源。优先级 `official_confirmed > manual_override > predicted_42d`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryStatus {
    OfficialConfirmed,
    ManualOverride,
    Predicted42d,
}

impl EntryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            EntryStatus::OfficialConfirmed => "official_confirmed",
            EntryStatus::ManualOverride => "manual_override",
            EntryStatus::Predicted42d => "predicted_42d",
        }
    }
}

impl FromStr for EntryStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "official_confirmed" => Ok(EntryStatus::OfficialConfirmed),
            "manual_override" => Ok(EntryStatus::ManualOverride),
            "predicted_42d" => Ok(EntryStatus::Predicted42d),
            other => anyhow::bail!("未知状态：{other}"),
        }
    }
}

// ---------------------------------------------------------------------------
// SQLite 行模型
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Game {
    pub id: String,
    pub name: String,
    pub miyoushe_gid: i64,
    pub miyoushe_slug: String,
    pub default_cycle_days: i64,
    pub livestream_offset_days: i64,
    pub livestream_default_time: String,
    /// 版本号小版本回卷阈值（`3.9 → 4.0` 时为 9）。
    pub minor_rollover: i64,
    pub last_sync_at: Option<String>,
}

impl Game {
    pub fn livestream_time(&self) -> NaiveTime {
        parse_clock(&self.livestream_default_time)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionAnchor {
    pub id: i64,
    pub game_id: String,
    pub version_code: String,
    pub version_title: Option<String>,
    pub livestream_at: Option<String>,
    pub livestream_status: EntryStatus,
    pub start_at: String,
    pub start_status: EntryStatus,
    pub duration_days: i64,
    pub source_post_id: Option<String>,
    pub source_url: Option<String>,
    pub updated_at: String,
}

impl VersionAnchor {
    pub fn start_dt(&self) -> DateTime<FixedOffset> {
        parse_flexible(&self.start_at, NaiveTime::from_hms_opt(6, 0, 0).expect("06:00 合法"))
            .unwrap_or_else(|_| now_cst())
    }

    pub fn livestream_dt(&self) -> Option<DateTime<FixedOffset>> {
        self.livestream_at
            .as_deref()
            .and_then(|s| parse_flexible(s, NaiveTime::from_hms_opt(19, 30, 0).expect("19:30 合法")).ok())
    }

    pub fn version(&self) -> Option<VersionCode> {
        VersionCode::parse(&self.version_code)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncLog {
    pub id: i64,
    /// 汇总行为 `None`，单游戏行为该游戏代号。
    pub game_id: Option<String>,
    pub scanned_count: i64,
    pub updated_count: i64,
    pub status: String,
    pub detail: Option<String>,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// REST 出参
// ---------------------------------------------------------------------------

/// 前端展示用的单条版本排期（DB 锚点 + 42 天推算的合成结果）。
#[derive(Debug, Clone, Serialize)]
pub struct ScheduleEntry {
    pub game_id: String,
    pub version_code: String,
    pub version_title: Option<String>,
    pub start_at: String,
    pub start_status: EntryStatus,
    pub start_weekday: String,
    pub livestream_at: Option<String>,
    pub livestream_status: Option<EntryStatus>,
    pub livestream_weekday: Option<String>,
    pub end_at: String,
    pub duration_days: i64,
    pub days_until_start: i64,
    pub seconds_until_start: i64,
    pub seconds_until_livestream: Option<i64>,
    pub is_current: bool,
    /// 该版本是否已在 SQLite 中留下锚点记录（false = 纯推算）。
    pub is_anchored: bool,
    pub source_post_id: Option<String>,
    pub source_url: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameSchedule {
    pub id: String,
    pub name: String,
    pub miyoushe_gid: i64,
    pub miyoushe_slug: String,
    pub default_cycle_days: i64,
    pub livestream_offset_days: i64,
    pub livestream_default_time: String,
    pub minor_rollover: i64,
    pub last_sync_at: Option<String>,
    /// 当前正在进行的版本（`start_at <= now`）。
    pub current: Option<ScheduleEntry>,
    /// 下一个即将开启的版本。
    pub upcoming: Option<ScheduleEntry>,
    /// 下一个尚未开播的前瞻直播所属版本。
    pub next_livestream: Option<ScheduleEntry>,
    /// 当前版本 + 未来 6 个版本。
    pub entries: Vec<ScheduleEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NextUpItem {
    pub game_id: String,
    pub game_name: String,
    pub entry: ScheduleEntry,
}

#[derive(Debug, Clone, Serialize)]
pub struct NextUp {
    pub livestream: Option<NextUpItem>,
    pub start: Option<NextUpItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchedulesResponse {
    pub server_time: String,
    pub tz: String,
    pub games: Vec<GameSchedule>,
    pub sync: Option<SyncLog>,
    pub next_up: NextUp,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncGameResult {
    pub game_id: String,
    pub scanned_count: i64,
    pub updated_count: i64,
    pub status: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncResponse {
    pub ok: bool,
    pub scanned_count: i64,
    pub updated_count: i64,
    pub results: Vec<SyncGameResult>,
    pub schedules: SchedulesResponse,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverrideResponse {
    pub ok: bool,
    pub deleted: bool,
    pub schedules: SchedulesResponse,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    pub error: String,
}

// ---------------------------------------------------------------------------
// REST 入参
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct OverrideRequest {
    pub game_id: String,
    pub version_code: String,
    pub new_version_code: Option<String>,
    pub version_title: Option<String>,
    pub start_at: Option<String>,
    pub livestream_at: Option<String>,
    pub duration_days: Option<i64>,
}

impl<'de> Deserialize<'de> for OverrideRequest {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            game_id: String,
            version_code: String,
            #[serde(default)]
            new_version_code: Option<String>,
            #[serde(default)]
            version_title: Option<String>,
            #[serde(default)]
            start_at: Option<String>,
            #[serde(default)]
            livestream_at: Option<String>,
            #[serde(default)]
            duration_days: Option<i64>,
        }
        let raw = Raw::deserialize(d)?;
        Ok(OverrideRequest {
            game_id: raw.game_id.trim().to_string(),
            version_code: raw.version_code.trim().to_string(),
            new_version_code: non_empty(raw.new_version_code),
            version_title: non_empty(raw.version_title),
            start_at: non_empty(raw.start_at),
            livestream_at: non_empty(raw.livestream_at),
            duration_days: raw.duration_days,
        })
    }
}

fn non_empty(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[derive(Debug, Clone, Deserialize)]
pub struct IcsQuery {
    #[serde(default)]
    pub games: Option<String>,
    #[serde(default)]
    pub include_predicted: Option<String>,
    /// 每游戏仅输出未来最近 N 个版本（含其前瞻直播）；缺省 = 全部版本。
    #[serde(default)]
    pub versions: Option<String>,
}

impl IcsQuery {
    /// 解析 `?games=ys,sr,zzz`；缺省为全部游戏。
    pub fn game_ids(&self) -> Option<Vec<String>> {
        self.games.as_deref().map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
    }

    pub fn include_predicted(&self) -> bool {
        match self.include_predicted.as_deref().map(|s| s.trim().to_ascii_lowercase()) {
            None => true,
            Some(v) => matches!(v.as_str(), "1" | "true" | "yes" | "on"),
        }
    }

    /// 解析 `?versions=N`：每游戏未来最近 N 个版本；无效值或缺省 = 全部。
    pub fn versions_limit(&self) -> Option<usize> {
        self.versions
            .as_deref()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .filter(|n| *n > 0)
    }
}

// ---------------------------------------------------------------------------
// 米游社官方接口载荷
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct NewsListResponse {
    #[serde(default)]
    pub retcode: i64,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub data: Option<NewsListData>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewsListData {
    #[serde(default)]
    pub list: Vec<NewsItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewsItem {
    pub post: NewsPost,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewsPost {
    #[serde(default, deserialize_with = "de_string_or_number")]
    pub post_id: String,
    #[serde(default)]
    pub subject: String,
    /// 列表接口通常返回空串，故同时收集 `structured_content` 作为兜底正文。
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub structured_content: String,
    #[serde(default)]
    pub created_at: i64,
}

impl NewsPost {
    /// 供正则匹配的全部文本（标题 + 摘要 + 结构化正文）。
    pub fn haystack(&self) -> String {
        let mut s = String::with_capacity(self.subject.len() + self.content.len() + 64);
        s.push_str(&self.subject);
        s.push('\n');
        s.push_str(&self.content);
        s.push('\n');
        s.push_str(&self.structured_content);
        s
    }

    /// 官方帖发布时间（UTC+8）。
    pub fn created_cst(&self) -> Option<DateTime<FixedOffset>> {
        if self.created_at <= 0 {
            return None;
        }
        Utc.timestamp_opt(self.created_at, 0).single().map(|d| d.with_timezone(&cst()))
    }
}

fn de_string_or_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Str(String),
        Num(i64),
    }
    Ok(match Raw::deserialize(d)? {
        Raw::Str(s) => s,
        Raw::Num(n) => n.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_code_orders_numerically_not_lexically() {
        let a = VersionCode::parse("3.9").unwrap();
        let b = VersionCode::parse("3.10").unwrap();
        let c = VersionCode::parse("4.0").unwrap();
        assert!(a < b && b < c);
        assert!(VersionCode::parse("7.1").unwrap() > VersionCode::parse("6.8").unwrap());
    }

    #[test]
    fn next_code_rolls_over_at_threshold() {
        assert_eq!(VersionCode::parse("3.3").unwrap().next(9).render(), "3.4");
        assert_eq!(VersionCode::parse("3.9").unwrap().next(9).render(), "4.0");
        assert_eq!(VersionCode::parse("6.8").unwrap().next(8).render(), "7.0");
    }

    #[test]
    fn parse_flexible_handles_naive_and_offset_forms() {
        let fallback = NaiveTime::from_hms_opt(6, 0, 0).unwrap();
        let a = parse_flexible("2026-11-04T06:00", fallback).unwrap();
        assert_eq!(fmt_iso(a), "2026-11-04T06:00:00+08:00");
        let b = parse_flexible("2026-11-04", fallback).unwrap();
        assert_eq!(fmt_iso(b), "2026-11-04T06:00:00+08:00");
        let c = parse_flexible("2026-11-04T06:00:00Z", fallback).unwrap();
        assert_eq!(fmt_iso(c), "2026-11-04T14:00:00+08:00");
    }

    #[test]
    fn news_post_haystack_merges_subject_and_body() {
        let post: NewsPost = serde_json::from_value(serde_json::json!({
            "post_id": 78805120,
            "subject": "《绝区零》3.3版本「重返天空的旅程」前瞻特别节目",
            "content": "10月9日 19:30 开播",
            "created_at": 1791548411
        }))
        .unwrap();
        assert_eq!(post.post_id, "78805120");
        assert!(post.haystack().contains("10月9日 19:30"));
        assert_eq!(
            post.created_cst().map(|d| fmt_iso(d)).unwrap(),
            "2026-10-09T20:20:11+08:00"
        );
    }
}
