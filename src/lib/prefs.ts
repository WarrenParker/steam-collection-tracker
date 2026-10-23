import type { View } from './nav.svelte.ts';

/**
 * Interface choices kept in the app window's own storage: where the app
 * opens, and how a game's items are shown. Losing them only resets the
 * defaults, so reads and writes never fail.
 */
export interface Prefs {
  startView: View;
  gameItems: 'list' | 'grid';
}

const KEY = 'interface-prefs';
const DEFAULTS: Prefs = { startView: 'overview', gameItems: 'list' };

export function loadPrefs(): Prefs {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? '{}');
    // Before 0.7.0 the item layout was kept on its own.
    const layout = localStorage.getItem('game-items-layout');
    return {
      startView: ['overview', 'games', 'items'].includes(saved.startView) ? saved.startView : DEFAULTS.startView,
      gameItems: saved.gameItems === 'grid' || (!saved.gameItems && layout === 'grid') ? 'grid' : 'list',
    };
  } catch {
    return { ...DEFAULTS };
  }
}

export function savePrefs(patch: Partial<Prefs>): Prefs {
  const next = { ...loadPrefs(), ...patch };
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    // Only a convenience.
  }
  return next;
}
