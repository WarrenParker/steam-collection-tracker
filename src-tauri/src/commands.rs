//! Everything the interface can ask the engine to do.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State, WebviewWindow};

use crate::completion::{self, AccountCompletion, Availability, GameCompletion, Group, ItemView};
use crate::db::{self, GameRow};
use crate::diagnostics::{self, DiagStep, DiagnosticsResult};
use crate::housekeeping;
use crate::prices::{self, PriceHistory};
use crate::profile_input::{self, ProfileRef};
use crate::scan::{self, ScanStatus, ScanSummary, PRIVATE_GAMES, PRIVATE_PROFILE};
use crate::session;
use crate::settings::{PriceWindow, SettingsView, Theme};
use crate::shared_prices;
use crate::state::AppState;
use crate::steam::ProfileView;
use crate::updater::{self, UpdateState, VersionList};

const BUSY: &str = "The app is already talking to Steam. Wait for that to finish, then try again.";
const SCANNING: &str = "A scan is running. Pause it first, then run the test.";

#[derive(Serialize)]
pub struct AppInfo {
    version: String,
    build: Option<String>,
    data_dir: String,
    data_size_bytes: u64,
    portable: bool,
    updates_enabled: bool,
}

#[derive(Deserialize)]
pub struct PreferencesPatch {
    theme: Option<Theme>,
    auto_update: Option<bool>,
    include_family: Option<bool>,
    keep_card_set: Option<bool>,
    shared_prices: Option<bool>,
}

#[derive(Serialize)]
pub struct ProfileCheck {
    profile: ProfileView,
    games_visible: bool,
    warnings: Vec<String>,
}

/// The interface calls this once it has drawn, so the window never flashes white.
#[tauri::command]
pub fn app_ready(window: WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

/// The interface reports WebView2's browser identity, used for Steam's web pages.
#[tauri::command]
pub fn report_browser_agent(state: State<'_, AppState>, agent: String) {
    state.steam.set_browser_agent(&agent);
}

#[tauri::command]
pub async fn get_app_info(state: State<'_, AppState>) -> Result<AppInfo, String> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        build: updater::build_id(),
        data_dir: state.paths.root.display().to_string(),
        data_size_bytes: housekeeping::folder_size(&state.paths.root),
        portable: state.paths.portable,
        updates_enabled: updater::enabled(),
    })
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> SettingsView {
    state.settings().view()
}

#[tauri::command]
pub fn update_preferences(
    window: WebviewWindow,
    state: State<'_, AppState>,
    patch: PreferencesPatch,
) -> Result<SettingsView, String> {
    let updated = state.update_settings(|s| {
        if let Some(theme) = patch.theme {
            s.theme = theme;
        }
        if let Some(auto) = patch.auto_update {
            s.auto_update = auto;
        }
        if let Some(family) = patch.include_family {
            s.include_family = family;
        }
        if let Some(keep) = patch.keep_card_set {
            s.keep_card_set = keep;
        }
        if let Some(shared) = patch.shared_prices {
            s.shared_prices = shared;
        }
        Ok(())
    })?;
    if patch.shared_prices == Some(true) {
        shared_prices::check_again(&state);
    }
    let _ = window.set_theme(Some(updated.theme.window_theme()));
    Ok(updated.view())
}

#[tauri::command]
pub async fn save_api_key(state: State<'_, AppState>, key: String) -> Result<SettingsView, String> {
    let key: String = key
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    if key.len() != 32 || !key.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(
            "That doesn't look like a Steam API key. A key is 32 letters and numbers, like 1A2B3C4D…"
                .to_string(),
        );
    }
    let quiet = |_: String| {};
    state.steam.check_api_key(&key, &quiet).await?;
    let updated = state.update_settings(|s| s.set_api_key(&key))?;
    Ok(updated.view())
}

/// Saves the GitHub token that publishes the shared price file, after
/// checking it can publish there.
#[tauri::command]
pub async fn save_github_token(
    state: State<'_, AppState>,
    token: String,
) -> Result<SettingsView, String> {
    let token: String = token.chars().filter(|c| !c.is_whitespace()).collect();
    let looks_right = (token.starts_with("github_pat_") || token.starts_with("ghp_"))
        && token.len() <= 255
        && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !looks_right {
        return Err(
            "That doesn't look like a GitHub token. Tokens start with github_pat_ or ghp_."
                .to_string(),
        );
    }
    shared_prices::check_token(state.steam.http(), &token).await?;
    let updated = state.update_settings(|s| s.set_github_token(&token))?;
    Ok(updated.view())
}

