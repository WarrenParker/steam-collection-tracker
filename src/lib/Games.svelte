<script lang="ts">
  import { onMount } from 'svelte';
  import { api, errorText, on, type GameRow, type ScanStatus } from './api';
  import { money, num, pct } from './format';
  import { nav } from './nav.svelte.ts';

  type Filter = 'all' | 'owned' | 'unowned' | 'family' | 'items' | 'cards' | 'complete';
  type SortKey = 'name' | 'complete' | 'cards' | 'items' | 'yours' | 'value' | 'cost' | 'badge' | 'achievements';

  const filters: { value: Filter; label: string }[] = [
    { value: 'all', label: 'All' },
    { value: 'owned', label: 'Owned' },
    { value: 'unowned', label: 'Unowned' },
    { value: 'family', label: 'Family' },
    { value: 'items', label: 'With items' },
    { value: 'cards', label: 'With cards' },
    { value: 'complete', label: 'Complete' },
  ];

  let games = $state<GameRow[]>([]);
  let loaded = $state(false);
  let error = $state('');
  let query = $state('');
  let filter = $state<Filter>('all');
  let sortKey = $state<SortKey>('name');
  let descending = $state(false);

  const counts = $derived({
    all: games.length,
    owned: games.filter((g) => g.owned).length,
    unowned: games.filter((g) => !g.owned && !g.family).length,
    family: games.filter((g) => !g.owned && g.family).length,
    items: games.filter((g) => g.items > 0).length,
    cards: games.filter((g) => g.has_cards === true).length,
    complete: games.filter((g) => g.percent === 100).length,
  });

  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = games.filter((g) => {
      if (q && !g.name.toLowerCase().includes(q) && String(g.appid) !== q) return false;
      switch (filter) {
        case 'owned':
          return g.owned;
        case 'unowned':
          return !g.owned && !g.family;
        case 'family':
          return !g.owned && g.family;
        case 'items':
          return g.items > 0;
        case 'cards':
          return g.has_cards === true;
        case 'complete':
          return g.percent === 100;
        default:
          return true;
      }
    });
    const value = (g: GameRow): number => {
      switch (sortKey) {
        case 'complete':
          return g.percent ?? -1;
        case 'cards':
          return g.cards;
        case 'items':
          return g.items;
        case 'yours':
          return g.your_items;
        case 'value':
          return g.value ?? -1;
        case 'cost':
          // By money; games that only need points sit just below every game
          // that needs money, ordered by their points.
          if (g.cost !== null && g.cost > 0) return 1 + g.cost;
          if (g.cost_points > 0) return g.cost_points / 1e9;
          return g.cost !== null ? 0 : -1;
        case 'badge':
          return (g.badge_level ?? -1) + (g.foil_badge ? 0.5 : 0);
        case 'achievements':
          return g.ach_total ? (g.ach_unlocked ?? 0) / g.ach_total : -1;
        default:
          return 0;
      }
    };
    const sorted = [...list].sort((a, b) => {
      const byName = a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });
      if (sortKey === 'name') return descending ? -byName : byName;
      const diff = value(a) - value(b);
      return (descending ? -diff : diff) || byName;
    });
    return sorted;
  });

  async function load() {
    try {
      games = await api.games();
      error = '';
    } catch (e) {
      error = errorText(e);
    } finally {
      loaded = true;
    }
  }

  onMount(() => {
    const unlisteners: Array<() => void> = [];
    load();
    let lastState = '';
    on<ScanStatus>('scan-status', (s) => {
      // Refresh when a scan stops, and now and then while it runs.
      if (s.state !== 'running' && lastState === 'running') load();
      lastState = s.state;
    }).then((u) => unlisteners.push(u));
    on('scan-summary', () => load()).then((u) => unlisteners.push(u));
    return () => unlisteners.forEach((u) => u());
  });

  function sortBy(key: SortKey) {
    if (sortKey === key) {
      descending = !descending;
    } else {
      sortKey = key;
      descending = key !== 'name';
    }
  }

  function ariaSort(key: SortKey): 'ascending' | 'descending' | 'none' {
    if (sortKey !== key) return 'none';
    return descending ? 'descending' : 'ascending';
  }

  // Completion changes when an item's availability is set by hand, or a
  // game's order books are read.
  let seenVersion = nav.dataVersion;
  $effect(() => {
    if (nav.dataVersion !== seenVersion) {
      seenVersion = nav.dataVersion;
      refresh();
    }
  });

  async function refresh() {
    await load();
    if (nav.game) nav.game = games.find((g) => g.appid === nav.game!.appid) ?? nav.game;
  }
</script>

