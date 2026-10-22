//! 42 天周期级联推算引擎。
//!
//! 数据源优先级：`official_confirmed` / `manual_override`（SQLite 锚点，锁死时间）
//! → `predicted_42d`（由引擎按周期公式向后滚动推算）。
//!
//! ```text
//! start_at(i) = start_at(i-1) + duration_days(i-1)          // 默认 42 天
//! livestream_at(i) = start_at(i) - livestream_offset_days   // 默认 12 天
//!    └─ 若结果不是周五：对齐到“start_at(i) 所在周的上一周周五”
//!       + 该游戏 livestream_default_time（原神 20:00 / 星铁、绝区零 19:30）
//! ```

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveTime, TimeZone, Weekday};

use crate::models::{
    cst, fmt_iso, weekday_zh, EntryStatus, Game, GameSchedule, NextUp, NextUpItem, ScheduleEntry,
    VersionAnchor, VersionCode,
};

/// 向后推算的版本数量（未来 6 个）。
pub const HORIZON: usize = 6;

/// 在指定日期上取某个时刻（UTC+8）。
fn at_time(date: chrono::NaiveDate, time: NaiveTime) -> DateTime<FixedOffset> {
    cst()
        .from_local_datetime(&date.and_time(time))
        .single()
        .expect("UTC+8 无夏令时歧义")
}

/// 前瞻直播预测：优先“版本开启前 N 天且为周五”，否则回退到开启周的前一周周五。
pub fn predict_livestream(
    start: DateTime<FixedOffset>,
    offset_days: i64,
    default_time: NaiveTime,
) -> DateTime<FixedOffset> {
    let target = start - Duration::days(offset_days.max(0));
    if target.weekday() == Weekday::Fri {
        return at_time(target.date_naive(), default_time);
    }
    let start_date = start.date_naive();
    let monday = start_date - Duration::days(start_date.weekday().num_days_from_monday() as i64);
    let prev_friday = monday - Duration::days(3);
    at_time(prev_friday, default_time)
}

/// 由锚点行直接构造排期条目（未锁定前瞻时间时用推算值补齐）。
fn entry_from_anchor(
    game: &Game,
    anchor: &VersionAnchor,
    now: DateTime<FixedOffset>,
    is_current: bool,
) -> ScheduleEntry {
    let start = anchor.start_dt();
    let duration = anchor.duration_days.max(1);
    let (livestream, livestream_status) = match anchor.livestream_dt() {
        Some(dt) => (Some(dt), Some(anchor.livestream_status)),
        None => (
            Some(predict_livestream(start, game.livestream_offset_days, game.livestream_time())),
            Some(EntryStatus::Predicted42d),
        ),
    };
    build_entry(
        game,
        anchor.version_code.clone(),
        anchor.version_title.clone(),
        start,
        anchor.start_status,
        livestream,
        livestream_status,
        duration,
        now,
        is_current,
        true,
        anchor.source_post_id.clone(),
        anchor.source_url.clone(),
        Some(anchor.updated_at.clone()),
    )
}

