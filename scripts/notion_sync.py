#!/usr/bin/env python3
"""把 mihoyo-schedule-tracker 的版本排期同步到 Notion 数据库。

默认只同步每个游戏「未来最近 N 个版本」（N=1 即 T+1，含该版本的前瞻直播），
把滚动窗口外、由本脚本写入过的旧行归档，保持日历干净。

用法：
    NOTION_TOKEN=ntn_xxx NOTION_DATABASE_ID=xxx python3 scripts/notion_sync.py
    python3 scripts/notion_sync.py --dry-run          # 只打印将要写入的行，不调用 Notion
    python3 scripts/notion_sync.py --versions 2       # 每个游戏最近 2 个版本
    python3 scripts/notion_sync.py --no-prune         # 不归档旧行（会随时间累积）

环境变量：
    NOTION_TOKEN        集成 token（integration secret），必填（--dry-run 除外）
    NOTION_DATABASE_ID  目标数据库 id，必填（--dry-run 除外）
    TRACKER_BASE        tracker 地址，默认 http://127.0.0.1:8080
    GAMES               游戏 id 逗号分隔，默认 ys,sr,zzz
    VERSIONS            每游戏同步最近 N 个未来版本，默认 1；0 = 全部

Notion 数据库属性（名称需与下列完全一致；集成需先共享到该数据库）：
    名称     title       如「原神 7.2」
    游戏     select      原神 / 崩坏：星穹铁道 / 绝区零
    版本     rich_text   如 7.2
    开启时间 date        如 2026-11-04T06:00:00+08:00
    前瞻直播 date        可空，如 2026-10-25T19:30:00+08:00
    状态     select      官方定档 / 手动校准 / 42天预测
    时长天   number      如 42
    来源     url         可空，米游社原帖链接

定时任务示例（macOS crontab，每天 06:00）：
    6 0 * * * cd /path/to/repo && NOTION_TOKEN=... NOTION_DATABASE_ID=... python3 scripts/notion_sync.py >> /tmp/notion_sync.log 2>&1
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request

NOTION_API = "https://api.notion.com/v1"
NOTION_VERSION = "2022-06-28"

STATUS_LABELS = {
    "official_confirmed": "官方定档",
    "manual_override": "手动校准",
    "predicted42d": "42天预测",
}


def fetch_schedules(base: str) -> list[dict]:
    """GET /api/schedules，返回 games 列表。"""
    url = base.rstrip("/") + "/api/schedules"
    with urllib.request.urlopen(url, timeout=30) as resp:
        data = json.loads(resp.read().decode("utf-8"))
    games = data.get("games")
    if not isinstance(games, list):
        raise SystemExit(f"API 响应异常（无 games 列表）：{str(data)[:200]}")
    return games


def pick_entries(games: list[dict], game_ids: list[str], versions: int) -> list[tuple[dict, dict]]:
    """每个游戏取未来最近 N 个版本（entries 已按时间升序）。返回 (game, entry) 列表。"""
    wanted: list[tuple[dict, dict]] = []
    for g in games:
        if g.get("id") not in game_ids:
            continue
        entries = [e for e in g.get("entries", []) if e.get("seconds_until_start", 0) > 0]
        if versions > 0:
            entries = entries[:versions]
        wanted.extend((g, e) for e in entries)
    return wanted


def build_properties(game: dict, entry: dict) -> dict:
    """按约定属性名构造 Notion page properties。"""
    props: dict = {
        "名称": {"title": [{"text": {"content": f"{game['name']} {entry['version_code']}"}}]},
        "游戏": {"select": {"name": game["name"]}},
        "版本": {"rich_text": [{"text": {"content": entry["version_code"]}}]},
        "开启时间": {"date": {"start": entry["start_at"]}},
        "状态": {"select": {"name": STATUS_LABELS.get(entry["start_status"], entry["start_status"])}},
        "时长天": {"number": entry["duration_days"]},
    }
    if entry.get("livestream_at"):
        props["前瞻直播"] = {"date": {"start": entry["livestream_at"]}}
    if entry.get("source_url"):
        props["来源"] = {"url": entry["source_url"]}
    return props


def notion(token: str, method: str, path: str, payload: dict | None = None) -> dict:
    req = urllib.request.Request(
        f"{NOTION_API}{path}",
        data=json.dumps(payload).encode("utf-8") if payload is not None else None,
        method=method,
        headers={
            "Authorization": f"Bearer {token}",
            "Notion-Version": NOTION_VERSION,
            "Content-Type": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "replace")
        raise SystemExit(f"Notion API {e.code} {method} {path}\n{body[:500]}") from e


def upsert_page(token: str, db_id: str, game: dict, entry: dict, dry_run: bool) -> None:
    props = build_properties(game, entry)
    title = f"{game['name']} {entry['version_code']}"
    # 按 游戏 + 版本 定位既有行（游戏 select 精确匹配，版本 rich_text 精确匹配）。
    query = {
        "filter": {
            "and": [
                {"property": "游戏", "select": {"equals": game["name"]}},
                {"property": "版本", "rich_text": {"equals": entry["version_code"]}},
            ]
        }
    }
    existing = notion(token, "POST", f"/v1/databases/{db_id}/query", query)["results"]
    if existing:
        page_id = existing[0]["id"]
        action = "更新"
        if not dry_run:
            notion(token, "PATCH", f"/v1/pages/{page_id}", {"properties": props})
    else:
        action = "新建"
        if not dry_run:
            notion(token, "POST", "/v1/pages", {"parent": {"database_id": db_id}, "properties": props})
    print(f"  [{action}] {title} 开启 {entry['start_at']} 直播 {entry.get('livestream_at') or '-'} "
          f"({STATUS_LABELS.get(entry['start_status'], entry['start_status'])})")


def is_managed(result: dict) -> bool:
    """是否由本脚本写过：状态属三个枚举值之一，且有时长天。用于安全归档。"""
    props = result.get("properties", {})
    status = props.get("状态", {}).get("select")
    days = props.get("时长天", {}).get("number")
    return status is not None and status.get("name") in STATUS_LABELS.values() and days is not None


def prune_stale(token: str, db_id: str, games: list[dict], desired: set[str], dry_run: bool) -> None:
    """归档滚动窗口外、且由本脚本管理的行（游戏匹配、版本不在目标集合）。"""
    for g in games:
        query = {"filter": {"property": "游戏", "select": {"equals": g["name"]}}}
        rows = notion(token, "POST", f"/v1/databases/{db_id}/query", query)["results"]
        for r in rows:
            if not is_managed(r):
                continue
            version = (r.get("properties", {}).get("版本", {}).get("rich_text") or [{}])[0].get("text", {}).get("content")
            if version is not None and version not in desired:
                if not dry_run:
                    notion(token, "PATCH", f"/v1/pages/{r['id']}", {"archived": True})
                print(f"  [归档] {g['name']} {version}")


def main() -> None:
    ap = argparse.ArgumentParser(description="同步游戏版本排期到 Notion")
    ap.add_argument("--base", default=os.environ.get("TRACKER_BASE", "http://127.0.0.1:8080"))
    ap.add_argument("--games", default=os.environ.get("GAMES", "ys,sr,zzz"))
    ap.add_argument("--versions", type=int, default=int(os.environ.get("VERSIONS", "1")))
    ap.add_argument("--dry-run", action="store_true", help="不调用 Notion，只打印将要写入的行")
    ap.add_argument("--no-prune", action="store_true", help="不归档窗口外旧行")
    args = ap.parse_args()

    game_ids = [s.strip().lower() for s in args.games.split(",") if s.strip()]
    if args.versions < 0:
        raise SystemExit("--versions 必须 >= 0（0 = 全部未来版本）")

    games = fetch_schedules(args.base)
    wanted = pick_entries(games, game_ids, args.versions)
    if not wanted:
        print("目标集合为空（无未来版本或 API 不可达）——跳过写入与归档，避免误删。")
        return
    if args.dry_run:
        print(f"dry-run：将同步 {len(wanted)} 行（--versions={args.versions}，游戏={','.join(game_ids)}）：")
        for g, e in wanted:
            print(f"  [{g['name']}] {e['version_code']} 开启 {e['start_at']} 直播 {e.get('livestream_at') or '-'} "
                  f"({STATUS_LABELS.get(e['start_status'], e['start_status'])})")
        return

    token = os.environ.get("NOTION_TOKEN")
    db_id = os.environ.get("NOTION_DATABASE_ID")
    if not token or not db_id:
        raise SystemExit("需要 NOTION_TOKEN 与 NOTION_DATABASE_ID（或使用 --dry-run）")

    print(f"同步 {len(wanted)} 行 → 数据库 {db_id}：")
    for g, e in wanted:
        upsert_page(token, db_id, g, e, dry_run=False)
    if not args.no_prune:
        desired = {e["version_code"] for g, e in wanted}
        prune_stale(token, db_id, [g for g in games if g.get("id") in game_ids], desired, dry_run=False)
    print("完成。")


if __name__ == "__main__":
    main()
