<script lang="ts">
  import { onMount, tick } from 'svelte';
  import {
    api,
    errorText,
    on,
    type AppInfo,
    type ProfileView,
    type SettingsView,
    type UpdateState,
  } from './lib/api';
  import Setup from './lib/Setup.svelte';
  import Overview from './lib/Overview.svelte';
  import Games from './lib/Games.svelte';
  import Items from './lib/Items.svelte';
  import GameDetail from './lib/GameDetail.svelte';
  import ItemPanel from './lib/ItemPanel.svelte';
  import SearchPalette from './lib/SearchPalette.svelte';
  import SettingsPanel from './lib/SettingsPanel.svelte';
  import UpdateBar from './lib/UpdateBar.svelte';
  import Icon from './lib/Icon.svelte';
  import { closeTop, dataChanged, nav, typing, type View } from './lib/nav.svelte.ts';

  let settings = $state<SettingsView | null>(null);
  let info = $state<AppInfo | null>(null);
  let update = $state<UpdateState | null>(null);
  let settingsOpen = $state(false);
  let settingsFocus = $state<'updates' | null>(null);
  let fatal = $state('');
  // The Items list stays open in the background once shown, keeping its place.
  let itemsShown = $state(false);

  const VIEWS: { value: View; label: string }[] = [
    { value: 'overview', label: 'Overview' },
    { value: 'games', label: 'Games' },
    { value: 'items', label: 'Items' },
  ];

  // Figures change when these settings do, so the lists read them again.
  const dataKey = $derived(
    settings
      ? `${settings.include_family}-${settings.keep_card_set}-${settings.currency}-${settings.price_window}-${settings.spike_percent}-${settings.spike_min_cents}`
      : '',
  );

  $effect(() => {
    if (nav.view === 'items') itemsShown = true;
  });

  // Ctrl+K or / searches everything; Esc closes the topmost panel.
  function onkeydown(e: KeyboardEvent) {
    if (!settings || needsSetup || settingsOpen) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      nav.searchOpen = !nav.searchOpen;
    } else if (e.key === '/' && !typing(e) && !nav.searchOpen) {
      e.preventDefault();
      nav.searchOpen = true;
    } else if (e.key === 'Escape' && !nav.searchOpen) {
      if (closeTop()) e.preventDefault();
    }
  }

  const updateWaiting = $derived(!!update && (update.ready || update.available));

  function openSettings(focus: 'updates' | null = null) {
    settingsFocus = focus;
    settingsOpen = true;
  }

  const needsSetup = $derived(
    settings !== null && (!settings.has_api_key || settings.profile === null),
  );

  $effect(() => {
    document.documentElement.dataset.theme = settings?.theme ?? 'greyscale';
  });

  function profileChanged(profile: ProfileView) {
    if (settings) settings = { ...settings, profile };
  }

  onMount(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const [s, i, u] = await Promise.all([api.settings(), api.appInfo(), api.updateInfo()]);
        settings = s;
        info = i;
        update = u;
      } catch (e) {
        fatal = errorText(e);
      }
      await tick();
      requestAnimationFrame(() => {
        api.appReady().catch(() => {});
      });
      // Steam's web pages expect a browser; the app's requests use WebView2's own identity.
      api.reportBrowserAgent(navigator.userAgent).catch(() => {});
      unlisten = await on<UpdateState>('update-status', (u) => (update = u));
    })();
    return () => unlisten?.();
  });
</script>

<svelte:window {onkeydown} />

{#if fatal}
  <main class="fatal">
    <h1>The app couldn't start</h1>
    <p>{fatal}</p>
  </main>
{:else if settings && info}
  {#if needsSetup}
    <Setup {settings} oncomplete={(s) => (settings = s)} />
  {:else}
    <div class="shell">
      <header class="topbar">
        <div class="topbar-left">
          <div class="brand"><span class="mark" aria-hidden="true"></span>Steam Collection Tracker</div>
          <nav class="tabs" aria-label="Views">
            {#each VIEWS as v (v.value)}
              <button aria-current={nav.view === v.value ? 'page' : undefined} onclick={() => (nav.view = v.value)}>
                {v.label}
              </button>
            {/each}
          </nav>
        </div>
        <div class="topbar-right">
          <button class="search-trigger" onclick={() => (nav.searchOpen = true)} title="Search games and items">
            <Icon name="search" size={15} />
            <span>Search</span>
            <kbd>Ctrl K</kbd>
          </button>
          {#if settings.profile}
            <div class="who">
              {#if settings.profile.avatar_url}
                <img src={settings.profile.avatar_url} alt="" />
              {/if}
              <span>{settings.profile.persona_name}</span>
            </div>
          {/if}
          <button
            class="btn ghost small version-chip"
            class:waiting={updateWaiting}
            onclick={() => openSettings('updates')}
            title="Updates and version history"
          >
            {#if updateWaiting}<span class="update-dot" aria-hidden="true"></span>Update available{:else}v{info.version}{/if}
          </button>
          <button
            class="btn ghost icon-btn"
            onclick={() => openSettings()}
            aria-label="Settings"
            title="Settings"
          >
            <Icon name="settings" size={18} />
          </button>
        </div>
      </header>
      {#if update}
        <UpdateBar {update} />
      {/if}
      <main class="content">
        <div class:hidden={nav.view !== 'overview'}>
          <Overview
            signedIn={settings.signed_in}
            includeFamily={settings.include_family}
            keepCardSet={settings.keep_card_set}
            priceKey={dataKey}
            onprofile={profileChanged}
            onsettings={(s) => (settings = s)}
          />
        </div>
        {#if nav.view === 'games'}
          {#key dataKey}
            <Games />
          {/key}
        {/if}
        {#if itemsShown}
          <div class="fill" class:hidden={nav.view !== 'items'}>
            <Items {dataKey} />
          </div>
        {/if}
      </main>
    </div>
    {#if nav.game}
      {#key nav.game.appid}
        <GameDetail
          game={nav.game}
          steamId={settings.profile?.steam_id ?? null}
          onclose={() => (nav.game = null)}
          onchange={dataChanged}
        />
      {/key}
    {/if}
    {#if nav.item}
      <ItemPanel itemKey={nav.item} onclose={() => (nav.item = null)} />
    {/if}
    {#if nav.searchOpen}
      <SearchPalette onclose={() => (nav.searchOpen = false)} />
    {/if}
  {/if}
  {#if settingsOpen}
    <SettingsPanel
      {settings}
      {info}
      {update}
      focus={settingsFocus}
      onclose={() => (settingsOpen = false)}
      onchange={(s) => (settings = s)}
    />
  {/if}
{/if}
