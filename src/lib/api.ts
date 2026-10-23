import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type Theme = 'greyscale' | 'dark' | 'light';

export interface ProfileView {
  steam_id: string;
  persona_name: string;
  avatar_url: string | null;
  profile_url: string | null;
  is_public: boolean;
}

export type PriceWindow = 'month' | 'quarter' | 'year' | 'all';

export interface SettingsView {
  theme: Theme;
  auto_update: boolean;
  currency: string;
  price_window: PriceWindow;
  spike_percent: number;
  spike_min_cents: number;
  profile: ProfileView | null;
  has_api_key: boolean;
  api_key_hint: string | null;
  signed_in: boolean;
  include_family: boolean;
  keep_card_set: boolean;
}

export interface AppInfo {
  version: string;
  build: string | null;
  data_dir: string;
  data_size_bytes: number;
  portable: boolean;
  updates_enabled: boolean;
}

export interface ProfileCheck {
  profile: ProfileView;
  games_visible: boolean;
  warnings: string[];
}

export interface TypeCount {
  label: string;
  count: number;
  quantity: number;
}

export interface ScanSummary {
  scanned_at: string;
  duration_ms: number;
  requests: number;
  profile: ProfileView;
  games_visible: boolean;
  games_owned: number;
  game_badges: number;
  foil_badges: number;
  other_badges: number;
  player_level: number | null;
  inventory_visible: boolean;
  inventory_items: number;
  foil_cards: number;
  by_type: TypeCount[];
  apps_with_items: number;
  apps_not_owned: number;
  warnings: string[];
  needs_sign_in: boolean;
  inventory_route: string | null;
  family_games: number | null;
  catalog: CatalogCounts | null;
}

export interface KindCount {
  label: string;
  count: number;
}

export interface CatalogCounts {
  games_listed: number;
  games_owned: number;
  games_unowned: number;
  games_family: number;
  games_with_cards: number;
  games_cataloged: number;
  items: number;
  points_items: number;
  by_kind: KindCount[];
  owned_with_items: number;
  ach_games: number;
  ach_started: number;
  ach_perfect: number;
  ach_average: number | null;
  ach_unlocked: number;
  ach_total: number;
}

export type ScanState = 'idle' | 'running' | 'paused' | 'waiting' | 'done' | 'failed';

export interface ScanStatus {
  state: ScanState;
  stage: number;
  label: string;
  done: number;
  total: number;
  seconds_left: number | null;
  remaining: number;
  remaining_seconds: number | null;
  resume_at: number | null;
  prices_remaining: number;
  prices_seconds: number | null;
  message: string | null;
  pause_requested: boolean;
}

export interface GameRow {
  appid: number;
  name: string;
  owned: boolean;
  family: boolean;
  event: boolean;
  has_cards: boolean | null;
  cataloged: boolean;
  cards: number;
  items: number;
  points_items: number;
  your_items: number;
  badge_level: number | null;
  foil_badge: boolean;
  playtime_minutes: number;
  ach_unlocked: number | null;
  ach_total: number | null;
  percent: number | null;
  complete_items: number;
  counted_items: number;
  cards_complete: boolean;
  /** Mills (thousandths) of the shown currency. */
  value: number | null;
  cost: number | null;
  cost_points: number;
  currency: string;
}

export type Availability = 'available' | 'limited' | 'unobtainable';

export type Group = 'cards' | 'foil_cards' | 'emoticons' | 'backgrounds' | 'points_shop' | 'boosters';

export interface ItemView {
  key: string;
  appid: number;
  class: number;
  item_type: number | null;
  foil: boolean;
  group: Group;
  kind: string;
  name: string;
  image: string | null;
  held: number;
  have: number;
  needed: number;
  progress: number;
  counted: boolean;
  points_cost: number | null;
  marketable: boolean;
  availability: Availability;
  auto_availability: Availability;
  note: string | null;
  overridden: boolean;
  tags: string[];
  price: ItemPrice | null;
  sellable: number;
  value: number | null;
  cost: number | null;
  cost_estimated: boolean;
  cost_capped: boolean;
  cost_points: number | null;
  alt_points: number | null;
  unpriced: boolean;
}

