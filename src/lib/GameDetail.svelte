<script lang="ts">
  import { onMount } from 'svelte';
  import {
    api,
    errorText,
    links,
    type Availability,
    type GameDetail,
    type GameRow,
    type Group,
    type ItemView,
  } from './api';
  import { num, owned, pct } from './format';
  import Icon from './Icon.svelte';

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

  const done = $derived(detail?.completion ?? null);
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

  onMount(load);

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

  function status(item: ItemView): string {
    if (item.needed > 1) return `${item.have} of ${item.needed}`;
    if (item.held > 1) return `${num(item.held)} owned`;
    return item.held > 0 ? 'Owned' : '';
  }

  function statusTitle(item: ItemView): string {
    if (item.group === 'cards') {
      return `Each card needs ${item.needed} copies' worth. Your badge level and the copies you hold count toward it. You hold ${num(item.held)}.`;
    }
    if (item.group === 'foil_cards') return `The foil badge or a copy you hold counts. You hold ${num(item.held)}.`;
    return '';
  }

  function availabilityLabel(a: Availability): string {
    return AVAILABILITY.find((x) => x.value === a)?.label ?? a;
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') onclose();
  }
</script>

<svelte:window {onkeydown} />

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

      {#each sections as s (s.group)}
        <section class="item-section">
          <h3>{s.label} <span class="count">{num(s.items.length)}</span></h3>
          <ul class="items">
            {#each s.items as item (item.key)}
              <li class:has={item.have >= item.needed} class:out={!item.counted}>
                <div class="thumb" class:foil-edge={item.foil}>
                  {#if item.image}<img src={item.image} alt="" loading="lazy" />{/if}
                </div>
                <div class="item-main">
                  <div class="item-name">{item.name}</div>
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
                <div class="item-status" title={statusTitle(item)}>
                  {#if item.have >= item.needed}<Icon name="check" size={14} />{/if}
                  {status(item)}
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
        </section>
      {/each}
    {/if}
  </div>
</aside>
