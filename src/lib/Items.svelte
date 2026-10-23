<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api, errorText, on, type Availability, type GameName, type Group, type ItemRow } from './api';
  import { change, GROUP_LABELS, GROUP_ORDER, holding, money, num } from './format';
  import Icon from './Icon.svelte';
  import VirtualList from './VirtualList.svelte';
  import { nav, openGame, openItem, typing } from './nav.svelte.ts';

  // Read again when prices or completion rules change.
  let { dataKey }: { dataKey: string } = $props();

  type Have = 'all' | 'owned' | 'missing';
  type SortKey = 'name' | 'game' | 'kind' | 'owned' | 'price' | 'average' | 'change';

  const ROW = 44;
  // Shown by the game column instead of on every row.
  const GAME_TAGS = new Set(['Unowned', 'Family shared']);
  const TAG_ORDER = [
    'Price spike',
    'Price dip',
    'Limited/Event',
    'Unobtainable',
    'Points',
    'Not marketable',
    'Unverified',
    'Unowned',
    'Family shared',
  ];
  const AVAILABILITY: { value: Availability; label: string }[] = [
    { value: 'available', label: 'Available' },
    { value: 'limited', label: 'Limited' },
    { value: 'unobtainable', label: 'Unobtainable' },
  ];

  let items = $state<ItemRow[]>([]);
  let games = $state<Map<number, GameName>>(new Map());
  let currency = $state('USD');
  let loaded = $state(false);
  let error = $state('');

  let query = $state('');
  let have = $state<Have>('all');
  let kind = $state<Group | ''>('');
  let availability = $state<Availability | ''>('');
  let gameFilter = $state<number | 0>(0);
  let tags = $state<string[]>([]);
  let minPrice = $state<number | null>(null);
  let maxPrice = $state<number | null>(null);
  let sortKey = $state<SortKey>('game');
  let descending = $state(false);
  let active = $state(-1);
  let list: { reveal: (i: number) => void; toTop: () => void } | undefined = $state();
  let tagsOpen = $state(false);

  const gameName = (appid: number) => games.get(appid)?.name ?? `App ${appid}`;
  const isOwned = (i: ItemRow) => i.have > 0 || i.held > 0;
  const isMissing = (i: ItemRow) => i.counted && i.have < i.needed;

  const counts = $derived({
    all: items.length,
    owned: items.filter(isOwned).length,
    missing: items.filter(isMissing).length,
  });

  const tagCounts = $derived.by(() => {
    const m = new Map<string, number>();
    for (const i of items) for (const t of i.tags) m.set(t, (m.get(t) ?? 0) + 1);
    return [...m.entries()].sort(
      (a, b) => rank(a[0]) - rank(b[0]) || a[0].localeCompare(b[0]),
    );
  });

  const gameOptions = $derived(
    [...new Set(items.map((i) => i.appid))]
      .map((appid) => ({ appid, name: gameName(appid) }))
      .sort((a, b) => a.name.localeCompare(b.name)),
  );

  const filtersOn = $derived(
    have !== 'all' ||
      kind !== '' ||
      availability !== '' ||
      gameFilter !== 0 ||
      tags.length > 0 ||
      minPrice !== null ||
      maxPrice !== null ||
      query.trim() !== '',
  );

  const shown = $derived.by(() => {
    const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const lo = minPrice !== null ? minPrice * 1000 : null;
    const hi = maxPrice !== null ? maxPrice * 1000 : null;
    const rows = items.filter((i) => {
      if (have === 'owned' && !isOwned(i)) return false;
      if (have === 'missing' && !isMissing(i)) return false;
      if (kind && i.group !== kind) return false;
      if (availability && i.availability !== availability) return false;
      if (gameFilter && i.appid !== gameFilter) return false;
      if (tags.length && !tags.every((t) => i.tags.includes(t))) return false;
      if (lo !== null && (i.price === null || i.price < lo)) return false;
      if (hi !== null && (i.price === null || i.price > hi)) return false;
      if (words.length) {
        const text = `${i.name} ${gameName(i.appid)} ${i.kind}`.toLowerCase();
        if (!words.every((w) => text.includes(w))) return false;
      }
      return true;
    });
    const groupRank = (g: Group) => GROUP_ORDER.indexOf(g);
    const byName = (a: ItemRow, b: ItemRow) => a.name.localeCompare(b.name);
    const byGame = (a: ItemRow, b: ItemRow) =>
      gameName(a.appid).localeCompare(gameName(b.appid)) || a.appid - b.appid;
    const byKind = (a: ItemRow, b: ItemRow) => groupRank(a.group) - groupRank(b.group) || a.kind.localeCompare(b.kind);
    // Missing numbers sort last whichever way the list is sorted.
    const numeric = (f: (i: ItemRow) => number | null) => (a: ItemRow, b: ItemRow) => {
      const x = f(a);
      const y = f(b);
      if (x === null && y === null) return 0;
      if (x === null) return 1;
      if (y === null) return -1;
      return descending ? y - x : x - y;
    };
    const flip = (n: number) => (descending ? -n : n);
    let cmp: (a: ItemRow, b: ItemRow) => number;
    switch (sortKey) {
      case 'name':
        cmp = (a, b) => flip(byName(a, b)) || byGame(a, b);
        break;
      case 'kind':
        cmp = (a, b) => flip(byKind(a, b)) || byGame(a, b) || byName(a, b);
        break;
      case 'owned':
        cmp = (a, b) => flip(a.have / a.needed - b.have / b.needed || a.held - b.held) || byGame(a, b) || byName(a, b);
        break;
      case 'price':
        cmp = (a, b) => numeric((i) => i.price)(a, b) || byGame(a, b) || byName(a, b);
        break;
      case 'average':
        cmp = (a, b) => numeric((i) => i.average)(a, b) || byGame(a, b) || byName(a, b);
        break;
      case 'change':
        cmp = (a, b) => numeric((i) => i.change)(a, b) || byGame(a, b) || byName(a, b);
        break;
      default:
        cmp = (a, b) => flip(byGame(a, b)) || byKind(a, b) || byName(a, b);
    }
    return rows.sort(cmp);
  });

  function rank(tag: string): number {
    const i = TAG_ORDER.indexOf(tag);
    return i < 0 ? TAG_ORDER.length : i;
  }

  // A new filter or sort starts back at the top.
  $effect(() => {
    void shown;
    active = -1;
    list?.toTop();
  });

  onMount(() => {
    let unlisten: (() => void) | undefined;
    let gone = false;
    on('scan-summary', () => load()).then((stop) => {
      if (gone) stop();
      else unlisten = stop;
    });
    return () => {
      gone = true;
      unlisten?.();
    };
  });

  // The dashboard can open the list with a filter and sort already set.
  $effect(() => {
    const preset = nav.itemsPreset;
    if (!preset) return;
    clearFilters();
    if (preset.have) have = preset.have;
    if (preset.sort) {
      sortKey = preset.sort;
      descending = preset.descending ?? false;
    }
    nav.itemsPreset = null;
  });

  // Loads on first show, and again when settings or saved data change.
  $effect(() => {
    void dataKey;
    void nav.dataVersion;
    load();
  });

  async function load() {
    try {
      const v = await api.items();
      items = v.items;
      games = new Map(v.games.map((g) => [g.appid, g]));
      currency = v.currency;
      error = '';
    } catch (e) {
      error = errorText(e);
    } finally {
      loaded = true;
    }
  }

  function sortBy(key: SortKey) {
    if (sortKey === key) {
      descending = !descending;
    } else {
      sortKey = key;
      descending = key === 'price' || key === 'average' || key === 'change' || key === 'owned';
    }
  }

  function ariaSort(key: SortKey): 'ascending' | 'descending' | 'none' {
    if (sortKey !== key) return 'none';
    return descending ? 'descending' : 'ascending';
  }

  function toggleTag(tag: string) {
    tags = tags.includes(tag) ? tags.filter((t) => t !== tag) : [...tags, tag];
  }

  function clearFilters() {
    query = '';
    have = 'all';
    kind = '';
    availability = '';
    gameFilter = 0;
    tags = [];
    minPrice = null;
    maxPrice = null;
  }

  function priceInput(e: Event): number | null {
    const v = (e.currentTarget as HTMLInputElement).value.trim();
    if (v === '') return null;
    const n = Number(v);
    return Number.isFinite(n) && n >= 0 ? n : null;
  }

  async function move(to: number) {
    if (!shown.length) return;
    active = Math.max(0, Math.min(shown.length - 1, to));
    // With an item's panel open, the panel follows along.
    if (nav.item) openItem(shown[active].key);
    await tick();
    list?.reveal(active);
  }

  // Arrow keys move through the list; Enter opens the item. They work from
  // the search box too, so you can type and then pick, and while an item's
  // panel is open, so you can step through items.
  function onkeydown(e: KeyboardEvent) {
    if (nav.view !== 'items' || nav.searchOpen || nav.game) return;
    const inSearch = (e.target as HTMLElement | null)?.classList.contains('items-search');
    if (typing(e) && !inSearch) return;
    const page = Math.max(1, Math.floor(600 / ROW));
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        move(active + 1);
        break;
      case 'ArrowUp':
        e.preventDefault();
        move(active - 1);
        break;
      case 'PageDown':
        if (inSearch) return;
        e.preventDefault();
        move(active + page);
        break;
      case 'PageUp':
        if (inSearch) return;
        e.preventDefault();
        move(active - page);
        break;
      case 'Home':
        if (inSearch) return;
        e.preventDefault();
        move(0);
        break;
      case 'End':
        if (inSearch) return;
        e.preventDefault();
        move(shown.length - 1);
        break;
      case 'Enter':
        if (!nav.item && active >= 0 && shown[active]) {
          e.preventDefault();
          openItem(shown[active].key);
        }
        break;
    }
  }

  function openRow(i: ItemRow, index: number) {
    active = index;
    openItem(i.key);
  }
