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

export interface SettingsView {
  theme: Theme;
  auto_update: boolean;
  currency: string;
  profile: ProfileView | null;
  has_api_key: boolean;
  api_key_hint: string | null;
  signed_in: boolean;
  include_family: boolean;
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
  updatePreferences: (patch: { theme?: Theme; auto_update?: boolean; include_family?: boolean }) =>
    invoke<SettingsView>('update_preferences', { patch }),
  saveApiKey: (key: string) => invoke<SettingsView>('save_api_key', { key }),
  setProfile: (input: string) => invoke<ProfileCheck>('set_profile', { input }),
  startScan: (resume = false) => invoke<ScanStatus>('start_scan', { resume }),
  pauseScan: () => invoke<ScanStatus>('pause_scan'),
  scanStatus: () => invoke<ScanStatus>('get_scan_status'),
  lastScan: () => invoke<ScanSummary | null>('get_last_scan'),
  games: () => invoke<GameRow[]>('get_games'),
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
};