#[tauri::command]
pub fn clear_github_token(state: State<'_, AppState>) -> Result<SettingsView, String> {
    let updated = state.update_settings(|s| {
        s.clear_github_token();
        Ok(())
    })?;
    Ok(updated.view())
}

#[tauri::command]
pub fn get_shared_prices(state: State<'_, AppState>) -> shared_prices::Status {
    shared_prices::status(&state)
}

#[tauri::command]
pub async fn publish_prices_now(
    state: State<'_, AppState>,
) -> Result<shared_prices::Status, String> {
    shared_prices::publish_now(&state).await
}

#[tauri::command]
pub async fn set_profile(
    state: State<'_, AppState>,
    input: String,
) -> Result<ProfileCheck, String> {
    let key = state.api_key()?;
    let quiet = |_: String| {};
    let steam_id = match profile_input::parse(&input)? {
        ProfileRef::SteamId(id) => id,
        ProfileRef::Vanity(name) => state.steam.resolve_vanity(&key, &name, &quiet).await?,
    };
    let profile = state.steam.player_summary(&key, &steam_id, &quiet).await?;
    let games = state.steam.owned_games(&key, &steam_id, &quiet).await?;

    let mut warnings = Vec::new();
    if !profile.is_public {
        warnings.push(PRIVATE_PROFILE.to_string());
    } else if !games.visible {
        warnings.push(PRIVATE_GAMES.to_string());
    }

    let saved = profile.clone();
    state.update_settings(move |s| {
        s.profile = Some(saved);
        Ok(())
    })?;
    Ok(ProfileCheck {
        profile,
        games_visible: games.visible,
        warnings,
    })
}

/// Starts a scan, or continues a paused one. Returns right away; progress
/// arrives as `scan-status` events.
#[tauri::command]
pub fn start_scan(app: AppHandle, resume: Option<bool>) -> ScanStatus {
    scan::start(&app, resume.unwrap_or(false))
}

#[tauri::command]
pub fn pause_scan(app: AppHandle) -> ScanStatus {
    scan::pause(&app)
}

#[tauri::command]
pub fn get_scan_status(app: AppHandle) -> ScanStatus {
    scan::status(&app)
}

#[tauri::command]
pub fn get_last_scan(state: State<'_, AppState>) -> Option<ScanSummary> {
    scan::last_summary(&state)
}

fn rules(state: &AppState) -> completion::Rules {
    completion::Rules::from_settings(&state.settings())
}

/// Every listed game: owned, unowned with items, and Steam Family games when
/// that setting is on, with how complete each one is.
#[tauri::command]
pub fn get_games(state: State<'_, AppState>) -> Result<Vec<GameRow>, String> {
    game_rows(&state)
}

fn game_rows(state: &AppState) -> Result<Vec<GameRow>, String> {
    let rules = rules(state);
    let snap = state.snapshot(rules)?;
    state.db.read(|c| {
        let mut games = db::games(c, rules.include_family)?;
        for g in &mut games {
            if let Some(done) = snap.games.get(&g.appid) {
                g.percent = done.percent;
                g.complete_items = done.complete;
                g.counted_items = done.total;
                g.cards_complete = done.cards_complete;
                let p = &done.prices;
                g.value = (p.value_copies > 0).then_some(p.value);
                g.cost = (p.cost_items > 0 || (done.total > 0 && done.complete == done.total))
                    .then_some(p.cost);
                g.cost_points = p.cost_points;
                g.currency = snap.account.currency.clone();
            }
        }
        Ok(games)
    })
}

/// Completion for the whole account.
#[tauri::command]
pub fn get_completion(state: State<'_, AppState>) -> Result<AccountCompletion, String> {
    let rules = rules(&state);
    Ok(state.snapshot(rules)?.account.clone())
}

#[derive(Serialize)]
pub struct GameDetail {
    completion: Option<GameCompletion>,
    items: Vec<ItemView>,
    /// The currency prices are shown in.
    currency: String,
}

fn game_detail(state: &AppState, appid: u64) -> Result<GameDetail, String> {
    let snap = state.snapshot(rules(state))?;
    Ok(GameDetail {
        completion: snap.games.get(&appid).cloned(),
        items: snap.game_items(appid),
        currency: snap.account.currency.clone(),
    })
}

/// One game's items, with what the user owns and each item's tags.
#[tauri::command]
pub fn get_game_detail(state: State<'_, AppState>, appid: u64) -> Result<GameDetail, String> {
    game_detail(&state, appid)
}

