<script lang="ts">
  import { statusMeta, formatWall } from './utils';
  import type { EntryStatus, GameSchedule } from './types';

  let { game, tz }: { game: GameSchedule; tz: string } = $props();

  // “现在”位置线：每 60 秒刷新一次。
  let now = $state(Date.now());
  $effect(() => {
    const id = setInterval(() => {
      now = Date.now();
    }, 60000);
    return () => clearInterval(id);
  });

  const DAY_MS = 86400000;

  const range = $derived.by(() => {
    const starts = game.entries.map((e) => new Date(e.start_at).getTime());
    const ends = game.entries.map((e) => new Date(e.end_at).getTime());
    const startMin = starts.length ? Math.min(...starts) : now;
    const start = Math.min(startMin, now);
    const end = ends.length ? Math.max(...ends, now) : start + 42 * DAY_MS;
    const total = Math.max(end - start, 42 * DAY_MS);
    return { start, total };
  });

  const rows = $derived(
    game.entries.map((entry, i) => {
      const s = new Date(entry.start_at).getTime();
      const e = new Date(entry.end_at).getTime();
      const left = ((s - range.start) / range.total) * 100;
      const width = (Math.max(e - s, DAY_MS) / range.total) * 100;
      const meta = statusMeta(entry.start_status);
      const ls = entry.livestream_at ? new Date(entry.livestream_at).getTime() : null;
      const lsLeft = ls !== null ? ((ls - range.start) / range.total) * 100 : null;
      return { entry, i, left, width, fill: meta.fill, lsLeft };
    })
  );

  const nowPct = $derived(Math.max(0, Math.min(100, ((now - range.start) / range.total) * 100)));

  // 月份刻度：从范围起点所在月的 1 号开始，逐月 +1，直到范围终点所在月。
  const months = $derived.by(() => {
    const list: { left: number; label: string }[] = [];
    const d = new Date(range.start);
    const endD = new Date(range.start + range.total);
    const cur = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), 1));
    while (cur.getTime() <= endD.getTime()) {
      const left = ((cur.getTime() - range.start) / range.total) * 100;
      const label = `${cur.getUTCFullYear()}-${String(cur.getUTCMonth() + 1).padStart(2, '0')}`;
      if (left >= 0 && left <= 100) list.push({ left, label });
      cur.setUTCMonth(cur.getUTCMonth() + 1);
    }
    return list;
  });

  // 周网格线：每 7 天一根。
  const weekLines = $derived.by(() => {
    const list: number[] = [];
    const start = Math.ceil(range.start / DAY_MS) * DAY_MS;
    for (let t = start; t <= range.start + range.total; t += 7 * DAY_MS) {
      const pct = ((t - range.start) / range.total) * 100;
      if (pct > 0 && pct < 100) list.push(pct);
    }
    return list;
  });

  const ROW_H = 44;
  const AXIS_H = 28;
  const bodyH = $derived(rows.length * ROW_H);
  const totalH = $derived(AXIS_H + bodyH);

  const LEGEND: EntryStatus[] = ['official_confirmed', 'manual_override', 'predicted_42d'];
</script>

<section id="game-{game.id}" class="scroll-mt-24 rounded-2xl border border-border bg-surface-raised p-4 shadow-sm sm:p-5">
  <header class="mb-3 flex flex-wrap items-center gap-2">
    <h2 class="text-lg font-bold">{game.name}</h2>
    <div class="ml-auto flex flex-wrap items-center gap-4 text-xs text-ink-muted">
      {#each LEGEND as st}
        <span class="inline-flex items-center gap-1.5">
          <span
            class="h-2.5 w-2.5 rounded-full"
            style="background:{statusMeta(st).fill}"
          ></span>
          {statusMeta(st).label}
        </span>
      {/each}
    </div>
  </header>

  {#if rows.length === 0}
    <p class="text-sm text-ink-faint">该游戏暂无版本记录。</p>
  {:else}
    <div class="overflow-x-auto">
      <div class="relative min-w-[760px]" style="height:{totalH}px">
        <!-- 时间轴 -->
        <div class="absolute top-0 left-0 right-0" style="height:{AXIS_H}px">
          {#each weekLines as pct (pct)}
            <div
              class="absolute top-0 bottom-0 w-px bg-border/50"
              style="left:{pct}%"
            ></div>
          {/each}
          {#each months as m (m.label)}
            <div
              class="absolute top-0 h-full w-px bg-border"
              style="left:{m.left}%"
            >
              <span class="ml-1 text-[11px] text-ink-faint whitespace-nowrap">{m.label}</span>
            </div>
          {/each}
        </div>

        <!-- 行区 -->
        <div class="absolute top-0 left-0 right-0" style="top:{AXIS_H}px;height:{bodyH}px">
          {#each rows as row (row.entry.version_code)}
            <div
              class="absolute left-0 right-0 border-b border-border/40"
              style="top:{row.i * ROW_H}px;height:{ROW_H}px"
            >
              <span
                class="absolute top-1/2 -translate-y-1/2 text-xs text-ink-faint pl-1.5 select-none"
              >{row.entry.version_code}</span>
            </div>
            <!-- 甘特条 -->
            <div
              class="absolute top-1/2 -translate-y-1/2 flex h-6 items-center rounded-full px-2 text-[11px] font-semibold text-white shadow"
              style="left:{row.left}%;width:{row.width}%;background:{row.fill};top:{row.i * ROW_H + ROW_H / 2}px"
              title="{row.entry.version_code} {row.entry.start_status}"
            >
              <span class="truncate">{row.entry.version_code}</span>
            </div>
            <!-- 前瞻直播标记（菱形） -->
            {#if row.lsLeft !== null}
              <div
                class="absolute -translate-x-1/2 -translate-y-1/2"
                style="left:{row.lsLeft}%;top:{row.i * ROW_H + ROW_H / 2}px"
              >
                <div
                  class="h-3 w-3 rotate-45 border border-white bg-ink"
                  title="前瞻直播"
                ></div>
              </div>
            {/if}
          {/each}

          <!-- 现在线 -->
          <div
            class="absolute top-0 bottom-0 z-10 w-px bg-red-500/80"
            style="left:{nowPct}%"
            aria-hidden="true"
          ></div>
        </div>
      </div>
      <p class="mt-2 text-xs text-ink-faint">
        时间轴自 {formatWall(new Date(range.start).toISOString(), tz, 'date')}
        至 {formatWall(new Date(range.start + range.total).toISOString(), tz, 'date')}
        （北京时间）绘制，菱形标记为前瞻直播，红竖线为当前时间。
      </p>
    </div>
  {/if}
</section>
