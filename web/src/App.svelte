<script lang="ts">
  import NextUpBanner from './lib/NextUpBanner.svelte';
  import TableView from './lib/TableView.svelte';
  import GanttView from './lib/GanttView.svelte';
  import OverrideModal from './lib/OverrideModal.svelte';
  import IcsPopover from './lib/IcsPopover.svelte';
  import { formatWall } from './lib/utils';
  import type {
    Entry,
    GameFilter,
    SchedulesResponse,
    SyncResponse,
    ViewMode
  } from './lib/types';

  const FILTERS: { id: GameFilter; label: string }[] = [
    { id: 'all', label: '全部' },
    { id: 'ys', label: '原神' },
    { id: 'sr', label: '崩坏：星穹铁道' },
    { id: 'zzz', label: '绝区零' }
  ];

  let schedules = $state<SchedulesResponse | null>(null);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let filter = $state<GameFilter>('all');
  let view = $state<ViewMode>('table');
  let dark = $state(false);
  let syncing = $state(false);
  let icsOpen = $state(false);
  let editing = $state<{ gameId: string; entry: Entry } | null>(null);
  let toast = $state<{ text: string; kind: 'ok' | 'err' } | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  let initialized = false;

  const tz = $derived(schedules?.tz ?? 'Asia/Shanghai');
  const visibleGames = $derived(
    filter === 'all'
      ? (schedules?.games ?? [])
      : (schedules?.games.filter((g) => g.id === filter) ?? [])
  );

  // 深色模式：localStorage 持久化，默认跟随系统。
  try {
    const stored = localStorage.getItem('theme');
    dark = stored === 'dark' || (stored === null && matchMedia('(prefers-color-scheme: dark)').matches);
  } catch {
    dark = false;
  }
  $effect(() => {
    document.documentElement.classList.toggle('dark', dark);
    try {
      localStorage.setItem('theme', dark ? 'dark' : 'light');
    } catch {
      /* 隐私模式下忽略 */
    }
  });

  function showToast(text: string, kind: 'ok' | 'err' = 'ok') {
    toast = { text, kind };
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), 4000);
  }

  async function load() {
    loading = true;
    loadError = null;
    try {
      const res = await fetch('/api/schedules');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = (await res.json()) as SchedulesResponse;
      schedules = data;
    } catch (err) {
      // JSON 解析失败也归为加载错误，绝不让页面空白。
      loadError = err instanceof Error ? err.message : String(err);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (!initialized) {
      initialized = true;
      load();
    }
  });

  function refresh(data: SchedulesResponse) {
    schedules = data;
    editing = null;
  }

  async function syncNow() {
    if (syncing) return;
    syncing = true;
    try {
      const res = await fetch('/api/sync', { method: 'POST' });
      const data = (await res.json()) as SyncResponse & { error?: string };
      if (!res.ok) {
        showToast(`同步失败：${data.error ?? `HTTP ${res.status}`}`, 'err');
        return;
      }
      if (data.schedules) refresh(data.schedules);
      showToast(
        `同步完成：扫描 ${data.scanned_count} 条，更新 ${data.updated_count} 条`
      );
    } catch (err) {
      showToast(`同步失败：${err instanceof Error ? err.message : String(err)}`, 'err');
    } finally {
      syncing = false;
    }
  }

  function selectGame(gameId: string) {
    filter = (FILTERS.some((f) => f.id === gameId) ? gameId : 'all') as GameFilter;
    // 等一帧让视图渲染后再滚动定位。
    setTimeout(() => {
      document.getElementById(`game-${gameId}`)?.scrollIntoView({
        behavior: 'smooth',
        block: 'start'
      });
    }, 50);
  }
</script>

