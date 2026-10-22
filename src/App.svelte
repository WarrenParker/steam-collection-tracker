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
  import SettingsPanel from './lib/SettingsPanel.svelte';
  import UpdateBar from './lib/UpdateBar.svelte';
  import Icon from './lib/Icon.svelte';

  let settings = $state<SettingsView | null>(null);
  let info = $state<AppInfo | null>(null);
  let update = $state<UpdateState | null>(null);
  let settingsOpen = $state(false);
  let settingsFocus = $state<'updates' | null>(null);
  let view = $state<'overview' | 'games'>('overview');
  let fatal = $state('');

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
      unlisten = await on<UpdateState>('update-status', (u) => (update = u));
    })();
    return () => unlisten?.();
  });
</script>

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
            <button aria-current={view === 'overview' ? 'page' : undefined} onclick={() => (view = 'overview')}>
              Overview
            </button>
            <button aria-current={view === 'games' ? 'page' : undefined} onclick={() => (view = 'games')}>
              Games
            </button>
          </nav>
        </div>
        <div class="topbar-right">
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
        <div class:hidden={view !== 'overview'}>
          <Overview
            signedIn={settings.signed_in}
            includeFamily={settings.include_family}
            onprofile={profileChanged}
            onsettings={(s) => (settings = s)}
          />
        </div>
        {#if view === 'games'}
          {#key settings.include_family}
            <Games steamId={settings.profile?.steam_id ?? null} />
          {/key}
        {/if}
      </main>
    </div>
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
