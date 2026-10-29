<script lang="ts">
  import { api, type ProgressPoint } from './api';
  import { localDay, money, num, pct, points } from './format';

  type Metric = 'percent' | 'value' | 'cost' | 'items';
  type Range = 30 | 90 | 365 | 0;

  const METRICS: { value: Metric; label: string }[] = [
    { value: 'percent', label: 'Completion' },
    { value: 'value', label: 'Value' },
    { value: 'cost', label: 'To complete' },
    { value: 'items', label: 'Items owned' },
  ];
  const RANGES: { value: Range; label: string }[] = [
    { value: 30, label: '30 days' },
    { value: 90, label: '90 days' },
    { value: 365, label: '1 year' },
    { value: 0, label: 'All' },
  ];

  let all = $state<ProgressPoint[] | null>(null);
  let metric = $state<Metric>('percent');
  let range = $state<Range>(90);
  let hover = $state<number | null>(null);

  /** Changes whenever the dashboard reloads, for example after a scan, so the chart does too. */
  let { stamp = null }: { stamp?: unknown } = $props();

  $effect(() => {
    void stamp;
    api
      .progress()
      .then((p) => (all = p))
      .catch(() => (all = all ?? []));
  });

  /** The currency of the newest priced day; older days in another one are left out. */
  const currency = $derived([...(all ?? [])].reverse().find((p) => p.value !== null)?.currency ?? 'USD');

  function valueOf(p: ProgressPoint, m: Metric): number | null {
    switch (m) {
      case 'percent':
        return p.percent;
      case 'value':
        return p.currency === currency ? p.value : null;
      case 'cost':
        return p.currency === currency ? p.cost : null;
      case 'items':
        return p.items_owned;
    }
  }

  function label(v: number, m: Metric): string {
    if (m === 'percent') return pct(v);
    if (m === 'items') return num(v);
    return money(v, currency);
  }

  const W = 640;
  const H = 150;
  const PAD = 8;

  const chart = $derived.by(() => {
    const since = range ? Date.now() / 1000 - range * 86_400 : 0;
    const pts = (all ?? [])
      .filter((p) => p.at >= since)
      .map((p) => ({ p, v: valueOf(p, metric) }))
      .filter((q): q is { p: ProgressPoint; v: number } => q.v !== null);
    if (pts.length === 0) return null;
    const x0 = pts[0].p.at;
    const x1 = pts[pts.length - 1].p.at;
    let y0 = Math.min(...pts.map((q) => q.v));
    let y1 = Math.max(...pts.map((q) => q.v));
    if (y1 - y0 < 1e-9) {
      // A flat line sits in the middle.
      const pad = Math.max(Math.abs(y1) * 0.05, metric === 'percent' ? 0.5 : 1);
      y0 -= pad;
      y1 += pad;
    }
    const sx = (t: number) => (x1 === x0 ? W / 2 : PAD + ((t - x0) / (x1 - x0)) * (W - 2 * PAD));
    const sy = (v: number) => H - PAD - ((v - y0) / (y1 - y0)) * (H - 2 * PAD);
    const dots = pts.map((q) => ({ x: sx(q.p.at), y: sy(q.v), p: q.p, v: q.v }));
    const line = dots.map((d) => `${d.x.toFixed(1)},${d.y.toFixed(1)}`).join(' ');
    const first = pts[0];
    const last = pts[pts.length - 1];
    return {
      dots,
      line,
      area: `${dots[0].x.toFixed(1)},${H - PAD} ${line} ${dots[dots.length - 1].x.toFixed(1)},${H - PAD}`,
      first,
      last,
      lo: Math.min(...pts.map((q) => q.v)),
      hi: Math.max(...pts.map((q) => q.v)),
      delta: pts.length > 1 ? last.v - first.v : null,
    };
  });

  const shown = $derived(chart && hover !== null ? chart.dots[Math.min(hover, chart.dots.length - 1)] : null);

  function move(e: PointerEvent) {
    if (!chart) return;
    const box = (e.currentTarget as SVGElement).getBoundingClientRect();
    const x = ((e.clientX - box.left) / box.width) * W;
    let best = 0;
    chart.dots.forEach((d, i) => {
      if (Math.abs(d.x - x) < Math.abs(chart.dots[best].x - x)) best = i;
    });
    hover = best;
  }

  /** Whether more of the line is a good thing. */
  const upIsGood = $derived(metric !== 'cost');

  function deltaText(d: number): string {
    if (Math.abs(d) < (metric === 'percent' ? 0.05 : 0.5)) return 'No change';
    const sign = d > 0 ? '+' : d < 0 ? '−' : '';
    const abs = Math.abs(d);
    if (metric === 'percent') return `${sign}${(Math.round(abs * 10) / 10).toFixed(1)} points`;
    return `${sign}${label(abs, metric)}`;
  }
