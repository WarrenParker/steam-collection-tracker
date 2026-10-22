<script lang="ts">
  import { onMount, tick } from 'svelte';
  import {
    api,
    errorText,
    type AppInfo,
    type SettingsView,
    type SignInResult,
    type Theme,
    type UpdateState,
  } from './api';
  import { bytes } from './format';
  import Icon from './Icon.svelte';
  import SignInButton from './SignInButton.svelte';
  import VersionHistory from './VersionHistory.svelte';

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
