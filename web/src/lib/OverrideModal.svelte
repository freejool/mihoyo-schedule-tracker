<script lang="ts">
  import { formatWall } from './utils';
  import type { Entry, OverrideResponse, SchedulesResponse } from './types';

  let {
    gameId,
    entry,
    onClose,
    onSaved
  }: {
    gameId: string;
    entry: Entry;
    onClose: () => void;
    onSaved: (schedules: SchedulesResponse) => void;
  } = $props();

  let versionCode = $state('');
  let versionTitle = $state('');
  let startAt = $state('');
  let livestreamAt = $state('');
  let durationDays = $state(42);
  let saving = $state(false);
  let deleting = $state(false);
  let error = $state<string | null>(null);

  let modalEl: HTMLDivElement | undefined = $state();

  const isManual = $derived(entry.start_status === 'manual_override');
  const codeChanged = $derived(versionCode.trim() !== entry.version_code);

  // 从当前行的 entry 初始化表单；行切换（重新传入 entry）时重填。
  $effect(() => {
    versionCode = entry.version_code;
    versionTitle = entry.version_title ?? '';
    startAt = formatWall(entry.start_at, 'Asia/Shanghai', 'datetime-local');
    livestreamAt = entry.livestream_at
      ? formatWall(entry.livestream_at, 'Asia/Shanghai', 'datetime-local')
      : '';
    durationDays = entry.duration_days;
  });

  // 挂载时保存焦点来源，并聚焦第一个输入框；关闭时还原。
  let restoreFocus: HTMLElement | null = null;
  $effect(() => {
    restoreFocus = document.activeElement as HTMLElement | null;
    modalEl?.querySelector<HTMLElement>('input, select, button')?.focus();
    return () => {
      restoreFocus?.focus?.();
    };
  });

  // Esc 关闭 + 简易焦点圈闭。
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
      } else if (e.key === 'Tab' && modalEl) {
        const focusables = Array.from(
          modalEl.querySelectorAll<HTMLElement>('input, select, button, a[href]')
        ).filter((el) => !el.hasAttribute('disabled'));
        if (focusables.length === 0) return;
        const first = focusables[0];
        const last = focusables[focusables.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  function setDuration(d: number) {
    durationDays = d;
  }

  async function save() {
    if (!startAt) {
      error = '版本开启时间必填';
      return;
    }
    saving = true;
    error = null;
    try {
      const body: Record<string, unknown> = {
        game_id: gameId,
        version_code: entry.version_code,
        start_at: startAt
      };
      if (codeChanged) body.new_version_code = versionCode.trim();
      if (versionTitle.trim() && versionTitle.trim() !== (entry.version_title ?? '')) {
        body.version_title = versionTitle.trim();
      }
      if (livestreamAt) body.livestream_at = livestreamAt;
      body.duration_days = durationDays;
      const res = await fetch('/api/override', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body)
      });
      if (!res.ok) {
        error = await readError(res);
        return;
      }
      const data = (await res.json()) as OverrideResponse;
      onSaved(data.schedules);
    } catch (err) {
      error = `请求失败：${err instanceof Error ? err.message : String(err)}`;
    } finally {
      saving = false;
    }
  }

  async function restoreAuto() {
    deleting = true;
    error = null;
    try {
      const res = await fetch(
        `/api/override/${encodeURIComponent(gameId)}/${encodeURIComponent(entry.version_code)}`,
        { method: 'DELETE' }
      );
      if (res.status === 409) {
        error = '该记录不是手动校准，无法恢复自动推算';
        return;
      }
      if (!res.ok) {
        error = await readError(res);
        return;
      }
      const data = (await res.json()) as { ok: boolean; deleted: boolean; schedules: SchedulesResponse };
      onSaved(data.schedules);
    } catch (err) {
      error = `请求失败：${err instanceof Error ? err.message : String(err)}`;
    } finally {
      deleting = false;
    }
  }

  async function readError(res: Response): Promise<string> {
    try {
      const data = (await res.json()) as { error?: string };
      return data.error ?? `HTTP ${res.status}`;
    } catch {
      return `HTTP ${res.status}`;
    }
  }
</script>

<div
  class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
  role="dialog"
  aria-modal="true"
  aria-label="校准 / 改期"
  tabindex="-1"
  onclick={(e) => {
    if (e.target === e.currentTarget) onClose();
  }}
  onkeydown={(e) => {
    if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) onClose();
  }}