/** One row of the all-items list. Prices are in mills. */
export interface ItemRow {
  key: string;
  appid: number;
  group: Group;
  kind: string;
  foil: boolean;
  name: string;
  image: string | null;
  held: number;
  have: number;
  needed: number;
  counted: boolean;
  availability: Availability;
  tags: string[];
  on_market: boolean;
  price: number | null;
  average: number | null;
  change: number | null;
  points_cost: number | null;
  value: number | null;
  cost: number | null;
  cost_points: number | null;
}

export interface GameName {
  appid: number;
  name: string;
  owned: boolean;
  family: boolean;
  event: boolean;
}

export interface ItemsView {
  items: ItemRow[];
  games: GameName[];
  currency: string;
}

export interface ItemDetail {
  item: ItemView;
  game_name: string | null;
  currency: string;
}

/** Prices are in mills: thousandths of the shown currency's main unit. */
export interface ItemPrice {
  hash_name: string;
  on_market: boolean;
  lowest: number | null;
  listings: number;
  average: number | null;
  change: number | null;
  avg_30: number | null;
  avg_90: number | null;
  avg_365: number | null;
  avg_all: number | null;
  low: number | null;
  low_at: number | null;
  high: number | null;
  high_at: number | null;
  sales_30: number | null;
  buy_max: number | null;
  seen_at: number;
  history_at: number | null;
  book_at: number | null;
}

export interface PriceTotals {
  value: number;
  value_copies: number;
  value_unpriced: number;
  points_value: number;
  cost: number;
  cost_items: number;
  cost_estimated: number;
  cost_capped: number;
  cost_points: number;
  cost_points_items: number;
  alt_points: number;
  alt_money: number;
  alt_items: number;
  unpriced: number;
  spikes: number;
  dips: number;
  marketable: number;
  with_history: number;
}

export interface ChartPoint {
  t: number;
  price: number;
  volume: number;
}

export interface PriceHistory {
  points: ChartPoint[];
  full: boolean;
  currency: string;
  read_at: number | null;
}

export interface GroupProgress {
  group: Group;
  label: string;
  owned: number;
  complete: number;
  total: number;
}

export interface GameCompletion {
  appid: number;
  percent: number | null;
  owned: number;
  complete: number;
  total: number;
  groups: GroupProgress[];
  unobtainable: number;
  unobtainable_owned: number;
  limited: number;
  badge_level: number;
  foil_badge: boolean;
  has_cards: boolean;
  cards_complete: boolean;
  counts_for_account: boolean;
  prices: PriceTotals;
}

export interface AccountCompletion {
  percent: number | null;
  owned: number;
  complete: number;
  total: number;
  game_average: number | null;
  games: number;
  games_started: number;
  games_complete: number;
  card_games: number;
  card_sets_complete: number;
  groups: GroupProgress[];
  unobtainable: number;
  unobtainable_owned: number;
  limited: number;
  keep_card_set: boolean;
  include_family: boolean;
  prices: PriceTotals;
  currency: string;
  currency_note: string | null;
  prices_at: number | null;
}

export interface GameDetail {
  completion: GameCompletion | null;
  items: ItemView[];
  currency: string;
}

export type StepStatus = 'pending' | 'running' | 'ok' | 'warn' | 'fail' | 'skipped';

export interface DiagStep {
  key: string;
  label: string;
  status: StepStatus;
  summary: string;
  elapsed_ms: number;
}

export interface DiagnosticsResult {
  report_path: string;
  steps: DiagStep[];
  estimate: string;
  total_seconds: number;
}

export interface UpdateState {
  enabled: boolean;
  current: string;
  latest: string | null;
  available: boolean;
  ready: boolean;
  checking: boolean;
  notes: string | null;
  error: string | null;
}

export interface SignInResult {
  settings: SettingsView;
  switched_profile: boolean;
}

export interface ReleaseInfo {
  version: string;
  notes: string;
  published_at: string | null;
  prerelease: boolean;
  installable: boolean;
  current: boolean;
  newer: boolean;
}

export interface VersionList {
  current: string;
  build: string | null;
  releases: ReleaseInfo[];
}

