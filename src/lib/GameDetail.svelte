<script lang="ts">
  import { onMount } from 'svelte';
  import {
    api,
    errorText,
    links,
    on,
    type Availability,
    type BookProgress,
    type GameDetail,
    type GameRow,
    type Group,
    type ItemView,
  } from './api';
  import { change, holding, money, num, owned, pct, points } from './format';
  import Icon from './Icon.svelte';
  import { nav, openItem } from './nav.svelte.ts';

  let {
    game,
    steamId,
    onclose,
    onchange,
  }: {
    game: GameRow;
    steamId: string | null;
    onclose: () => void;
    onchange: () => void;
  } = $props();

  const ORDER: Group[] = ['cards', 'foil_cards', 'emoticons', 'backgrounds', 'points_shop', 'boosters'];
  const AVAILABILITY: { value: Availability; label: string }[] = [
    { value: 'available', label: 'Available' },
    { value: 'limited', label: 'Limited' },
    { value: 'unobtainable', label: 'Unobtainable' },
  ];
  // Game-level tags are shown once in the header, not on every item.
  const GAME_TAGS = new Set(['Unowned', 'Family shared']);

  let detail = $state<GameDetail | null>(null);
  let error = $state('');
  let saving = $state<string | null>(null);
  // Items as a list with prices, or as a grid of their art.
  let layout = $state<'list' | 'grid'>(savedLayout());
  // Reading the Market's listings for cards that need several copies.
  let books = $state<BookProgress | null>(null);
  let booksNote = $state('');
  let booksRead = false;

  const done = $derived(detail?.completion ?? null);
  const cur = $derived(detail?.currency ?? 'USD');
  const prices = $derived(done?.prices ?? null);
  const sections = $derived.by(() => {
    if (!detail) return [];
    return ORDER.map((group) => ({
      group,
      label: detail!.completion?.groups.find((g) => g.group === group)?.label ?? labelFor(group),
      items: detail!.items.filter((i) => i.group === group),
    })).filter((s) => s.items.length > 0);
  });

  function labelFor(group: Group): string {
    return (
      {
        cards: 'Cards',
        foil_cards: 'Foil cards',
        emoticons: 'Emoticons',
        backgrounds: 'Backgrounds',
        points_shop: 'Points Shop items',
        boosters: 'Booster packs',
      } as const
    )[group];
  }

  // Read again when something it shows was saved elsewhere, like an item's availability.
  let seenVersion = nav.dataVersion;
  $effect(() => {
    if (nav.dataVersion !== seenVersion) {
      seenVersion = nav.dataVersion;
      load();
    }
  });

  onMount(() => {
    let unlisten: (() => void) | undefined;
    let gone = false;
    (async () => {
      const stop = await on<BookProgress>('book-progress', (p) => {
        if (p.appid !== game.appid) return;
        if (p.total > 0) booksRead = true;
        books = p.done < p.total ? p : null;
      });
      if (gone) stop();
      else unlisten = stop;
      await load();
      if (!gone && !error) readBooks();
    })();
    return () => {
      gone = true;
      unlisten?.();
    };
  });

  /** Reads the order books this game's cost needs, then shows the updated cost. */
  async function readBooks() {
    let read = false;
    try {
      const updated = await api.readGameBooks(game.appid);
      read = true;
      detail = updated;
    } catch (e) {
      booksNote = errorText(e);
      await load();
    } finally {
      books = null;
    }
    if (read && booksRead) onchange();
  }

  async function load() {
    try {
      detail = await api.gameDetail(game.appid);
      error = '';
    } catch (e) {
      error = errorText(e);
    }
  }

  async function setAvailability(item: ItemView, value: string) {
    saving = item.key;
    try {
      detail = await api.setItemAvailability(
        game.appid,
        item.key,
        value === 'auto' ? null : (value as Availability),
      );
      onchange();
    } catch (e) {
      error = errorText(e);
    } finally {
      saving = null;
    }
  }

  function savedLayout(): 'list' | 'grid' {
    try {
      return localStorage.getItem('game-items-layout') === 'grid' ? 'grid' : 'list';
    } catch {
      return 'list';
    }
  }

  function setLayout(l: 'list' | 'grid') {
    layout = l;
    try {
      localStorage.setItem('game-items-layout', l);
    } catch {
      // Remembering the choice is only a convenience.
    }
  }

  function statusTitle(item: ItemView): string {
    if (item.group === 'cards') {
      return `Each card needs ${item.needed} copies' worth. Your badge level and the copies you hold count toward it. You hold ${num(item.held)}.`;
    }
    if (item.group === 'foil_cards') return `The foil badge or a copy you hold counts. You hold ${num(item.held)}.`;
    return '';
  }

  function priceTitle(item: ItemView): string {
    const p = item.price;
    if (!p) return item.points_cost !== null ? 'Sold in the Points Shop' : 'Not on the Market';
    if (!p.on_market) return 'Not for sale on the Market right now';
    const avg = p.average !== null ? `, average ${money(p.average, cur)}` : '';
    return `Cheapest of ${num(p.listings)} listed${avg}.`;
  }

  function availabilityLabel(a: Availability): string {
    return AVAILABILITY.find((x) => x.value === a)?.label ?? a;
  }