>
  <div
    bind:this={modalEl}
    class="max-h-[90vh] w-full max-w-md overflow-y-auto rounded-2xl border border-border bg-surface-raised p-5 shadow-xl"
  >
    <header class="mb-4 flex items-center justify-between gap-2">
      <h2 class="text-lg font-bold">
        校准 / 改期
        <span class="ml-2 text-sm font-normal text-ink-muted">
          {gameId} · {entry.version_code}
        </span>
      </h2>
      <button
        type="button"
        aria-label="关闭"
        onclick={onClose}
        class="rounded-lg p-1 text-ink-faint hover:bg-surface-muted hover:text-ink focus-visible:outline-2 focus-visible:outline-accent"
      >✕</button>
    </header>

    {#if error}
      <p role="alert" class="mb-3 rounded-lg border border-red-500/40 bg-red-500/10 px-3 py-2 text-sm text-red-600 dark:text-red-400">
        {error}
      </p>
    {/if}

    <form
      onsubmit={(e) => {
        e.preventDefault();
        save();
      }}
      class="space-y-4"
    >
      <div>
        <label for="ov-code" class="mb-1 block text-sm font-medium">版本号</label>
        <input
          id="ov-code"
          bind:value={versionCode}
          type="text"
          class="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm focus:border-accent focus:outline-none"
        />
        {#if codeChanged}
          <p class="mt-1 text-xs text-ink-faint">
            版本号已变更（{entry.version_code} → {versionCode.trim()}），保存时将以
            <code>new_version_code</code> 提交。
          </p>
        {/if}
      </div>

      <div>
        <label for="ov-start" class="mb-1 block text-sm font-medium">版本开启（北京时间）</label>
        <input
          id="ov-start"
          bind:value={startAt}
          type="datetime-local"
          required
          class="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm tabular-nums focus:border-accent focus:outline-none"
        />
      </div>

      <div>
        <label for="ov-ls" class="mb-1 block text-sm font-medium">
          前瞻直播（北京时间，可留空清除预测）
        </label>
        <input
          id="ov-ls"
          bind:value={livestreamAt}
          type="datetime-local"
          class="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm tabular-nums focus:border-accent focus:outline-none"
        />
      </div>

      <div>
        <label for="ov-dur" class="mb-1 block text-sm font-medium">版本持续天数</label>
        <div class="flex items-center gap-2">
          <div class="flex gap-1">
            {#each [35, 42, 49] as pill (pill)}
              <button
                type="button"
                onclick={() => setDuration(pill)}
                class="rounded-lg px-3 py-2 text-sm font-medium border {durationDays === pill
                  ? 'border-accent bg-accent/10 text-accent'
                  : 'border-border text-ink-muted hover:border-accent'}"
              >{pill}</button>
            {/each}
          </div>
          <input
            id="ov-dur"
            bind:value={durationDays}
            type="number"
            min="1"
            class="w-24 rounded-lg border border-border bg-surface px-3 py-2 text-sm tabular-nums focus:border-accent focus:outline-none"
          />
          <span class="text-sm text-ink-faint">天</span>
        </div>
      </div>

      <div>
        <label for="ov-title" class="mb-1 block text-sm font-medium">版本名称（可选）</label>
        <input
          id="ov-title"
          bind:value={versionTitle}
          type="text"
          placeholder="如：往冥府的安魂歌"
          class="w-full rounded-lg border border-border bg-surface px-3 py-2 text-sm focus:border-accent focus:outline-none"
        />
      </div>

      <div class="flex flex-wrap items-center justify-end gap-2 pt-1">
        <button
          type="button"
          onclick={onClose}
          class="rounded-lg border border-border px-4 py-2 text-sm font-medium hover:bg-surface-muted focus-visible:outline-2 focus-visible:outline-accent"
        >取消</button>
        <button
          type="button"
          onclick={restoreAuto}
          disabled={!isManual || saving || deleting}
          class="rounded-lg border border-border px-4 py-2 text-sm font-medium disabled:opacity-40 hover:enabled:border-red-500 hover:enabled:text-red-500 focus-visible:outline-2 focus-visible:outline-accent"
        >{deleting ? '恢复中…' : '恢复自动推算'}</button>
        <button
          type="submit"
          disabled={saving || deleting}
          class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-white disabled:opacity-40 hover:enabled:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
        >{saving ? '保存中…' : '保存'}</button>
      </div>
    </form>
  </div>
</div>
