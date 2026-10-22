<script lang="ts">
  import { statusMeta } from './utils';
  import type { EntryStatus } from './types';

  let { status }: { status: EntryStatus | null } = $props();
  const meta = $derived(statusMeta(status));
</script>

<span
  class="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium whitespace-nowrap {meta.badge}"
>
  <span aria-hidden="true">{meta.emoji}</span>
  {meta.label}
</span>
