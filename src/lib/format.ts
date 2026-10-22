const numbers = new Intl.NumberFormat('en-US');

export function num(n: number): string {
  return numbers.format(n);
}

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? '' : 's'}`;
}

export function ago(iso: string): string {
  const seconds = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
  if (seconds < 45) return 'just now';
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${plural(minutes, 'minute')} ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${plural(hours, 'hour')} ago`;
  return `${plural(Math.round(hours / 24), 'day')} ago`;
}

export function duration(ms: number): string {
  const total = Math.round(ms / 1000);
  if (total < 60) return plural(total, 'second');
  const minutes = Math.floor(total / 60);
  const rest = total % 60;
  return rest ? `${plural(minutes, 'minute')} ${plural(rest, 'second')}` : plural(minutes, 'minute');
}

/** "about 3 minutes", for time estimates. */
export function roughly(seconds: number): string {
  if (seconds < 60) return 'under a minute';
  const minutes = Math.ceil(seconds / 60);
  if (minutes < 60) return `about ${plural(minutes, 'minute')}`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest
    ? `about ${plural(hours, 'hour')} ${plural(rest, 'minute')}`
    : `about ${plural(hours, 'hour')}`;
}

/** "3:45 PM" for a Unix time. */
export function clock(unixSeconds: number): string {
  return new Date(unixSeconds * 1000).toLocaleTimeString('en-US', {
    hour: 'numeric',
    minute: '2-digit',
  });
}

export function date(iso: string | null): string {
  if (!iso) return '';
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? ''
    : d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
}

/** "42.5%", or a dash when there's nothing to count. */
export function pct(n: number | null | undefined): string {
  if (n === null || n === undefined) return '–';
  return `${n % 1 === 0 ? n.toFixed(0) : n.toFixed(1)}%`;
}

/** Owned item counts, which can include part of a card. */
export function owned(n: number): string {
  return n % 1 === 0 ? num(n) : n.toLocaleString('en-US', { maximumFractionDigits: 1 });
}

export function bytes(n: number): string {
  if (n < 1024 * 1024) return `${Math.max(1, Math.round(n / 1024))} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export type NoteBlock = { kind: 'heading' | 'item' | 'text'; text: string };

/** Turns simple Markdown release notes into blocks, without rendering HTML. */
export function noteBlocks(markdown: string): NoteBlock[] {
  const clean = (t: string) => t.replace(/\*\*(.+?)\*\*/g, '$1').replace(/`([^`]+)`/g, '$1').trim();
  return markdown
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .map((line): NoteBlock => {
      if (/^#{1,6}\s/.test(line)) return { kind: 'heading', text: clean(line.replace(/^#+\s*/, '')) };
      if (/^[-*]\s/.test(line)) return { kind: 'item', text: clean(line.slice(2)) };
      return { kind: 'text', text: clean(line) };
    });
}