/// One row of the all-items list: what the list shows, sorts and filters on.
/// The item panel asks for the full item with `get_item`.
#[derive(Serialize)]
pub struct ItemRow {
    key: String,
    appid: u64,
    group: Group,
    /// What kind of item it is, like "Emoticon" or "Animated avatar".
    kind: &'static str,
    foil: bool,
    name: String,
    image: Option<String>,
    held: u64,
    have: u32,
    needed: u32,
    counted: bool,
    availability: Availability,
    tags: Vec<&'static str>,
    on_market: bool,
    /// Today's cheapest listing, in mills.
    price: Option<i64>,
    average: Option<i64>,
    change: Option<f64>,
    points_cost: Option<i64>,
    value: Option<i64>,
    cost: Option<i64>,
    cost_points: Option<i64>,
}

impl From<&ItemView> for ItemRow {
    fn from(i: &ItemView) -> Self {
        let p = i.price.as_ref();
        Self {
            key: i.key.clone(),
            appid: i.appid,
            group: i.group,
            kind: i.kind,
            foil: i.foil,
            name: i.name.clone(),
            image: i.image.clone(),
            held: i.held,
            have: i.have,
            needed: i.needed,
            counted: i.counted,
            availability: i.availability,
            tags: i.tags.clone(),
            on_market: p.is_some_and(|p| p.on_market),
            price: p.filter(|p| p.on_market).and_then(|p| p.lowest),
            average: p.and_then(|p| p.average),
            change: p.and_then(|p| p.change),
            points_cost: i.points_cost,
            value: i.value,
            cost: i.cost,
            cost_points: i.cost_points,
        }
    }
}

#[derive(Serialize)]
pub struct GameName {
    appid: u64,
    name: String,
    owned: bool,
    family: bool,
    event: bool,
}

#[derive(Serialize)]
pub struct ItemsView {
    items: Vec<ItemRow>,
    /// The games the items belong to, for names and the game filter.
    games: Vec<GameName>,
    currency: String,
}

/// Every item of every listed game, for the all-items list and search.
#[tauri::command]
pub fn get_items(state: State<'_, AppState>) -> Result<ItemsView, String> {
    let rules = rules(&state);
    let snap = state.snapshot(rules)?;
    let games = state.db.read(|c| db::games(c, rules.include_family))?;
    let listed: std::collections::HashSet<u64> = games.iter().map(|g| g.appid).collect();
    Ok(ItemsView {
        items: snap
            .items
            .iter()
            .filter(|i| listed.contains(&i.appid))
            .map(ItemRow::from)
            .collect(),
        games: games
            .into_iter()
            .map(|g| GameName {
                appid: g.appid,
                name: g.name,
                owned: g.owned,
                family: g.family,
                event: g.event,
            })
            .collect(),
        currency: snap.account.currency.clone(),
    })
}

#[derive(Serialize)]
pub struct ItemDetail {
    item: ItemView,
    game_name: Option<String>,
    currency: String,
}

fn item_detail(state: &AppState, key: &str) -> Result<ItemDetail, String> {
    let rules = rules(state);
    let snap = state.snapshot(rules)?;
    let item = snap
        .items
        .iter()
        .find(|i| i.key == key)
        .cloned()
        .ok_or_else(|| "That item isn't in your collection's lists any more.".to_string())?;
    let game_name = state
        .db
        .read(|c| db::games(c, true))?
        .into_iter()
        .find(|g| g.appid == item.appid)
        .map(|g| g.name);
    Ok(ItemDetail {
        item,
        game_name,
        currency: snap.account.currency.clone(),
    })
}

/// An item gained or lost between two scans, for the dashboard.
#[derive(Serialize)]
pub struct DashChange {
    at: i64,
    key: String,
    appid: u64,
    name: String,
    game: String,
    image: Option<String>,
    foil: bool,
    before: u64,
    after: u64,
}

#[derive(Serialize)]
pub struct Dashboard {
    /// Games that count, started but not finished, closest to done first.
    closest: Vec<GameRow>,
    /// Games that count and aren't finished, cheapest to finish first.
    cheapest: Vec<GameRow>,
    /// Items you could sell, listed well above their average now.
    sell: Vec<ItemRow>,
    /// Items you still need, listed well below their average now.
    buy: Vec<ItemRow>,
    changes: Vec<DashChange>,
    /// Names of the games `sell` and `buy` belong to.
    names: std::collections::BTreeMap<u64, String>,
    /// The account's figures, one point a day, for the last 90 days.
    progress: Vec<crate::history::ProgressPoint>,
    currency: String,
}

