<script lang="ts">
  import StatusBadge from './StatusBadge.svelte';
  import Countdown from './Countdown.svelte';
  import { formatWall } from './utils';
  import type { Entry, GameSchedule } from './types';

  let {
    game,
    tz,
    onCalibrate
  }: {
    game: GameSchedule;
    tz: string;
    onCalibrate: (entry: Entry) => void;
  } = $props();

  const currentCode = $derived(game.current?.version_code ?? null);
  const upcomingCode = $derived(game.upcoming?.version_code ?? null);

  function rowClass(entry: Entry): string {
    if (entry.version_code === currentCode)
      return 'border-l-2 border-status-official bg-status-official/5';
    if (entry.version_code === upcomingCode)
      return 'border-l-2 border-accent bg-accent/5';
    return '';
  }

  function upcomingBadge(entry: Entry): string {
    if (entry.version_code === currentCode) return '当前版本';
    if (entry.version_code === upcomingCode) return '下一版本';
    return '';
  }
</script>

<section id="game-{game.id}" class="scroll-mt-24 rounded-2xl border border-border bg-surface-raised p-4 shadow-sm sm:p-5">
  <header class="mb-3 flex flex-wrap items-center gap-2">
    <h2 class="text-lg font-bold">{game.name}</h2>
    {#if game.current && game.upcoming}
      <p class="text-sm text-ink-muted">
        当前 {game.current.version_code}
        {game.current.version_title ? `《${game.current.version_title}》` : ''}
        · 下一版 {game.upcoming.version_code}
        {game.upcoming.version_title ? `《${game.upcoming.version_title}》` : ''}
      </p>
    {:else}
      <p class="text-sm text-ink-muted">暂无排期数据</p>
    {/if}
  </header>

  {#if game.entries.length === 0}
    <p class="text-sm text-ink-faint">该游戏暂无版本记录。</p>
  {:else}
    <!-- 桌面端：表格 -->
    <div class="hidden md:block overflow-x-auto">
      <table class="w-full min-w-[860px] border-collapse text-sm">
        <thead>
          <tr class="border-b border-border text-left text-xs text-ink-faint">
            <th class="px-3 py-2 font-medium">版本号</th>
            <th class="px-3 py-2 font-medium">状态</th>
            <th class="px-3 py-2 font-medium">前瞻时间</th>
            <th class="px-3 py-2 font-medium">版本开启</th>
            <th class="px-3 py-2 font-medium">持续天数</th>
            <th class="px-3 py-2 font-medium">距开启</th>
            <th class="px-3 py-2 font-medium">官方原帖</th>
            <th class="px-3 py-2 font-medium" aria-label="操作"></th>
          </tr>
        </thead>
        <tbody>
          {#each game.entries as entry (entry.version_code)}
            <tr class="border-b border-border/60 {rowClass(entry)}">
              <td class="px-3 py-2">
                <div class="flex items-center gap-2">
                  <span class="font-semibold">{entry.version_code}</span>
                  {#if upcomingBadge(entry)}
                    <span
                      class="rounded-full px-2 py-0.5 text-[11px] font-medium {entry.version_code === currentCode
                        ? 'bg-status-official/15 text-status-official'
                        : 'bg-accent/15 text-accent'}"
                    >{upcomingBadge(entry)}</span>
                  {/if}
                </div>
                {#if entry.version_title}
                  <div class="text-xs text-ink-faint">{entry.version_title}</div>
                {/if}
              </td>
              <td class="px-3 py-2"><StatusBadge status={entry.start_status} /></td>
              <td class="px-3 py-2 tabular-nums whitespace-nowrap">
                {#if entry.livestream_at}
                  {formatWall(entry.livestream_at, tz, 'short')}
                  <span class="text-xs text-ink-faint">（{entry.livestream_weekday ?? ''}）</span>
                {:else}
                  <span class="text-ink-faint">—</span>
                {/if}
              </td>
              <td class="px-3 py-2 tabular-nums whitespace-nowrap">
                {formatWall(entry.start_at, tz, 'date')}
                <span class="text-xs text-ink-faint">（{entry.start_weekday}）</span>
              </td>
              <td class="px-3 py-2 tabular-nums">{entry.duration_days} 天</td>
              <td class="px-3 py-2 whitespace-nowrap">
                <Countdown target={entry.start_at} />
              </td>
              <td class="px-3 py-2">
                {#if entry.source_url}
                  <a
                    class="text-accent hover:underline focus-visible:outline-2 focus-visible:outline-accent"
                    href={entry.source_url}
                    target="_blank"
                    rel="noopener noreferrer"
                  >查看原帖 ↗</a>
                {:else}
                  <span class="text-ink-faint">—</span>
                {/if}
              </td>
              <td class="px-3 py-2 text-right">
                <button
                  type="button"
                  aria-label="校准{game.name} {entry.version_code}"
                  onclick={() => onCalibrate(entry)}
                  class="rounded-lg border border-border px-2.5 py-1 text-xs font-medium hover:border-accent hover:text-accent focus-visible:outline-2 focus-visible:outline-accent"
                >校准</button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    <!-- 移动端：堆叠卡片 -->
    <ul class="grid gap-3 md:hidden">
      {#each game.entries as entry (entry.version_code)}
        <li class="rounded-xl border border-border p-3 {rowClass(entry)}">
          <div class="flex flex-wrap items-center gap-2">
            <span class="text-base font-bold">{entry.version_code}</span>
            {#if upcomingBadge(entry)}
              <span
                class="rounded-full px-2 py-0.5 text-[11px] font-medium {entry.version_code === currentCode
                  ? 'bg-status-official/15 text-status-official'
                  : 'bg-accent/15 text-accent'}"
              >{upcomingBadge(entry)}</span>
            {/if}
            <StatusBadge status={entry.start_status} />
          </div>
          {#if entry.version_title}
            <p class="mt-1 text-sm text-ink-muted">{entry.version_title}</p>
          {/if}
          <dl class="mt-2 space-y-1 text-sm">
            <div class="flex justify-between gap-2">
              <dt class="text-ink-faint">前瞻时间</dt>
              <dd class="tabular-nums">
                {#if entry.livestream_at}
                  {formatWall(entry.livestream_at, tz, 'short')}
                  <span class="text-xs text-ink-faint">（{entry.livestream_weekday ?? ''}）</span>
                {:else}—{/if}
              </dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-ink-faint">版本开启</dt>
              <dd class="tabular-nums">
                {formatWall(entry.start_at, tz, 'date')}
                <span class="text-xs text-ink-faint">（{entry.start_weekday}）</span>
              </dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-ink-faint">持续天数</dt>
              <dd class="tabular-nums">{entry.duration_days} 天</dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-ink-faint">距开启</dt>
              <dd><Countdown target={entry.start_at} /></dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-ink-faint">官方原帖</dt>
              <dd>
                {#if entry.source_url}
                  <a
                    class="text-accent hover:underline"
                    href={entry.source_url}
                    target="_blank"
                    rel="noopener noreferrer"
                  >查看原帖 ↗</a>
                {:else}<span class="text-ink-faint">—</span>{/if}
              </dd>
            </div>
          </dl>
          <button
            type="button"
            aria-label="校准{game.name} {entry.version_code}"
            onclick={() => onCalibrate(entry)}
            class="mt-3 w-full rounded-lg border border-border px-2.5 py-1.5 text-xs font-medium hover:border-accent hover:text-accent focus-visible:outline-2 focus-visible:outline-accent"
          >校准 / 改期</button>
        </li>
      {/each}
    </ul>
  {/if}
</section>
