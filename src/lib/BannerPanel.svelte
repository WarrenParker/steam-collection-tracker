<script lang="ts">
  import { onMount } from 'svelte';
  import { api, errorText, type BannerData, type Theme } from './api';
  import { localDay, money, num, pct, points } from './format';
  import Icon from './Icon.svelte';

  let { onclose }: { onclose: () => void } = $props();

  type Shape = 'wide' | 'square';
  const SHAPES: { value: Shape; label: string; w: number; h: number }[] = [
    { value: 'wide', label: 'Wide · 1500 × 500', w: 1500, h: 500 },
    { value: 'square', label: 'Square · 1080 × 1080', w: 1080, h: 1080 },
  ];
  const THEMES: { value: Theme; label: string }[] = [
    { value: 'greyscale', label: 'Greyscale' },
    { value: 'dark', label: 'Dark' },
    { value: 'light', label: 'Light' },
  ];
  const FOIL = ['#b9c7ff', '#f0c3f7', '#bff3e6', '#fbe7b0'];
  const DISPLAY = '"Segoe UI Variable Display", "Segoe UI", system-ui, sans-serif';
  const TEXT = '"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif';

  let data = $state<BannerData | null>(null);
  let shape = $state<Shape>('wide');
  let theme = $state<Theme>((document.documentElement.dataset.theme as Theme) || 'greyscale');
  let canvas = $state<HTMLCanvasElement | null>(null);
  let error = $state('');
  let message = $state('');
  let saving = $state(false);
  let images = new Map<string, HTMLImageElement>();

  onMount(() => {
    api
      .banner()
      .then(async (d) => {
        const urls = [d.avatar, ...d.games.map((g) => g.art)].filter((u): u is string => !!u);
        await Promise.all(
          urls.map(async (u) => {
            const img = new Image();
            img.src = u;
            try {
              await img.decode();
              images.set(u, img);
            } catch {
              /* drawn without it */
            }
          }),
        );
        data = d;
      })
      .catch((e) => (error = errorText(e)));
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') onclose();
  }

  /** The chosen theme's colours, read from the app's own design tokens. */
  function palette(t: Theme) {
    const probe = document.createElement('div');
    probe.dataset.theme = t;
    probe.style.display = 'none';
    document.body.appendChild(probe);
    const css = getComputedStyle(probe);
    const v = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
    const p = {
      bg: v('--bg', '#262626'),
      surface: v('--surface', '#2d2d2d'),
      line: v('--line', '#3f3f3f'),
      text: v('--text', '#ececec'),
      text2: v('--text-2', '#b3b3b3'),
      text3: v('--text-3', '#8c8c8c'),
      accent: v('--accent', '#ececec'),
    };
    probe.remove();
    return p;
  }

  function fit(ctx: CanvasRenderingContext2D, text: string, width: number): string {
    if (ctx.measureText(text).width <= width) return text;
    let s = text;
    while (s.length > 1 && ctx.measureText(`${s}…`).width > width) s = s.slice(0, -1);
    return `${s.trimEnd()}…`;
  }

  function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
    ctx.beginPath();
    ctx.roundRect(x, y, w, h, r);
  }

  function foil(ctx: CanvasRenderingContext2D, x: number, w: number): CanvasGradient {
    const g = ctx.createLinearGradient(x, 0, x + w, 0);
    FOIL.forEach((c, i) => g.addColorStop(i / (FOIL.length - 1), c));
    return g;
  }

  function bar(ctx: CanvasRenderingContext2D, c: ReturnType<typeof palette>, x: number, y: number, w: number, h: number, percent: number) {
    roundRect(ctx, x, y, w, h, h / 2);
    ctx.fillStyle = c.line;
    ctx.fill();
    const filled = Math.max(0, Math.min(1, percent / 100)) * w;
    if (filled > 0) {
      roundRect(ctx, x, y, Math.max(filled, h), h, h / 2);
      ctx.fillStyle = foil(ctx, x, w);
      ctx.fill();
    }
  }

  function picture(ctx: CanvasRenderingContext2D, url: string | null, x: number, y: number, w: number, h: number, r: number, c: ReturnType<typeof palette>, fallback: string) {
    ctx.save();
    roundRect(ctx, x, y, w, h, r);
    ctx.clip();
    const img = url ? images.get(url) : undefined;
    if (img) {
      // Cover the box, cropping the longer side.
      const scale = Math.max(w / img.naturalWidth, h / img.naturalHeight);
      const iw = img.naturalWidth * scale;
      const ih = img.naturalHeight * scale;
      ctx.drawImage(img, x + (w - iw) / 2, y + (h - ih) / 2, iw, ih);
    } else {
      ctx.fillStyle = c.surface;
      ctx.fillRect(x, y, w, h);
      ctx.fillStyle = c.text3;
      ctx.font = `600 ${Math.round(h * 0.4)}px ${DISPLAY}`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(fallback.slice(0, 1).toUpperCase(), x + w / 2, y + h / 2);
      ctx.textAlign = 'left';
      ctx.textBaseline = 'alphabetic';
    }
    ctx.restore();
  }

  function stats(d: BannerData): { label: string; value: string }[] {
    return [
      { label: 'Value', value: d.priced ? money(d.value, d.currency) : '–' },
      { label: 'To complete', value: d.priced ? money(d.cost, d.currency) : '–' },
      { label: 'Items owned', value: num(d.items_owned) },
      { label: 'Badges', value: num(d.badges) },
      { label: 'Achievements', value: d.ach_average === null ? '–' : pct(d.ach_average) },
      { label: 'Perfect games', value: num(d.ach_perfect) },
    ];
  }

  function draw() {
    if (!canvas || !data) return;
    const d = data;
    const s = SHAPES.find((x) => x.value === shape) ?? SHAPES[0];
    canvas.width = s.w;
    canvas.height = s.h;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    const c = palette(theme);
    ctx.fillStyle = c.bg;
    ctx.fillRect(0, 0, s.w, s.h);
    ctx.fillStyle = foil(ctx, 0, s.w);
    ctx.fillRect(0, 0, s.w, 8);

    const wide = shape === 'wide';
    const pad = wide ? 56 : 64;
    const today = localDay(Date.now() / 1000);

    // Who.
    const av = wide ? 112 : 128;
    picture(ctx, d.avatar, pad, pad, av, av, 18, c, d.name);
    const nx = pad + av + 28;
    ctx.fillStyle = c.text;
    ctx.font = `600 ${wide ? 44 : 52}px ${DISPLAY}`;
    ctx.fillText(fit(ctx, d.name, (wide ? 820 : s.w - pad) - nx), nx, pad + (wide ? 50 : 58));
    ctx.fillStyle = c.text3;
    ctx.font = `400 ${wide ? 22 : 26}px ${TEXT}`;
    ctx.fillText(`Steam collection · ${today}`, nx, pad + (wide ? 90 : 104));

    // Completion.
    const left = wide ? 800 : s.w - 2 * pad;
    const bigY = wide ? 292 : 400;
    ctx.fillStyle = c.text;
    ctx.font = `600 ${wide ? 112 : 160}px ${DISPLAY}`;
    const big = pct(d.percent);
    ctx.fillText(big, pad - 4, bigY);
    const bw = ctx.measureText(big).width;
    ctx.fillStyle = c.text2;
    ctx.font = `500 ${wide ? 28 : 34}px ${TEXT}`;
    ctx.fillText('complete', pad + bw + 16, bigY);
    ctx.font = `400 ${wide ? 22 : 26}px ${TEXT}`;
    ctx.fillStyle = c.text2;
    const sub = `${num(d.complete)} of ${num(d.total)} items · average game ${pct(d.game_average)} · ${num(d.card_sets_complete)} card sets done`;
    ctx.fillText(fit(ctx, sub, left), pad, bigY + (wide ? 42 : 52));
    bar(ctx, c, pad, bigY + (wide ? 64 : 80), left, wide ? 14 : 16, d.percent ?? 0);

    // Figures.
    const list = stats(d);
    if (wide) {
      const y = 430;
      const cell = left / list.length;
      list.forEach((st, i) => {
        const x = pad + i * cell;
        ctx.fillStyle = c.text3;
        ctx.font = `500 17px ${TEXT}`;
        ctx.fillText(st.label, x, y);
        ctx.fillStyle = c.text;
        ctx.font = `600 27px ${DISPLAY}`;
        ctx.fillText(fit(ctx, st.value, cell - 12), x, y + 36);
      });
    } else {
      const y0 = 590;
      const cell = left / 3;
      list.forEach((st, i) => {
        const x = pad + (i % 3) * cell;
        const y = y0 + Math.floor(i / 3) * 100;
        ctx.fillStyle = c.text3;
        ctx.font = `500 22px ${TEXT}`;
        ctx.fillText(st.label, x, y);
        ctx.fillStyle = c.text;
        ctx.font = `600 38px ${DISPLAY}`;
        ctx.fillText(fit(ctx, st.value, cell - 16), x, y + 46);
      });
    }
    if (d.priced && d.cost_points > 0) {
      ctx.fillStyle = c.text3;
      ctx.font = `400 ${wide ? 15 : 20}px ${TEXT}`;
      const note = `To complete also needs ${points(d.cost_points)}.`;
      ctx.fillText(note, pad, wide ? s.h - 18 : 812);
    }

    // Closest to complete.
    const games = d.games.slice(0, 3);
    ctx.fillStyle = c.text3;
    if (wide) {
      const gx = 900;
      const gw = s.w - pad - gx;
      ctx.font = `500 18px ${TEXT}`;
      ctx.fillText('Closest to complete', gx, pad + 18);
      if (!games.length) {
        ctx.font = `400 20px ${TEXT}`;
        ctx.fillText('Games show here once their collections are started.', gx, pad + 60);
      }
      games.forEach((g, i) => {
        const y = pad + 38 + i * 118;
        const aw = 214;
        const ah = 100;
        picture(ctx, g.art, gx, y, aw, ah, 10, c, g.name);
        const tx = gx + aw + 22;
        ctx.fillStyle = c.text;
        ctx.font = `600 22px ${DISPLAY}`;
        ctx.fillText(fit(ctx, g.name, gw - aw - 22), tx, y + 34);
        ctx.fillStyle = c.text2;
        ctx.font = `500 20px ${TEXT}`;
        ctx.fillText(`${pct(g.percent)} complete`, tx, y + 66);
        bar(ctx, c, tx, y + 82, gw - aw - 22, 8, g.percent ?? 0);
      });
    } else {
      const gy = 870;
      ctx.font = `500 22px ${TEXT}`;
      ctx.fillText('Closest to complete', pad, gy);
      if (!games.length) {
        ctx.font = `400 24px ${TEXT}`;
        ctx.fillText('Games show here once their collections are started.', pad, gy + 50);
      }
      const gap = 24;
      const tw = (left - 2 * gap) / 3;
      games.forEach((g, i) => {
        const x = pad + i * (tw + gap);
        const th = Math.round(tw * (215 / 460));
        picture(ctx, g.art, x, gy + 20, tw, th, 12, c, g.name);
        ctx.fillStyle = c.text;
        ctx.font = `600 22px ${DISPLAY}`;
        ctx.fillText(fit(ctx, g.name, tw), x, gy + 20 + th + 34);
        bar(ctx, c, x, gy + 20 + th + 48, tw - 70, 8, g.percent ?? 0);
        ctx.fillStyle = c.text2;
        ctx.font = `500 18px ${TEXT}`;
        ctx.textAlign = 'right';
        ctx.fillText(pct(g.percent), x + tw, gy + 20 + th + 57);
        ctx.textAlign = 'left';
      });
    }

    // Signature.
    ctx.fillStyle = c.text3;
    ctx.font = `500 ${wide ? 15 : 18}px ${TEXT}`;
    ctx.textAlign = 'right';
    ctx.fillText('Steam Collection Tracker', s.w - pad, s.h - (wide ? 18 : 28));
    ctx.textAlign = 'left';
  }

  $effect(() => {
    // Redraw when the data, shape or theme change.
    void [data, shape, theme, canvas];
    draw();
  });

  async function save() {
    if (!canvas) return;
    saving = true;
    error = '';
    message = '';
    try {
      const path = await api.saveBanner(canvas.toDataURL('image/png'), shape);
      message = `Saved to ${path}`;
    } catch (e) {
      error = errorText(e);
    } finally {
      saving = false;
    }
  }

  async function copy() {
    if (!canvas) return;
    error = '';
    message = '';
    try {
      const blob = await new Promise<Blob>((ok, fail) =>
        canvas!.toBlob((b) => (b ? ok(b) : fail(new Error('no image'))), 'image/png'),
      );
      await navigator.clipboard.write([new ClipboardItem({ 'image/png': blob })]);
      message = 'Copied. Paste it into a chat or post.';
    } catch {
      error = "The banner couldn't be copied. Save it instead.";
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="scrim" onclick={onclose} role="presentation"></div>
<aside class="drawer wide" role="dialog" aria-modal="true" aria-label="Banner">
  <header class="drawer-head">
    <h2>Banner</h2>
    <button class="btn ghost icon-btn" onclick={onclose} aria-label="Close">
      <Icon name="x" size={18} />
    </button>
  </header>

  <div class="drawer-body">
    <p class="desc">An image of your progress to share. Pick a shape and theme, then save it or copy it.</p>
    <div class="banner-options">
      <div class="segmented banner-seg" role="radiogroup" aria-label="Shape">
        {#each SHAPES as s (s.value)}
          <button role="radio" aria-checked={shape === s.value} onclick={() => (shape = s.value)}>{s.label}</button>
        {/each}
      </div>
      <div class="segmented banner-seg" role="radiogroup" aria-label="Theme">
        {#each THEMES as t (t.value)}
          <button role="radio" aria-checked={theme === t.value} onclick={() => (theme = t.value)}>{t.label}</button>
        {/each}
      </div>
    </div>

    <div class="banner-preview" class:square={shape === 'square'}>
      {#if data}
        <canvas bind:this={canvas} aria-label="Banner preview"></canvas>
      {:else if !error}
        <p class="desc">Getting your figures and pictures…</p>
      {/if}
    </div>

    <div class="actions">
      <button class="btn primary" onclick={save} disabled={!data || saving}>
        <Icon name="folder" />
        {saving ? 'Saving…' : 'Save PNG'}
      </button>
      <button class="btn" onclick={copy} disabled={!data}><Icon name="copy" /> Copy image</button>
    </div>
    {#if error}<p class="field-error" role="alert">{error}</p>{/if}
    {#if message}<p class="desc good">{message}</p>{/if}
  </div>
</aside>

<style>
  .banner-options {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin: 12px 0;
  }

  .banner-seg {
    grid-template-columns: none;
    grid-auto-flow: column;
  }

  .banner-seg button {
    padding: 0 12px;
    font-size: 12.5px;
  }

  .banner-preview {
    margin-bottom: 14px;
    border: 1px solid var(--line);
    border-radius: 10px;
    overflow: hidden;
    background: var(--sunken);
  }

  .banner-preview.square {
    max-width: 520px;
  }

  .banner-preview canvas {
    display: block;
    width: 100%;
    height: auto;
  }

  .banner-preview p {
    padding: 40px 16px;
    text-align: center;
  }
</style>