const DASH_ROWS: usize = 6;

/// Everything the dashboard shows beyond the account's completion.
#[tauri::command]
pub fn get_dashboard(state: State<'_, AppState>) -> Result<Dashboard, String> {
    let rules = rules(&state);
    let snap = state.snapshot(rules)?;
    let games = game_rows(&state)?;
    let names: std::collections::HashMap<u64, &str> =
        games.iter().map(|g| (g.appid, g.name.as_str())).collect();
    let counts = |appid: u64| snap.games.get(&appid).is_some_and(|g| g.counts_for_account);
    let unfinished: Vec<&GameRow> = games
        .iter()
        .filter(|g| counts(g.appid) && g.percent.is_some_and(|p| p < 100.0))
        .collect();

    let mut closest: Vec<GameRow> = unfinished
        .iter()
        .filter(|g| g.percent.is_some_and(|p| p > 0.0))
        .map(|g| (*g).clone())
        .collect();
    closest.sort_by(|a, b| {
        b.percent
            .unwrap_or(0.0)
            .total_cmp(&a.percent.unwrap_or(0.0))
            .then(a.name.cmp(&b.name))
    });
    closest.truncate(DASH_ROWS);

    let mut cheapest: Vec<GameRow> = unfinished
        .iter()
        .filter(|g| g.cost.is_some_and(|c| c > 0))
        .map(|g| (*g).clone())
        .collect();
    cheapest.sort_by_key(|g| (g.cost.unwrap_or(i64::MAX), g.cost_points));
    cheapest.truncate(DASH_ROWS);

    let change = |i: &ItemView| i.price.as_ref().and_then(|p| p.change).unwrap_or(0.0);
    let mut sell: Vec<&ItemView> = snap
        .items
        .iter()
        .filter(|i| i.sellable > 0 && i.tags.contains(&"Price spike"))
        .collect();
    sell.sort_by(|a, b| change(b).total_cmp(&change(a)));
    let mut buy: Vec<&ItemView> = snap
        .items
        .iter()
        .filter(|i| {
            i.counted && i.have < i.needed && counts(i.appid) && i.tags.contains(&"Price dip")
        })
        .collect();
    buy.sort_by(|a, b| change(a).total_cmp(&change(b)));

    let by_key: std::collections::HashMap<&str, &ItemView> =
        snap.items.iter().map(|i| (i.key.as_str(), i)).collect();
    let since = db::now() - 90 * 86_400;
    let (recent, progress) = state.db.read(|c| {
        Ok((
            crate::history::recent_changes(c, 12)?,
            crate::history::progress(c, since)?,
        ))
    })?;
    let changes = recent
        .into_iter()
        .map(|ch| {
            let item = by_key.get(ch.key.as_str());
            DashChange {
                game: names
                    .get(&ch.appid)
                    .map_or_else(|| format!("App {}", ch.appid), |n| n.to_string()),
                image: item.and_then(|i| i.image.clone()),
                foil: item.is_some_and(|i| i.foil),
                at: ch.at,
                key: ch.key,
                appid: ch.appid,
                name: ch.name,
                before: ch.before,
                after: ch.after,
            }
        })
        .collect();

    let sell: Vec<ItemRow> = sell
        .into_iter()
        .take(DASH_ROWS)
        .map(ItemRow::from)
        .collect();
    let buy: Vec<ItemRow> = buy.into_iter().take(DASH_ROWS).map(ItemRow::from).collect();
    let item_names = sell
        .iter()
        .chain(buy.iter())
        .filter_map(|i| names.get(&i.appid).map(|n| (i.appid, n.to_string())))
        .collect();
    Ok(Dashboard {
        sell,
        buy,
        names: item_names,
        closest,
        cheapest,
        changes,
        progress,
        currency: snap.account.currency.clone(),
    })
}

/// One item in full, for the item panel.
#[tauri::command]
pub fn get_item(state: State<'_, AppState>, key: String) -> Result<ItemDetail, String> {
    item_detail(&state, &key)
}

/// Sets an item's availability by hand, or hands it back to the app with `None`.
#[tauri::command]
pub fn set_item_availability(
    state: State<'_, AppState>,
    appid: u64,
    key: String,
    availability: Option<Availability>,
) -> Result<GameDetail, String> {
    if !key.starts_with(&format!("{appid}-")) {
        return Err("That item isn't part of this game.".to_string());
    }
    state
        .db
        .write(|tx| completion::set_override(tx, &key, availability, db::now()))?;
    game_detail(&state, appid)
}

