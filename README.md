<p align="center">
  <img src="src-tauri/icons/128x128.png" width="96" height="96" alt="">
</p>

<h1 align="center">Steam Collection Tracker</h1>

<p align="center">
  See how much of each game's Steam items you own, and what's left to collect.
</p>

<p align="center">
  <a href="https://github.com/WarrenParker/steam-collection-tracker/releases/latest"><img src="https://img.shields.io/github/v/release/WarrenParker/steam-collection-tracker?label=release" alt="Latest release"></a>
  <a href="https://github.com/WarrenParker/steam-collection-tracker/actions/workflows/build.yml"><img src="https://img.shields.io/github/actions/workflow/status/WarrenParker/steam-collection-tracker/build.yml?branch=main&label=build" alt="Build status"></a>
  <img src="https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-informational" alt="Windows 10 and 11">
</p>

Steam Collection Tracker is a small Windows app for people who collect Steam items. It reads your profile and works out how complete your collection is for every game: trading cards, foil cards, emoticons, profile backgrounds, badges and Points Shop items.

The app is in early development. It reads your profile, games, badges and inventory, and builds a catalog of every item your games have. Completion percentages and prices are next; see the [roadmap](docs/ROADMAP.md).

## Features

- One portable exe. Nothing to install, and it updates itself.
- Lists every game you own or have items for, and optionally your Steam Family's shared games.
- Catalogs every item those games have: cards, foil cards, emoticons, backgrounds, booster packs and Points Shop items.
- Shows how complete each game and your whole account is, what your items are worth, and what finishing would cost on the Market and in the Points Shop.
- Every item in one fast, sortable list with filters, an art grid for each game, and Ctrl+K search across games and items.
- Scans in the background, and can pause and pick up where it left off.
- Starts from a shared daily price file, so most prices arrive in seconds instead of hours.
- A progress chart over time, and a banner image of your progress to share.
- Optional sign-in through Steam's own page, so the app can read your inventory and Points Shop items.
- Greyscale, dark and light themes.

Planned: per-game and account-wide completion, account value, cost to complete, price history, and a shareable progress banner.

## Download

1. Download `SteamCollectionTracker.exe` from the [latest release](https://github.com/WarrenParker/steam-collection-tracker/releases/latest).
2. Put it in a folder of its own. The app keeps its settings and data in a `data` folder next to the exe.
3. Run it. Windows may warn you the first time because the app isn't code-signed yet. Click **More info**, then **Run anyway**.

You'll need:

- Windows 10 or 11
- A [Steam Web API key](https://steamcommunity.com/dev/apikey). Steam only issues keys to accounts that have spent at least $5.
- A Steam profile with **Game details** set to Public

Signing in with Steam is optional but recommended. Steam only shares your inventory and Points Shop items with apps that are signed in.

## Privacy

Everything stays on your PC. Your API key is encrypted for your Windows account, and sign-in happens on Steam's own page, so the app never sees your password. The app only talks to Steam, to GitHub for updates and the shared price file, and to Frankfurter for exchange rates when you pick another currency. Nothing about you is uploaded unless you publish prices yourself; the published file holds Market prices only, though its list of games shows which games your app priced.

## Building from source

You'll need Node.js 22 and stable Rust.

```sh
npm install
npm run tauri dev      # run with live reload
npm run tauri build    # build the release exe
```

The release exe ends up in `src-tauri/target/release/`. [Architecture](docs/ARCHITECTURE.md) explains how the app is organized and how releases are published.

## Documentation

- [Changelog](CHANGELOG.md)
- [Roadmap](docs/ROADMAP.md)
- [Ideas](docs/IDEAS.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Steam data sources](docs/STEAM-DATA.md)

Built with [Tauri](https://tauri.app), Rust and Svelte.
