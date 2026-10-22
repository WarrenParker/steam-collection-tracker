# Changelog

Every change to Steam Collection Tracker, newest first. Version numbers follow [semantic versioning](https://semver.org) (MAJOR.MINOR.PATCH): PATCH for fixes, MINOR for new features, MAJOR for big changes. Versions starting with 0 are early development.

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
