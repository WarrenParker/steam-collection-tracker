<script lang="ts">
  import { api, errorText, links, type Availability, type ItemDetail } from './api';
  import { holding, num, points } from './format';
  import Icon from './Icon.svelte';
  import PricePanel from './PricePanel.svelte';
  import { dataChanged, nav, openGame, typing } from './nav.svelte.ts';

  let { itemKey, onclose }: { itemKey: string; onclose: () => void } = $props();

  const AVAILABILITY: { value: Availability; label: string }[] = [
    { value: 'available', label: 'Available' },
    { value: 'limited', label: 'Limited' },
    { value: 'unobtainable', label: 'Unobtainable' },
  ];

  let detail = $state<ItemDetail | null>(null);
  let error = $state('');
  let saving = $state(false);
  let copied = $state(false);
  let copyTimer: ReturnType<typeof setTimeout> | undefined;

  const item = $derived(detail?.item ?? null);
  const done = $derived(!!item && item.have >= item.needed);

  // A new key (arrow keys in the list) loads that item.
  $effect(() => {
    load(itemKey);
  });

  async function load(key: string) {
    try {
      const d = await api.item(key);
      if (key === itemKey) {
        detail = d;
        error = '';
      }
    } catch (e) {
      if (key === itemKey) error = errorText(e);
    }
  }

  async function setAvailability(value: string) {
    if (!item) return;
    saving = true;
    try {
      await api.setItemAvailability(item.appid, item.key, value === 'auto' ? null : (value as Availability));
      await load(item.key);
      dataChanged();
    } catch (e) {
      error = errorText(e);
    } finally {
      saving = false;
    }
  }

  function market() {
    if (item?.price) api.openLink(links.market(item.price.hash_name)).catch(() => {});
  }

  function pointsShop() {
    if (item && item.points_cost !== null) api.openLink(links.pointsShop(item.appid)).catch(() => {});
  }

  async function copyName() {
    if (!item) return;
    try {
      await navigator.clipboard.writeText(item.name);
      copied = true;
      clearTimeout(copyTimer);
      copyTimer = setTimeout(() => (copied = false), 1500);
    } catch {
      error = "The name couldn't be copied.";
    }
  }

  function availabilityLabel(a: Availability): string {
    return AVAILABILITY.find((x) => x.value === a)?.label ?? a;
  }

  // Quick actions: M for the Market, P for the Points Shop, C to copy the name.
  function onkeydown(e: KeyboardEvent) {
    if (nav.searchOpen || typing(e) || e.ctrlKey || e.metaKey || e.altKey) return;
    const k = e.key.toLowerCase();
    if (k === 'm') market();
    else if (k === 'p') pointsShop();
    else if (k === 'c') copyName();
  }
</script>

<svelte:window {onkeydown} />

<div class="scrim upper" onclick={onclose} role="presentation"></div>
<aside class="drawer upper item-drawer" role="dialog" aria-modal="true" aria-label={item?.name ?? 'Item'}>
  <header class="drawer-head">
    <h2 class="detail-title">{item?.name ?? ''}</h2>
    <button class="btn ghost icon-btn" onclick={onclose} aria-label="Close" title="Close (Esc)">
      <Icon name="x" size={18} />
    </button>
  </header>

  <div class="drawer-body">
    {#if error}<p class="field-error" role="alert">{error}</p>{/if}

    {#if !item && !error}
      <p class="desc detail-loading">Loading…</p>
    {:else if item && detail}
      <section class="item-hero">
        <div class="hero-art" class:foil-edge={item.foil} class:missing={!done && item.held === 0}>
          {#if item.image}<img src={item.image} alt="" />{/if}
        </div>
        <div class="hero-text">
          <div class="hero-kind">{item.kind}{item.foil && !item.kind.toLowerCase().includes('foil') ? ' · Foil' : ''}</div>
          <button class="link-button hero-game" onclick={() => openGame(item!.appid)} title="Open this game">
            {detail.game_name ?? `App ${item.appid}`}
          </button>
          <div class="hero-status" class:done>
            {#if done}<Icon name="check" size={14} />{/if}
            {#if done}
              {holding(item) || 'Complete'}
            {:else if item.counted}
              {item.have > 0 ? `${holding(item)}, still needed` : 'Still needed'}
            {:else}
              {holding(item) || 'Not owned'}<span class="muted"> · not counted</span>
            {/if}
          </div>
          {#if item.tags.length}
            <div class="hero-tags">
              {#each item.tags as t (t)}
                <span class="tag" class:spike={t === 'Price spike'} class:dip={t === 'Price dip'}>{t}</span>
              {/each}
            </div>
          {/if}
        </div>
      </section>

      <div class="actions item-actions">
        {#if item.price}
          <button class="btn small" onclick={market} title="Open its Market page (M)">
            <Icon name="external" size={14} /> Market
          </button>
        {/if}
        {#if item.points_cost !== null}
          <button class="btn small" onclick={pointsShop} title="Open this game's Points Shop (P)">
            <Icon name="external" size={14} /> Points Shop · {num(item.points_cost)}
          </button>
        {/if}
        <button class="btn small ghost" onclick={copyName} title="Copy the item's name (C)">
          <Icon name={copied ? 'check' : 'copy'} size={14} />
          {copied ? 'Copied' : 'Copy name'}
        </button>
      </div>

      <dl class="facts item-facts">
        {#if item.group === 'cards'}
          <dt>Copies</dt>
          <dd title="Each card needs five copies' worth. Your badge level and the copies you hold count toward it.">
            {num(item.held)} held · {item.have} of {item.needed} counted
          </dd>
        {:else if item.held > 1}
          <dt>Copies</dt>
          <dd>{num(item.held)} held</dd>
        {/if}
        {#if item.group !== 'boosters'}
          <dt>Availability</dt>
          <dd>
            <select
              class="mini-select"
              aria-label="Availability"
              title={item.overridden ? 'Set by you' : 'Worked out by the app'}
              value={item.overridden ? item.availability : 'auto'}
              disabled={saving}
              onchange={(e) => setAvailability((e.currentTarget as HTMLSelectElement).value)}
            >
              <option value="auto">Auto: {availabilityLabel(item.auto_availability)}</option>
              {#each AVAILABILITY as a (a.value)}
                <option value={a.value}>{a.label}</option>
              {/each}
            </select>
          </dd>
        {/if}
        {#if item.points_cost !== null}
          <dt>Points Shop</dt>
          <dd>{points(item.points_cost)}</dd>
        {/if}
        <dt>Counts toward completion</dt>
        <dd>{item.counted ? 'Yes' : 'No'}</dd>
      </dl>

      {#if item.note}
        <p class="desc detail-note">{item.note}</p>
      {/if}

      <section class="item-price">
        <h3>Price</h3>
        {#key item.key}
          <PricePanel {item} currency={detail.currency} marketLink={false} onupdated={() => load(item!.key)} />
        {/key}
      </section>
    {/if}
  </div>
</aside>