#[derive(Deserialize)]
pub struct PriceSettingsPatch {
    currency: Option<String>,
    price_window: Option<PriceWindow>,
    spike_percent: Option<u32>,
    spike_min_cents: Option<u32>,
}

/// Changes how prices are shown. A new currency reads exchange rates straight
/// away, so prices switch over without a scan.
#[tauri::command]
pub async fn update_price_settings(
    state: State<'_, AppState>,
    patch: PriceSettingsPatch,
) -> Result<SettingsView, String> {
    if let Some(code) = &patch.currency {
        if crate::market::currency_id(code).is_none() {
            return Err("That currency isn't one Steam uses.".to_string());
        }
    }
    let updated = state.update_settings(|s| {
        if let Some(code) = &patch.currency {
            s.currency = code.to_ascii_uppercase();
        }
        if let Some(w) = patch.price_window {
            s.price_window = w;
        }
        if let Some(p) = patch.spike_percent {
            s.spike_percent = p.clamp(5, 500);
        }
        if let Some(c) = patch.spike_min_cents {
            s.spike_min_cents = c.min(10_000);
        }
        Ok(())
    })?;
    if patch.currency.is_some() {
        // The overview explains it if the rates couldn't be read.
        let _ = prices::refresh_rates(&state).await;
    }
    Ok(updated.view())
}

/// Reads the order books of a game's cards that still need several copies,
/// when the game is opened, and returns the game with its updated cost.
/// Progress goes out as "book-progress" events.
#[tauri::command]
pub async fn read_game_books(
    app: AppHandle,
    state: State<'_, AppState>,
    appid: u64,
) -> Result<GameDetail, String> {
    let cookie = match session::current(&app).await {
        Ok(Some(s)) => Some(s.community_header()),
        _ => None,
    };
    let emit = |p: prices::BookProgress| {
        let _ = app.emit("book-progress", p);
    };
    prices::game_books(&state, appid, cookie.as_deref(), &emit).await?;
    game_detail(&state, appid)
}

/// An item's price history: the stored last year (read from Steam the first
/// time), or the whole history read fresh with `full`.
#[tauri::command]
pub async fn get_price_history(
    app: AppHandle,
    state: State<'_, AppState>,
    hash_name: String,
    full: Option<bool>,
) -> Result<PriceHistory, String> {
    let cookie = match session::current(&app).await {
        Ok(Some(s)) => Some(s.community_header()),
        _ => None,
    };
    prices::item_history(&state, &hash_name, full.unwrap_or(false), cookie.as_deref()).await
}

#[tauri::command]
pub fn get_diagnostics_plan() -> Vec<DiagStep> {
    diagnostics::plan()
}

