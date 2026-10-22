<script lang="ts">
  let { games, onClose }: { games: { id: string; name: string }[]; onClose: () => void } = $props();

  const initialGameIds = $derived(games.map((g) => g.id));
  let selected = $state<Set<string>>(new Set());
  $effect(() => {
    selected = new Set(initialGameIds);
  });
  let includePredicted = $state(true);
  let versions = $state<'all' | '1' | '2'>('all');
  let copied = $state(false);

  const url = $derived.by(() => {
    const gamesParam = [...selected].sort().join(',');
    const base = `${location.origin}/calendar.ics`;
    const params = new URLSearchParams();
    if (gamesParam) params.set('games', gamesParam);
    params.set('include_predicted', String(includePredicted));
    if (versions !== 'all') params.set('versions', versions);
    return `${base}?${params.toString()}`;
  });

  function toggle(gameId: string) {
    const next = new Set(selected);
    if (next.has(gameId)) next.delete(gameId);
    else next.add(gameId);
    selected = next;
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(url);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      copied = false;
    }
  }

  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
</script>

<div
  class="absolute top-full right-0 z-40 mt-2 w-[320px] rounded-2xl border border-border bg-surface-raised p-4 shadow-xl"
  role="dialog"
  aria-label="订阅 .ics 日历"
>
  <header class="mb-3 flex items-center justify-between">
    <h3 class="text-sm font-bold">复制 .ics 订阅链接</h3>
    <button
      type="button"
      aria-label="关闭"
      onclick={onClose}
      class="rounded-lg p-1 text-ink-faint hover:bg-surface-muted hover:text-ink focus-visible:outline-2 focus-visible:outline-accent"
    >✕</button>
  </header>

  <fieldset class="mb-3">
    <legend class="mb-1 text-xs text-ink-faint">选择游戏</legend>
    <div class="flex flex-wrap gap-2">
      {#each games as g (g.id)}
        <label
          class="inline-flex cursor-pointer items-center gap-1.5 rounded-full border px-3 py-1 text-sm {selected.has(g.id)
            ? 'border-accent bg-accent/10 text-accent'
            : 'border-border text-ink-muted'}"
        >
          <input
            type="checkbox"
            checked={selected.has(g.id)}
            onclick={() => toggle(g.id)}
            class="sr-only"
          />
          {g.name}
        </label>
      {/each}
    </div>
  </fieldset>

  <label class="mb-3 inline-flex cursor-pointer items-center gap-2 text-sm">
    <input
      type="checkbox"
      bind:checked={includePredicted}
      class="h-4 w-4 rounded accent-[var(--color-accent)]"
    />
    包含预测节点
  </label>

  <label class="mb-3 flex items-center justify-between gap-2 text-sm">
    <span class="text-ink-muted">日历范围</span>
    <select
      bind:value={versions}
      class="rounded-lg border border-border bg-surface-raised px-2 py-1 text-sm text-ink focus-visible:outline-2 focus-visible:outline-accent"
    >
      <option value="all">全部版本</option>
      <option value="1">仅下一版本（T+1）</option>
      <option value="2">最近两个版本</option>
    </select>
  </label>

  <p class="mb-2 rounded-lg bg-surface-muted px-3 py-2 text-xs break-all text-ink-muted tabular-nums">
    {url}
  </p>

  <div class="flex items-center justify-between gap-2">
    <button
      type="button"
      onclick={copy}
      class="rounded-lg bg-accent px-3 py-2 text-sm font-semibold text-white hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
    >{copied ? '已复制 ✓' : '复制链接'}</button>
    <a
      href={url}
      target="_blank"
      rel="noopener noreferrer"
      class="text-sm text-accent hover:underline focus-visible:outline-2 focus-visible:outline-accent"
    >在新标签打开 ↗</a>
  </div>
</div>
