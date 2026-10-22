# Steam Collection Tracker

A small Windows app that shows how complete your Steam item collection is: trading cards, foil cards, emoticons, profile backgrounds, badges, Points Shop items and everything else tied to your games.

It's in early development: it counts your games, badges and inventory items, signs in to Steam, and tests the Steam connection. Completion percentages, full item lists and prices come next. See [CHANGELOG.md](CHANGELOG.md) for what changed in each version.

## Documentation

- [Changelog](CHANGELOG.md): what changed in each version
- [Roadmap](docs/ROADMAP.md): the plan and where things stand
- [Idea board](docs/IDEAS.md): ideas on the back burner, and requests that shipped
- [Architecture](docs/ARCHITECTURE.md): how the app is built, stored and released
- [Steam data sources](docs/STEAM-DATA.md): what each Steam source needs, and what tests showed

## Get it

1. Open the [latest release](../../releases/latest) and download `SteamCollectionTracker.exe`.
2. Put it in its own folder, for example `Documents\Steam Collection Tracker`. The app keeps its settings in a `data` folder next to the exe.
3. Double-click it. The first time, Windows may show "Windows protected your PC" because the app isn't code-signed. Click **More info**, then **Run anyway**.

The app updates itself when new versions come out. The **Updates** button in the top bar shows your version, and Settings has the full version history, where you can switch to any published version.

## What you need

- Windows 10 or 11.
- A Steam Web API key. The app walks you through getting one. Steam only gives keys to accounts that have spent at least $5.
- A public Steam profile, with **Game details** set to Public in Steam's privacy settings.
- Signing in with Steam is recommended: Steam only shares your inventory, market prices and Points Shop items with apps that are signed in.

## Privacy

Everything stays on your PC. Your API key is encrypted with Windows' own protection for your user account. Signing in happens on Steam's own sign-in page; the app keeps Steam's sign-in cookie in its data folder (encrypted by Windows, like a browser's) and never reads or stores your password. The app only talks to Steam, and to GitHub to check for updates.

## Building from source

You need Node.js 22 and Rust (stable).

```
npm install
npm run tauri dev      # run in development
npm run tauri build    # build src-tauri/target/release/steam-collection-tracker.exe
```

## Versioning and releases

Versions follow [semantic versioning](https://semver.org): MAJOR.MINOR.PATCH. PATCH is for fixes, MINOR for new features, MAJOR for big changes.

To release a version:

1. Set the new version in `src-tauri/Cargo.toml` and `package.json`.
2. Add a section for it at the top of `CHANGELOG.md`.
3. Push to `main`.

GitHub Actions (`.github/workflows/build.yml`) builds and tests every push to `main`. When the version is new, it publishes a release tagged `vX.Y.Z` with that version's changelog section as its notes. The build stops if the changelog has no entry for the version.
