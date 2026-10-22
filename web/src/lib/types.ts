/** 状态：官方定档 / 手动校准 / 42天预测 */
export type EntryStatus = 'official_confirmed' | 'manual_override' | 'predicted_42d';

export type ViewMode = 'table' | 'gantt';

export type GameFilter = 'all' | 'ys' | 'sr' | 'zzz';

export interface Entry {
  game_id: string;
  version_code: string;
  version_title: string | null;
  start_at: string;
  start_status: EntryStatus;
  start_weekday: string;
  livestream_at: string | null;
  livestream_status: EntryStatus | null;
  livestream_weekday: string | null;
  end_at: string;
  duration_days: number;
  days_until_start: number;
  seconds_until_start: number;
  seconds_until_livestream: number | null;
  is_current: boolean;
  is_anchored: boolean;
  source_post_id: string | null;
  source_url: string | null;
  updated_at: string | null;
}

export interface GameSchedule {
  id: string;
  name: string;
  miyoushe_gid: number;
  miyoushe_slug: string;
  default_cycle_days: number;
  livestream_offset_days: number;
  livestream_default_time: string;
  minor_rollover: number;
  last_sync_at: string | null;
  current: Entry | null;
  upcoming: Entry | null;
  next_livestream: Entry | null;
  entries: Entry[];
}

export interface SyncLog {
  game_id: string | null;
  scanned_count: number;
  updated_count: number;
  status: string;
  detail: string | null;
  created_at: string;
}

export interface NextUpItem {
  game_id: string;
  game_name: string;
  entry: Entry;
}

export interface NextUp {
  livestream: NextUpItem | null;
  start: NextUpItem | null;
}

export interface SchedulesResponse {
  server_time: string;
  tz: string;
  games: GameSchedule[];
  sync: SyncLog | null;
  next_up: NextUp;
}

export interface SyncResult {
  game_id: string;
  scanned_count: number;
  updated_count: number;
  status: string;
  detail: string | null;
}

export interface SyncResponse {
  ok: boolean;
  scanned_count: number;
  updated_count: number;
  results: SyncResult[];
  schedules: SchedulesResponse;
}

export interface OverrideRequest {
  game_id: string;
  version_code: string;
  new_version_code?: string;
  version_title?: string;
  start_at?: string;
  livestream_at?: string;
  duration_days?: number;
}

export interface OverrideResponse {
  ok: boolean;
  schedules: SchedulesResponse;
}

export interface ApiError {
  error: string;
}
