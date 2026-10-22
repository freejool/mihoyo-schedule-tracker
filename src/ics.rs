//! RFC 5545 iCalendar（.ics）动态生成器。
//!
//! 订阅端点 `GET /calendar.ics?games=ys,sr,zzz&include_predicted=true&versions=1`
//! 每次请求读取 SQLite 最新锚点并动态推算，返回标准 `text/calendar`。
//! `versions=N` 可只输出每个游戏未来最近的 N 个版本（T+1 即 `versions=1`）。
//! 每个版本生成两个 `VEVENT`：前瞻特别节目（精确起止 + 提前 30 分钟 VALARM）
//! 与版本开启（06:00–11:00 停服维护至开服）。

use chrono::{DateTime, Duration, FixedOffset, Utc};
use std::fmt::Write as _;

use crate::models::{EntryStatus, GameSchedule, IcsQuery};

const CALNAME: &str = "米哈游版本前瞻日历";
const PRODID: &str = "-//mihoyo-schedule-tracker//版本前瞻日历//CN";

/// RFC 5545 文本转义。
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// 超过 75 字节的行按 RFC 5545 折叠（续行以空格开头），字符边界安全。
fn fold_line(line: &str) -> String {
    if line.len() <= 75 {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len() + 16);
    let mut idx = 0usize;
    let mut first = true;
    while idx < line.len() {
        let limit = if first { 73 } else { 74 };
        let mut end = (idx + limit).min(line.len());
        while end > idx && !line.is_char_boundary(end) {
            end -= 1;
        }
        if end == idx {
            // 极限值恰好落在一个超长字符内部（极端情况）：整体截取该字符
            let mut step = idx + 1;
            while step < line.len() && !line.is_char_boundary(step) {
                step += 1;
            }
            end = step;
        }
        if !first {
            out.push_str("\r\n ");
        }
        out.push_str(&line[idx..end]);
        idx = end;
        first = false;
    }
    out
}

/// 本地墙钟格式：`20261021T060000`（配合 `TZID=Asia/Shanghai`）。
fn local_format(dt: &DateTime<FixedOffset>) -> String {
    dt.format("%Y%m%dT%H%M%S").to_string()
}

fn vtimezone_asia_shanghai() -> String {
    [
        "BEGIN:VTIMEZONE",
        "TZID:Asia/Shanghai",
        "BEGIN:STANDARD",
        "DTSTART:19700101T000000",
        "TZOFFSETFROM:+0800",
        "TZOFFSETTO:+0800",
        "TZNAME:CST",
        "END:STANDARD",
        "END:VTIMEZONE",
    ]
    .join("\r\n")
}

struct Event {
    uid: String,
    dtstart: String,
    dtend: String,
    summary: String,
    description: String,
    url: Option<String>,
    alarm: bool,
}

fn status_label(status: &str) -> &'static str {
    match status {
        "official_confirmed" => "定档",
        "manual_override" => "校准",
        _ => "预测",
    }
}

fn build_event(
    game: &GameSchedule,
    version_code: &str,
    version_title: Option<&str>,
    kind: &str,
    dtstart: DateTime<FixedOffset>,
    dtend: DateTime<FixedOffset>,
    status: &str,
    source_url: Option<&str>,
) -> Event {
    let game_name = game.name.as_str();
    let label = status_label(status);
    let (summary, desc, url) = match kind {
        "livestream" => {
            let title = version_title.map(|t| format!("《{t}》")).unwrap_or_default();
            (
                format!("[{label}] 《{game_name}》{version_code}版本前瞻特别节目{title}"),
                format!(
                    "米哈游《{game_name}》{version_code}版本前瞻特别节目（{label}）。\n官方链接：{}",
                    source_url.unwrap_or("暂无")
                ),
                source_url,
            )
        }
        _ => (
            format!("[{label}] 《{game_name}》{version_code}版本开启"),
            format!(
                "米哈游《{game_name}》{version_code}版本开启，06:00 停服维护至 11:00 开服（{label}）。\n官方链接：{}",
                source_url.unwrap_or("暂无")
            ),
            source_url,
        ),
    };

    Event {
        uid: format!("{}-{}-{}@mihoyo-schedule-tracker", game.id, version_code, kind),
        dtstart: local_format(&dtstart),
        dtend: local_format(&dtend),
        summary,
        description: desc,
        url: url.map(String::from),
        alarm: kind == "livestream",
    }
}