/// 纯推算条目（`predicted_42d`）。
fn predicted_entry(
    game: &Game,
    version_code: String,
    start: DateTime<FixedOffset>,
    duration_days: i64,
    now: DateTime<FixedOffset>,
) -> ScheduleEntry {
    let livestream = predict_livestream(start, game.livestream_offset_days, game.livestream_time());
    build_entry(
        game,
        version_code,
        None,
        start,
        EntryStatus::Predicted42d,
        Some(livestream),
        Some(EntryStatus::Predicted42d),
        duration_days,
        now,
        false,
        false,
        None,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_entry(
    game: &Game,
    version_code: String,
    version_title: Option<String>,
    start: DateTime<FixedOffset>,
    start_status: EntryStatus,
    livestream: Option<DateTime<FixedOffset>>,
    livestream_status: Option<EntryStatus>,
    duration_days: i64,
    now: DateTime<FixedOffset>,
    is_current: bool,
    is_anchored: bool,
    source_post_id: Option<String>,
    source_url: Option<String>,
    updated_at: Option<String>,
) -> ScheduleEntry {
    let end = start + Duration::days(duration_days);
    ScheduleEntry {
        game_id: game.id.clone(),
        version_code,
        version_title,
        start_at: fmt_iso(start),
        start_status,
        start_weekday: weekday_zh(start.weekday()).to_string(),
        livestream_at: livestream.map(fmt_iso),
        livestream_status: livestream.map(|_| livestream_status.unwrap_or(EntryStatus::Predicted42d)),
        livestream_weekday: livestream.map(|d| weekday_zh(d.weekday()).to_string()),
        end_at: fmt_iso(end),
        duration_days,
        days_until_start: (start.date_naive() - now.date_naive()).num_days(),
        seconds_until_start: (start - now).num_seconds(),
        seconds_until_livestream: livestream.map(|d| (d - now).num_seconds()),
        is_current,
        is_anchored,
        source_post_id,
        source_url,
        updated_at,
    }
}

/// 单游戏：锚点 + 级联推算 → 当前版本与未来 6 个版本。
pub fn build_game_schedule(
    game: &Game,
    anchors: &[VersionAnchor],
    now: DateTime<FixedOffset>,
) -> GameSchedule {
    let mut mine: Vec<&VersionAnchor> = anchors.iter().filter(|a| a.game_id == game.id).collect();
    mine.sort_by(|a, b| match (a.version(), b.version()) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.id.cmp(&b.id),
    });

    let mut current: Option<ScheduleEntry> = None;
    let mut future: Vec<ScheduleEntry> = Vec::new();

    for a in &mine {
        let entry = entry_from_anchor(game, a, now, false);
        if a.start_dt() <= now {
            current = Some(entry);
        } else {
            future.push(entry);
        }
    }

    // 从前一锚点开始的 42 天级联：逐个补足未来 6 个版本。
    if let Some(last) = mine.last() {
        let mut cur_code = last.version_code.clone();
        let mut cur_start = last.start_dt();
        let mut cur_duration = last.duration_days.max(1);
        let mut guard = 0usize;
        while future.len() < HORIZON && guard < 128 {
            guard += 1;
            let next = VersionCode::parse(&cur_code)
                .unwrap_or(VersionCode { major: 0, minor: 0 })
                .next(game.minor_rollover.max(1) as u32)
                .render();
            let start = cur_start + Duration::days(cur_duration);
            cur_code = next.clone();
            cur_start = start;
            cur_duration = game.default_cycle_days.max(1);
            if future.iter().any(|e| e.version_code == next) {
                continue;
            }
            future.push(predicted_entry(game, next, start, cur_duration, now));
        }
    }

    future.sort_by(|a, b| a.start_at.cmp(&b.start_at));

    if let Some(cur) = current.as_mut() {
        cur.is_current = true;
    }

    let mut entries: Vec<ScheduleEntry> = Vec::with_capacity(future.len() + 1);
    if let Some(cur) = current.clone() {
        entries.push(cur);
    }
    entries.extend(future.iter().cloned());

    let upcoming = entries
        .iter()
        .find(|e| e.seconds_until_start > 0)
        .cloned();
    let next_livestream = entries
        .iter()
        .filter(|e| e.seconds_until_livestream.map(|s| s > 0).unwrap_or(false))
        .min_by_key(|e| e.seconds_until_livestream.unwrap_or(i64::MAX))
        .cloned();

    GameSchedule {
        id: game.id.clone(),
        name: game.name.clone(),
        miyoushe_gid: game.miyoushe_gid,
        miyoushe_slug: game.miyoushe_slug.clone(),
        default_cycle_days: game.default_cycle_days,
        livestream_offset_days: game.livestream_offset_days,
        livestream_default_time: game.livestream_default_time.clone(),
        minor_rollover: game.minor_rollover,
        last_sync_at: game.last_sync_at.clone(),
        current,
        upcoming,
        next_livestream,
        entries,
    }
}

pub fn build_all(games: &[Game], anchors: &[VersionAnchor], now: DateTime<FixedOffset>) -> Vec<GameSchedule> {
    games
        .iter()
        .map(|g| build_game_schedule(g, anchors, now))
        .collect()
}

/// 跨游戏「最近即将到来的前瞻直播 / 版本开启」。
pub fn next_up(schedules: &[GameSchedule]) -> NextUp {
    let mut livestream: Option<NextUpItem> = None;
    let mut start: Option<NextUpItem> = None;

    for g in schedules {
        for e in &g.entries {
            if let Some(sec) = e.seconds_until_livestream {
                if sec > 0
                    && livestream
                        .as_ref()
                        .map(|c| c.entry.seconds_until_livestream.unwrap_or(i64::MAX) > sec)
                        .unwrap_or(true)
                {
                    livestream = Some(NextUpItem {
                        game_id: g.id.clone(),
                        game_name: g.name.clone(),
                        entry: e.clone(),
                    });
                }
            }
            if e.seconds_until_start > 0
                && start
                    .as_ref()
                    .map(|c| c.entry.seconds_until_start > e.seconds_until_start)
                    .unwrap_or(true)
            {
                start = Some(NextUpItem {
                    game_id: g.id.clone(),
                    game_name: g.name.clone(),
                    entry: e.clone(),
                });
            }
        }
    }

    NextUp { livestream, start }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::parse_flexible;

    fn game(id: &str, clock: &str, rollover: i64) -> Game {
        Game {
            id: id.to_string(),
            name: id.to_string(),
            miyoushe_gid: 2,
            miyoushe_slug: id.to_string(),
            default_cycle_days: 42,
            livestream_offset_days: 12,
            livestream_default_time: clock.to_string(),
            minor_rollover: rollover,
            last_sync_at: None,
        }
    }

    fn ts(s: &str) -> DateTime<FixedOffset> {
        parse_flexible(s, NaiveTime::from_hms_opt(6, 0, 0).unwrap()).unwrap()
    }

    fn anchor(id: i64, game_id: &str, code: &str, start: &str, livestream: Option<&str>) -> VersionAnchor {
        VersionAnchor {
            id,
            game_id: game_id.to_string(),
            version_code: code.to_string(),
            version_title: None,
            livestream_at: livestream.map(|s| s.to_string()),
            livestream_status: if livestream.is_some() {
                EntryStatus::OfficialConfirmed
            } else {
                EntryStatus::Predicted42d
            },
            start_at: start.to_string(),
            start_status: EntryStatus::OfficialConfirmed,
            duration_days: 42,
            source_post_id: None,
            source_url: None,
            updated_at: "2026-10-15T00:00:00+08:00".to_string(),
        }
    }

    #[test]
    fn seed_anchor_cascades_in_42_day_steps() {
        let now = ts("2026-10-15T16:00:00+08:00");
        let g = game("zzz", "19:30", 9);
        let anchors = vec![anchor(1, "zzz", "3.3", "2026-10-21T06:00:00+08:00", Some("2026-10-09T19:30:00+08:00"))];

        let s = build_game_schedule(&g, &anchors, now);

        // 3.3 尚未开启 → 无 current，首条即 3.3（锚点）。
        assert!(s.current.is_none());
        let codes: Vec<&str> = s.entries.iter().map(|e| e.version_code.as_str()).collect();
        assert_eq!(codes, ["3.3", "3.4", "3.5", "3.6", "3.7", "3.8"]);

        // 3.4 = 2026-10-21 + 42 天 = 2026-12-02，前瞻 = 前 12 天且正好周五 2026-11-20。
        let v34 = &s.entries[1];
        assert_eq!(v34.start_at, "2026-12-02T06:00:00+08:00");
        assert_eq!(v34.start_weekday, "周三");
        assert_eq!(v34.livestream_at.as_deref(), Some("2026-11-20T19:30:00+08:00"));
        assert_eq!(v34.livestream_weekday.as_deref(), Some("周五"));
        assert_eq!(v34.start_status, EntryStatus::Predicted42d);
        assert!(!v34.is_anchored);

        // 锚点行保留官方状态与来源。
        assert_eq!(s.entries[0].start_status, EntryStatus::OfficialConfirmed);
        assert!(s.entries[0].is_anchored);
        assert_eq!(s.entries[0].livestream_at.as_deref(), Some("2026-10-09T19:30:00+08:00"));

        // 已直播的 3.3 不再是 next_livestream，落到 3.4。
        assert_eq!(s.next_livestream.as_ref().unwrap().version_code, "3.4");
        assert_eq!(s.upcoming.as_ref().unwrap().version_code, "3.3");
    }

    #[test]
    fn monday_start_snaps_livestream_to_previous_week_friday() {
        let now = ts("2026-10-15T16:00:00+08:00");
        let g = game("sr", "19:30", 9);
        // 星铁 4.6 于周一（2026-09-28）开启。
        let anchors = vec![anchor(1, "sr", "4.6", "2026-09-28T06:00:00+08:00", Some("2026-09-20T19:30:00+08:00"))];

        let s = build_game_schedule(&g, &anchors, now);
        assert_eq!(s.current.as_ref().unwrap().version_code, "4.6");

        // 4.7 = 2026-11-09（周一）：-12 天为 10-28（周三）→ 对齐到 11-06（上一周周五）。
        let v47 = &s.entries[1];
        assert_eq!(v47.start_at, "2026-11-09T06:00:00+08:00");
        assert_eq!(v47.start_weekday, "周一");
        assert_eq!(v47.livestream_at.as_deref(), Some("2026-11-06T19:30:00+08:00"));
        assert_eq!(v47.livestream_weekday.as_deref(), Some("周五"));
    }

    #[test]
    fn rollover_produces_major_bump() {
        let now = ts("2026-10-15T16:00:00+08:00");
        let g = game("ys", "20:00", 8);
        let anchors = vec![anchor(1, "ys", "6.6", "2026-06-10T06:00:00+08:00", None)];

        let s = build_game_schedule(&g, &anchors, now);
        let codes: Vec<&str> = s.entries.iter().map(|e| e.version_code.as_str()).collect();
        // 6.8 之后再递增应跳到 7.0（minor_rollover = 8）。
        assert!(codes.contains(&"6.8"), "codes = {codes:?}");
        assert!(codes.contains(&"7.0"), "codes = {codes:?}");
    }

    #[test]
    fn future_anchor_overrides_prediction_and_keeps_official_status() {
        let now = ts("2026-10-15T16:00:00+08:00");
        let g = game("zzz", "19:30", 9);
        let anchors = vec![
            anchor(1, "zzz", "3.3", "2026-10-21T06:00:00+08:00", Some("2026-10-09T19:30:00+08:00")),
            anchor(2, "zzz", "3.4", "2026-12-09T06:00:00+08:00", None),
        ];

        let s = build_game_schedule(&g, &anchors, now);
        let v34 = s.entries.iter().find(|e| e.version_code == "3.4").unwrap();
        // 官方锁定 12-09 ≠ 推算 12-02，锚点优先。
        assert_eq!(v34.start_at, "2026-12-09T06:00:00+08:00");
        assert_eq!(v34.start_status, EntryStatus::OfficialConfirmed);
        assert!(v34.is_anchored);
        // 未锁定前瞻 → 推算（12-09 前 12 天为 11-27，恰为周五 → 保留）。
        assert_eq!(v34.livestream_at.as_deref(), Some("2026-11-27T19:30:00+08:00"));
        assert_eq!(v34.livestream_status, Some(EntryStatus::Predicted42d));
    }

    #[test]
    fn manual_duration_shifts_following_versions() {
        let now = ts("2026-10-15T16:00:00+08:00");
        let g = game("zzz", "19:30", 9);
        let mut a = anchor(1, "zzz", "3.3", "2026-10-21T06:00:00+08:00", None);
        a.duration_days = 35; // 手动校准为 35 天
        a.start_status = EntryStatus::ManualOverride;
        let s = build_game_schedule(&g, &[a], now);

        assert_eq!(s.entries[0].start_status, EntryStatus::ManualOverride);
        assert_eq!(s.entries[1].start_at, "2026-11-25T06:00:00+08:00");
        assert_eq!(s.entries[2].start_at, "2027-01-06T06:00:00+08:00");
    }

    #[test]
    fn next_up_picks_soonest_across_games() {
        let now = ts("2026-10-15T16:00:00+08:00");
        let zzz = build_game_schedule(
            &game("zzz", "19:30", 9),
            &[anchor(1, "zzz", "3.3", "2026-10-21T06:00:00+08:00", Some("2026-10-09T19:30:00+08:00"))],
            now,
        );
        let ys = build_game_schedule(
            &game("ys", "20:00", 8),
            &[anchor(1, "ys", "7.1", "2026-09-23T06:00:00+08:00", Some("2026-09-12T20:00:00+08:00"))],
            now,
        );
        let up = next_up(&[zzz, ys]);
        assert_eq!(up.start.as_ref().unwrap().game_id, "zzz");
        assert_eq!(up.start.as_ref().unwrap().entry.version_code, "3.3");
        // 原神 7.2 前瞻（2026-11-04 前 12 天 = 10-23 周五）早于绝区零 3.4 的 11-20。
        assert_eq!(up.livestream.as_ref().unwrap().game_id, "ys");
        assert_eq!(up.livestream.as_ref().unwrap().entry.version_code, "7.2");
    }
}
