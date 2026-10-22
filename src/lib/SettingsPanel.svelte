<script lang="ts">
  import {
    api,
    errorText,
    type AppInfo,
    type SettingsView,
    type Theme,
    type UpdateState,
  } from './api';
  import Icon from './Icon.svelte';

  let {
    settings,
    info,
    update,
    onclose,
    onchange,
  }: {
    settings: SettingsView;
    info: AppInfo;
    update: UpdateState | null;
    onclose: () => void;
    onchange: (s: SettingsView) => void;
  } = $props();

  const themes: { value: Theme; label: string }[] = [
    { value: 'greyscale', label: 'Greyscale' },
    { value: 'dark', label: 'Dark' },
    { value: 'light', label: 'Light' },
  ];

  let editing = $state<'none' | 'key' | 'profile'>('none');
  let keyInput = $state('');
  let profileInput = $state('');
  let busy = $state(false);
  let error = $state('');
  let saved = $state('');
  let checking = $state(false);
  let updateMessage = $state('');

  const updateText = $derived(
    !info.updates_enabled
      ? 'Updates are turned off in this copy of the app.'
      : updateMessage || update?.error || '',
  );

  function startEditing(what: 'key' | 'profile') {
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
      error = errorText(e);
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

  async function checkUpdates() {
    checking = true;
    updateMessage = '';
    try {
      const u = await api.checkForUpdates();
      if (u.error) updateMessage = u.error;
      else if (u.ready) updateMessage = `Version ${u.latest} is ready. Restart the app to finish updating.`;
      else if (u.available) updateMessage = `Version ${u.latest} is available.`;
      else updateMessage = "You're on the newest version.";
    } catch (e) {
      updateMessage = errorText(e);
    } finally {
      checking = false;
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
      <div class="segmented" role="radiogroup" aria-label="Theme">
        {#each themes as t (t.value)}
          <button role="radio" aria-checked={settings.theme === t.value} onclick={() => setTheme(t.value)}>
            {t.label}
          </button>
        {/each}
      </div>
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
      <h3>Updates</h3>
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
      <div class="setting-row">
        <div>
          <div>Version {info.version}</div>
          {#if updateText}<div class="desc">{updateText}</div>{/if}
        </div>
        <button class="btn small" onclick={checkUpdates} disabled={checking || !info.updates_enabled}>
          {checking ? 'Checking…' : 'Check now'}
        </button>
      </div>
    </section>

    <section class="setting-group">
      <h3>Storage</h3>
      <p class="desc">
        {info.portable
          ? 'Settings and saved data live in the data folder next to the app.'
          : "The app's own folder can't be written to, so data is saved here instead:"}
      </p>
      <p class="path selectable">{info.data_dir}</p>
    </section>
  </div>
</aside>
