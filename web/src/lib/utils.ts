import type { EntryStatus } from './types';

export type WallFormat = 'datetime' | 'date' | 'time' | 'short' | 'minute' | 'datetime-local';

/**
 * 按指定时区格式化时间，数据一律为 UTC+8，绝不使用浏览器本地时区。
 * `short` = MM-DD HH:mm，`datetime-local` = <input type="datetime-local"> 可用的值。
 */
export function formatWall(value: string | null, tz: string, pattern: WallFormat): string {
  if (!value) return '—';
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return '—';
  const parts = new Intl.DateTimeFormat('en-CA', {
    timeZone: tz,
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false
  }).formatToParts(d);
  const g = (t: string) => parts.find((x) => x.type === t)?.value ?? '';
  const year = g('year');
  const month = g('month');
  const day = g('day');
  const hour = g('hour');
  const minute = g('minute');
  const second = g('second');
  switch (pattern) {
    case 'date':
      return `${year}-${month}-${day}`;
    case 'time':
      return `${hour}:${minute}`;
    case 'short':
      return `${month}-${day} ${hour}:${minute}`;
    case 'minute':
      return `${year}-${month}-${day} ${hour}:${minute}`;
    case 'datetime-local':
      return `${year}-${month}-${day}T${hour}:${minute}`;
    default:
      return `${year}-${month}-${day} ${hour}:${minute}:${second}`;
  }
}

export interface StatusMeta {
  label: string;
  emoji: string;
  /** badge classes (theme tokens，亮/暗色都可读) */
  badge: string;
  /** 甘特条 / 标记填充色 */
  fill: string;
}

export function statusMeta(status: EntryStatus | null): StatusMeta {
  switch (status) {
    case 'official_confirmed':
      return {
        label: '官方定档',
        emoji: '🟢',
        badge: 'bg-status-official/15 text-status-official',
        fill: 'var(--color-status-official)'
      };
    case 'manual_override':
      return {
        label: '手动校准',
        emoji: '🔵',
        badge: 'bg-status-manual/15 text-status-manual',
        fill: 'var(--color-status-manual)'
      };
    case 'predicted_42d':
      return {
        label: '42天预测',
        emoji: '🟠',
        badge: 'bg-status-predicted/15 text-status-predicted',
        fill: 'var(--color-status-predicted)'
      };
    default:
      return { label: '未知', emoji: '⚪', badge: 'bg-surface-muted text-ink-muted', fill: '#888' };
  }
}

export interface CountdownParts {
  negative: boolean;
  days: number;
  hours: number;
  minutes: number;
  seconds: number;
}

/** 距目标时间的剩余量；目标已过时 negative=true（分量仍为绝对值）。 */
export function countdownParts(targetIso: string, now: number): CountdownParts {
  const diff = new Date(targetIso).getTime() - now;
  const abs = Math.floor(Math.abs(diff) / 1000);
  return {
    negative: diff < 0,
    days: Math.floor(abs / 86400),
    hours: Math.floor((abs % 86400) / 3600),
    minutes: Math.floor((abs % 3600) / 60),
    seconds: abs % 60
  };
}
