import { api, type GameRow } from './api';

export type View = 'overview' | 'games' | 'items';

/** A starting filter and sort for the Items list, set from the dashboard. */
export interface ItemsPreset {
  have?: 'all' | 'owned' | 'missing';
  sort?: 'name' | 'game' | 'kind' | 'owned' | 'price' | 'average' | 'change';
  descending?: boolean;
}

/** What's on screen: the view, and the panels open on top of it. */
class Nav {
  view = $state<View>('overview');
  /** The game whose details are open. */
  game = $state<GameRow | null>(null);
  /** The item whose panel is open, by key. */
  item = $state<string | null>(null);
  searchOpen = $state(false);
  itemsPreset = $state<ItemsPreset | null>(null);
  /** Goes up when something the views show was saved, so they read it again. */
  dataVersion = $state(0);
}

export const nav = new Nav();

/** Shows the Items list with a filter and sort, clearing any others. */
export function showItems(preset: ItemsPreset) {
  nav.itemsPreset = preset;
  nav.view = 'items';
}

export function openItem(key: string) {
  nav.item = key;
}

/** Opens a game's details from anywhere, by its id. */
export async function openGame(appid: number) {
  if (nav.game?.appid === appid) return;
  const games = await api.games();
  const game = games.find((g) => g.appid === appid);
  if (game) nav.game = game;
}

export function dataChanged() {
  nav.dataVersion++;
}

/**
 * Closes the topmost panel. Returns false when nothing was open, so the key
 * can do something else.
 */
export function closeTop(): boolean {
  if (nav.searchOpen) nav.searchOpen = false;
  else if (nav.item) nav.item = null;
  else if (nav.game) nav.game = null;
  else return false;
  return true;
}

/** True while the user is typing in a field, so single-key shortcuts stay out of the way. */
export function typing(e: KeyboardEvent): boolean {
  const t = e.target as HTMLElement | null;
  if (!t) return false;
  return t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName);
}
