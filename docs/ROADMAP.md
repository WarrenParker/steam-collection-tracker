# Roadmap

The plan for Steam Collection Tracker, phase by phase. Finished items link to the version that shipped them in the [changelog](../CHANGELOG.md). Ideas that aren't scheduled yet are in [IDEAS.md](IDEAS.md).

**Now:** version 0.5.0. Phase 2 is done; Phase 3 is next.

## Goal

A lightweight Windows app, one exe in one folder with nothing to install, that loads your own Steam profile and shows how complete your collection is for every game. That covers cards, foil cards, emoticons, backgrounds, badges, Points Shop items and anything else tied to a game. It also shows account value, cost to complete, and price insight.

## Phase 0: Setup and proof (done)

- ✅ Windows build pipeline on GitHub, published releases and a self-updater. [0.1.0](../CHANGELOG.md#010---2026-10-21)
- ✅ First-time setup, overview counts and a connection test that runs on the user's PC. [0.1.0](../CHANGELOG.md#010---2026-10-21)
- ✅ Steam sign-in, moved up from a later phase. The first connection test showed that Steam refuses anonymous inventory and market requests. [0.2.0](../CHANGELOG.md#020---2026-10-21)
- ✅ Updates button, version history and switching, and automatic tidy-up. [0.2.0](../CHANGELOG.md#020---2026-10-21)
- ✅ Confirm the signed-in sources with a 0.2.0 connection test. Inventory, Points Shop items and the Steam Family library all work. The market still refused, which carries over to Phase 1. Results are in [STEAM-DATA.md](STEAM-DATA.md).

## Phase 1: Scanner and local database (done)

- ✅ A working route to the market. Steam was turning away the app's identity, not the connection; the app now identifies as its built-in browser. Market search and price history both work. [0.4.1](../CHANGELOG.md#041---2026-10-22)
- ✅ Full item catalog for every game. [0.3.0](../CHANGELOG.md#030---2026-10-22)
  - cards and foil cards, from each game's card page, numbered to match Steam's own item numbers
  - emoticons and backgrounds, from the Points Shop
  - booster packs, one per game with cards
  - Points Shop items, from the public catalog
- ✅ A local SQLite database in the data folder. [0.3.0](../CHANGELOG.md#030---2026-10-22)
- ✅ Resumable, rate-safe background scanning, with incremental rescans and a time estimate before a long first scan. [0.3.0](../CHANGELOG.md#030---2026-10-22)
- ✅ Every game listed: owned, unowned (tagged) and family-shared (behind a setting). [0.3.0](../CHANGELOG.md#030---2026-10-22)
- ✅ Confirmed with connection tests and full scans of a real account, and cross-checked against SteamCardExchange. [Results](STEAM-DATA.md#cross-checks)
- ✅ Owned games with items, and achievement progress. [0.4.0](../CHANGELOG.md#040---2026-10-22)

## Phase 2: Completion and tags (done)

- ✅ Completion per game and for the whole account. Cards count by badge level plus copies held, and a game's cards are complete at badge level 5 plus the foil badge. A setting can require a set of cards in the inventory too. Booster packs are shown but not counted. [0.5.0](../CHANGELOG.md#050---2026-10-22)
- ✅ Each game broken down by kind: cards, foil cards, emoticons, backgrounds and Points Shop items. The account shows items complete (every item weighs the same) and the average game (every game weighs the same). [0.5.0](../CHANGELOG.md#050---2026-10-22)
- ✅ Steam Family games count toward the account-wide figure when the Steam Family setting is on. Unowned games and sale events show their own completion but don't count. [0.5.0](../CHANGELOG.md#050---2026-10-22)
- ✅ Tags: Unowned, Family shared, Points, Unobtainable, Limited/Event, Not marketable, Unverified. Price spike and dip move to Phase 3 with prices. [0.5.0](../CHANGELOG.md#050---2026-10-22)
- ✅ Availability detection from the Points Shop's dates, items that can't be bought on their own, bundles and sale events, with a manual override per item. Unobtainable items are tagged but left out of the percentage, and old items Steam no longer lists appear only if you own them. [0.5.0](../CHANGELOG.md#050---2026-10-22)
- ✅ Owned items matched by game, item class, item number and foil, falling back to the name when Steam doesn't give the number. [0.5.0](../CHANGELOG.md#050---2026-10-22)

## Phase 3: Prices

- ⬜ Current, lowest historical and average price for every item.
- ⬜ Price spike and Price dip tags with adjustable sensitivity.
- ⬜ Confirm which items are marketable from the Market itself, replacing the rule of thumb behind the Points and Not marketable tags.
- ⬜ Currency setting, USD by default.
- ⬜ Account value before market fees.
- ⬜ Cost to complete, with dollars and points shown separately.

## Phase 4: Interface

- ⬜ Dashboard, sortable game list, sortable list of all items, and per-game art grid.
- ⬜ Item detail with a small price chart.
- ⬜ Full settings panel.
- ⬜ Virtualized lists, so tens of thousands of items stay smooth.

## Phase 5: History and banner

- ⬜ Progress-over-time chart for your account.
- ⬜ Banner generator that exports a PNG of your progress and stats.

## Phase 6: Polish and 1.0

- ⬜ First-run guide for friends.
- ⬜ A speed and memory pass.
- ⬜ Version 1.0.

## Decisions

- **Own profile only.** Each person loads their own profile and shares a banner image. Comparing profiles and leaderboards are listed in [IDEAS.md](IDEAS.md).
- **Tauri.** A Rust engine with screens drawn by the Edge engine built into Windows. It opens in its own window, never a browser, and the exe is around 7 MB.
- **Portable.** A `data` folder next to the exe holds settings, cache and reports. Every setting is changed inside the app.
- **Updates from GitHub Releases.** Versions follow semantic versioning, and each release's notes come from the changelog. Any version can be installed from the app.
- **Each person uses their own API key.** It's encrypted for their Windows account and never bundled in the exe.
- **The catalog doesn't need the market.** Card pages and the Points Shop describe every item in a set, so the market is only needed for prices.
- **Sign-in is recommended.** Steam refuses inventory and market requests from apps that aren't signed in. Sign-in happens on Steam's own page, and the app never handles the password.
- **Plain language everywhere.** Every message says what happened and what to do next.
- **How completion counts.** Booster packs don't count. Each card needs five copies' worth, from badge levels and copies held. Reward items that can't be bought on their own are tagged Limited and still count unless marked Unobtainable. The account figure covers owned games, plus Steam Family games when that setting is on.
