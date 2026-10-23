import type { Group } from './api';

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

const moneyFormats = new Map<string, Intl.NumberFormat>();

/** The kinds of item, in the order lists show them. */
export const GROUP_ORDER: Group[] = ['cards', 'foil_cards', 'emoticons', 'backgrounds', 'points_shop', 'boosters'];

export const GROUP_LABELS: Record<Group, string> = {
  cards: 'Cards',
  foil_cards: 'Foil cards',
  emoticons: 'Emoticons',
  backgrounds: 'Backgrounds',
  points_shop: 'Points Shop items',
  boosters: 'Booster packs',
};

/** How much of an item the user has: "3 of 5", "2 owned", "Owned", "Complete" or "". */
export function holding(i: { have: number; needed: number; held: number }): string {
  if (i.needed > 1) return `${i.have} of ${i.needed}`;
  if (i.held > 1) return `${num(i.held)} owned`;
  if (i.held > 0) return 'Owned';
  // A foil card counts through the foil badge too.
  return i.have >= i.needed ? 'Complete' : '';
}

/** Money from mills (thousandths of the main unit), like "$12.34". */
export function money(mills: number | null | undefined, currency: string): string {
  if (mills === null || mills === undefined) return '–';
  let f = moneyFormats.get(currency);
  if (!f) {
    try {
      f = new Intl.NumberFormat('en-US', { style: 'currency', currency });
    } catch {
      f = new Intl.NumberFormat('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 });
    }
    moneyFormats.set(currency, f);
  }
  return f.format(mills / 1000);
}

/** "12,500 points". */
export function points(n: number): string {
  return `${num(n)} ${n === 1 ? 'point' : 'points'}`;
}

/** "+32%" or "−18%". */
export function change(pctValue: number | null | undefined): string {
  if (pctValue === null || pctValue === undefined) return '';
  const rounded = Math.round(pctValue);
  return `${rounded > 0 ? '+' : rounded < 0 ? '−' : ''}${Math.abs(rounded)}%`;
}

/** "Mar 4, 2021" for a Unix time. */
export function day(unixSeconds: number | null | undefined): string {
  if (!unixSeconds) return '';
  return new Date(unixSeconds * 1000).toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
    timeZone: 'UTC',
  });
}

/** "3 hours ago" for a Unix time. */
export function agoUnix(unixSeconds: number | null | undefined): string {
  if (!unixSeconds) return '';
  return ago(new Date(unixSeconds * 1000).toISOString());
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