</script>

<svelte:window {onkeydown} />

<div class="page wide-page items-page">
  <header class="page-head">
    <div>
      <h1>Items</h1>
      <p class="meta">
        {#if items.length}
          {num(items.length)} items across {num(gameOptions.length)} games.
          {#if filtersOn}Showing {num(shown.length)}.{/if}
        {:else}
          Every item of every game in your list.
        {/if}
      </p>
    </div>
    <input
      class="search items-search"
      type="search"
      bind:value={query}
      placeholder="Search items or games"
      spellcheck="false"
      autocomplete="off"
      aria-label="Search items"
    />
  </header>

  <div class="item-filters">
    <div class="segmented" role="radiogroup" aria-label="Show">
      {#each [['all', 'All'], ['owned', 'Owned'], ['missing', 'Missing']] as [value, label] (value)}
        <button
          role="radio"
          aria-checked={have === value}
          onclick={() => (have = value as Have)}
          title={value === 'missing' ? 'Items you still need, including cards that need more copies' : undefined}
        >
          {label} <span class="count">{num(counts[value as Have])}</span>
        </button>
      {/each}
    </div>

    <select class="filter-select" bind:value={kind} aria-label="Kind">
      <option value="">Every kind</option>
      {#each GROUP_ORDER as g (g)}
        <option value={g}>{GROUP_LABELS[g]}</option>
      {/each}
    </select>

    <select class="filter-select" bind:value={availability} aria-label="Availability">
      <option value="">Any availability</option>
      {#each AVAILABILITY as a (a.value)}
        <option value={a.value}>{a.label}</option>
      {/each}
    </select>

    <select class="filter-select game-select" bind:value={gameFilter} aria-label="Game">
      <option value={0}>Every game</option>
      {#each gameOptions as g (g.appid)}
        <option value={g.appid}>{g.name}</option>
      {/each}
    </select>

    <div class="popover-wrap">
      <button
        class="filter-select tag-button"
        class:on={tags.length > 0}
        aria-expanded={tagsOpen}
        onclick={() => (tagsOpen = !tagsOpen)}
      >
        {tags.length ? `Tags: ${tags.length}` : 'Any tags'}
        <Icon name="chevron" size={14} />
      </button>
      {#if tagsOpen}
        <div class="scrim clear" onclick={() => (tagsOpen = false)} role="presentation"></div>
        <div class="popover" role="group" aria-label="Tags">
          {#each tagCounts as [tag, n] (tag)}
            <label class="check-row">
              <input type="checkbox" checked={tags.includes(tag)} onchange={() => toggleTag(tag)} />
              <span>{tag}</span>
              <span class="count">{num(n)}</span>
            </label>
          {:else}
            <p class="muted">No tags yet.</p>
          {/each}
          {#if tags.length}
            <button class="btn small ghost" onclick={() => (tags = [])}>Clear tags</button>
          {/if}
        </div>
      {/if}
    </div>

    <div class="price-range" title="Today's cheapest listing, in {currency}">
      <input
        type="number"
        min="0"
        step="0.01"
        placeholder="Min price"
        aria-label="Lowest price"
        value={minPrice ?? ''}
        oninput={(e) => (minPrice = priceInput(e))}
      />
      <span class="muted">to</span>
      <input
        type="number"
        min="0"
        step="0.01"
        placeholder="Max price"
        aria-label="Highest price"
        value={maxPrice ?? ''}
        oninput={(e) => (maxPrice = priceInput(e))}
      />
    </div>

    {#if filtersOn}
      <button class="btn small ghost" onclick={clearFilters}>Clear filters</button>
    {/if}
  </div>

  {#if error}
    <p class="field-error" role="alert">{error}</p>
  {/if}

  {#if loaded && !items.length && !error}
    <div class="empty">
      <h2>No items yet</h2>
      <p>Scan your collection from the Overview, and every item of every game in your list appears here.</p>
    </div>
  {:else if items.length}
    <div class="item-table" role="table" aria-label="Items" aria-rowcount={shown.length}>
      <div class="item-row item-head" role="row">
        <span role="columnheader"></span>
        <span role="columnheader" aria-sort={ariaSort('name')}><button onclick={() => sortBy('name')}>Item</button></span>
        <span role="columnheader" aria-sort={ariaSort('game')}><button onclick={() => sortBy('game')}>Game</button></span>
        <span role="columnheader" aria-sort={ariaSort('kind')}><button onclick={() => sortBy('kind')}>Kind</button></span>
        <span role="columnheader" class="num" aria-sort={ariaSort('owned')}>
          <button onclick={() => sortBy('owned')} title="Copies' worth you have. Cards need five, counting your badge level.">Owned</button>
        </span>
        <span role="columnheader" class="num" aria-sort={ariaSort('price')}>
          <button onclick={() => sortBy('price')} title="Today's cheapest listing on the Market">Price</button>
        </span>
        <span role="columnheader" class="num" aria-sort={ariaSort(sortKey === 'change' ? 'change' : 'average')}>
          <button
            onclick={() => sortBy(sortKey === 'average' ? 'change' : 'average')}
            title="Average sale price over the span chosen in Settings. Click again to sort by how far today's price is from it."
          >
            {sortKey === 'change' ? 'Change' : 'Average'}
          </button>
        </span>
        <span role="columnheader">Tags</span>
      </div>
      <VirtualList bind:this={list} items={shown} rowHeight={ROW} key={(i) => i.key} role="rowgroup">
        {#snippet row(i: ItemRow, index: number)}
          {@const done = i.have >= i.needed}
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_interactive_supports_focus -->
          <div
            class="item-row"
            class:active={index === active}
            class:out={!i.counted}
            role="row"
            tabindex="-1"
            onclick={() => openRow(i, index)}
            onkeydown={(e) => e.key === 'Enter' && openRow(i, index)}
          >
            <span class="thumb small" class:foil-edge={i.foil} class:missing={!done && i.held === 0}>
              {#if i.image}<img src={i.image} alt="" loading="lazy" />{/if}
            </span>
            <span class="cell-name" title={i.name}>{i.name}</span>
            <span class="cell-game">
              <button
                class="link-button"
                title="Open {gameName(i.appid)}"
                onclick={(e) => {
                  e.stopPropagation();
                  openGame(i.appid);
                }}>{gameName(i.appid)}</button
              >
            </span>
            <span class="cell-kind">{i.kind}</span>
            <span class="num cell-owned" class:done>
              {#if done}<Icon name="check" size={13} />{/if}{holding(i) || '–'}
            </span>
            <span class="num">
              {#if i.on_market}
                {money(i.price, currency)}
              {:else if i.points_cost !== null}
                <span class="muted">{num(i.points_cost)} pts</span>
              {:else}
                <span class="muted">–</span>
              {/if}
            </span>
            <span class="num">
              {#if i.average !== null}
                {money(i.average, currency)}{#if i.change !== null && Math.round(i.change) !== 0}<span
                    class="chg"
                    class:up={i.change > 0}
                    class:down={i.change < 0}>{change(i.change)}</span
                  >{/if}
              {:else}
                <span class="muted">–</span>
              {/if}
            </span>
            <span class="cell-tags">
              {#each i.tags.filter((t) => !GAME_TAGS.has(t)) as t (t)}
                <span class="tag" class:spike={t === 'Price spike'} class:dip={t === 'Price dip'}>{t}</span>
              {/each}
            </span>
          </div>
        {/snippet}
        {#snippet empty()}
          <div class="list-empty">
            <p>No items match.</p>
            <button class="btn small" onclick={clearFilters}>Clear filters</button>
          </div>
        {/snippet}
      </VirtualList>
    </div>
  {/if}
</div>
