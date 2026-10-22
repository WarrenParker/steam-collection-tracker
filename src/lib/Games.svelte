<script lang="ts">
  import { onMount } from 'svelte';
  import { api, errorText, links, on, type GameRow, type ScanStatus } from './api';
  import { num } from './format';

  let { steamId }: { steamId: string | null } = $props();

  type Filter = 'all' | 'owned' | 'unowned' | 'family' | 'cards';
  type SortKey = 'name' | 'cards' | 'items' | 'yours' | 'badge';

  const filters: { value: Filter; label: string }[] = [
    { value: 'all', label: 'All' },
    { value: 'owned', label: 'Owned' },
    { value: 'unowned', label: 'Unowned' },
    { value: 'family', label: 'Family' },
    { value: 'cards', label: 'With cards' },
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
    cards: games.filter((g) => g.has_cards === true).length,
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
        case 'cards':
          return g.has_cards === true;
        default:
          return true;
      }
    });
    const value = (g: GameRow): number => {
      switch (sortKey) {
        case 'cards':
          return g.cards;
        case 'items':
          return g.items;
        case 'yours':
          return g.your_items;
        case 'badge':
          return (g.badge_level ?? -1) + (g.foil_badge ? 0.5 : 0);
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

  function openGame(g: GameRow) {
    const url = g.has_cards && steamId ? links.cards(steamId, g.appid) : links.store(g.appid);
    api.openLink(url).catch(() => {});
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
          <th class="num" aria-sort={ariaSort('cards')}>
            <button onclick={() => sortBy('cards')}>Cards</button>
          </th>
          <th class="num" aria-sort={ariaSort('items')}>
            <button onclick={() => sortBy('items')} title="Every item that exists for the game">Items</button>
          </th>
          <th class="num" aria-sort={ariaSort('yours')}>
            <button onclick={() => sortBy('yours')} title="Items for this game in your inventory">Yours</button>
          </th>
          <th class="num" aria-sort={ariaSort('badge')}>
            <button onclick={() => sortBy('badge')}>Badge</button>
          </th>
        </tr>
      </thead>
      <tbody>
        {#each shown as g (g.appid)}
          <tr>
            <td>
              <button class="game-name" onclick={() => openGame(g)} title="Open on Steam">{g.name}</button>
              {#if !g.owned && g.family}<span class="tag">Family</span>{/if}
              {#if !g.owned && !g.family}<span class="tag">{g.event ? 'Event' : 'Unowned'}</span>{/if}
              {#if g.has_cards && !g.cataloged}<span class="tag quiet" title="Its item list is read in the next scan">Not scanned</span>{/if}
            </td>
            <td class="num">{g.cards ? num(g.cards) : g.has_cards === false ? '–' : ''}</td>
            <td class="num">{g.items ? num(g.items) : ''}</td>
            <td class="num">{g.your_items ? num(g.your_items) : ''}</td>
            <td class="num">
              {#if g.badge_level}Level {g.badge_level}{/if}
              {#if g.foil_badge}<span class="foil-dot" title="Foil badge"></span>{/if}
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
