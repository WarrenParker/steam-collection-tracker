<script lang="ts">
  import { onMount } from 'svelte';
  import { api, errorText, links, type ItemView, type PriceHistory } from './api';
  import { agoUnix, change, day, money, num, points } from './format';

  let {
    item,
    currency,
    onupdated,
    marketLink = true,
  }: {
    item: ItemView;
    currency: string;
    onupdated: () => void;
    /** False when the page around the panel already links to the Market. */
    marketLink?: boolean;
  } = $props();

  const p = $derived(item.price);
  let history = $state<PriceHistory | null>(null);
  let loading = $state(false);
  let error = $state('');

  onMount(() => {
    if (item.price) load(false);
  });

  async function load(full: boolean) {
    if (!item.price) return;
    const firstRead = !item.price.history_at;
    loading = true;
    error = '';
    try {
      history = await api.priceHistory(item.price.hash_name, full);
      // A fresh read updates the item's figures too.
      if (full || firstRead) onupdated();
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  const W = 600;
  const H = 110;
  const PAD = 6;
  const chart = $derived.by(() => {
    const pts = history?.points ?? [];
    if (pts.length < 2) return null;
    const xs = pts.map((q) => q.t);
    const ys = pts.map((q) => q.price);
    const x0 = Math.min(...xs);
    const x1 = Math.max(...xs);
    const y0 = Math.min(...ys);
    const y1 = Math.max(...ys);
    const sx = (t: number) => PAD + ((t - x0) / Math.max(1, x1 - x0)) * (W - 2 * PAD);
    const sy = (v: number) => H - PAD - ((v - y0) / Math.max(1, y1 - y0)) * (H - 2 * PAD);
    const line = pts.map((q) => `${sx(q.t).toFixed(1)},${sy(q.price).toFixed(1)}`).join(' ');
    return { line, area: `${PAD},${H - PAD} ${line} ${W - PAD},${H - PAD}`, lo: y0, hi: y1, from: x0, to: x1 };
  });

  const copies = $derived(Math.max(0, item.needed - item.have));
</script>

<div class="price-panel">
  {#if p}
    <div class="price-grid">
      <dl class="facts price-facts">
        <dt>Cheapest listing</dt>
        <dd>
          {#if p.on_market}
            {money(p.lowest, currency)}<span class="muted"> · {num(p.listings)} listed</span>
          {:else}
            Not for sale right now
          {/if}
        </dd>
        {#if p.buy_max !== null}
          <dt title="The highest price someone has offered to pay">Best offer to buy</dt>
          <dd>{money(p.buy_max, currency)}</dd>
        {/if}
        <dt title="The average sale price over the span chosen in Settings">Average</dt>
        <dd>
          {money(p.average, currency)}
          {#if p.change !== null}<span class="chg" class:up={p.change > 0} class:down={p.change < 0}>{change(p.change)} today</span>{/if}
        </dd>
        <dt>Lowest ever</dt>
        <dd>{money(p.low, currency)}{#if p.low_at}<span class="muted"> · {day(p.low_at)}</span>{/if}</dd>
        <dt>Highest ever</dt>
        <dd>{money(p.high, currency)}{#if p.high_at}<span class="muted"> · {day(p.high_at)}</span>{/if}</dd>
        <dt>Sold in the last 30 days</dt>
        <dd>{p.sales_30 !== null ? num(p.sales_30) : '–'}</dd>
      </dl>
      <dl class="facts price-facts">
        <dt>Average, 30 days</dt>
        <dd>{money(p.avg_30, currency)}</dd>
        <dt>Average, 90 days</dt>
        <dd>{money(p.avg_90, currency)}</dd>
        <dt>Average, 1 year</dt>
        <dd>{money(p.avg_365, currency)}</dd>
        <dt>Average, all time</dt>
        <dd>{money(p.avg_all, currency)}</dd>
        {#if item.cost !== null}
          <dt>To finish</dt>
          <dd>
            {money(item.cost, currency)}{copies > 1 ? ` for ${copies} copies` : ''}{#if item.cost_capped}<span
                class="muted"
                title="Listed at more than three times its usual price, so it's counted at its 90-day average sale price"
              > · estimated: listed far above its usual price</span
              >{:else if item.cost_estimated}<span
                class="muted"
                title="Priced at today's cheapest listing until the order book is read"> · estimated</span
              >{/if}
          </dd>
        {/if}
        {#if item.alt_points !== null}
          <dt>Or in the Points Shop</dt>
          <dd>{points(item.alt_points)}</dd>
        {/if}
        {#if item.value !== null}
          <dt title="Copies you could sell, at today's price, before Steam's fees">
            Your {item.sellable === 1 ? 'copy' : `${num(item.sellable)} copies`}
          </dt>
          <dd>{money(item.value, currency)}</dd>
        {/if}
      </dl>
    </div>

    {#if chart}
      <figure class="price-chart">
        <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" role="img" aria-label="Price history">
          <polygon points={chart.area} class="area" />
          <polyline points={chart.line} class="line" />
        </svg>
        <figcaption>
          <span>{day(chart.from)}</span>
          <span>{money(chart.lo, currency)} to {money(chart.hi, currency)}</span>
          <span>{day(chart.to)}</span>
        </figcaption>
      </figure>
    {:else if loading}
      <p class="muted price-wait">Reading the price history…</p>
    {:else if history}
      <p class="muted price-wait">Not enough sales for a chart yet.</p>
    {/if}
    {#if error}
      {#if p.history_at}
        <p class="field-error">{error}</p>
      {:else}
        <p class="muted price-wait">No price history yet. The scan reads it, a few items at a time. {error}</p>
      {/if}
    {/if}

    <div class="actions price-actions">
      <button class="btn small" onclick={() => load(true)} disabled={loading || history?.full}>
        {history?.full ? 'Showing the full history' : loading ? 'Reading…' : 'Show full history'}
      </button>
      {#if marketLink}
        <button class="btn small ghost" onclick={() => api.openLink(links.market(p.hash_name)).catch(() => {})}>
          Market page
        </button>
      {/if}
      <span class="muted price-when">
        {history?.full ? 'Every sale since the first' : 'The last year'}{p.history_at
          ? ` · history read ${agoUnix(p.history_at)}`
          : ''} · prices {agoUnix(p.seen_at)}
      </span>
    </div>
  {:else}
    <p class="muted">
      {#if item.cost_points !== null}
        Not on the Market. {points(item.cost_points)} in the Points Shop.
      {:else if item.points_cost !== null}
        Not on the Market. Sold for {points(item.points_cost)} in the Points Shop.
      {:else}
        Not on the Market.
      {/if}
    </p>
  {/if}
</div>
