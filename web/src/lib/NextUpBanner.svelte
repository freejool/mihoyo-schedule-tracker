<script lang="ts">
  import StatusBadge from './StatusBadge.svelte';
  import Countdown from './Countdown.svelte';
  import type { NextUp, NextUpItem } from './types';

  let { nextUp, onSelectGame }: { nextUp: NextUp; onSelectGame: (gameId: string) => void } =
    $props();

  interface NextCard {
    key: 'livestream' | 'start';
    item: NextUpItem | null;
    title: string;
    prefix: string;
    done: string;
    target: 'livestream_at' | 'start_at';
    statusKey: 'livestream_status' | 'start_status';
  }

  const cards = $derived<NextCard[]>([
    {
      key: 'livestream',
      item: nextUp.livestream,
      title: '即将到来 · 前瞻直播',
      prefix: '距前瞻直播',
      done: '已直播',
      target: 'livestream_at',
      statusKey: 'livestream_status'
    },
    {
      key: 'start',
      item: nextUp.start,
      title: '即将到来 · 版本开启',
      prefix: '距版本开启',
      done: '已开始',
      target: 'start_at',
      statusKey: 'start_status'
    }
  ]);
</script>

{#if !nextUp.livestream && !nextUp.start}
  <p class="text-sm text-ink-muted">暂无已排期的前瞻直播或版本开启。</p>
{:else}
  <div class="grid gap-3 sm:grid-cols-2">
    {#each cards as card (card.key)}
      {#if card.item}
        <button
          type="button"
          aria-label="筛选到{card.item.game_name}"
          onclick={() => onSelectGame(card.item!.game_id)}
          class="group rounded-xl border border-border bg-surface-raised p-4 text-left shadow-sm transition hover:border-accent focus-visible:outline-2 focus-visible:outline-accent"
        >
          <div class="flex flex-wrap items-center gap-2">
            <span class="text-xs font-semibold tracking-wide text-ink-faint uppercase">
              {card.title}
            </span>
            <StatusBadge status={card.item.entry[card.statusKey]} />
          </div>
          <div class="mt-2 flex flex-wrap items-baseline gap-x-3 gap-y-1">
            <span class="text-lg font-bold">{card.item.game_name}</span>
            <span class="rounded-md bg-surface-muted px-1.5 py-0.5 text-sm font-medium">
              {card.item.entry.version_code}
            </span>
            {#if card.item.entry.version_title}
              <span class="text-sm text-ink-muted">{card.item.entry.version_title}</span>
            {/if}
          </div>
          <div class="mt-3 flex items-baseline justify-between gap-2">
            <span class="text-sm text-ink-muted">{card.prefix}</span>
            <Countdown target={card.item.entry[card.target]} doneLabel={card.done} />
          </div>
        </button>
      {/if}
    {/each}
  </div>
{/if}
