<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api, errorText, type GameName, type ItemRow } from './api';
  import { holding } from './format';
  import Icon from './Icon.svelte';
  import { nav, openGame, openItem } from './nav.svelte.ts';

  // Search every game and item, and jump straight to one.
  let { onclose }: { onclose: () => void } = $props();

  type Hit =
    | { type: 'game'; id: string; game: GameName; score: number }
    | { type: 'item'; id: string; item: ItemRow; game: string; score: number };

  const MAX_GAMES = 6;
  const MAX_ITEMS = 40;

  let input: HTMLInputElement | undefined = $state();
  let listEl: HTMLDivElement | undefined = $state();
  let query = $state('');
  let games = $state<GameName[]>([]);
  let items = $state<ItemRow[]>([]);
  let error = $state('');
  let active = $state(0);

  const names = $derived(new Map(games.map((g) => [g.appid, g.name])));

  /** Higher is better: whole name, start of the name, start of a word, anywhere. */
  function score(text: string, words: string[]): number {
    const t = text.toLowerCase();
    let s = 0;
    for (const w of words) {
      const at = t.indexOf(w);
      if (at < 0) return -1;
      s += at === 0 ? 3 : /\W/.test(t[at - 1] ?? '') ? 2 : 1;
    }
    if (t === words.join(' ')) s += 10;
    return s;
  }

  const hits = $derived.by((): Hit[] => {
    const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return [];
    const gameHits: Hit[] = [];
    for (const g of games) {
      const s = score(g.name, words);
      if (s >= 0) gameHits.push({ type: 'game', id: `g${g.appid}`, game: g, score: s });
    }
    const itemHits: Hit[] = [];
    for (const i of items) {
      const game = names.get(i.appid) ?? '';
      const full = score(`${i.name} ${game}`, words);
      if (full < 0) continue;
      // Items matching by their own name rank above ones matching by their game's.
      const own = score(i.name, words);
      itemHits.push({ type: 'item', id: i.key, item: i, game, score: own >= 0 ? own : full - 5 });
    }
    const order = (a: Hit, b: Hit) => {
      const an = a.type === 'game' ? a.game.name : a.item.name;
      const bn = b.type === 'game' ? b.game.name : b.item.name;
      return b.score - a.score || an.length - bn.length || an.localeCompare(bn);
    };
    return [...gameHits.sort(order).slice(0, MAX_GAMES), ...itemHits.sort(order).slice(0, MAX_ITEMS)];
  });

  const firstItem = $derived(hits.findIndex((h) => h.type === 'item'));

  $effect(() => {
    void hits;
    active = 0;
  });

  onMount(async () => {
    input?.focus();
    try {
      const v = await api.items();
      games = v.games;
      items = v.items;
    } catch (e) {
      error = errorText(e);
    }
  });

  function choose(h: Hit | undefined) {
    if (!h) return;
    onclose();
    if (h.type === 'game') openGame(h.game.appid);
    else openItem(h.item.key);
  }

  async function move(to: number) {
    if (!hits.length) return;
    active = (to + hits.length) % hits.length;
    await tick();
    listEl?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      move(active + 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      move(active - 1);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      choose(hits[active]);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    }
  }
</script>

<div class="scrim top" onclick={onclose} role="presentation"></div>
<div class="palette" role="dialog" aria-modal="true" aria-label="Search">
  <div class="palette-input">
    <Icon name="search" size={18} />
    <input
      bind:this={input}
      bind:value={query}
      {onkeydown}
      placeholder="Search games and items"
      spellcheck="false"
      autocomplete="off"
      role="combobox"
      aria-expanded={hits.length > 0}
      aria-controls="palette-results"
      aria-activedescendant={hits[active] ? `hit-${hits[active].id}` : undefined}
    />
    <kbd>Esc</kbd>
  </div>
  <div class="palette-results" id="palette-results" role="listbox" bind:this={listEl}>
    {#if error}
      <p class="field-error">{error}</p>
    {:else if !query.trim()}
      <p class="palette-hint">
        Type a game or item name. <kbd>↑</kbd> <kbd>↓</kbd> to move, <kbd>Enter</kbd> to open.
        {#if nav.view !== 'items'}In the Items list, try the filters too.{/if}
      </p>
    {:else if !hits.length}
      <p class="palette-hint">Nothing matches “{query.trim()}”.</p>
    {:else}
      {#each hits as h, n (h.id)}
        {#if n === 0 && h.type === 'game'}<div class="palette-group">Games</div>{/if}
        {#if n === firstItem}<div class="palette-group">Items</div>{/if}
        <div
          class="palette-hit"
          id="hit-{h.id}"
          role="option"
          tabindex="-1"
          aria-selected={n === active}
          onclick={() => choose(h)}
          onkeydown={(e) => e.key === 'Enter' && choose(h)}
          onmousemove={() => (active = n)}
        >
          {#if h.type === 'game'}
            <span class="hit-icon"><Icon name="game" size={16} /></span>
            <span class="hit-name">{h.game.name}</span>
            <span class="hit-meta">
              {h.game.owned ? 'Game' : h.game.family ? 'Family' : h.game.event ? 'Event' : 'Unowned'}
            </span>
          {:else}
            <span class="thumb tiny" class:foil-edge={h.item.foil}>
              {#if h.item.image}<img src={h.item.image} alt="" loading="lazy" />{/if}
            </span>
            <span class="hit-name">
              {h.item.name}
              <span class="hit-sub">{h.game} · {h.item.kind}</span>
            </span>
            <span class="hit-meta">{holding(h.item) || 'Missing'}</span>
          {/if}
        </div>
      {/each}
    {/if}
  </div>
</div>
