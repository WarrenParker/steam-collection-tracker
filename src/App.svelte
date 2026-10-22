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
  import SettingsPanel from './lib/SettingsPanel.svelte';
  import UpdateBar from './lib/UpdateBar.svelte';
  import Icon from './lib/Icon.svelte';

  let settings = $state<SettingsView | null>(null);
  let info = $state<AppInfo | null>(null);
  let update = $state<UpdateState | null>(null);
  let settingsOpen = $state(false);
  let fatal = $state('');

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
        <div class="brand"><span class="mark" aria-hidden="true"></span>Steam Collection Tracker</div>
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
            class="btn ghost icon-btn"
            onclick={() => (settingsOpen = true)}
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
        <Overview onprofile={profileChanged} />
      </main>
    </div>
  {/if}
  {#if settingsOpen}
    <SettingsPanel
      {settings}
      {info}
      {update}
      onclose={() => (settingsOpen = false)}
      onchange={(s) => (settings = s)}
    />
  {/if}
{/if}