<div class="min-h-screen bg-surface text-ink">
  <!-- 工具栏 -->
  <header class="sticky top-0 z-30 border-b border-border bg-surface/90 backdrop-blur">
    <div class="mx-auto max-w-6xl px-4 py-3">
      <div class="flex flex-wrap items-center gap-x-4 gap-y-2">
        <h1 class="text-lg font-bold">🎮 米哈游版本排期追踪</h1>
        <nav aria-label="游戏筛选" class="flex flex-wrap gap-1">
          {#each FILTERS as f (f.id)}
            <button
              type="button"
              aria-pressed={filter === f.id}
              onclick={() => (filter = f.id)}
              class="rounded-full px-3 py-1.5 text-sm font-medium {filter === f.id
                ? 'bg-accent text-white'
                : 'bg-surface-muted text-ink-muted hover:text-ink'}"
            >{f.label}</button>
          {/each}
        </nav>
        <div class="ml-auto flex flex-wrap items-center gap-2">
          <!-- 视图切换 -->
          <div class="flex rounded-lg border border-border p-0.5" role="tablist" aria-label="视图切换">
            {#each [
              { id: 'table', label: '聚合时间表' },
              { id: 'gantt', label: '42天周期甘特条' }
            ] as v (v.id)}
              <button
                type="button"
                role="tab"
                aria-selected={view === v.id}
                onclick={() => (view = v.id as ViewMode)}
                class="rounded-md px-3 py-1 text-sm {view === v.id
                  ? 'bg-accent text-white'
                  : 'text-ink-muted hover:text-ink'}"
              >{v.label}</button>
            {/each}
          </div>
          <!-- 深色模式 -->
          <button
            type="button"
            aria-label={dark ? '切换到浅色模式' : '切换到深色模式'}
            onclick={() => (dark = !dark)}
            class="rounded-lg border border-border p-2 text-base leading-none hover:bg-surface-muted focus-visible:outline-2 focus-visible:outline-accent"
          >{dark ? '☀️' : '🌙'}</button>
          <!-- 同步 -->
          <button
            type="button"
            onclick={syncNow}
            disabled={syncing}
            class="flex items-center gap-2 rounded-lg bg-accent px-3 py-2 text-sm font-semibold text-white disabled:opacity-60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
          >
            {#if syncing}
              <span
                class="inline-block h-4 w-4 animate-spin rounded-full border-2 border-white/40 border-t-white"
                aria-hidden="true"
              ></span>
              <span>同步中…</span>
            {:else}
              <span aria-hidden="true">🔄</span>
              <span>立即同步官方公告</span>
            {/if}
          </button>
          <!-- .ics 订阅 -->
          <div class="relative">
            <button
              type="button"
              aria-haspopup="dialog"
              aria-expanded={icsOpen}
              onclick={() => (icsOpen = !icsOpen)}
              class="rounded-lg border border-border px-3 py-2 text-sm font-medium hover:bg-surface-muted focus-visible:outline-2 focus-visible:outline-accent"
            >📅 复制 .ics 订阅链接</button>
            {#if icsOpen}
              <IcsPopover
                games={schedules?.games.map((g) => ({ id: g.id, name: g.name })) ?? []}
                onClose={() => (icsOpen = false)}
              />
            {/if}
          </div>
        </div>
      </div>

      <!-- 同步状态行 -->
      <div class="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm">
        {#if schedules?.sync}
          <span class="font-medium">
            {#if schedules.sync.status === 'success'}
              <span class="rounded-full bg-status-official/15 px-2 py-0.5 text-xs font-semibold text-status-official">同步成功</span>
            {:else if schedules.sync.status === 'error'}
              <span class="rounded-full bg-red-500/15 px-2 py-0.5 text-xs font-semibold text-red-600 dark:text-red-400">同步出错</span>
            {:else}
              <span class="rounded-full bg-surface-muted px-2 py-0.5 text-xs font-semibold text-ink-muted">同步中</span>
            {/if}
          </span>
          <span class="tabular-nums text-ink-muted">
            {formatWall(schedules.sync.created_at, tz, 'minute')}
          </span>
          <span class="tabular-nums text-ink-muted">
            扫描 {schedules.sync.scanned_count} 条 · 更新 {schedules.sync.updated_count} 条
          </span>
          {#if schedules.sync.detail}
            <span class="truncate text-ink-faint" title={schedules.sync.detail}>
              {schedules.sync.detail}
            </span>
          {/if}
        {:else}
          <span class="text-ink-faint">尚未同步</span>
        {/if}
      </div>
    </div>
  </header>

  <main class="mx-auto max-w-6xl px-4 py-5">
    {#if loading}
      <!-- 首屏骨架 -->
      <div class="space-y-4" aria-label="加载中">
        {#each [1, 2, 3] as n (n)}
          <div class="animate-pulse rounded-2xl border border-border bg-surface-raised p-5">
            <div class="h-5 w-32 rounded bg-surface-muted"></div>
            <div class="mt-3 h-4 w-full rounded bg-surface-muted"></div>
            <div class="mt-2 h-4 w-2/3 rounded bg-surface-muted"></div>
          </div>
        {/each}
      </div>
    {:else if loadError}
      <!-- 加载失败 -->
      <div
        role="alert"
        class="rounded-2xl border border-red-500/40 bg-red-500/10 p-6 text-center"
      >
        <p class="text-sm text-red-600 dark:text-red-400">
          加载数据失败：{loadError}
        </p>
        <button
          type="button"
          onclick={load}
          class="mt-3 rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-white hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
        >重试</button>
      </div>
    {:else if schedules}
      <div class="space-y-5">
        <NextUpBanner nextUp={schedules.next_up} onSelectGame={selectGame} />
        {#if view === 'table'}
          {#each visibleGames as game (game.id)}
            <TableView
              game={game}
              tz={tz}
              onCalibrate={(entry: Entry) => (editing = { gameId: game.id, entry })}
            />
          {/each}
          {#if visibleGames.length === 0}
            <p class="text-sm text-ink-faint">该筛选下暂无游戏数据。</p>
          {/if}
        {:else}
          {#each visibleGames as game (game.id)}
            <GanttView game={game} tz={tz} />
          {/each}
          {#if visibleGames.length === 0}
            <p class="text-sm text-ink-faint">该筛选下暂无游戏数据。</p>
          {/if}
        {/if}
      </div>
    {/if}
  </main>

  <!-- Toast -->
  {#if toast}
    <div
      role="status"
      class="fixed right-4 bottom-4 z-50 max-w-sm rounded-xl border px-4 py-3 text-sm shadow-lg {toast.kind === 'ok'
        ? 'border-status-official/40 bg-surface-raised text-ink'
        : 'border-red-500/40 bg-surface-raised text-red-600 dark:text-red-400'}"
    >
      {toast.kind === 'ok' ? '✅ ' : '⚠️ '}{toast.text}
    </div>
  {/if}

  <!-- 校准弹窗 -->
  {#if editing}
    <OverrideModal
      gameId={editing.gameId}
      entry={editing.entry}
      onClose={() => (editing = null)}
      onSaved={(data: SchedulesResponse) => {
        refresh(data);
        showToast('校准已保存');
      }}
    />
  {/if}
</div>
