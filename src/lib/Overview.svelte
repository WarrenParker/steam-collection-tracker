<script lang="ts">
  import { onMount } from 'svelte';
  import {
    api,
    errorText,
    links,
    on,
    type ProfileView,
    type ScanProgress,
    type ScanSummary,
  } from './api';
  import { ago, duration, num } from './format';
  import Diagnostics from './Diagnostics.svelte';
  import Icon from './Icon.svelte';

  let { onprofile }: { onprofile: (p: ProfileView) => void } = $props();

  let scan = $state<ScanSummary | null>(null);
  let loaded = $state(false);
  let scanning = $state(false);
  let progress = $state<ScanProgress | null>(null);
  let note = $state('');
  let error = $state('');
  // Re-render the "scanned 3 minutes ago" text every minute.
  let now = $state(Date.now());

  const maxType = $derived(scan ? Math.max(1, ...scan.by_type.map((t) => t.count)) : 1);
  const percent = $derived(
    progress ? Math.min(100, Math.round(((progress.step - 0.5) / progress.total) * 100)) : 0,
  );
  const scannedText = $derived.by(() => {
    void now;
    return scan
      ? `Last scanned ${ago(scan.scanned_at)}. It took ${duration(scan.duration_ms)}.`
      : 'Not scanned yet.';
  });

  onMount(() => {
    const unlisteners: Array<() => void> = [];
    api
      .lastScan()
      .then((s) => (scan = s))
      .catch(() => {})
      .finally(() => (loaded = true));
    on<ScanProgress>('scan-progress', (p) => {
      progress = p;
      note = '';
    }).then((u) => unlisteners.push(u));
    on<string>('scan-note', (m) => (note = m)).then((u) => unlisteners.push(u));
    const timer = setInterval(() => (now = Date.now()), 60_000);
    return () => {
      clearInterval(timer);
      unlisteners.forEach((u) => u());
    };
  });

  async function runScan() {
    scanning = true;
    error = '';
    note = '';
    progress = { step: 1, total: 4, message: 'Starting…' };
    try {
      scan = await api.runScan();
      onprofile(scan.profile);
    } catch (e) {
      error = errorText(e);
    } finally {
      scanning = false;
      progress = null;
      note = '';
    }
  }

  function quantityNote(label: string, count: number, quantity: number): string {
    return quantity > count ? `${num(quantity)} in ${num(count)} stacks` : '';
  }
</script>

<div class="page">
  <header class="page-head">
    <div>
      <h1>Overview</h1>
      <p class="meta">{scannedText}</p>
    </div>
    <button class="btn primary" onclick={runScan} disabled={scanning}>
      <Icon name="scan" />
      {scanning ? 'Scanning…' : scan ? 'Scan again' : 'Scan my profile'}
    </button>
  </header>

  {#if scanning && progress}
    <div class="progress" aria-hidden="true"><span style:width="{percent}%"></span></div>
    <p class="progress-text" role="status">{note || progress.message}</p>
  {/if}

  {#if error}
    <div class="notice bad" role="alert"><Icon name="alert" /><p>{error}</p></div>
  {/if}

  {#if loaded && !scan && !scanning}
    <div class="empty">
      <h2>Scan your profile to get started</h2>
      <p>
        The app counts your games, badges and inventory items. The first scan takes about a minute,
        depending on the size of your inventory.
      </p>
    </div>
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

    <div class="split">
      <section>
        <h2>Inventory by type</h2>
        {#if scan.by_type.length}
          <ul class="typelist">
            {#each scan.by_type as t (t.label)}
              <li>
                <span class="type-name">{t.label}</span>
                <span class="bar"><span style:width="{(t.count / maxType) * 100}%"></span></span>
                <span class="n" title={quantityNote(t.label, t.count, t.quantity)}>
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
        <h2>From this scan</h2>
        <dl class="facts">
          <dt>Foil cards</dt>
          <dd>{num(scan.foil_cards)}</dd>
          <dt>Steam level</dt>
          <dd>{scan.player_level ?? '–'}</dd>
          <dt>Other badges</dt>
          <dd>{num(scan.other_badges)}</dd>
          <dt>Games with items or badges</dt>
          <dd>{num(scan.apps_with_items)}</dd>
          <dt
            title="Games you have cards or badges for but don't own. Steam sale events can show up here until a later version sorts them out."
          >
            Of those, not owned <span class="tag">Unowned</span>
          </dt>
          <dd>{num(scan.apps_not_owned)}</dd>
          <dt>Requests to Steam</dt>
          <dd>{num(scan.requests)}</dd>
        </dl>
      </section>
    </div>
  {/if}

  <Diagnostics />

  <p class="phase-note">
    This is version 0.1. Completion percentages, full item lists and prices arrive in the next
    versions.
  </p>
</div>
