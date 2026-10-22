<script lang="ts">
  import { countdownParts } from './utils';

  let { target, doneLabel = '已开始' }: { target: string | null; doneLabel?: string } = $props();

  // 每秒刷新一次，用 Date.now() 与目标时间对表。
  let now = $state(Date.now());
  $effect(() => {
    if (!target) return;
    const id = setInterval(() => {
      now = Date.now();
    }, 1000);
    return () => clearInterval(id);
  });

  const parts = $derived(target ? countdownParts(target, now) : null);
  const text = $derived(
    parts === null
      ? '—'
      : parts.negative
        ? doneLabel
        : `${parts.days}天 ${String(parts.hours).padStart(2, '0')}:${String(parts.minutes).padStart(2, '0')}:${String(parts.seconds).padStart(2, '0')}`
  );
</script>

<span class="tabular-nums font-mono text-sm">{text}</span>
