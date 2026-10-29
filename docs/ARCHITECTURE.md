# How the app is built

A guide to the code, the data folder, and how versions get built and released.

## Stack

- **Engine:** Rust, in `src-tauri/`. Talks to Steam, keeps settings and the collection database, runs scans and tests, and updates the app.
- **Database:** SQLite, through [rusqlite](https://github.com/rusqlite/rusqlite) with SQLite compiled into the exe.
- **Interface:** Svelte 5 and TypeScript, in `src/`. Built with Vite into static files that are embedded in the exe.
- **Shell:** [Tauri 2](https://tauri.app). It opens a native window and draws the interface with WebView2, the Edge engine built into Windows 10 and 11. The result is one exe of about 9 MB.

## Repository layout

```text
.github/workflows/build.yml   Builds and tests every push to main; releases new versions
.github/workflows/tidy-branches.yml  Deletes branches already merged into main
CHANGELOG.md                  Every version's changes; release notes come from here
docs/                         Roadmap, ideas, architecture, Steam data notes
index.html, src/              The interface
  App.svelte                  Top bar and tabs, setup vs main views, settings drawer
  app.css                     Design tokens and themes (greyscale, dark, light)
  lib/api.ts                  Typed calls into the engine
  lib/nav.svelte.ts           What's on screen: the view and the panels open over it
  lib/VirtualList.svelte      Draws only the rows in view, for long lists
  lib/*.svelte                Screens and components
src-tauri/                    The engine
  tauri.conf.json             App name, bundle settings, content security policy
  capabilities/default.json   What the interface window is allowed to do
  icons/                      App icon (generated, see below)
  src/lib.rs                  Startup: tidy-up, settings, main window, update check
  src/commands.rs             Everything the interface can ask the engine to do
  src/steam/                  Paced Steam client, response parsing, inventory reading
  src/catalog/                Reading each item source into one catalog shape
  src/db.rs                   The collection database: schema, saving and queries
  src/completion.rs           Completion, tags, availability, value and cost to complete
  src/prices.rs               The scan's Market stages, price history and exchange rates
  src/shared_prices.rs        The shared price file: downloading, using and publishing it
  src/banner.rs               Pictures for the banner, and saving it
  src/market/                 Market answers, currencies, order books, price history math,
                              and the shared price file's format (shared.rs)
  src/scan.rs                 The background scan, its stages, pausing and estimates
  src/session.rs              Steam sign-in, renewal and sign-out
  src/diagnostics.rs          The connection test and its report
  src/updater.rs              Update checks, version history, installing any version
  src/housekeeping.rs         Automatic tidy-up and folder size
  src/settings.rs             Settings file; the API key is encrypted with secret.rs
  src/secret.rs               Windows DPAPI encryption
  src/paths.rs                Where the data folder is
  src/profile_input.rs        Understands pasted profile links, names and IDs
  src/jsonutil.rs             Reads Steam's mixed number, string and 1/0 values
```

## The data folder

Everything the app writes goes in a `data` folder next to the exe. If that folder can't be written to, for example under Program Files, it uses `%LOCALAPPDATA%\SteamCollectionTracker` instead.

```text
data/
  settings.json        Theme, update choice, profile, encrypted API key
  collection.db        Games, item catalogs, owned items and scan history
  reports/             Connection test reports (the newest 5 are kept)
  update/              Downloads in progress (cleared at every launch)
  webview/             WebView2 profile, including Steam's sign-in cookies
```

At every launch, `housekeeping.rs` clears these:

- leftover update downloads
- exe files left over from an update swap
- old reports
- WebView caches

The sign-in cookies are kept. Version 0.2.0's `cache/last_scan.json` is moved into the database the first time 0.3.0 opens, and the `cache` folder is removed.

## The collection database

`collection.db` holds:

| Table | What's in it |
|---|---|
| `apps` | Every game the app knows: owned, Steam Family, or with items or badges. Store details, whether it has trading cards, and when each source was last read. |
| `items` | Every item in each game's catalog, keyed by game, item class, type number and foil. Each row records whether it came from the item list (`in_catalog`) and whether the Points Shop sells it (`in_points`, with the price in points, availability dates, and `points_buyable`: 1 sold on its own, 2 only in a bundle, 0 neither). |
| `inventory` | One row per kind of item the user holds, with quantities. |
| `profile_items` | Emoticons and profile items from the signed-in user's account, including Points Shop purchases. |
| `badges` | The user's badges and levels. |
| `item_overrides` | Availability the user set by hand, keyed by item. Kept across scans. |
| `market` | One row per item on the Market, keyed by its Market name: the cheapest listing and how many are listed, a summary of its price history (averages over 30 days, 90 days, a year and all time; lowest and highest days; sales in the last 30 days), the last year of history in a compact form, and the order book's cheapest levels. Prices are kept in thousandths of the currency (mills). |
| `scans` | One row per scan, with the overview summary it produced. |
| `meta` | Small values, such as which catalog route works on this connection. |
| `owned_seen` | What the user held after the last scan that read the inventory, item by item, to compare the next scan with. |
| `changes` | Items gained or lost between scans (copies before and after), kept for a year, for the dashboard. |
| `progress` | The account's completion, items owned, value and cost to complete, one row a day, for the dashboard's trends and the progress chart. `items_owned` was added in 0.9.0 without a schema bump, so older versions can still open the database. |

The schema version is kept in SQLite's `user_version`. A database the app can't read, or one written by a newer version, is renamed to `collection.old.db` and a fresh one is started; scanning again refills it.

## The scan

A scan runs in the background in six stages. Progress is saved as it goes, so a scan can be paused, or the app closed, and continued later.

1. **Library:** profile, owned games, badges, inventory, achievement progress and, when signed in, the Steam Family library and profile items.
2. **Store check:** which games have trading cards (the store's category 29) and names for games the user doesn't own. 100 games per request.
3. **Points Shop:** every item each game sells for points, 20 games per request, plus the list of apps that are Steam sale events. Each batch is read twice, with and without items that can't be bought directly, to tell sale rewards from items for sale. Games where the user owns an item no list includes are read again, at most once a day.
4. **Item catalog:** each card game's card set, from its card page. The page numbers cards by their place in the set, so the app takes Steam's real item numbers from any card the user owns. Emoticons and backgrounds come from the Points Shop stage, and every game with cards also gets its booster pack.
5. **Market prices:** each card game that counts for the account, and any game the user holds a sellable item from, searched on the Market ten items per request, with each item's cheapest listing.
Price history isn't read during scans. Steam refused the app's bulk history reads for days at a time (see [STEAM-DATA.md](STEAM-DATA.md)), so an item's history is read when its panel is opened, kept for about a month, and read again the next time it's opened after that. When a due history can't be read, the stored one is shown.

Order books are read when the user opens a game, not during scans: one request per card that still needs two or more copies, which is over a thousand on a large account. Books read in the last 6 days are kept, and one game's books are read at a time.

Rescans only fetch what's missing or out of date: store details after 30 days, Points Shop items after 7, item catalogs after 30, and Market prices after 20 hours. An item's price history falls due about a month after it was read (23 to 37 days, spread by the item's name) and is read again when the item is next opened. A first catalog expected to take more than 10 minutes waits for the user to continue it, with a time estimate. If Steam refuses card page or Market requests, the scan stops, waits (30 minutes, longer after refusals in a row), and carries on by itself while the app is open, or once the wait is over after the app opens again. Only one copy of the app runs at a time; opening it again brings the existing window forward.

## Completion

`completion.rs` works out completion from the database whenever the interface asks; nothing extra is stored.

- **Matching.** Owned items (inventory and profile items) are matched to the catalog by game, item class, Steam's item number and foil. Profile items always carry the number; some inventory items don't, so those fall back to the name. Each game has one booster pack, matched whatever its number. An owned item that matches nothing is added to its game's list, so items Steam no longer lists only show up for people who own them.
- **Cards** count by copies. Each badge level uses one copy of every card, so a card needs 5 copies' worth: the badge level plus copies held. A foil card needs the foil badge or one copy. The "Also need the cards in my inventory" setting adds one copy to each.
- **What counts.** Every catalog item except booster packs and items that are Unobtainable. A game's percentage is the share of its counted items owned, with cards counted by their share of copies.
- **Account.** Games owned, plus Steam Family games when that setting is on. The headline weighs every item equally; the average game weighs every game equally, including games at 0%.
- **Availability.** Cards and booster packs are available. A Points Shop item is Unobtainable when Steam marks it inactive or its end date has passed, and Limited when it has a future end or start date, can't be bought on its own (sale rewards, event items), or belongs to a Steam sale event. Owned items no list includes are Unobtainable once their game's Points Shop has been read. The user's override wins.
- **Tags.** Unowned and Family shared come from the game. Points means the item is sold for points and not on the Market. Not marketable means the game's Market search didn't list it (cards and booster packs always count as marketable, and a copy the user can sell settles it); before a game's first Market search, cards, booster packs and a card set's emoticons and backgrounds are taken to be marketable. Unverified marks items matched by name, and owned items whose game list hasn't been read yet. Price spike and Price dip are below.

## Prices

`completion.rs` also works out prices whenever the interface asks, from the `market` table.

- **Matching.** Market items are tied to catalog items by the Market name of a copy the user holds, then by item number when Steam adds one to tell items with the same name apart ("…(Profile Background)-13"), then by game, kind and name (Steam adds suffixes like "(Profile Background)" to tell items apart; they're dropped). Each game has one booster pack. Emoticons and backgrounds the Market lists but no other source does are added to their game's list.
- **Retired items.** An item Steam no longer hands out counts as available while someone is selling it on the Market, and Unobtainable when nobody is.
- **Current price** is the cheapest listing in the game's latest Market search. Items the latest search didn't list aren't for sale right now.
- **Average** is the volume-weighted average sale price over the span chosen in Settings (30 days by default). **Price spike** and **Price dip** tag items whose cheapest listing is at least 25% above or below that average and by at least 5 cents; both numbers are settings.
- **Account value** counts copies the user can sell, at the cheapest listing (or the average when nothing is listed), before Steam's fees. It covers every listed game and Steam's own items, such as sacks of gems. Items that can't be sold are shown separately as a points value, at their Points Shop price.
- **Cost to complete** covers what each counted item still needs. Items on the Market are priced in money: one copy at the cheapest listing, several copies by walking up the order book when one was read in the last 7 days (opening the game reads it), otherwise at the cheapest listing and marked as estimated. Items only the Points Shop sells are priced in points. Items sold both ways count in money, and the points they'd cost instead are shown alongside. Items nobody sells right now are counted as unpriced.
- **Outliers.** A listing more than three times an item's usual price (its 90-day average sale price, or its 1-year average when nothing sold in 90 days) counts at the usual price in cost to complete and value, and is marked as estimated. A background listed at $876.48 that last sold for $2.26 made up most of a real account's cost to complete before this rule.
- **Price history** is fetched whole (Steam has no way to ask for part of it) when an item is opened, without sign-in, turned into one point per day, summarised, and stored as the last year: daily for 90 days, weekly before that, packed into a few bytes per point. "Show full history" fetches it again and shapes it for a chart (daily for 90 days, weekly for two years, then monthly).
- **Currency.** Steam answers in the account's currency, which the app learns from the Market. Other currencies are converted with daily exchange rates from api.frankfurter.dev, read only when another currency is chosen.

## Shared prices

Reading every game's prices from Steam takes hours for a large library, so one copy of the app (the owner's) publishes what it has read, and every other copy starts from that.

- **The file** is plain text, one record per line, gzip-compressed (about 400 KB for 5,000 items). It lists each game whose Market search finished, with when it started, and every `market` row: current price, price history summary and compact history, and order book. Columns are matched by name, so later versions can add some. The format and its checks are in `market/shared.rs`.
- **Where it lives.** A release tagged `prices` in a separate repository, the app's own with `-prices` added (`WarrenParker/steam-collection-tracker-prices`), as `prices.txt.gz`. A separate repository means the token that publishes it can't change the app's code.
- **Downloading.** At the start of the Market stage, at most every 6 hours, the app downloads the file from GitHub (16 MB cap, 64 MB unpacked, at most 100,000 items and 20,000 games). For the games this user prices, it keeps whichever is newer, its own or the file's, separately for the current price, the price history and the order book. A game whose search in the file is newer than the app's own is marked as searched then, so the scan skips it. Rows in another currency than the user's Market currency are skipped. A copy that publishes doesn't download.
- **Publishing.** When a GitHub token is saved (encrypted with DPAPI, like the API key), the app publishes after a scan's Market stage finishes, at most every 20 hours, or from Settings with Publish now. It makes the release if it's missing, uploads the new file under a temporary name, removes the old one and renames the new one, so the file is only missing for a moment. The token needs Contents: Read and write on the prices repository only.

## Talking to Steam

- All requests go through `SteamClient` in `src/steam/mod.rs`. Each kind of request has a minimum gap:

  | Kind of request | Minimum gap |
  |---|---|
  | Web API | 0.3 s |
  | Community pages | 3.5 s |
  | Inventory | 4 s |
  | Market | 4 s |
  | Store | 1 s |

- Community, inventory, Store and Market requests identify as the browser built into the app (the interface reports WebView2's own identity at startup) and send browser headers. Steam turns the Market away otherwise.
- When Steam refuses a request (HTTP 429), the client stops sending that kind for a while instead of retrying: 30 minutes, then 1, 2 and 4 hours for refusals in a row, back to 30 minutes after an answer. Only Web API calls are retried. The details are in [STEAM-DATA.md](STEAM-DATA.md).
- **Market budget.** On top of the 4-second gap, Market requests draw from a budget (`steam/budget.rs`): a burst of 60 goes out at the normal pace, then about one every 7.5 seconds (480 an hour). A refusal makes the pace half again as slow, up to one every 30 seconds, and every 50 answers in a row speed it back up a step, and the slower pace is saved in the database's `meta` table for later runs, along with the Market's refusals in a row and the end of any wait, so a restart doesn't ask too soon.
- **Sign-in** opens Steam's own sign-in page in a second window, which shares the WebView2 profile. When the `steamLoginSecure` cookie appears, the app reads it through Tauri's cookie API and closes the window.
- **Renewal.** When the cookie is about to expire, the app loads a Steam page in a hidden window, and Steam renews the session itself.
- **Sign-out** loads Steam in a hidden window and posts to `/login/logout/`, so Steam revokes the session on its side too.
- **Web API token.** Calls made as the signed-in user use a Web API token from `pointssummary/ajaxgetasyncconfig`. If that's unavailable, they use the token inside the sign-in cookie.

## Versions, builds and updates

- **Semantic versioning** (MAJOR.MINOR.PATCH). The version is set in `src-tauri/Cargo.toml` and `package.json`, and every version has a section in `CHANGELOG.md`.
- **To release a version:** set the new version in `src-tauri/Cargo.toml` and `package.json`, add a section for it at the top of `CHANGELOG.md`, and push to `main`.
- **Every push to `main`** runs `.github/workflows/build.yml` on a Windows runner: interface build, `cargo test`, `tauri build`, then packaging. Feature branches are built by starting the same workflow on the branch (`gh workflow run build.yml --ref <branch>`); branch builds never publish a release. After every push to `main`, `tidy-branches.yml` deletes branches that are fully merged into it.
  - The build posts a short report (test results, warnings, size) as a check annotation, or the tail of the log if it fails. These can be read through the GitHub API.
  - Pushes that only change `docs/` or `README.md` don't trigger a build.
- **When the version is new,** the workflow publishes a GitHub release tagged `vX.Y.Z`. Its notes are that version's changelog section, and it carries two files: `SteamCollectionTracker.exe` and `SteamCollectionTracker.exe.sha256`. The build stops if the changelog has no entry for the version.
- **The app** checks the latest release when it starts. The repository and build commit are baked in at build time through `UPDATE_REPO` and `GITHUB_SHA`. When it finds a newer version, the app:
  1. downloads it
  2. checks the SHA-256
  3. swaps the exe with `self_replace`
  4. offers "Restart now"

  Settings lists every release and can install any of them. Installing an older version turns automatic updates off.

## Security and privacy

- **API key:** encrypted with Windows DPAPI for the current user. It never reaches the interface; only its last four characters are shown.
- **Steam sign-in:** happens on Steam's own page. The app never reads or stores the password. The cookies live in the WebView2 profile, which Windows encrypts.
- **Reports:** every secret (API key, cookies, tokens) is blanked out before a connection test report is written.
- **External links:** the interface can only open https pages on Steam and GitHub (checked by parsing the address, with no user name, password or port), and only in the user's own browser.
- **Content security policy:** the interface loads nothing from the internet except images from Steam's image servers.
- **Updates:** only downloaded from this repository's own GitHub releases, checked against the release's SHA-256 file, and refused when far larger than the app.
- **Shared prices:** downloaded only from the prices repository's release over https, size-capped before and after unpacking, and checked line by line: unknown formats and newer versions are refused, and a row with any value out of range is dropped whole. Stored price history must decode, and prices in another currency are skipped. The publishing token is encrypted like the API key, never reaches the interface (only its last four characters), and is meant for the prices repository alone.
- **Banner pictures:** only fetched over https from Steam's image servers, at most 4 MB each, and handed to the interface as data URLs. Banners are only saved as PNG, to a folder in the user's Pictures.
- **Dependencies:** `Cargo.lock` and `package-lock.json` are committed, so every build uses the same package versions. Each build runs `cargo audit` and `npm audit` and lists any advisories in the build report. The audit's remaining warnings are for Linux-only crates (`glib`, `zbus`) that the Windows app doesn't include.

## Conventions

- **Messages are plain language.** They say what happened and what to do next, and never show URLs, codes or secrets unless that helps.
- **Commits** describe the change in plain words. Each release is one version bump with a matching changelog entry.
- **Icons** in `src-tauri/icons/` are generated from a script (a stack of cards with a foil front). Regenerate all sizes together if the icon changes.