#[tauri::command]
pub async fn run_diagnostics(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DiagnosticsResult, String> {
    if state.scan.is_running() {
        return Err(SCANNING.to_string());
    }
    let _busy = state.try_begin().ok_or_else(|| BUSY.to_string())?;
    let note_app = app.clone();
    let notify = move |message: String| {
        let _ = note_app.emit("diag-note", message);
    };
    let step_app = app.clone();
    let emit_step = move |step: DiagStep| {
        let _ = step_app.emit("diag-progress", step);
    };
    diagnostics::run(&app, &notify, &emit_step).await
}

/// Only files inside the reports folder can be read or shown.
fn report_path(state: &AppState, path: &str) -> Result<PathBuf, String> {
    let dir = state
        .paths
        .reports_dir()
        .canonicalize()
        .map_err(|_| "The reports folder is missing.".to_string())?;
    let requested = Path::new(path)
        .canonicalize()
        .map_err(|_| "That report file no longer exists.".to_string())?;
    if !requested.starts_with(&dir) || !requested.is_file() {
        return Err("Only connection test reports can be opened here.".to_string());
    }
    // Explorer doesn't take Windows' long \\?\ form that canonicalize gives.
    Ok(PathBuf::from(path))
}

#[tauri::command]
pub fn read_report(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let file = report_path(&state, &path)?;
    std::fs::read_to_string(file).map_err(|e| format!("Couldn't read the report: {e}"))
}

#[tauri::command]
pub fn reveal_report(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let file = report_path(&state, &path)?;
    tauri_plugin_opener::reveal_item_in_dir(file)
        .map_err(|e| format!("Couldn't open the folder: {e}"))
}

/// Sites the app opens in the user's browser.
const ALLOWED_HOSTS: &[&str] = &[
    "steamcommunity.com",
    "store.steampowered.com",
    "help.steampowered.com",
    "github.com",
];

/// The link, tidied, when it's an https page on one of the allowed sites
/// with no user name, password or port.
fn allowed_link(url: &str) -> Option<String> {
    let u = reqwest::Url::parse(url.trim()).ok()?;
    let ok = u.scheme() == "https"
        && u.username().is_empty()
        && u.password().is_none()
        && u.port().is_none()
        && u.host_str().is_some_and(|h| ALLOWED_HOSTS.contains(&h));
    ok.then(|| u.to_string())
}

/// Opens a Steam or GitHub page in the user's normal web browser.
#[tauri::command]
pub fn open_link(url: String) -> Result<(), String> {
    let Some(url) = allowed_link(&url) else {
        return Err("That link can't be opened from the app.".to_string());
    };
    tauri_plugin_opener::open_url(&url, None::<&str>)
        .map_err(|e| format!("Couldn't open the link: {e}"))
}

#[tauri::command]
pub fn get_update_info(state: State<'_, AppState>) -> UpdateState {
    updater::snapshot(&state)
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<UpdateState, String> {
    Ok(updater::check(&app, false).await)
}

/// Installs the newest version and restarts the app.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    updater::install_latest_and_restart(&app).await
}

/// Every published version, for the version history.
#[tauri::command]
pub async fn get_versions(app: AppHandle) -> Result<VersionList, String> {
    updater::versions(&app).await
}

/// Installs a specific version (newer or older) and restarts the app.
#[tauri::command]
pub async fn install_version(app: AppHandle, version: String) -> Result<(), String> {
    updater::install_version_and_restart(&app, &version).await
}

#[derive(Serialize)]
pub struct SignInResult {
    settings: SettingsView,
    /// True when signing in switched the tracked profile to a different account.
    switched_profile: bool,
}

/// Opens Steam's sign-in window and waits for the user to sign in.
#[tauri::command]
pub async fn sign_in(app: AppHandle, state: State<'_, AppState>) -> Result<SignInResult, String> {
    let signed = session::sign_in(&app).await?;
    let previous = state.settings().profile.map(|p| p.steam_id);
    let switched_profile =
        previous.is_some() && previous.as_deref() != Some(signed.steam_id.as_str());

    // The app tracks the signed-in account.
    let quiet = |_: String| {};
    let profile = match state.api_key() {
        Ok(key) => state
            .steam
            .player_summary(&key, &signed.steam_id, &quiet)
            .await
            .ok(),
        Err(_) => None,
    };
    let steam_id = signed.steam_id.clone();
    let updated = state.update_settings(move |s| {
        let keep_current = s.profile.as_ref().is_some_and(|p| p.steam_id == steam_id);
        if let Some(p) = profile {
            s.profile = Some(p);
        } else if !keep_current {
            s.profile = Some(ProfileView {
                steam_id: steam_id.clone(),
                persona_name: "Steam user".to_string(),
                avatar_url: None,
                profile_url: None,
                is_public: true,
            });
        }
        s.signed_in_steam_id = Some(steam_id);
        Ok(())
    })?;
    *state.session.lock().await = Some(signed);
    Ok(SignInResult {
        settings: updated.view(),
        switched_profile,
    })
}

/// Signs out of Steam, on Steam's side too.
#[tauri::command]
pub async fn sign_out(app: AppHandle, state: State<'_, AppState>) -> Result<SettingsView, String> {
    let updated = state.update_settings(|s| {
        s.signed_in_steam_id = None;
        Ok(())
    })?;
    session::sign_out(&app).await;
    Ok(updated.view())
}

#[cfg(test)]
mod tests {
    use super::allowed_link;

    #[test]
    fn opens_only_steam_and_github_pages() {
        assert!(allowed_link("https://steamcommunity.com/market/listings/753/570-Foo").is_some());
        assert!(allowed_link("https://store.steampowered.com/points/shop/app/570").is_some());
        assert!(allowed_link("https://github.com/WarrenParker/steam-collection-tracker").is_some());
        for bad in [
            "http://steamcommunity.com/",
            "https://steamcommunity.com.evil.com/",
            "https://evil.com/?https://steamcommunity.com/",
            "https://user:pass@steamcommunity.com/",
            "https://steamcommunity.com:8443/",
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "not a link",
        ] {
            assert!(allowed_link(bad).is_none(), "{bad}");
        }
    }
}