</script>

<section class="dash-card progress-card">
  <div class="progress-head">
    <h2>Progress over time</h2>
    <div class="progress-controls">
      <div class="segmented compact" role="radiogroup" aria-label="Figure">
        {#each METRICS as m (m.value)}
          <button role="radio" aria-checked={metric === m.value} onclick={() => (metric = m.value)}>{m.label}</button>
        {/each}
      </div>
      <div class="segmented compact" role="radiogroup" aria-label="Range">
        {#each RANGES as r (r.value)}
          <button role="radio" aria-checked={range === r.value} onclick={() => (range = r.value)}>{r.label}</button>
        {/each}
      </div>
    </div>
  </div>

  {#if all === null}
    <p class="muted">Loading…</p>
  {:else if !chart}
    <p class="muted progress-empty">
      {metric === 'items'
        ? 'Items owned are recorded from version 0.9.0 on. The chart fills in after your next scans.'
        : 'Nothing recorded for this span yet. The app records your figures once a day, after a scan.'}
    </p>
  {:else}
    <div class="progress-now">
      <span class="progress-figure">{label((shown ?? chart.dots[chart.dots.length - 1]).v, metric)}</span>
      <span class="muted">
        {#if shown}
          {localDay(shown.p.at)}
        {:else if chart.delta !== null}
          <span class:up={upIsGood ? chart.delta > 0 : chart.delta < 0} class:down={upIsGood ? chart.delta < 0 : chart.delta > 0}
            >{deltaText(chart.delta)}</span
          >
          since {localDay(chart.first.p.at)}
        {:else}
          {localDay(chart.last.p.at)} · one day recorded so far
        {/if}
        {#if metric === 'cost' && (shown ?? chart.dots[chart.dots.length - 1]).p.cost_points}
          · plus {points((shown ?? chart.dots[chart.dots.length - 1]).p.cost_points ?? 0)}
        {/if}
      </span>
    </div>
    <figure class="price-chart progress-chart">
      <svg
        viewBox="0 0 {W} {H}"
        preserveAspectRatio="none"
        role="img"
        aria-label="{METRICS.find((m) => m.value === metric)?.label} over time"
        onpointermove={move}
        onpointerleave={() => (hover = null)}
      >
        {#if chart.dots.length > 1}
          <polygon points={chart.area} class="area" />
          <polyline points={chart.line} class="line" />
        {/if}
        {#if shown}
          <line x1={shown.x} x2={shown.x} y1={PAD} y2={H - PAD} class="cursor" />
        {/if}
      </svg>
      <!-- Dots are drawn over the chart so they stay round when it stretches. -->
      <div class="progress-dots" aria-hidden="true">
        {#each chart.dots as d, i (d.p.at)}
          <span
            class:on={hover === i}
            style:left="{(d.x / W) * 100}%"
            style:top="{(d.y / H) * 100}%"
          ></span>
        {/each}
      </div>
      <figcaption>
        <span>{localDay(chart.first.p.at)}</span>
        <span>{chart.lo === chart.hi ? label(chart.lo, metric) : `${label(chart.lo, metric)} to ${label(chart.hi, metric)}`}</span>
        <span>{localDay(chart.last.p.at)}</span>
      </figcaption>
    </figure>
  {/if}
</section>

<style>
  .progress-card {
    margin-bottom: 14px;
  }

  .progress-empty {
    display: grid;
    place-items: center;
    min-height: 200px;
    margin: 0;
    text-align: center;
  }

  .progress-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 8px;
  }

  .progress-head h2 {
    margin: 0;
  }

  .progress-controls {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .segmented.compact {
    grid-template-columns: none;
    grid-auto-flow: column;
  }

  .segmented.compact button {
    height: 24px;
    padding: 0 10px;
    font-size: 12.5px;
  }

  .progress-now {
    display: flex;
    align-items: baseline;
    gap: 10px;
    font-variant-numeric: tabular-nums;
  }

  .progress-figure {
    font-size: 22px;
    font-weight: 600;
  }

  .up {
    color: var(--good);
  }

  .down {
    color: var(--bad);
  }

  .progress-chart {
    position: relative;
  }

  .progress-chart svg {
    height: 150px;
    cursor: crosshair;
    touch-action: none;
  }

  .progress-chart .cursor {
    stroke: var(--line-strong);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }

  .progress-dots {
    position: absolute;
    inset: 0 0 auto 0;
    height: 150px;
    pointer-events: none;
  }

  .progress-dots span {
    position: absolute;
    width: 6px;
    height: 6px;
    margin: -3px 0 0 -3px;
    border-radius: 50%;
    background: var(--accent);
  }

  .progress-dots span.on {
    width: 9px;
    height: 9px;
    margin: -4.5px 0 0 -4.5px;
    box-shadow: 0 0 0 2px var(--surface);
  }
</style>