fn render_event(ev: &Event, now_utc: DateTime<Utc>) -> String {
    let mut out = String::new();
    let _ = write!(out, "BEGIN:VEVENT\r\n");
    let _ = write!(out, "UID:{}\r\n", esc(&ev.uid));
    let _ = write!(out, "DTSTAMP:{}\r\n", now_utc.format("%Y%m%dT%H%M%SZ"));
    let _ = write!(out, "DTSTART;TZID=Asia/Shanghai:{}\r\n", ev.dtstart);
    let _ = write!(out, "DTEND;TZID=Asia/Shanghai:{}\r\n", ev.dtend);
    let _ = write!(out, "SUMMARY:{}\r\n", fold_line(&esc(&ev.summary)));
    let _ = write!(out, "DESCRIPTION:{}\r\n", fold_line(&esc(&ev.description)));
    let _ = write!(out, "LOCATION:米游社 / 哔哩哔哩官方直播间\r\n");
    let _ = write!(out, "STATUS:CONFIRMED\r\n");
    if let Some(u) = &ev.url {
        let _ = write!(out, "URL:{}\r\n", esc(u));
    }
    if ev.alarm {
        let _ = write!(out, "BEGIN:VALARM\r\n");
        let _ = write!(out, "ACTION:DISPLAY\r\n");
        let _ = write!(out, "DESCRIPTION:前瞻特别节目即将开始\r\n");
        let _ = write!(out, "TRIGGER:-PT30M\r\n");
        let _ = write!(out, "END:VALARM\r\n");
    }
    let _ = write!(out, "END:VEVENT\r\n");
    out
}

/// 生成完整日历正文（不含 HTTP 层）。
pub fn build_calendar(schedules: &[GameSchedule], query: &IcsQuery) -> String {
    let now = Utc::now();
    let wanted: Option<Vec<String>> = query.game_ids();
    let include_predicted = query.include_predicted();
    let versions_limit = query.versions_limit();
    let mut out = String::new();

    let _ = write!(out, "BEGIN:VCALENDAR\r\n");
    let _ = write!(out, "VERSION:2.0\r\n");
    let _ = write!(out, "PRODID:{}\r\n", PRODID);
    let _ = write!(out, "CALSCALE:GREGORIAN\r\n");
    let _ = write!(out, "METHOD:PUBLISH\r\n");
    let _ = write!(out, "X-WR-CALNAME:{}\r\n", fold_line(&esc(CALNAME)));
    let _ = write!(out, "X-WR-TIMEZONE:Asia/Shanghai\r\n");
    let _ = write!(out, "X-PUBLISHED-TTL:PT6H\r\n");
    let _ = write!(out, "REFRESH-INTERVAL;VALUE=DURATION:PT6H\r\n");
    out.push_str(&vtimezone_asia_shanghai());
    out.push_str("\r\n");

    for g in schedules {
        if let Some(ids) = &wanted {
            if !ids.contains(&g.id) {
                continue;
            }
        }
        let entries: Vec<&crate::models::ScheduleEntry> = match versions_limit {
            Some(n) => g
                .entries
                .iter()
                .filter(|e| e.seconds_until_start > 0)
                .take(n)
                .collect(),
            None => g.entries.iter().collect(),
        };
        for e in entries {
            if !include_predicted && e.start_status == EntryStatus::Predicted42d {
                continue;
            }
            let title = e.version_title.as_deref();
            let url = e.source_url.as_deref();

            if let Some(livestream_at) = e.livestream_at.as_deref() {
                if let Ok(start) = crate::models::parse_flexible(livestream_at, crate::models::parse_clock("19:30")) {
                    let ev = build_event(
                        g,
                        &e.version_code,
                        title,
                        "livestream",
                        start,
                        start + Duration::hours(1),
                        e.livestream_status.as_ref().map(|s| s.as_str()).unwrap_or("predicted_42d"),
                        url,
                    );
                    out.push_str(&render_event(&ev, now));
                }
            }

            if let Ok(start) = crate::models::parse_flexible(&e.start_at, crate::models::parse_clock("06:00")) {
                let ev = build_event(
                    g,
                    &e.version_code,
                    title,
                    "start",
                    start,
                    start + Duration::hours(5),
                    e.start_status.as_str(),
                    url,
                );
                out.push_str(&render_event(&ev, now));
            }
        }
    }

    out.push_str("END:VCALENDAR\r\n");
    out
}

