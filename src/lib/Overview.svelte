<script lang="ts">
  import { onMount } from 'svelte';
  import {
    api,
    errorText,
    links,
    on,
    type AccountCompletion,
    type Dashboard,
    type DashChange,
    type GameRow,
    type ItemRow,
    type ProfileView,
    type ScanStatus,
    type ScanSummary,
    type SettingsView,
  } from './api';
  import { ago, agoUnix, change, clock, duration, localDay, money, num, owned, pct, points, roughly } from './format';
  import Diagnostics from './Diagnostics.svelte';
  import Icon from './Icon.svelte';
  import ProgressChart from './ProgressChart.svelte';
  import SignInButton from './SignInButton.svelte';
  import { nav, openGame, openItem, showItems } from './nav.svelte.ts';

  let {
    signedIn,
    includeFamily,
    keepCardSet,
    priceKey = '',
    onprofile,
    onsettings,
  }: {
    signedIn: boolean;
    includeFamily: boolean;
    keepCardSet: boolean;
    priceKey?: string;
    onprofile: (p: ProfileView) => void;
    onsettings: (s: SettingsView) => void;
  } = $props();

  let scan = $state<ScanSummary | null>(null);
  let completion = $state<AccountCompletion | null>(null);
  let dash = $state<Dashboard | null>(null);
  let detailsOpen = $state(false);
  let status = $state<ScanStatus | null>(null);
  let loaded = $state(false);
  let error = $state('');
  let justSignedIn = $state(false);
  // Re-render the "scanned 3 minutes ago" text every minute.
  let now = $state(Date.now());

  // Each stage's share of the progress bar.
  const STAGES: Record<number, [number, number]> = {
    1: [0, 8],
    2: [8, 12],
    3: [12, 16],
    4: [16, 45],
    5: [45, 65],
    6: [65, 100],
  };

  const running = $derived(status?.state === 'running');
  const catalog = $derived(scan?.catalog && scan.catalog.games_listed > 0 ? scan.catalog : null);
  const canContinue = $derived(
    !!status &&
      !running &&
      (status.remaining > 0 || status.prices_remaining > 0) &&
      (status.state === 'paused' ||
        status.state === 'waiting' ||
        (status.state === 'idle' && !!catalog && catalog.games_cataloged > 0)),
  );
  const prices = $derived(completion?.prices ?? null);
  const cur = $derived(completion?.currency ?? 'USD');
  const hasPrices = $derived(!!completion && completion.prices_at !== null);
  const percent = $derived.by(() => {
    if (!status || !running) return 0;
    const [from, to] = STAGES[status.stage] ?? [0, 5];
    const part = status.total > 0 ? status.done / status.total : 0.5;
    return Math.round(from + (to - from) * Math.min(1, part));
  });
  const progressText = $derived.by(() => {
    if (!status || !running) return '';
    const parts = [status.label || 'Scanning…'];
    if ((status.stage === 4 || status.stage === 5) && status.total > 0)
      parts.push(`${num(status.done)} of ${num(status.total)} games`);
    if (status.stage === 6 && status.total > 0) parts.push(`${num(status.done)} of ${num(status.total)} items`);
    if (status.seconds_left !== null && status.seconds_left > 0) parts.push(`${roughly(status.seconds_left)} left`);
    return parts.join(' · ');
  });
  const scannedText = $derived.by(() => {
    void now;
    return scan
      ? `Last scanned ${ago(scan.scanned_at)}. Reading your library took ${duration(scan.duration_ms)}.`
      : 'Not scanned yet.';
  });
  const maxType = $derived(scan ? Math.max(1, ...scan.by_type.map((t) => t.count)) : 1);
  const maxKind = $derived(catalog ? Math.max(1, ...catalog.by_kind.map((k) => k.count)) : 1);

  function loadCompletion() {
    api
      .completion()
      .then((c) => (completion = c))
      .catch(() => {});
    api
      .dashboard()
      .then((d) => (dash = d))
      .catch(() => {});
  }

  // Since the day before the latest figures, when there's one.
  const trend = $derived.by(() => {
    const pts = dash?.progress ?? [];
    if (pts.length < 2) return null;
    const last = pts[pts.length - 1];
    const prev = pts[pts.length - 2];
    const line = sparkline(pts.map((p) => p.percent ?? 0));
    return {
      since: prev.at,
      percent: last.percent !== null && prev.percent !== null ? last.percent - prev.percent : null,
      value: last.value !== null && prev.value !== null && last.currency === prev.currency ? last.value - prev.value : null,
      cost: last.cost !== null && prev.cost !== null && last.currency === prev.currency ? last.cost - prev.cost : null,
      line,
    };
  });

  /** Points for a small line chart 120 wide and 32 high. */
  function sparkline(values: number[]): string {
    const lo = Math.min(...values);
    const hi = Math.max(...values);
    const span = Math.max(hi - lo, 0.0001);
    return values
      .map((v, i) => `${((i / Math.max(1, values.length - 1)) * 120).toFixed(1)},${(30 - ((v - lo) / span) * 28).toFixed(1)}`)
      .join(' ');
  }

  function signed(n: number, text: string): string {
    return `${n > 0 ? '+' : n < 0 ? '−' : ''}${text}`;
  }

  function changeText(c: DashChange): string {
    const d = c.after - c.before;
    return d > 0 ? `+${num(d)}` : `−${num(-d)}`;
  }

  function costText(g: GameRow): string {
    const parts: string[] = [];
    if (g.cost !== null && g.cost > 0) parts.push(money(g.cost, g.currency));
    if (g.cost_points > 0) parts.push(`${num(g.cost_points)} pts`);
    return parts.join(' + ');
  }

  // Recent changes, grouped by the day of the scan that saw them.
  const changeDays = $derived.by(() => {
    const days: { label: string; rows: DashChange[] }[] = [];
    for (const c of dash?.changes ?? []) {
      const label = localDay(c.at);
      const last = days[days.length - 1];
      if (last && last.label === label) last.rows.push(c);
      else days.push({ label, rows: [c] });
    }
    return days;
  });

  // The settings that change what counts.
  $effect(() => {
    void includeFamily;
    void keepCardSet;
    void priceKey;
    loadCompletion();
  });

  onMount(() => {
    const unlisteners: Array<() => void> = [];
    Promise.all([api.lastScan(), api.scanStatus()])
      .then(([s, st]) => {
        scan = s;
        status = st;
      })
      .catch(() => {})
      .finally(() => (loaded = true));
    on<ScanStatus>('scan-status', (s) => (status = s)).then((u) => unlisteners.push(u));
    on<ScanSummary>('scan-summary', (s) => {
      scan = s;
      onprofile(s.profile);
      loadCompletion();
    }).then((u) => unlisteners.push(u));
    const timer = setInterval(() => (now = Date.now()), 60_000);
    return () => {
      clearInterval(timer);
      unlisteners.forEach((u) => u());
    };
  });

  async function start(resume = false) {
    error = '';
    justSignedIn = false;
    try {
      status = await api.startScan(resume);
    } catch (e) {
      error = errorText(e);
    }
  }

  async function pause() {
    try {
      status = await api.pauseScan();
    } catch (e) {
      error = errorText(e);
    }
  }

  function quantityNote(count: number, quantity: number): string {
    return quantity > count ? `${num(quantity)} in ${num(count)} stacks` : '';
  }