<div class="page">
  <header class="page-head">
    <div>
      <h1>Games</h1>
      <p class="meta">
        {#if games.length}
          {num(games.length)} games: the ones you own, the ones you have items for, and your Steam Family's
          when that's turned on in Settings.
        {:else}
          Every game you own or have items for.
        {/if}
      </p>
    </div>
    <input
      class="search"
      type="search"
      bind:value={query}
      placeholder="Search games"
      spellcheck="false"
      autocomplete="off"
      aria-label="Search games"
    />
  </header>

  <div class="filters" role="radiogroup" aria-label="Show">
    {#each filters as f (f.value)}
      {#if f.value !== 'family' || counts.family > 0}
        <button role="radio" aria-checked={filter === f.value} onclick={() => (filter = f.value)}>
          {f.label} <span class="count">{num(counts[f.value])}</span>
        </button>
      {/if}
    {/each}
  </div>

  {#if error}
    <p class="field-error" role="alert">{error}</p>
  {/if}

  {#if loaded && !games.length && !error}
    <div class="empty">
      <h2>No games yet</h2>
      <p>Scan your collection from the Overview, and every game you own or have items for appears here.</p>
    </div>
  {:else if games.length}
    <table class="games">
      <thead>
        <tr>
          <th aria-sort={ariaSort('name')}>
            <button onclick={() => sortBy('name')}>Game</button>
          </th>
          <th class="num complete-col" aria-sort={ariaSort('complete')}>
            <button
              onclick={() => sortBy('complete')}
              title="How much of the game's collection you have. Items that can't be had any more aren't counted."
            >
              Complete
            </button>
          </th>
          <th class="num" aria-sort={ariaSort('cards')}>
            <button onclick={() => sortBy('cards')}>Cards</button>
          </th>
          <th class="num" aria-sort={ariaSort('items')}>
            <button onclick={() => sortBy('items')} title="Every item that exists for the game">Items</button>
          </th>
          <th class="num" aria-sort={ariaSort('yours')}>
            <button onclick={() => sortBy('yours')} title="Items for this game in your inventory">Yours</button>
          </th>
          <th class="num" aria-sort={ariaSort('value')}>
            <button onclick={() => sortBy('value')} title="What your items you could sell are worth, before Steam's fees">
              Value
            </button>
          </th>
          <th class="num" aria-sort={ariaSort('cost')}>
            <button onclick={() => sortBy('cost')} title="What the rest costs: Market items in money, Points Shop-only items in points">
              To complete
            </button>
          </th>
          <th class="num" aria-sort={ariaSort('badge')}>
            <button onclick={() => sortBy('badge')}>Badge</button>
          </th>
          <th class="num wide" aria-sort={ariaSort('achievements')}>
            <button onclick={() => sortBy('achievements')}>Achievements</button>
          </th>
        </tr>
      </thead>
      <tbody>
        {#each shown as g (g.appid)}
          <tr>
            <td>
              <button class="game-name" onclick={() => (nav.game = g)} title="Show this game's items">{g.name}</button>
              {#if !g.owned && g.family}<span class="tag">Family</span>{/if}
              {#if !g.owned && !g.family}<span class="tag">{g.event ? 'Event' : 'Unowned'}</span>{/if}
              {#if g.has_cards && !g.cataloged}<span class="tag quiet" title="Its item list is read in the next scan">Not scanned</span>{/if}
            </td>
            <td class="num complete-col">
              {#if g.percent !== null}
                <span class="complete-cell" class:full={g.percent === 100}>
                  <span class="bar"><span style:width="{g.percent}%"></span></span>
                  <span class="complete-n">{pct(g.percent)}</span>
                </span>
              {/if}
            </td>
            <td class="num">{g.cards ? num(g.cards) : g.has_cards === false ? '–' : ''}</td>
            <td class="num">{g.items ? num(g.items) : ''}</td>
            <td class="num">{g.your_items ? num(g.your_items) : ''}</td>
            <td class="num">{g.value !== null ? money(g.value, g.currency) : ''}</td>
            <td class="num cost-cell">
              {#if g.cost !== null && g.cost > 0}<span class="money">{money(g.cost, g.currency)}</span>{:else if g.cost === 0 && g.cost_points === 0}Done{/if}
              {#if g.cost_points > 0}<span class="pts">{g.cost !== null && g.cost > 0 ? '+ ' : ''}{num(g.cost_points)} pts</span>{/if}
            </td>
            <td class="num">
              {#if g.badge_level}Level {g.badge_level}{/if}
              {#if g.foil_badge}<span class="foil-dot" title="Foil badge"></span>{/if}
            </td>
            <td class="num wide">
              {#if g.ach_total}
                <span class:perfect={(g.ach_unlocked ?? 0) >= g.ach_total}>{num(g.ach_unlocked ?? 0)} / {num(g.ach_total)}</span>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
    {#if !shown.length}
      <p class="muted">No games match.</p>
    {/if}
  {/if}
</div>