export const api = {
  appReady: () => invoke<void>('app_ready'),
  reportBrowserAgent: (agent: string) => invoke<void>('report_browser_agent', { agent }),
  appInfo: () => invoke<AppInfo>('get_app_info'),
  settings: () => invoke<SettingsView>('get_settings'),
  updatePreferences: (patch: {
    theme?: Theme;
    auto_update?: boolean;
    include_family?: boolean;
    keep_card_set?: boolean;
  }) => invoke<SettingsView>('update_preferences', { patch }),
  saveApiKey: (key: string) => invoke<SettingsView>('save_api_key', { key }),
  setProfile: (input: string) => invoke<ProfileCheck>('set_profile', { input }),
  startScan: (resume = false) => invoke<ScanStatus>('start_scan', { resume }),
  pauseScan: () => invoke<ScanStatus>('pause_scan'),
  scanStatus: () => invoke<ScanStatus>('get_scan_status'),
  lastScan: () => invoke<ScanSummary | null>('get_last_scan'),
  games: () => invoke<GameRow[]>('get_games'),
  completion: () => invoke<AccountCompletion>('get_completion'),
  gameDetail: (appid: number) => invoke<GameDetail>('get_game_detail', { appid }),
  items: () => invoke<ItemsView>('get_items'),
  item: (key: string) => invoke<ItemDetail>('get_item', { key }),
  setItemAvailability: (appid: number, key: string, availability: Availability | null) =>
    invoke<GameDetail>('set_item_availability', { appid, key, availability }),
  readGameBooks: (appid: number) => invoke<GameDetail>('read_game_books', { appid }),
  priceHistory: (hashName: string, full = false) =>
    invoke<PriceHistory>('get_price_history', { hashName, full }),
  updatePriceSettings: (patch: {
    currency?: string;
    price_window?: PriceWindow;
    spike_percent?: number;
    spike_min_cents?: number;
  }) => invoke<SettingsView>('update_price_settings', { patch }),
  diagnosticsPlan: () => invoke<DiagStep[]>('get_diagnostics_plan'),
  runDiagnostics: () => invoke<DiagnosticsResult>('run_diagnostics'),
  readReport: (path: string) => invoke<string>('read_report', { path }),
  revealReport: (path: string) => invoke<void>('reveal_report', { path }),
  openLink: (url: string) => invoke<void>('open_link', { url }),
  updateInfo: () => invoke<UpdateState>('get_update_info'),
  checkForUpdates: () => invoke<UpdateState>('check_for_updates'),
  installUpdate: () => invoke<void>('install_update'),
  versions: () => invoke<VersionList>('get_versions'),
  installVersion: (version: string) => invoke<void>('install_version', { version }),
  signIn: () => invoke<SignInResult>('sign_in'),
  signOut: () => invoke<SettingsView>('sign_out'),
};

/** Progress reading a game's order books, sent as "book-progress". */
export interface BookProgress {
  appid: number;
  done: number;
  total: number;
}

export function on<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => handler(e.payload));
}

export function errorText(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e && typeof e === 'object' && 'message' in e) return String((e as { message: unknown }).message);
  return 'Something went wrong. Try again.';
}

export const links = {
  apiKey: 'https://steamcommunity.com/dev/apikey',
  privacy: 'https://steamcommunity.com/my/edit/settings',
  store: (appid: number) => `https://store.steampowered.com/app/${appid}/`,
  cards: (steamId: string, appid: number) =>
    `https://steamcommunity.com/profiles/${steamId}/gamecards/${appid}/`,
  pointsShop: (appid: number) => `https://store.steampowered.com/points/shop/app/${appid}`,
  market: (hashName: string) =>
    `https://steamcommunity.com/market/listings/753/${encodeURIComponent(hashName)}`,
};

/** Currencies Steam uses, by code. */
export const CURRENCIES = [
  'USD', 'GBP', 'EUR', 'CHF', 'RUB', 'PLN', 'BRL', 'JPY', 'NOK', 'IDR', 'MYR', 'PHP', 'SGD', 'THB',
  'VND', 'KRW', 'TRY', 'UAH', 'MXN', 'CAD', 'AUD', 'NZD', 'CNY', 'INR', 'CLP', 'PEN', 'COP', 'ZAR',
  'HKD', 'TWD', 'SAR', 'AED', 'SEK', 'ARS', 'ILS', 'BYN', 'KZT', 'KWD', 'QAR', 'CRC', 'UYU', 'BGN',
  'CZK', 'DKK', 'HUF', 'RON',
] as const;