</script>

<div class="page dash">
  <header class="page-head">
    <div>
      <h1>Dashboard</h1>
      <p class="meta">{scannedText}</p>
    </div>
    <div class="head-actions">
      {#if running || status?.state === 'waiting'}
        <button class="btn" onclick={pause} disabled={status?.pause_requested}>
          {status?.pause_requested ? 'Pausing…' : 'Pause'}
        </button>
      {/if}
      {#if !running}
        {#if canContinue}
          <button class="btn primary" onclick={() => start(true)}>
            <Icon name="scan" /> Continue scan
          </button>
        {:else}
          <button class="btn primary" onclick={() => start(false)}>
            <Icon name="scan" />
            {scan ? 'Scan again' : 'Scan my collection'}
          </button>
        {/if}
      {/if}
    </div>
  </header>

  {#if running}
    <div class="progress" aria-hidden="true"><span style:width="{percent}%"></span></div>
    <p class="progress-text" role="status">{progressText}</p>
  {/if}

  {#if error}
    <div class="notice bad" role="alert"><Icon name="alert" /><p>{error}</p></div>
  {/if}

  {#if status && !running && status.message}
    <div
      class="notice"
      class:good={status.state === 'done'}
      class:warn={status.state === 'paused' || status.state === 'waiting' || status.state === 'idle'}
      class:bad={status.state === 'failed'}
      role="status"
    >
      <Icon name={status.state === 'done' ? 'check' : 'alert'} />
      <div>
        <p>{status.message}</p>
        {#if status.state === 'waiting' && status.resume_at}
          <p class="notice-sub">Carries on at {clock(status.resume_at)}.</p>
        {/if}
      </div>
    </div>
  {:else if canContinue && status}
    <div class="notice warn" role="status">
      <Icon name="alert" />
      <p>
        {#if status.remaining > 0}
          The last scan didn't finish. {num(status.remaining)}
          {status.remaining === 1 ? 'game still needs' : 'games still need'} an item list{status.remaining_seconds
            ? `, which takes ${roughly(status.remaining_seconds)}`
            : ''}.
        {:else}
          The last scan didn't finish. {num(status.prices_remaining)}
          {status.prices_remaining === 1 ? 'item still needs its' : 'items still need their'} price history{status.prices_seconds
            ? `, which takes ${roughly(status.prices_seconds)}`
            : ''}.
        {/if}
      </p>
    </div>
  {/if}

  {#if loaded && !scan && !running}
    <div class="empty">
      <h2>Scan your collection to get started</h2>
      <p>
        The app reads your games, badges and inventory, then each game's full list of items: cards,
        foil cards, emoticons, backgrounds and Points Shop items. The first scan usually takes a few
        minutes. You can pause it and carry on later, even after closing the app.
      </p>
    </div>
  {/if}

  {#if scan?.needs_sign_in && !signedIn}
    <div class="notice warn">
      <Icon name="alert" />
      <div>
        <p>
          Steam won't share your inventory with an app that isn't signed in, so items couldn't be counted.
          Sign in with Steam, then scan again.
        </p>
        <SignInButton
          small
          onsignedin={(r) => {
            onsettings(r.settings);
            justSignedIn = true;
          }}
        />
      </div>
    </div>
  {/if}
  {#if justSignedIn && !running}
    <div class="notice good"><Icon name="check" /><p>Signed in. Press Scan again to read your inventory.</p></div>
  {/if}

  {#if scan}
    {#each scan.warnings as warning}
      <div class="notice warn">
        <Icon name="alert" />
        <div>
          <p>{warning}</p>
          {#if warning.includes('privacy settings')}
            <button class="link" type="button" onclick={() => api.openLink(links.privacy)}>
              Open Steam's privacy settings
            </button>
          {/if}
        </div>
      </div>
    {/each}

    {#if completion && completion.total > 0}
      <div class="hero-cards">
        <button class="dash-card hero" onclick={() => (nav.view = 'games')} title="See every game's completion">
          <span class="card-label">Collection complete</span>
          <span class="card-value" class:foil={completion.percent === 100}>{pct(completion.percent)}</span>
          <span class="card-sub">{num(completion.complete)} of {num(completion.total)} items · average game {pct(completion.game_average)}</span>
          {#if trend}
            <span class="card-trend">
              <svg viewBox="0 0 120 32" preserveAspectRatio="none" aria-hidden="true"><polyline points={trend.line} /></svg>
              {#if trend.percent !== null && Math.abs(trend.percent) >= 0.05}
                <span class:up={trend.percent > 0}>{signed(trend.percent, `${Math.abs(trend.percent).toFixed(1)} pts`)} since {localDay(trend.since)}</span>
              {/if}
            </span>
          {/if}
        </button>

        {#if hasPrices && prices}
          <button
            class="dash-card hero"
            onclick={() => showItems({ have: 'owned', sort: 'price', descending: true })}
            title="See the items you have, most valuable first"
          >
            <span class="card-label">Value</span>
            <span class="card-value">{money(prices.value, cur)}</span>
            <span class="card-sub">{num(prices.value_copies)} items you could sell · {points(prices.points_value)} that can't be sold</span>
            {#if trend?.value}
              <span class="card-trend"><span class:up={trend.value > 0}>{signed(trend.value, money(Math.abs(trend.value), cur))} since {localDay(trend.since)}</span></span>
            {/if}
          </button>

          <button
            class="dash-card hero"
            onclick={() => showItems({ have: 'missing', sort: 'price', descending: false })}
            title="See what you still need, cheapest first"
          >
            <span class="card-label">To complete</span>
            <span class="card-value">{money(prices.cost, cur)}</span>
            <span class="card-sub">
              {#if prices.cost_points > 0}+ {points(prices.cost_points)} in the Points Shop{/if}{#if prices.unpriced > 0}{prices.cost_points > 0 ? ' · ' : ''}{num(prices.unpriced)} not for sale right now{/if}
            </span>
            {#if trend?.cost}
              <span class="card-trend"><span class:up={trend.cost < 0}>{signed(trend.cost, money(Math.abs(trend.cost), cur))} since {localDay(trend.since)}</span></span>
            {/if}
          </button>
        {:else}
          <div class="dash-card hero quiet">
            <span class="card-label">Value and cost to complete</span>
            <span class="card-sub">Prices are read at the end of a scan, game by game.</span>
          </div>
        {/if}
      </div>

      <ProgressChart />

      {#if dash}
        <div class="dash-grid">
          <section class="dash-card">
            <h2>Closest to complete</h2>
            {#if dash.closest.length}
              <ul class="dash-list">
                {#each dash.closest as g (g.appid)}
                  <li>
                    <button class="dash-row" onclick={() => openGame(g.appid)}>
                      <span class="row-name">{g.name}</span>
                      <span class="bar"><span style:width="{g.percent ?? 0}%"></span></span>
                      <span class="row-n">{pct(g.percent)}</span>
                    </button>
                  </li>
                {/each}
              </ul>
            {:else}
              <p class="muted">Nothing started yet.</p>
            {/if}
          </section>

          <section class="dash-card">
            <h2>Cheapest to finish</h2>
            {#if dash.cheapest.length}
              <ul class="dash-list">
                {#each dash.cheapest as g (g.appid)}
                  <li>
                    <button class="dash-row cost-row" onclick={() => openGame(g.appid)}>
                      <span class="row-name">{g.name}</span>
                      <span class="row-sub">{pct(g.percent)}</span>
                      <span class="row-n">{costText(g)}</span>
                    </button>
                  </li>
                {/each}
              </ul>
            {:else}
              <p class="muted">{hasPrices ? 'Every game that counts is done, or has no Market price.' : 'Shown once prices are read.'}</p>
            {/if}
          </section>

          {#snippet itemList(rows: ItemRow[], none: string)}
            {#if rows.length}
              <ul class="dash-list">
                {#each rows as i (i.key)}
                  <li>
                    <button class="dash-row with-art" onclick={() => openItem(i.key)}>
                      <span class="thumb tiny" class:foil-edge={i.foil}>{#if i.image}<img src={i.image} alt="" loading="lazy" />{/if}</span>
                      <span class="row-name">{i.name}<span class="row-game">{dash?.names[String(i.appid)] ?? ''}</span></span>
                      <span class="row-n">
                        {money(i.price, dash?.currency ?? cur)}
                        {#if i.change !== null}<span class="chg" class:up={i.change > 0} class:down={i.change < 0}>{change(i.change)}</span>{/if}
                      </span>
                    </button>
                  </li>
                {/each}
              </ul>
            {:else}
              <p class="muted">{none}</p>
            {/if}
          {/snippet}

          <section class="dash-card">
            <h2 title="Items you could sell, listed well above their average price right now">Good time to sell</h2>
            {@render itemList(dash.sell, hasPrices ? 'None of your items are up much on their average.' : 'Shown once price history is read.')}
          </section>

          <section class="dash-card">
            <h2 title="Items you still need, listed well below their average price right now">Good time to buy</h2>
            {@render itemList(dash.buy, hasPrices ? 'Nothing you need is down much on its average.' : 'Shown once price history is read.')}
          </section>

          <section class="dash-card">
            <h2>Recent changes</h2>
            {#if changeDays.length}
              {#each changeDays as d (d.label)}
                <div class="day-label">{d.label}</div>
                <ul class="dash-list">
                  {#each d.rows as c (c.key + c.at)}
                    <li>
                      <button class="dash-row with-art" onclick={() => openItem(c.key)}>
                        <span class="thumb tiny" class:foil-edge={c.foil}>{#if c.image}<img src={c.image} alt="" loading="lazy" />{/if}</span>
                        <span class="row-name">{c.name}<span class="row-game">{c.game}</span></span>
                        <span class="row-n delta" class:up={c.after > c.before}>{changeText(c)}</span>
                      </button>
                    </li>
                  {/each}
                </ul>
              {/each}
            {:else}
              <p class="muted">Items you gain, sell or use up show here after your next scans.</p>
            {/if}
          </section>

          <section class="dash-card">
            <h2>By kind</h2>
            <ul class="typelist compact">
              {#each completion.groups as g (g.group)}
                <li>
                  <span class="type-name">{g.label}{#if g.group === 'boosters'}<span class="muted"> · not counted</span>{/if}</span>
                  <span class="bar"><span style:width="{g.total ? (g.owned / g.total) * 100 : 0}%"></span></span>
                  <span class="n" title="{owned(g.owned)} of {num(g.total)}">{pct(g.total ? Math.round((1000 * g.owned) / g.total) / 10 : null)}</span>
                </li>
              {/each}
            </ul>
          </section>
        </div>
      {/if}
    {/if}

    <section class="details">
      <button class="details-toggle" aria-expanded={detailsOpen} onclick={() => (detailsOpen = !detailsOpen)}>
        <Icon name="chevron" size={16} /> All the figures
      </button>
      {#if detailsOpen}
        <div class="details-grid">
    <div class="figures">
      <div class="figure">
        <div class="figure-value">{scan.games_visible ? num(scan.games_owned) : '–'}</div>
        <div class="figure-label">Games owned</div>
        {#if !scan.games_visible}<div class="figure-note">Game details are private</div>{/if}
      </div>
      <div class="figure">
        <div class="figure-value">{num(scan.game_badges)}</div>
        <div class="figure-label">Game badges</div>
      </div>
      <div class="figure">
        <div class="figure-value foil">{num(scan.foil_badges)}</div>
        <div class="figure-label">Foil badges</div>
      </div>
      <div class="figure">
        <div class="figure-value">{scan.inventory_visible ? num(scan.inventory_items) : '–'}</div>
        <div class="figure-label">Inventory items</div>
        {#if !scan.inventory_visible}<div class="figure-note">Inventory is private</div>{/if}
      </div>
    </div>

          {#if completion && completion.total > 0}
          <section>
            <h2>Completion</h2>
            <dl class="facts">
              <dt title="Every item weighs the same">Items complete</dt>
              <dd>{pct(completion.percent)}</dd>
              <dt title="Every game weighs the same: the average of each game's percentage">Average game</dt>
              <dd>{pct(completion.game_average)}</dd>
              <dt>Games started</dt>
              <dd>{num(completion.games_started)} of {num(completion.games)}</dd>
              <dt>Games complete</dt>
              <dd>{num(completion.games_complete)} of {num(completion.games)}</dd>
              {#if completion.card_games > 0}
                <dt title="Level 5 badge and the foil badge">Card sets complete</dt>
                <dd>{num(completion.card_sets_complete)} of {num(completion.card_games)}</dd>
              {/if}
              {#if completion.limited > 0}
                <dt title="Items that are only around for a while, or handed out at sales and events">
                  Limited or event items <span class="tag">Limited/Event</span>
                </dt>
                <dd>{num(completion.limited)}</dd>
              {/if}
              {#if completion.unobtainable > 0}
                <dt title="Items that can't be had any more. They're listed but left out of the percentage.">
                  Can't be had any more <span class="tag">Unobtainable</span>
                </dt>
                <dd>{num(completion.unobtainable)}{completion.unobtainable_owned
                    ? `, ${num(completion.unobtainable_owned)} owned`
                    : ''}</dd>
              {/if}
            </dl>
          </section>
          {/if}
          {#if hasPrices && prices && completion}
          <section>
            <h2>Value</h2>
            <div class="collection-head">
              <div class="collection-percent money">{money(prices.value, cur)}</div>
              <p class="section-note">
                What the {num(prices.value_copies)} items you could sell are worth at today's cheapest listings,
                before Steam's fees.
              </p>
            </div>
            <dl class="facts">
              <dt title="Items you hold that can't be sold, such as Points Shop purchases, at their Points Shop price">
                Points value
              </dt>
              <dd>{points(prices.points_value)}</dd>
              {#if prices.value_unpriced > 0}
                <dt title="Items you could sell that nobody has listed or sold lately">Sellable, no price yet</dt>
                <dd>{num(prices.value_unpriced)}</dd>
              {/if}
              {#if prices.spikes > 0 || prices.dips > 0}
                <dt title="Today's cheapest listing compared with the average price. Change the sensitivity in Settings.">
                  Price spikes and dips
                </dt>
                <dd>{num(prices.spikes)} up, {num(prices.dips)} down</dd>
              {/if}
            </dl>
          </section>

          <section>
            <h2>Cost to complete</h2>
            <dl class="facts">
              <dt title="Everything still needed that the Market sells, buying the cheapest copies first">On the Market</dt>
              <dd>{money(prices.cost, cur)}</dd>
              <dt title="Items only the Points Shop sells">In the Points Shop</dt>
              <dd>{points(prices.cost_points)}</dd>
              {#if prices.unpriced > 0}
                <dt title="Still needed, but nobody is selling them right now">Not for sale right now</dt>
                <dd>{num(prices.unpriced)} {prices.unpriced === 1 ? 'item' : 'items'}</dd>
              {/if}
            </dl>
            {#if prices.alt_items > 0}
              <p class="section-note small-note">
                Or buy {num(prices.alt_items)} of those items in the Points Shop for {points(prices.alt_points)}
                instead of {money(prices.alt_money, cur)}.
              </p>
            {/if}
            {#if prices.cost_capped > 0}
              <p class="section-note small-note">
                {num(prices.cost_capped)} {prices.cost_capped === 1 ? 'item is' : 'items are'} listed at more than three
                times {prices.cost_capped === 1 ? 'its' : 'their'} usual price, so {prices.cost_capped === 1 ? "it's" : "they're"}
                counted at the 90-day average sale price.
              </p>
            {/if}
            {#if prices.cost_estimated > 0}
              <p class="section-note small-note">
                {num(prices.cost_estimated)} {prices.cost_estimated === 1 ? 'item is' : 'items are'} estimated. Opening a
                game reads the Market's listings for its cards, which makes its cost exact.
              </p>
            {/if}
            <p class="section-note small-note">
              Prices from {agoUnix(completion.prices_at)}. Price history read for {num(prices.with_history)} of
              {num(prices.marketable)} items; opening an item reads its history.
            </p>
            {#if completion.currency_note}
              <p class="section-note small-note">{completion.currency_note}</p>
            {/if}
          </section>
          {/if}
          {#if catalog}
          <section>
            <h2>Item catalog</h2>
            <p class="section-note">
              Every item that exists for the games you have, from cards to Points Shop items.
            </p>
            {#if catalog.by_kind.length}
              <ul class="typelist">
                {#each catalog.by_kind as k (k.label)}
                  <li>
                    <span class="type-name">{k.label}</span>
                    <span class="bar"><span style:width="{(k.count / maxKind) * 100}%"></span></span>
                    <span class="n">{num(k.count)}</span>
                  </li>
                {/each}
              </ul>
            {:else}
              <p class="muted">No items cataloged yet.</p>
            {/if}
          </section>

          <section>
            <h2>Games</h2>
            <dl class="facts">
              <dt>Games listed</dt>
              <dd>{num(catalog.games_listed)}</dd>
              <dt>Owned</dt>
              <dd>{num(catalog.games_owned)}</dd>
              <dt title="Owned games that have any collectible items: cards, emoticons, backgrounds or Points Shop items">
                Owned games with items
              </dt>
              <dd>{num(catalog.owned_with_items)} of {num(catalog.games_owned)}</dd>
              <dt
                title="Games you have cards, items or badges for but don't own. Steam sale events show up here too."
              >
                Not owned <span class="tag">Unowned</span>
              </dt>
              <dd>{num(catalog.games_unowned)}</dd>
              {#if includeFamily}
                <dt>Shared by your Steam Family <span class="tag">Family</span></dt>
                <dd>{num(catalog.games_family)}</dd>
              {/if}
              <dt>With trading cards</dt>
              <dd>{num(catalog.games_with_cards)}</dd>
              <dt>Item lists read</dt>
              <dd>{num(catalog.games_cataloged)}</dd>
              <dt>Catalog items</dt>
              <dd>{num(catalog.items)}</dd>
              <dt>Sold in the Points Shop</dt>
              <dd>{num(catalog.points_items)}</dd>
            </dl>
          </section>
          {/if}
        <section>
          <h2>Inventory by type</h2>
          {#if scan.by_type.length}
            <ul class="typelist">
              {#each scan.by_type as t (t.label)}
                <li>
                  <span class="type-name">{t.label}</span>
                  <span class="bar"><span style:width="{(t.count / maxType) * 100}%"></span></span>
                  <span class="n" title={quantityNote(t.count, t.quantity)}>
                    {t.quantity > t.count ? num(t.quantity) : num(t.count)}
                  </span>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="muted">No Steam items found in the inventory.</p>
          {/if}
        </section>

        <section>
          <h2>Your account</h2>
          <dl class="facts">
            {#if catalog && catalog.ach_games > 0}
              <dt title="The average completion of games you've unlocked at least one achievement in, the way Steam counts it">
                Average achievement completion
              </dt>
              <dd>{catalog.ach_average !== null ? `${catalog.ach_average.toFixed(1)}%` : '–'}</dd>
              <dt>Achievements unlocked</dt>
              <dd>{num(catalog.ach_unlocked)} of {num(catalog.ach_total)}</dd>
              <dt>Perfect games</dt>
              <dd>{num(catalog.ach_perfect)} of {num(catalog.ach_games)}</dd>
            {/if}
            <dt>Foil cards</dt>
            <dd>{num(scan.foil_cards)}</dd>
            <dt>Steam level</dt>
            <dd>{scan.player_level ?? '–'}</dd>
            <dt>Other badges</dt>
            <dd>{num(scan.other_badges)}</dd>
            {#if scan.family_games !== null}
              <dt>Steam Family games</dt>
              <dd>{num(scan.family_games)}</dd>
            {/if}
            <dt>Requests to Steam</dt>
            <dd>{num(scan.requests)}</dd>
            {#if scan.inventory_route}
              <dt>Inventory read through</dt>
              <dd class="route">{scan.inventory_route}</dd>
            {/if}
          </dl>
        </section>
        </div>
      {/if}
    </section>
  {/if}

  <Diagnostics />
</div>
