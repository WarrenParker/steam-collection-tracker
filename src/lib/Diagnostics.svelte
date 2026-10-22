<script lang="ts">
  import { onMount } from 'svelte';
  import { api, errorText, on, type DiagStep, type DiagnosticsResult } from './api';
  import Icon from './Icon.svelte';

  let steps = $state<DiagStep[]>([]);
  let running = $state(false);
  let result = $state<DiagnosticsResult | null>(null);
  let error = $state('');
  let note = $state('');
  let copied = $state(false);

  const started = $derived(steps.some((s) => s.status !== 'pending'));

  onMount(() => {
    const unlisteners: Array<() => void> = [];
    api.diagnosticsPlan().then((p) => (steps = p)).catch(() => {});
    on<DiagStep>('diag-progress', (s) => {
      steps = steps.map((x) => (x.key === s.key ? s : x));
      note = '';
    }).then((u) => unlisteners.push(u));
    on<string>('diag-note', (m) => (note = m)).then((u) => unlisteners.push(u));
    return () => unlisteners.forEach((u) => u());
  });

  async function run() {
    running = true;
    error = '';
    result = null;
    copied = false;
    try {
      steps = await api.diagnosticsPlan();
      result = await api.runDiagnostics();
      steps = result.steps;
    } catch (e) {
      error = errorText(e);
    } finally {
      running = false;
      note = '';
    }
  }

  async function showFile() {
    if (!result) return;
    try {
      await api.revealReport(result.report_path);
    } catch (e) {
      error = errorText(e);
    }
  }

  async function copyReport() {
    if (!result) return;
    try {
      const text = await api.readReport(result.report_path);
      await navigator.clipboard.writeText(text);
      copied = true;
      setTimeout(() => (copied = false), 2000);
    } catch {
      error = 'Copying didn\'t work. Use "Show report file" instead.';
    }
  }

  function seconds(ms: number): string {
    return ms ? `${(ms / 1000).toFixed(1)} s` : '';
  }
</script>

<section class="diag">
  <div class="diag-head">
    <div>
      <h2>Test the Steam connection</h2>
      <p>
        Checks each Steam source the app uses and saves a report. It sends about a dozen requests,
        spaced out to stay well within Steam's limits, and takes under a minute. Attach the report
        when you report a problem.
      </p>
    </div>
    <button class="btn" onclick={run} disabled={running}>
      {running ? 'Testing…' : result ? 'Run the test again' : 'Run the test'}
    </button>
  </div>

  {#if started}
    <ul class="steps">
      {#each steps as s (s.key)}
        <li>
          <span class="status-icon {s.status}">
            {#if s.status === 'running'}
              <span class="spinner"></span>
            {:else if s.status === 'ok'}
              <Icon name="check" />
            {:else if s.status === 'warn'}
              <Icon name="alert" />
            {:else if s.status === 'fail'}
              <Icon name="x" />
            {:else}
              <span class="dot"></span>
            {/if}
          </span>
          <span class="step-label">{s.label}</span>
          <span class="step-summary selectable">{s.summary}</span>
          <span class="ms">{seconds(s.elapsed_ms)}</span>
        </li>
      {/each}
    </ul>
  {/if}

  {#if note}<p class="progress-text" role="status">{note}</p>{/if}
  {#if error}
    <div class="notice bad" role="alert"><Icon name="alert" /><p>{error}</p></div>
  {/if}

  {#if result}
    <div class="diag-result">
      <p>{result.estimate}</p>
      <div class="actions">
        <button class="btn" onclick={showFile}><Icon name="folder" /> Show report file</button>
        <button class="btn ghost" onclick={copyReport}>
          <Icon name="copy" />
          {copied ? 'Copied' : 'Copy report'}
        </button>
      </div>
    </div>
  {/if}
</section>
