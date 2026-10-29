<script lang="ts">
  import { onMount, tick } from 'svelte';
  import {
    api,
    CURRENCIES,
    errorText,
    type AppInfo,
    type PriceWindow,
    type SettingsView,
    type SharedPrices,
    type SignInResult,
    type Theme,
    type UpdateState,
  } from './api';
  import { agoUnix, bytes, num } from './format';
  import Icon from './Icon.svelte';
  import SignInButton from './SignInButton.svelte';
  import VersionHistory from './VersionHistory.svelte';
  import { loadPrefs, savePrefs, type Prefs } from './prefs';

  let {
    settings,
    info,
    update,
    focus = null,
    onclose,
    onchange,
  }: {
    settings: SettingsView;
    info: AppInfo;
    update: UpdateState | null;
    focus?: 'updates' | null;
    onclose: () => void;
    onchange: (s: SettingsView) => void;
  } = $props();

  const windows: { value: PriceWindow; label: string }[] = [
    { value: 'month', label: '30 days' },
    { value: 'quarter', label: '90 days' },
    { value: 'year', label: '1 year' },
    { value: 'all', label: 'All time' },
  ];
  const percents = [10, 15, 20, 25, 30, 40, 50, 75, 100];
  const minimums = [0, 1, 2, 5, 10, 25, 50, 100];
  const currencyNames = (() => {
    try {
      return new Intl.DisplayNames(['en'], { type: 'currency' });
    } catch {
      return null;
    }
  })();
  const currencies = CURRENCIES.map((code) => ({
    code,
    label: `${code} · ${currencyNames?.of(code) ?? code}`,
  })).sort((a, b) => (a.code === 'USD' ? -1 : b.code === 'USD' ? 1 : a.code.localeCompare(b.code)));

  const themes: { value: Theme; label: string }[] = [
    { value: 'greyscale', label: 'Greyscale' },
    { value: 'dark', label: 'Dark' },
    { value: 'light', label: 'Light' },
  ];

  let prefs = $state<Prefs>(loadPrefs());
  function setPref(patch: Partial<Prefs>) {
    prefs = savePrefs(patch);
  }

  const SHORTCUTS: [string, string][] = [
    ['Ctrl K or /', 'Search games and items'],
    ['Esc', 'Close the topmost panel'],
    ['↑ ↓', 'Move through the Items list, or step through items with one open'],
    ['Enter', 'Open the selected item'],
    ['M', "Open an item's Market page"],
    ['P', "Open the game's Points Shop"],
    ['C', "Copy an item's name"],
  ];

  let editing = $state<'none' | 'key' | 'profile' | 'token'>('none');
  let keyInput = $state('');
  let profileInput = $state('');
  let busy = $state(false);
  let error = $state('');
  let saved = $state('');
  let checking = $state(false);
  let installing = $state(false);
  let updateMessage = $state('');
  let updateError = $state('');
  let updatesSection = $state<HTMLElement | null>(null);

  const versionText = $derived(
    `Version ${info.version}${info.build ? ` (build ${info.build})` : ''}`,
  );

  onMount(async () => {
    if (focus === 'updates') {
      await tick();
      updatesSection?.scrollIntoView({ block: 'start' });
    }
  });

  function startEditing(what: 'key' | 'profile' | 'token') {
    editing = editing === what ? 'none' : what;
    error = '';
    saved = '';
  }

  async function setTheme(theme: Theme) {
    try {
      onchange(await api.updatePreferences({ theme }));
    } catch (e) {
      error = errorText(e);
    }
  }

  async function toggleAutoUpdate() {
    try {
      onchange(await api.updatePreferences({ auto_update: !settings.auto_update }));
    } catch (e) {
      updateError = errorText(e);
    }
  }

  async function toggleFamily() {
    try {
      onchange(await api.updatePreferences({ include_family: !settings.include_family }));
    } catch (e) {
      error = errorText(e);
    }
  }

  async function toggleKeepSet() {
    try {
      onchange(await api.updatePreferences({ keep_card_set: !settings.keep_card_set }));
    } catch (e) {
      error = errorText(e);
    }
  }

  let priceBusy = $state(false);

  let shared = $state<SharedPrices | null>(null);
  let tokenInput = $state('');
  let sharedBusy = $state(false);
  let sharedError = $state('');
  let sharedSaved = $state('');

  onMount(() => {
    api.sharedPrices().then((s) => (shared = s)).catch(() => {});
  });

  async function sharedAction(run: () => Promise<void>) {
    sharedBusy = true;
    sharedError = '';
    sharedSaved = '';
    try {
      await run();
      shared = await api.sharedPrices();
    } catch (e) {
      sharedError = errorText(e);
    } finally {
      sharedBusy = false;
    }
  }

  const toggleShared = () =>
    sharedAction(async () => onchange(await api.updatePreferences({ shared_prices: !settings.shared_prices })));

  const saveToken = (e: SubmitEvent) => {
    e.preventDefault();
    return sharedAction(async () => {
      onchange(await api.saveGithubToken(tokenInput));
      tokenInput = '';
      editing = 'none';
      sharedSaved = 'Token saved. Your app will publish prices after its next scan.';
    });
  };

  const removeToken = () => sharedAction(async () => onchange(await api.clearGithubToken()));

  const publishNow = () =>
    sharedAction(async () => {
      shared = await api.publishPricesNow();
      sharedSaved = 'Prices published.';
    });

  /** What the "Use shared prices" row says. */
  function sharedText(s: SharedPrices | null): string {
    if (settings.publishing_prices) return 'This copy publishes the shared prices, so it reads its own from Steam.';
    if (!settings.shared_prices) return 'Off. Every price is read from Steam.';
    const u = s?.used;
    if (u && u.games > 0) {
      return `${num(u.games)} games' prices came from the shared file made ${agoUnix(u.made_at)}, so Steam was only asked about the rest.`;
    }
    if (s?.checked_at) return 'Checked. The shared file had nothing newer for your games.';
    return 'Games the shared file covers get their prices in seconds instead of from Steam. Checked at the start of each scan.';
  }

  async function setPrices(patch: Parameters<typeof api.updatePriceSettings>[0]) {
    priceBusy = true;
    error = '';
    try {
      onchange(await api.updatePriceSettings(patch));
    } catch (e) {
      error = errorText(e);
    } finally {
      priceBusy = false;
    }
  }

  async function saveKey(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    error = '';
    try {
      onchange(await api.saveApiKey(keyInput));
      keyInput = '';
      editing = 'none';
      saved = 'API key saved.';
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }

  async function saveProfile(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    error = '';
    try {
      const check = await api.setProfile(profileInput);
      onchange({ ...settings, profile: check.profile });
      profileInput = '';
      editing = 'none';
      saved = check.warnings[0] ?? 'Profile updated. Scan again to refresh your numbers.';
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }

  function signedIn(result: SignInResult) {
    onchange(result.settings);
    error = '';
    saved = result.switched_profile
      ? `Signed in. The app now tracks ${result.settings.profile?.persona_name ?? 'that account'}. Scan again to refresh your numbers.`
      : 'Signed in. Scan again to read your inventory.';
  }

  async function signOut() {
    busy = true;
    error = '';
    try {
      onchange(await api.signOut());
      saved = 'Signed out of Steam.';
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function checkUpdates() {
    checking = true;
    updateMessage = '';
    updateError = '';
    try {
      const u = await api.checkForUpdates();
      if (u.error) updateError = u.error;
      else if (u.ready) updateMessage = `Version ${u.latest} is downloaded. Restart the app to finish updating.`;
      else if (u.available) updateMessage = `Version ${u.latest} is available.`;
      else updateMessage = "You're on the newest version.";
    } catch (e) {
      updateError = errorText(e);
    } finally {
      checking = false;
    }
  }

  async function installLatest() {
    installing = true;
    updateError = '';
    try {
      await api.installUpdate();
    } catch (e) {
      updateError = errorText(e);
      installing = false;
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') onclose();
  }
</script>

<svelte:window {onkeydown} />

<div class="scrim" onclick={onclose} role="presentation"></div>
<aside class="drawer" role="dialog" aria-modal="true" aria-label="Settings">
  <header class="drawer-head">
    <h2>Settings</h2>
    <button class="btn ghost icon-btn" onclick={onclose} aria-label="Close settings">
      <Icon name="x" size={18} />
    </button>
  </header>

  <div class="drawer-body">
    <section class="setting-group">
      <h3>Appearance</h3>
      <div class="theme-picker" role="radiogroup" aria-label="Theme">
        {#each themes as t (t.value)}
          <button
            class="theme-swatch"
            data-theme={t.value}
            role="radio"
            aria-checked={settings.theme === t.value}
            onclick={() => setTheme(t.value)}
          >
            <span class="swatch-preview" aria-hidden="true">
              <span class="swatch-bar"></span>
              <span class="swatch-card"><span></span><span></span></span>
              <span class="swatch-card"><span></span><span></span></span>
            </span>
            <span class="swatch-label">{t.label}</span>
          </button>
        {/each}
      </div>
    </section>

    <section class="setting-group">
      <h3>Interface</h3>
      <div class="setting-row">
        <div>
          <div>Open on</div>
          <div class="desc">The tab the app shows when it starts.</div>
        </div>
        <select
          class="filter-select"
          value={prefs.startView}
          onchange={(e) => setPref({ startView: (e.currentTarget as HTMLSelectElement).value as Prefs['startView'] })}
        >
          <option value="overview">Dashboard</option>
          <option value="games">Games</option>
          <option value="items">Items</option>
        </select>
      </div>
      <div class="setting-row">
        <div>
          <div>Show a game's items as</div>
          <div class="desc">A list with prices, or a grid of their art.</div>
        </div>
        <div class="segmented two" role="radiogroup" aria-label="Show a game's items as">
          <button role="radio" aria-checked={prefs.gameItems === 'list'} onclick={() => setPref({ gameItems: 'list' })}>List</button>
          <button role="radio" aria-checked={prefs.gameItems === 'grid'} onclick={() => setPref({ gameItems: 'grid' })}>Art</button>
        </div>
      </div>
      <details class="shortcuts">
        <summary>Keyboard shortcuts</summary>
        <dl>
          {#each SHORTCUTS as [keys, what] (keys)}
            <dt>{#each keys.split(' or ') as k, n (k)}{#if n > 0}<span class="muted"> or </span>{/if}<kbd>{k}</kbd>{/each}</dt>
            <dd>{what}</dd>
          {/each}
        </dl>
      </details>
    </section>

    <section class="setting-group">
      <h3>Steam account</h3>
      <div class="setting-row">
        <div>
          <div>{settings.profile?.persona_name ?? 'No profile'}</div>
          <div class="desc">
            {settings.profile ? `Steam ID ${settings.profile.steam_id}` : 'Add your profile to scan it.'}
          </div>
        </div>
        <button class="btn small" onclick={() => startEditing('profile')}>Change</button>
      </div>
      {#if editing === 'profile'}
        <form class="inline-form" onsubmit={saveProfile}>
          <input
            bind:value={profileInput}
            placeholder="Profile link or custom URL name"
            spellcheck="false"
            autocomplete="off"
          />
          <button class="btn small primary" type="submit" disabled={busy}>
            {busy ? 'Looking it up…' : 'Save'}
          </button>
        </form>
      {/if}

      <div class="setting-row">
        <div>
          <div>Steam sign-in</div>
          <div class="desc">
            {settings.signed_in
              ? 'Signed in. The app can read your inventory, prices and Points Shop items.'
              : 'Steam only shares your inventory, prices and Points Shop items with apps that are signed in.'}
          </div>
        </div>
        {#if settings.signed_in}
          <button class="btn small" onclick={signOut} disabled={busy}>Sign out</button>
        {/if}
      </div>
      {#if !settings.signed_in}
        <SignInButton small onsignedin={signedIn} />
      {/if}

      <div class="setting-row">
        <div>
          <div>API key</div>
          <div class="desc">
            {settings.has_api_key ? `Saved, ends in ${settings.api_key_hint}` : 'Not saved'}
          </div>
        </div>
        <button class="btn small" onclick={() => startEditing('key')}>Replace</button>
      </div>
      {#if editing === 'key'}
        <form class="inline-form" onsubmit={saveKey}>
          <input
            type="password"
            bind:value={keyInput}
            placeholder="New API key"
            spellcheck="false"
            autocomplete="off"
          />
          <button class="btn small primary" type="submit" disabled={busy}>
            {busy ? 'Checking…' : 'Save'}
          </button>
        </form>
      {/if}

      {#if error}<p class="field-error" role="alert">{error}</p>{/if}
      {#if saved}<p class="desc good">{saved}</p>{/if}
    </section>

    <section class="setting-group">
      <h3>Collection</h3>
      <div class="setting-row">
        <div>
          <div>Include Steam Family games</div>
          <div class="desc">
            {settings.signed_in
              ? 'Lists games other members of your Steam Family share with you and counts them toward your account figure. Scan again to read their items.'
              : 'Lists games your Steam Family shares with you and counts them toward your account figure. Needs Steam sign-in.'}
          </div>
        </div>
        <button
          class="switch"
          role="switch"
          aria-checked={settings.include_family}
          aria-label="Include Steam Family games"
          onclick={toggleFamily}
        ></button>
      </div>
      <div class="setting-row">
        <div>
          <div>Also need the cards in my inventory</div>
          <div class="desc">
            {settings.keep_card_set
              ? "A game's cards count as complete when you hold one of each card on top of a level 5 badge and the foil badge."
              : "A badge counts as owning its cards. A game's cards are complete at badge level 5 plus the foil badge."}
          </div>
        </div>
        <button
          class="switch"
          role="switch"
          aria-checked={settings.keep_card_set}
          aria-label="Also need the cards in my inventory"
          onclick={toggleKeepSet}
        ></button>
      </div>
    </section>

    <section class="setting-group">
      <h3>Prices</h3>
      <div class="setting-row">
        <div>
          <div>Currency</div>
          <div class="desc">
            Steam reports prices in your account's currency. Other currencies are converted with daily exchange rates.
          </div>
        </div>
        <select
          class="mini-select setting-select"
          aria-label="Currency"
          value={settings.currency}
          disabled={priceBusy}
          onchange={(e) => setPrices({ currency: (e.currentTarget as HTMLSelectElement).value })}
        >
          {#each currencies as c (c.code)}
            <option value={c.code}>{c.label}</option>
          {/each}
        </select>
      </div>
      <div class="setting-row">
        <div>
          <div>Average price over</div>
          <div class="desc">The span used for each item's average price, and for spotting spikes and dips.</div>
        </div>
        <select
          class="mini-select setting-select"
          aria-label="Average price over"
          value={settings.price_window}
          disabled={priceBusy}
          onchange={(e) => setPrices({ price_window: (e.currentTarget as HTMLSelectElement).value as PriceWindow })}
        >
          {#each windows as w (w.value)}
            <option value={w.value}>{w.label}</option>
          {/each}
        </select>
      </div>
      <div class="setting-row">
        <div>
          <div>Spike and dip sensitivity</div>
          <div class="desc">
            An item is tagged Price spike or Price dip when today's cheapest listing is this far from its average,
            and by at least the amount on the right, so cheap items don't flicker.
          </div>
        </div>
        <div class="setting-pair">
          <select
            class="mini-select"
            aria-label="Percent from the average"
            value={settings.spike_percent}
            disabled={priceBusy}
            onchange={(e) => setPrices({ spike_percent: Number((e.currentTarget as HTMLSelectElement).value) })}
          >
            {#each percents as p (p)}
              <option value={p}>{p}%</option>
            {/each}
          </select>
          <select
            class="mini-select"
            aria-label="Least change"
            value={settings.spike_min_cents}
            disabled={priceBusy}
            onchange={(e) => setPrices({ spike_min_cents: Number((e.currentTarget as HTMLSelectElement).value) })}
          >
            {#each minimums as m (m)}
              <option value={m}>{m === 0 ? 'any amount' : `${m}¢ or more`}</option>
            {/each}
          </select>
        </div>
      </div>
      <div class="setting-row">
        <div>
          <div>Use shared prices</div>
          <div class="desc">{sharedText(shared)}</div>
        </div>
        <button
          class="switch"
          role="switch"
          aria-checked={settings.shared_prices}
          aria-label="Use shared prices"
          disabled={sharedBusy || settings.publishing_prices}
          onclick={toggleShared}
        ></button>
      </div>
      {#if shared?.repo}
        <div class="setting-row">
          <div>
            <div>Publish prices for friends</div>
            <div class="desc">
              {#if settings.publishing_prices && shared.published}
                Published {agoUnix(shared.published.at)}: {num(shared.published.items)} items from
                {num(shared.published.games)} games, {bytes(shared.published.bytes)}. Publishes once a day after a scan.
              {:else if settings.publishing_prices}
                Token saved, ends in {settings.github_token_hint}. Publishes once a day after a scan.
              {:else}
                For the app's owner: a GitHub token with Contents: Read and write on {shared.repo} only.
              {/if}
            </div>
          </div>
          {#if settings.publishing_prices}
            <div class="setting-pair">
              <button class="btn small" onclick={publishNow} disabled={sharedBusy}>
                {sharedBusy ? 'Publishing…' : 'Publish now'}
              </button>
              <button class="btn small" onclick={removeToken} disabled={sharedBusy}>Remove token</button>
            </div>
          {:else}
            <button class="btn small" onclick={() => startEditing('token')}>Add token</button>
          {/if}
        </div>
        {#if editing === 'token'}
          <form class="inline-form" onsubmit={saveToken}>
            <input
              type="password"
              bind:value={tokenInput}
              placeholder="github_pat_…"
              spellcheck="false"
              autocomplete="off"
            />
            <button class="btn small primary" type="submit" disabled={sharedBusy}>
              {sharedBusy ? 'Checking…' : 'Save'}
            </button>
          </form>
        {/if}
      {/if}
      {#if sharedError || shared?.error}
        <p class="field-error" role="alert">{sharedError || shared?.error}</p>
      {/if}
      {#if sharedSaved}<p class="desc good">{sharedSaved}</p>{/if}
    </section>

    <section class="setting-group" bind:this={updatesSection}>
      <h3>Updates</h3>
      <div class="setting-row">
        <div>
          <div>{versionText}</div>
          {#if !info.updates_enabled}
            <div class="desc">Updates are turned off in this copy of the app.</div>
          {:else if updateMessage}
            <div class="desc">{updateMessage}</div>
          {/if}
        </div>
        <button class="btn small" onclick={checkUpdates} disabled={checking || !info.updates_enabled}>
          {checking ? 'Checking…' : 'Check for updates'}
        </button>
      </div>

      {#if update?.ready}
        <div class="update-callout">
          <p>Version {update.latest} is downloaded. Restart the app to start using it.</p>
          <button class="btn small primary" onclick={installLatest} disabled={installing}>
            {installing ? 'Restarting…' : 'Restart now'}
          </button>
        </div>
      {:else if update?.available}
        <div class="update-callout">
          <p>Version {update.latest} is available. The app restarts by itself once it's installed.</p>
          <button class="btn small primary" onclick={installLatest} disabled={installing}>
            {installing ? 'Installing…' : 'Update now'}
          </button>
        </div>
      {/if}
      {#if updateError}<p class="field-error" role="alert">{updateError}</p>{/if}

      <div class="setting-row">
        <div>
          <div>Install updates automatically</div>
          <div class="desc">New versions download in the background and apply the next time the app opens.</div>
        </div>
        <button
          class="switch"
          role="switch"
          aria-checked={settings.auto_update}
          aria-label="Install updates automatically"
          onclick={toggleAutoUpdate}
        ></button>
      </div>

      {#if info.updates_enabled}
        <VersionHistory />
      {/if}
    </section>

    <section class="setting-group">
      <h3>Storage</h3>
      <p class="desc">
        {info.portable
          ? `Settings and saved data use ${bytes(info.data_size_bytes)} in the data folder next to the app. Old reports, update files and web caches are cleaned up automatically.`
          : "The app's own folder can't be written to, so data is saved here instead:"}
      </p>
      <p class="path selectable">{info.data_dir}</p>
    </section>
  </div>
</aside>
