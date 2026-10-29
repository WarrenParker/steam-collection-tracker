# Ideas

Ideas that aren't scheduled yet, and requests that have shipped. When an idea is ready to build, it moves to the [roadmap](ROADMAP.md). When it ships, it's listed below with the version that added it.

## Not scheduled yet

- ⬜ **Compare profiles.** View another user's profile side by side with yours. For now the app loads only your own profile, and sharing happens through the banner image. *Added Oct 21, 2026.*
- ⬜ **Leaderboard.** Rank profiles by completion. *Added Oct 21, 2026.*
- ⬜ **Android version.** Tauri, the toolkit the app is built with, can also build Android apps, so the screens and most of the engine could be shared. Windows-only parts would need Android equivalents: the encrypted API key storage, the Steam sign-in window and self-updating. *Added Oct 21, 2026.*
- ⬜ **Android testing and organization.** A plan for testing Android builds (emulator and real phones), building and publishing them alongside the Windows releases, and organizing the code so both versions share one engine. *Added Oct 21, 2026.*
- ⬜ **Monetization and advertising.** Ways to fund the app, with a mobile version as the main place for it: ads in a free mobile app, a paid or supporter version, or donations. Steam's Web API and branding terms need checking before any of this. *Added Oct 22, 2026.*
- ⬜ **More devices and platforms.** Tauri can also build for macOS, Linux and iOS from the same code. The Steam Deck runs Linux, so a Linux build would work in its desktop mode, and a controller-friendly layout would make it work in Game Mode too. *Added Oct 22, 2026.*
- ⬜ **SteamHunters and achievements.** Show achievement progress next to each game's collection, through SteamHunters or Steam's own achievement data. *Added Oct 22, 2026.*
- ⬜ **Prices from everyone's apps.** Let friends' apps upload the prices they read to the shared price file, so games outside the owner's library are covered too. The file is download-only for now (0.9.0). Uploads need a small server that checks what's sent before it's shared, such as a free Cloudflare Worker in front of the prices repository. *Added Oct 22, 2026.*
- ⬜ **Price history refresh setting.** Choose how often price histories are read again: monthly (about 1 GB of downloads a month for a large collection) or every three months (about a third of that). *Added Oct 22, 2026.*
- ⬜ **Always-on price updater on the owner's PC.** A small background mode that keeps the shared price file fresh every day, even when the app's window isn't open, so friends always download recent prices. *Added Oct 22, 2026.*
- ⬜ **Free cloud host for the shared prices.** Serve the price file from a free host with a cache in front (such as Cloudflare), and later accept checked price uploads from friends' apps there, so no home PC has to stay on or be reachable from the internet. *Added Oct 22, 2026.*
- ⬜ **Price history from another route.** Steam has refused the app's price history requests since Oct 22, 2026, in bulk and for single items alike, while current prices and order books keep working and the same request from a browser answers. Until it works, items have no averages, lowest or highest prices or charts, and the price spike and dip tags and the rule for listings far above an item's usual price don't apply. Routes to try: retrying now and then, history in the shared price file (0.9.0 can carry it, once some copy has read it, for example from a browser or another connection), a paid Market data service, or Steam's older signed-in history page. *Added Oct 24, 2026.*
- ⬜ **Shared item catalog.** Share each game's card set and Points Shop list the same way as prices, so a friend's first scan doesn't read every game's card page (one every 3.5 seconds). *Added Oct 28, 2026.*
- ⬜ **Shared prices in other currencies.** Use the shared file for accounts that pay in another currency too, converted with the daily exchange rates. For now it's only used when your Market currency matches the file's. *Added Oct 28, 2026.*
- ⬜ **Bulk price services.** Check whether paid Steam Market data services cover trading cards, emoticons and backgrounds, and what they cost, as another way to fill prices quickly. *Added Oct 22, 2026.*

## Shipped

| Request | Version |
|---|---|
| ✅ **Prices first, history later.** Scans read current prices only, and each item's history is read when it's opened. | [0.8.4](../CHANGELOG.md#084---2026-10-24) |
| ✅ **Order books when a game is opened**, instead of during every scan. | [0.6.3](../CHANGELOG.md#063---2026-10-22) |
| ✅ **Owned games with items**, shown as "X of Y", and a With items filter. | [0.4.0](../CHANGELOG.md#040---2026-10-22) |
| ✅ **Average achievement completion**, achievements unlocked and perfect games, plus an Achievements column. | [0.4.0](../CHANGELOG.md#040---2026-10-22) |
| ✅ **One copy of the app at a time.** Opening it again brings the open window forward. | [0.4.0](../CHANGELOG.md#040---2026-10-22) |
| ✅ **Every game listed**, including games you don't own (tagged Unowned) and Steam sale events (tagged Event). | [0.3.0](../CHANGELOG.md#030---2026-10-22) |
| ✅ **Steam Family games** behind a setting, tagged Family. | [0.3.0](../CHANGELOG.md#030---2026-10-22) |
| ✅ **Scan speed and a time estimate.** The scan uses Steam's fastest source, shows time left, and asks first before a long first scan. | [0.3.0](../CHANGELOG.md#030---2026-10-22) |
| ✅ **Steam sign-in.** Steam's own sign-in page in a small window, with a QR code for the Steam mobile app. | [0.2.0](../CHANGELOG.md#020---2026-10-21) |
| ✅ **Updates button** in the top bar, showing the version and "Update available". | [0.2.0](../CHANGELOG.md#020---2026-10-21) |
| ✅ **Version history and switching.** Notes for every version, and a button to switch to any of them. | [0.2.0](../CHANGELOG.md#020---2026-10-21) |
| ✅ **Semantic versioning and a changelog**, which feed the release notes and the in-app history. | [0.2.0](../CHANGELOG.md#020---2026-10-21) |
| ✅ **Automatic tidy-up** of old reports, update leftovers and web caches. | [0.2.0](../CHANGELOG.md#020---2026-10-21) |
| ✅ **Greyscale, dark and light themes.** | [0.1.0](../CHANGELOG.md#010---2026-10-21) |
| ✅ **Self-updating** single exe, kept in one folder. | [0.1.0](../CHANGELOG.md#010---2026-10-21) |
