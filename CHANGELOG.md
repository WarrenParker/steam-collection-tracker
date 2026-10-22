# Changelog

All notable changes to Steam Collection Tracker are listed here, newest first. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [semantic versioning](https://semver.org). Versions starting with 0 are early development.

## [0.3.0] - 2026-10-22

### Added

- **Item catalog.** Scans now read every item that exists for your games: cards, foil cards, emoticons, backgrounds, booster packs and Points Shop items. The app uses Steam's item list when your connection allows it, and each game's card page otherwise.
- **Games tab** listing every game you own or have items for, with search, filters and sorting. Games you don't own are tagged Unowned, and Steam sale events are tagged Event.
- **Steam Family games**, in Settings. When it's on, games your family shares with you are listed and cataloged too, tagged Family.
- **Background scanning** with progress and a time estimate. You can pause a scan and continue it later, even after closing the app. If Steam asks the app to slow down, the scan takes a 30-minute break and then carries on by itself.
- **Collection database.** Everything a scan reads is saved in `collection.db` in the data folder, so later scans only fetch what's new or out of date.
- The overview shows the item catalog by type and how many games are owned, unowned and shared.
- The connection test checks the new sources: Steam's item list, which games have trading cards, the Points Shop for several games at once, and your emoticons. The market check now runs first among the per-game checks.

### Changed

- The last scan is kept in the database. The old `cache` folder is removed.

## [0.2.0] - 2026-10-21

### Added

- **Sign in with Steam.** Steam's own sign-in page opens in a small window, with a QR code you can scan using the Steam mobile app. Steam won't share your inventory with apps that aren't signed in, so signing in is how the app reads it. The app doesn't read or store your password.
- **Updates button** in the top bar. It shows your version and changes to "Update available" when a new one is out.
- **Version history** in Settings, with the notes for every version and a button to switch to any of them. Switching to an older version turns automatic updates off so the app stays on it.
- **Automatic tidy-up** each time the app opens: leftover update files, old connection test reports (the newest 5 are kept) and web caches are removed.
- The connection test now checks the signed-in sources: inventory, market prices, price history, Points Shop items you own and your Steam Family library.

### Fixed

- The app no longer asks again after Steam refuses a request. It waits 30 minutes instead, so a temporary block doesn't get longer.

## [0.1.0] - 2026-10-21

### Added

- First-time setup for your Steam API key and profile.
- Overview with counts of your games, badges and inventory items.
- A connection test for every Steam source the app uses.
- Greyscale, dark and light themes.
- Updates itself when new versions are published.
