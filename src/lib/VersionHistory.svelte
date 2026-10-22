<script lang="ts">
  import { onMount } from 'svelte';
  import { api, errorText, type ReleaseInfo, type VersionList } from './api';
  import { date, noteBlocks } from './format';

  let list = $state<VersionList | null>(null);
  let loading = $state(true);
  let error = $state('');
  let open = $state<string | null>(null);
  let confirming = $state<string | null>(null);
  let installing = $state<string | null>(null);

  onMount(async () => {
    try {
      list = await api.versions();
      open = list.releases.find((r) => r.newer)?.version ?? list.releases.find((r) => r.current)?.version ?? null;
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  });

  function toggle(version: string) {
    open = open === version ? null : version;
  }

  function ask(r: ReleaseInfo) {
    if (r.newer) install(r.version);
    else confirming = r.version;
  }

  async function install(version: string) {
    confirming = null;
    installing = version;
    error = '';
    try {
      await api.installVersion(version);
    } catch (e) {
      error = errorText(e);
      installing = null;
    }
  }
</script>

<div class="versions">
  <h4>Version history</h4>
  {#if loading}
    <p class="desc">Loading versions…</p>
  {:else if list && list.releases.length}
    <ul>
      {#each list.releases as r (r.version)}
        <li class:current={r.current}>
          <div class="version-row">
            <button class="version-toggle" onclick={() => toggle(r.version)} aria-expanded={open === r.version}>
              <span class="version-number">{r.version}</span>
              {#if r.current}<span class="tag">Installed</span>{/if}
              {#if r.newer}<span class="tag new">New</span>{/if}
              {#if r.prerelease}<span class="tag">Test build</span>{/if}
              <span class="version-date">{date(r.published_at)}</span>
            </button>
            {#if !r.current && r.installable}
              <button
                class="btn small"
                class:primary={r.newer}
                onclick={() => ask(r)}
                disabled={installing !== null}
              >
                {installing === r.version ? 'Installing…' : r.newer ? 'Update' : 'Switch'}
              </button>
            {/if}
          </div>

          {#if confirming === r.version}
            <div class="confirm">
              <p>
                Switch to version {r.version}? Automatic updates will turn off so the app stays on it. You can
                turn them back on here any time.
              </p>
              <div class="actions">
                <button class="btn small primary" onclick={() => install(r.version)}>Switch to {r.version}</button>
                <button class="btn small ghost" onclick={() => (confirming = null)}>Cancel</button>
              </div>
            </div>
          {/if}

          {#if installing === r.version}
            <p class="desc">Downloading version {r.version}. The app will restart by itself when it's ready.</p>
          {/if}

          {#if open === r.version}
            <div class="notes selectable">
              {#each noteBlocks(r.notes || 'No notes for this version.') as block, i (i)}
                {#if block.kind === 'heading'}
                  <p class="notes-heading">{block.text}</p>
                {:else if block.kind === 'item'}
                  <p class="notes-item">{block.text}</p>
                {:else}
                  <p>{block.text}</p>
                {/if}
              {/each}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {:else if !error}
    <p class="desc">No versions have been published yet.</p>
  {/if}
  {#if error}<p class="field-error" role="alert">{error}</p>{/if}
</div>
