<script lang="ts">
  import { api, errorText, links, type ProfileCheck, type SettingsView } from './api';
  import Icon from './Icon.svelte';

  let { settings, oncomplete }: { settings: SettingsView; oncomplete: (s: SettingsView) => void } =
    $props();

  // Snapshot of the settings at the start of setup; updated as each step saves.
  let current = $state<SettingsView>({ ...settings });
  let step = $state(settings.has_api_key ? 2 : 1);
  let keyInput = $state('');
  let reveal = $state(false);
  let profileInput = $state('');
  let busy = $state(false);
  let error = $state('');
  let check = $state<ProfileCheck | null>(null);

  const steps = [
    { n: 1, label: 'API key' },
    { n: 2, label: 'Your profile' },
    { n: 3, label: 'Ready' },
  ];

  async function saveKey(e: SubmitEvent) {
    e.preventDefault();
    if (!keyInput.trim()) {
      error = 'Paste your API key first.';
      return;
    }
    busy = true;
    error = '';
    try {
      current = await api.saveApiKey(keyInput);
      keyInput = '';
      step = 2;
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }

  async function saveProfile(e: SubmitEvent) {
    e.preventDefault();
    if (!profileInput.trim()) {
      error = 'Paste your profile link first.';
      return;
    }
    busy = true;
    error = '';
    try {
      check = await api.setProfile(profileInput);
      current = { ...current, profile: check.profile };
      step = 3;
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }

  function goTo(n: number) {
    error = '';
    step = n;
  }
</script>

<div class="setup">
  <aside class="setup-rail">
    <div class="brand"><span class="mark" aria-hidden="true"></span>Steam Collection Tracker</div>
    <ol class="setup-steps">
      {#each steps as s (s.n)}
        <li class:done={step > s.n} class:active={step === s.n}>
          <span class="step-num">
            {#if step > s.n}<Icon name="check" size={12} />{:else}{s.n}{/if}
          </span>
          {s.label}
        </li>
      {/each}
    </ol>
    <p class="rail-note">Everything the app saves stays in its own folder on this PC.</p>
  </aside>

  <section class="setup-main">
    {#if step === 1}
      <h1>Connect a Steam API key</h1>
      <p class="lede">
        The app reads your games and badges through Steam's official API. Each person uses their own
        free key. It's stored encrypted on this PC and never shared.
      </p>
      <ol class="howto">
        <li>
          Open Steam's key page and sign in.
          <button class="link" type="button" onclick={() => api.openLink(links.apiKey)}>
            Open the key page
          </button>
        </li>
        <li>When it asks for a domain name, type <code>localhost</code>, agree to the terms and register.</li>
        <li>Copy the key it shows you and paste it below.</li>
      </ol>
      <p class="fine">Steam only gives keys to accounts that have spent at least $5 on Steam.</p>

      <form onsubmit={saveKey}>
        <label for="api-key">API key</label>
        <div class="input-row">
          <input
            id="api-key"
            type={reveal ? 'text' : 'password'}
            bind:value={keyInput}
            autocomplete="off"
            spellcheck="false"
            placeholder="32 letters and numbers"
          />
          <button type="button" class="btn ghost" onclick={() => (reveal = !reveal)}>
            {reveal ? 'Hide' : 'Show'}
          </button>
        </div>
        {#if error}<p class="field-error" role="alert">{error}</p>{/if}
        <div class="actions">
          <button class="btn primary" type="submit" disabled={busy}>
            {busy ? 'Checking the key…' : 'Save key'}
          </button>
        </div>
      </form>
    {:else if step === 2}
      <h1>Find your profile</h1>
      <p class="lede">
        Paste your Steam profile link. Your custom URL name or 17-digit Steam ID works too.
      </p>
      <form onsubmit={saveProfile}>
        <label for="profile">Profile</label>
        <input
          id="profile"
          bind:value={profileInput}
          autocomplete="off"
          spellcheck="false"
          placeholder="steamcommunity.com/id/yourname"
        />
        {#if error}<p class="field-error" role="alert">{error}</p>{/if}
        <div class="actions">
          <button class="btn primary" type="submit" disabled={busy}>
            {busy ? 'Looking it up…' : 'Find profile'}
          </button>
          <button class="btn ghost" type="button" onclick={() => goTo(1)}>Change API key</button>
        </div>
      </form>
    {:else if check}
      <h1>You're set up</h1>
      <div class="profile-card">
        {#if check.profile.avatar_url}<img src={check.profile.avatar_url} alt="" />{/if}
        <div>
          <div class="name">{check.profile.persona_name}</div>
          <div class="sub">{check.profile.is_public ? 'Public profile' : 'Private profile'}</div>
        </div>
      </div>
      {#each check.warnings as warning}
        <div class="notice warn">
          <Icon name="alert" />
          <div>
            <p>{warning}</p>
            <button class="link" type="button" onclick={() => api.openLink(links.privacy)}>
              Open Steam's privacy settings
            </button>
          </div>
        </div>
      {/each}
      <p class="lede">Next, scan your profile from the overview. The first scan takes about a minute.</p>
      <div class="actions">
        <button class="btn primary" type="button" onclick={() => oncomplete(current)}>Go to overview</button>
        <button class="btn ghost" type="button" onclick={() => goTo(2)}>Use a different profile</button>
      </div>
    {/if}
  </section>
</div>
