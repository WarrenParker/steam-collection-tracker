# Steam Collection Tracker

A small Windows app that shows how complete your Steam item collection is: trading cards, foil cards, emoticons, profile backgrounds, badges, Points Shop items and everything else tied to your games.

It's in early development. Version 0.1 counts your games, badges and inventory items and tests the Steam connection. Completion percentages, full item lists and prices come next.

## Get it

1. Open the [latest release](../../releases/latest) and download `SteamCollectionTracker.exe`.
2. Put it in its own folder, for example `Documents\Steam Collection Tracker`. The app keeps its settings in a `data` folder next to the exe.
3. Double-click it. The first time, Windows may show "Windows protected your PC" because the app isn't code-signed. Click **More info**, then **Run anyway**.

The app updates itself when new versions come out.

## What you need

- Windows 10 or 11.
- A Steam Web API key. The app walks you through getting one. Steam only gives keys to accounts that have spent at least $5.
- A public Steam profile, with **Game details** and **Inventory** set to Public in Steam's privacy settings.

## Privacy

Everything stays on your PC. Your API key is encrypted with Windows' own protection for your user account. The app only talks to Steam, and to GitHub to check for updates.

## Building from source

You need Node.js 22 and Rust (stable).

```
npm install
npm run tauri dev      # run in development
npm run tauri build    # build src-tauri/target/release/steam-collection-tracker.exe
```

GitHub Actions (`.github/workflows/build.yml`) builds every push to `main` and publishes a release whenever the version in `src-tauri/Cargo.toml` is new.