</script>

<div class="scrim" onclick={onclose} role="presentation"></div>
<aside class="drawer wide" role="dialog" aria-modal="true" aria-label={game.name}>
  <header class="drawer-head">
    <h2 class="detail-title">
      {game.name}
      {#if !game.owned && game.family}<span class="tag">Family</span>{/if}
      {#if !game.owned && !game.family}<span class="tag">{game.event ? 'Event' : 'Unowned'}</span>{/if}
    </h2>
    <button class="btn ghost icon-btn" onclick={onclose} aria-label="Close">
      <Icon name="x" size={18} />
    </button>
  </header>

  <div class="drawer-body">
    {#if error}<p class="field-error" role="alert">{error}</p>{/if}

    {#if !detail && !error}
      <p class="desc detail-loading">Loading items…</p>
    {:else if detail}
      <section class="detail-summary">
        <div class="detail-figure">
          <div class="figure-value" class:foil={done?.percent === 100}>{pct(done?.percent)}</div>
          <div class="figure-label">
            {#if done && done.total > 0}
              {num(done.complete)} of {num(done.total)} items complete
            {:else}
              Nothing to collect yet
            {/if}
          </div>
        </div>
        <dl class="facts detail-facts">
          {#if done?.has_cards}
            <dt>Badge</dt>
            <dd>
              {done.badge_level ? `Level ${done.badge_level}` : 'None'}{#if done.foil_badge}<span
                  class="foil-dot"
                  title="Foil badge"
                ></span>{/if}
            </dd>
            <dt title="Level 5 badge and the foil badge">Card set</dt>
            <dd>{done.cards_complete ? 'Complete' : 'Not yet'}</dd>
          {/if}
          {#if prices && prices.value_copies > 0}
            <dt title="Copies you could sell, at today's prices, before Steam's fees">Your items' value</dt>
            <dd>{money(prices.value, cur)}</dd>
          {/if}
          {#if prices && (prices.cost_items > 0 || prices.cost_points_items > 0)}
            <dt title="Everything still needed: Market items in money, Points Shop-only items in points">Cost to complete</dt>
            <dd>
              {#if prices.cost_items > 0}{money(prices.cost, cur)}{/if}{#if prices.cost_items > 0 && prices.cost_points > 0}
                +
              {/if}{#if prices.cost_points > 0}{points(prices.cost_points)}{/if}
            </dd>
          {/if}
          {#if game.ach_total}
            <dt>Achievements</dt>
            <dd>{num(game.ach_unlocked ?? 0)} of {num(game.ach_total)}</dd>
          {/if}
          {#if done && !done.counts_for_account}
            <dt>Account figure</dt>
            <dd title="Only games you own, and your Steam Family's when that's on, count toward the account-wide figure">
              Not counted
            </dd>
          {/if}
        </dl>
      </section>

      {#if done && done.groups.length}
        <ul class="typelist detail-groups">
          {#each done.groups as g (g.group)}
            <li>
              <span class="type-name">{g.label}{#if g.group === 'boosters'}<span class="muted"> · not counted</span>{/if}</span>
              <span class="bar"><span style:width="{g.total ? (g.owned / g.total) * 100 : 0}%"></span></span>
              <span class="n">{owned(g.owned)} / {num(g.total)}</span>
            </li>
          {/each}
        </ul>
      {/if}

      {#if books}
        <p class="desc detail-note" aria-live="polite">
          Reading the Market's listings for this game's cards: {num(books.done)} of {num(books.total)}…
        </p>
      {:else if booksNote}
        <p class="desc detail-note">{booksNote}</p>
      {/if}

      {#if prices && prices.alt_items > 0}
        <p class="desc detail-note">
          Or buy {num(prices.alt_items)} of the Market items in the Points Shop for {points(prices.alt_points)} instead
          of {money(prices.alt_money, cur)}.
        </p>
      {/if}

      {#if done && done.unobtainable > 0}
        <p class="desc detail-note">
          {num(done.unobtainable)}
          {done.unobtainable === 1 ? 'item' : 'items'} can't be had any more and {done.unobtainable === 1
            ? "isn't"
            : "aren't"} counted{done.unobtainable_owned ? `. You own ${num(done.unobtainable_owned)}` : ''}.
        </p>
      {/if}

      <div class="actions detail-links">
        {#if game.has_cards && steamId}
          <button class="btn small" onclick={() => api.openLink(links.cards(steamId!, game.appid)).catch(() => {})}>
            Card page on Steam
          </button>
        {/if}
        {#if !game.event}
          <button class="btn small ghost" onclick={() => api.openLink(links.store(game.appid)).catch(() => {})}>
            Store page
          </button>
        {/if}
      </div>

      <div class="items-toolbar">
        <h3>Items</h3>
        <div class="segmented two" role="radiogroup" aria-label="Show items as">
          <button role="radio" aria-checked={layout === 'list'} onclick={() => setLayout('list')} title="A list with prices">
            <Icon name="list" size={14} /> List
          </button>
          <button role="radio" aria-checked={layout === 'grid'} onclick={() => setLayout('grid')} title="A grid of their art">
            <Icon name="grid" size={14} /> Art
          </button>
        </div>
      </div>

      {#each sections as s (s.group)}
        <section class="item-section">
          <h3>{s.label} <span class="count">{num(s.items.length)}</span></h3>
          {#if layout === 'grid'}
            <ul class="art-grid" class:wide-art={s.group === 'backgrounds'}>
              {#each s.items as item (item.key)}
                {@const complete = item.have >= item.needed}
                <li>
                  <button
                    class="art-tile"
                    class:complete
                    class:missing={!complete && item.have === 0}
                    class:partial={!complete && item.have > 0}
                    class:foil={item.foil}
                    class:out={!item.counted}
                    onclick={() => openItem(item.key)}
                    title="{item.name}{holding(item) ? ` · ${holding(item)}` : ' · Missing'}"
                  >
                    <span class="art-img">
                      {#if item.image}<img src={item.image} alt="" loading="lazy" />{/if}
                    </span>
                    {#if item.needed > 1}
                      <span class="art-count">{item.have}/{item.needed}</span>
                    {:else if complete}
                      <span class="art-count"><Icon name="check" size={12} /></span>
                    {/if}
                    <span class="art-name">{item.name}</span>
                  </button>
                </li>
              {/each}
            </ul>
          {:else}
            <ul class="items">
              {#each s.items as item (item.key)}
                <li class:has={item.have >= item.needed} class:out={!item.counted}>
                  <div class="thumb" class:foil-edge={item.foil}>
                    {#if item.image}<img src={item.image} alt="" loading="lazy" />{/if}
                  </div>
                  <div class="item-main">
                    <button class="link-button item-name" onclick={() => openItem(item.key)} title="Show this item">
                      {item.name}
                    </button>
                    <div class="item-sub">
                      {#if item.points_cost !== null}<span>{num(item.points_cost)} points</span>{/if}
                      {#each item.tags.filter((t) => !GAME_TAGS.has(t)) as t (t)}
                        <span class="tag">{t}</span>
                      {/each}
                    </div>
                    {#if item.note && (item.availability !== 'available' || item.tags.includes('Unverified') || item.overridden)}
                      <div class="item-note">{item.note}</div>
                    {/if}
                  </div>
                  <button
                    class="price-cell"
                    class:spike={item.tags.includes('Price spike')}
                    class:dip={item.tags.includes('Price dip')}
                    title={priceTitle(item)}
                    onclick={() => openItem(item.key)}
                  >
                    {#if item.price?.on_market}
                      <span>{money(item.price.lowest, cur)}</span>
                      {#if item.price.change !== null}<span class="chg">{change(item.price.change)}</span>{/if}
                    {:else if item.points_cost !== null && !item.marketable}
                      <span>{num(item.points_cost)} pts</span>
                    {:else}
                      <span class="muted">–</span>
                    {/if}
                  </button>
                  <div class="item-status" title={statusTitle(item)}>
                    {#if item.have >= item.needed}<Icon name="check" size={14} />{/if}
                    {holding(item)}
                  </div>
                  {#if item.group !== 'boosters'}
                    <select
                      class="mini-select"
                      aria-label="Availability of {item.name}"
                      title={item.overridden ? 'Set by you' : 'Worked out by the app'}
                      value={item.overridden ? item.availability : 'auto'}
                      disabled={saving === item.key}
                      onchange={(e) => setAvailability(item, (e.currentTarget as HTMLSelectElement).value)}
                    >
                      <option value="auto">Auto: {availabilityLabel(item.auto_availability)}</option>
                      {#each AVAILABILITY as a (a.value)}
                        <option value={a.value}>{a.label}</option>
                      {/each}
                    </select>
                  {:else}
                    <span></span>
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      {/each}
    {/if}
  </div>
</aside>
