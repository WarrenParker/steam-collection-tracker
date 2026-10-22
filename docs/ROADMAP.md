# Roadmap

The plan for Steam Collection Tracker, phase by phase. Finished items link to the version that shipped them in the [changelog](../CHANGELOG.md). Ideas that aren't scheduled yet are in [IDEAS.md](IDEAS.md).

**Now:** version 0.2.0. Phase 0 is done; Phase 1 is next.

## Goal

A lightweight Windows app, one exe in one folder with nothing to install, that loads your own Steam profile and shows how complete your collection is for every game. That covers cards, foil cards, emoticons, backgrounds, badges, Points Shop items and anything else tied to a game. It also shows account value, cost to complete, and price insight.

## Phase 0: Setup and proof (done)

- ✅ Windows build pipeline on GitHub, published releases and a self-updater. [0.1.0](../CHANGELOG.md#010---2026-10-21)
- ✅ First-time setup, overview counts and a connection test that runs on the user's PC. [0.1.0](../CHANGELOG.md#010---2026-10-21)
- ✅ Steam sign-in, moved up from a later phase. The first connection test showed that Steam refuses anonymous inventory and market requests. [0.2.0](../CHANGELOG.md#020---2026-10-21)
- ✅ Updates button, version history and switching, and automatic tidy-up. [0.2.0](../CHANGELOG.md#020---2026-10-21)
- ✅ Confirm the signed-in sources with a 0.2.0 connection test. Inventory, Points Shop items and the Steam Family library all work. The market still refused, which carries over to Phase 1. Results are in [STEAM-DATA.md](STEAM-DATA.md).

## Phase 1: Scanner and local database (next)

- ⬜ Find a working route to the market. Re-test once the block left by 0.1.0 has expired. If Steam keeps refusing, find other sources for each game's emoticons, backgrounds and prices.
- ⬜ Full item catalog for every game:
  - cards and foil cards, from each game's card page
  - emoticons, backgrounds and boosters, from the market, signed in
  - Points Shop items, from the public catalog
- ⬜ A local SQLite database in the data folder.
- ⬜ Resumable, rate-safe background scanning, with incremental rescans and a time estimate before the first scan.
- ⬜ Every game listed: owned, unowned (tagged) and family-shared (behind a setting).

## Phase 2: Completion and tags

- ⬜ Completion per game and for the whole account. By default a badge counts as owning its cards; a setting can require the cards too.
- ⬜ Tags: Unowned, Family shared, Points (no market price), Unobtainable, Limited/Event, Not marketable, Unverified, Price spike, Price dip.
- ⬜ Availability detection, with manual tag override.

## Phase 3: Prices

- ⬜ Current, lowest historical and average price for every item.
- ⬜ Spike and dip tags with adjustable sensitivity.
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
- **Sign-in is recommended.** Steam refuses inventory and market requests from apps that aren't signed in. Sign-in happens on Steam's own page, and the app never handles the password.
- **Plain language everywhere.** Every message says what happened and what to do next.
