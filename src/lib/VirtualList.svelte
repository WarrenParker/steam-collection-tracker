<script lang="ts" generics="T">
  import type { Snippet } from 'svelte';

  // Draws only the rows in view (plus a few either side), so lists of tens of
  // thousands scroll smoothly. Every row is the same height.
  let {
    items,
    rowHeight,
    key,
    row,
    empty,
    overscan = 10,
    label,
    role = 'list',
  }: {
    items: T[];
    rowHeight: number;
    key: (item: T) => string | number;
    row: Snippet<[T, number]>;
    empty?: Snippet;
    overscan?: number;
    label?: string;
    role?: string;
  } = $props();

  let viewport: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let height = $state(0);

  const start = $derived(Math.max(0, Math.floor(scrollTop / rowHeight) - overscan));
  const end = $derived(Math.min(items.length, Math.ceil((scrollTop + height) / rowHeight) + overscan));
  const visible = $derived(items.slice(start, end));

  /** Scrolls just enough to show row `index`. */
  export function reveal(index: number) {
    if (!viewport) return;
    const top = index * rowHeight;
    if (top < viewport.scrollTop) viewport.scrollTop = top;
    else if (top + rowHeight > viewport.scrollTop + height) viewport.scrollTop = top + rowHeight - height;
  }

  export function toTop() {
    if (viewport) viewport.scrollTop = 0;
  }
</script>

<div
  class="vlist"
  bind:this={viewport}
  bind:clientHeight={height}
  onscroll={() => (scrollTop = viewport?.scrollTop ?? 0)}
  {role}
  aria-label={label}
>
  {#if items.length === 0 && empty}
    {@render empty()}
  {:else}
    <div class="vlist-space" style:height="{items.length * rowHeight}px">
      <div class="vlist-rows" style:transform="translateY({start * rowHeight}px)">
        {#each visible as item, i (key(item))}
          {@render row(item, start + i)}
        {/each}
      </div>
    </div>
  {/if}
</div>
