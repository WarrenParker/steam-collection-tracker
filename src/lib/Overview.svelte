<script lang="ts">
  import { onMount } from 'svelte';
  import {
    api,
    errorText,
    links,
    on,
    type ProfileView,
    type ScanStatus,
    type ScanSummary,
    type SettingsView,
  } from './api';
  import { ago, clock, duration, num, roughly } from './format';
  import Diagnostics from './Diagnostics.svelte';
  import Icon from './Icon.svelte';
  import SignInButton from './SignInButton.svelte';

  let {
    signedIn,
    includeFamily,
    onprofile,
    onsettings,
  }: {
    signedIn: boolean;
    includeFamily: boolean;
    onprofile: (p: ProfileView) => void;
    onsettings: (s: SettingsView) => void;
  } = $props();

  let scan = $state<ScanSummary | null>(null);
  let status = $state<ScanStatus | null>(null);
  let loaded = $state(false);
  let error = $state('');
  let justSignedIn = $state(false);
  // Re-render the "scanned 3 minutes ago" text every minute.
  let now = $state(Date.now());

  // Each stage's share of the progress bar.
  const STAGES: Record<number, [number, number]> = {
    1: [0, 15],
    2: [15, 25],
    3: [25, 35],
    4: [35, 100],
  };

  const running = $derived(status?.state === 'running');
  const catalog = $derived(scan?.catalog && scan.catalog.games_listed > 0 ? scan.catalog : null);
  const canContinue = $derived(
    !!status &&
      !running &&
      status.remaining > 0 &&
      (status.state === 'paused' ||
        status.state === 'waiting' ||
        (status.state === 'idle' && !!catalog && catalog.games_cataloged > 0)),
  );
  const percent = $derived.by(() => {
    if (!status || !running) return 0;
    const [from, to] = STAGES[status.stage] ?? [0, 5];
    const part = status.total > 0 ? status.done / status.total : 0.5;
    return Math.round(from + (to - from) * Math.min(1, part));
  });
  const progressText = $derived.by(() => {
    if (!status || !running) return '';
    const parts = [status.label || 'Scanning…'];
    if (status.stage === 4 && status.total > 0) parts.push(`${num(status.done)} of ${num(status.total)} games`);
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

<div class="page">
  <header class="page-head">
    <div>
      <h1>Overview</h1>
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
        The last scan didn't finish. {num(status.remaining)}
        {status.remaining === 1 ? 'game still needs' : 'games still need'} an item list{status.remaining_seconds
          ? `, which takes ${roughly(status.remaining_seconds)}`
          : ''}.
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

    {#if catalog}
      <section class="block">
        <div class="split">
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
        </div>
      </section>
    {/if}

    <section class="block">
      <div class="split">
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
    </section>
  {/if}

  <Diagnostics />

  <p class="phase-note">
    This is an early version. Completion percentages and prices arrive in the next versions.
  </p>
</div>
