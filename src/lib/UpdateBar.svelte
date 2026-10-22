<script lang="ts">
  import { api, errorText, type UpdateState } from './api';

  let { update }: { update: UpdateState } = $props();
  let busy = $state(false);
  let error = $state('');

  const visible = $derived(update.ready || (update.available && !update.checking));

  async function install() {
    busy = true;
    error = '';
    try {
      await api.installUpdate();
    } catch (e) {
      error = errorText(e);
      busy = false;
    }
  }
</script>

{#if visible}
  <div class="updatebar" role="status">
    <span>
      {update.ready
        ? `Version ${update.latest} is ready. Restart the app to finish updating.`
        : `Version ${update.latest} is available.`}
    </span>
    {#if error}
      <span class="updatebar-error">{error}</span>
    {/if}
    <button class="btn small primary" onclick={install} disabled={busy}>
      {busy ? 'Updating…' : update.ready ? 'Restart now' : 'Update now'}
    </button>
  </div>
{/if}