/// 响应头辅助：`text/calendar; charset=utf-8`。
pub fn content_type() -> &'static str {
    "text/calendar; charset=utf-8"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{parse_flexible, Game};
    use chrono::NaiveTime;

    #[test]
    fn text_escaping_handles_rfc_specials() {
        assert_eq!(esc(r"a\b;c,d"), r"a\\b\;c\,d");
        assert_eq!(esc("line1\nline2"), "line1\\nline2");
    }

    #[test]
    fn long_lines_are_folded_to_75_bytes() {
        let long = "《崩坏：星穹铁道》4.7版本「灰烬与明天的巡礼」前瞻特别节目，请准时收看。".repeat(3);
        let folded = fold_line(&long);
        assert!(folded.contains("\r\n "), "应出现折叠续行");
        for line in folded.split("\r\n") {
            let content = if line.starts_with(' ') { &line[1..] } else { line };
            assert!(content.len() <= 75, "行 {content:?} 超过 75 字节");
        }
    }

    #[test]
    fn calendar_contains_two_events_per_anchored_version() {
        // 用引擎直接构造最小 GameSchedule 以隔离测试。
        let g = crate::engine::build_game_schedule(
            &Game {
                id: "ys".into(),
                name: "原神".into(),
                miyoushe_gid: 2,
                miyoushe_slug: "ys".into(),
                default_cycle_days: 42,
                livestream_offset_days: 12,
                livestream_default_time: "20:00".into(),
                minor_rollover: 8,
                last_sync_at: None,
            },
            &[crate::models::VersionAnchor {
                id: 1,
                game_id: "ys".into(),
                version_code: "7.1".into(),
                version_title: Some("往冥府的安魂歌".into()),
                livestream_at: Some("2026-09-12T20:00:00+08:00".into()),
                livestream_status: EntryStatus::OfficialConfirmed,
                start_at: "2026-09-23T06:00:00+08:00".into(),
                start_status: EntryStatus::OfficialConfirmed,
                duration_days: 42,
                source_post_id: Some("166086".into()),
                source_url: Some("https://genshin.hoyoverse.com/zh-tw/news/detail/166086".into()),
                updated_at: "2026-10-15T00:00:00+08:00".into(),
            }],
            parse_flexible("2026-10-15T16:00:00+08:00", NaiveTime::from_hms_opt(6, 0, 0).unwrap()).unwrap(),
        );

        let q = IcsQuery { games: Some("ys".into()), include_predicted: Some("true".into()), versions: None };
        let cal = build_calendar(&[g], &q);
        assert!(cal.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(cal.ends_with("END:VCALENDAR\r\n"));
        let events: Vec<&str> = cal.split("BEGIN:VEVENT").skip(1).collect();
        assert_eq!(events.len(), 14, "锚点 7.1 + 6 个预测版本（7.2–7.7）各 2 个事件");
        assert!(cal.contains("SUMMARY:[预测] 《原神》7.2版本前瞻特别节目"));
        assert!(cal.contains("SUMMARY:[定档] 《原神》7.1版本前瞻特别节目《往冥府的安魂歌》"));
        assert!(cal.contains("SUMMARY:[定档] 《原神》7.1版本开启"));
        assert!(cal.contains("DTSTART;TZID=Asia/Shanghai:20260923T060000"));
        assert!(cal.contains("DTSTART;TZID=Asia/Shanghai:20260912T200000"));
        assert!(cal.contains("BEGIN:VTIMEZONE"));
        assert!(cal.contains("TZID:Asia/Shanghai"));
        assert!(cal.contains("TRIGGER:-PT30M"));
    }

    #[test]
    fn predicted_filter_drops_pure_predictions() {
        let now = parse_flexible("2026-10-15T16:00:00+08:00", NaiveTime::from_hms_opt(6, 0, 0).unwrap()).unwrap();
        let g = crate::engine::build_game_schedule(
            &Game {
                id: "zzz".into(),
                name: "绝区零".into(),
                miyoushe_gid: 8,
                miyoushe_slug: "zzz".into(),
                default_cycle_days: 42,
                livestream_offset_days: 12,
                livestream_default_time: "19:30".into(),
                minor_rollover: 9,
                last_sync_at: None,
            },
            &[crate::models::VersionAnchor {
                id: 1,
                game_id: "zzz".into(),
                version_code: "3.3".into(),
                version_title: None,
                livestream_at: None,
                livestream_status: EntryStatus::Predicted42d,
                start_at: "2026-10-21T06:00:00+08:00".into(),
                start_status: EntryStatus::OfficialConfirmed,
                duration_days: 42,
                source_post_id: None,
                source_url: None,
                updated_at: "2026-10-15T00:00:00+08:00".into(),
            }],
            now,
        );
        let q = IcsQuery { games: None, include_predicted: Some("false".into()), versions: None };
        let cal = build_calendar(&[g], &q);
        assert!(cal.contains("3.3版本开启"), "锚点版本开启事件应保留");
        assert!(!cal.contains("3.4"), "纯预测节点应被过滤");
        assert!(!cal.contains("3.5"), "纯预测节点应被过滤");
    }

    #[test]
    fn versions_limit_keeps_only_next() {
        let now = parse_flexible("2026-10-15T16:00:00+08:00", NaiveTime::from_hms_opt(6, 0, 0).unwrap()).unwrap();
        let g = crate::engine::build_game_schedule(
            &Game {
                id: "zzz".into(),
                name: "绝区零".into(),
                miyoushe_gid: 8,
                miyoushe_slug: "zzz".into(),
                default_cycle_days: 42,
                livestream_offset_days: 12,
                livestream_default_time: "19:30".into(),
                minor_rollover: 9,
                last_sync_at: None,
            },
            &[crate::models::VersionAnchor {
                id: 1,
                game_id: "zzz".into(),
                version_code: "3.3".into(),
                version_title: None,
                livestream_at: None,
                livestream_status: EntryStatus::OfficialConfirmed,
                start_at: "2026-10-21T06:00:00+08:00".into(),
                start_status: EntryStatus::OfficialConfirmed,
                duration_days: 42,
                source_post_id: None,
                source_url: None,
                updated_at: "2026-10-15T00:00:00+08:00".into(),
            }],
            now,
        );

        // versions=1：只保留最近一个未来版本（3.3），3.4/3.5 不出现。
        let q1 = IcsQuery { games: None, include_predicted: Some("true".into()), versions: Some("1".into()) };
        let cal1 = build_calendar(&[g.clone()], &q1);
        assert!(cal1.contains("SUMMARY:[定档] 《绝区零》3.3版本开启"), "最近版本应保留");
        assert!(!cal1.contains("3.4版本"), "versions=1 不应出现 3.4");
        assert!(!cal1.contains("3.5版本"), "versions=1 不应出现 3.5");

        // versions=2：3.3 + 3.4。
        let q2 = IcsQuery { games: None, include_predicted: Some("true".into()), versions: Some("2".into()) };
        let cal2 = build_calendar(&[g.clone()], &q2);
        assert!(cal2.contains("3.4版本"), "versions=2 应出现 3.4");
        assert!(!cal2.contains("3.5版本"), "versions=2 不应出现 3.5");

        // 无效值 → 退回全部版本。
        let q3 = IcsQuery { games: None, include_predicted: Some("true".into()), versions: Some("abc".into()) };
        let cal3 = build_calendar(&[g], &q3);
        assert!(cal3.contains("3.5版本"), "无效 versions 应退回全部版本");
    }
}
