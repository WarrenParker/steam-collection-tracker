<script lang="ts">
  import { api, errorText, type SignInResult } from './api';

  let {
    onsignedin,
    small = false,
    primary = true,
  }: { onsignedin: (r: SignInResult) => void; small?: boolean; primary?: boolean } = $props();

  let waiting = $state(false);
  let error = $state('');

  async function signIn() {
    waiting = true;
    error = '';
    try {
      onsignedin(await api.signIn());
    } catch (e) {
      const message = errorText(e);
      if (message !== 'Sign-in was cancelled.') error = message;
    } finally {
      waiting = false;
    }
  }
</script>

<div class="signin">
  <button class="btn" class:primary class:small onclick={signIn} disabled={waiting}>
    {waiting ? 'Waiting for you to sign in…' : 'Sign in with Steam'}
  </button>
  {#if waiting}
    <p class="desc">
      Finish in the Steam window that opened. Scanning its QR code with the Steam mobile app means you
      don't have to type your password.
    </p>
  {/if}
  {#if error}<p class="field-error" role="alert">{error}</p>{/if}
</div>
