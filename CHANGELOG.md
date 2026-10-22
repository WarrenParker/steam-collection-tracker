# Changelog

All notable changes to Steam Collection Tracker are listed here, newest first. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [semantic versioning](https://semver.org). Versions starting with 0 are early development.

## [0.6.1] - 2026-10-22

### Changed

- An item listed at more than three times its usual price counts at its 90-day average sale price in cost to complete and account value, marked as estimated. One background listed at $876 that sells for about $2 made up most of a whole account's cost to complete. Items that haven't sold in 90 days use their 1-year average, and the rule applies once an item's price history has been read.

### Fixed

- Items that share a name, like two backgrounds both called "Space Elevator", match the right Market item by number. One was listed twice before.
- After prices are read, the overview offers **Continue scan** to start reading price history, instead of Scan again.

## [0.6.0] - 2026-10-22

### Added

- **Prices.** Scans read every item's cheapest listing from the Steam Community Market, game by game, and again when prices are more than a day old.
- **Account value:** what the items you could sell are worth at today's prices, before Steam's fees. Items you can't sell, such as Points Shop purchases, are shown separately as a points value.
- **Cost to complete,** for every game and the whole account. Market items are counted in money, buying the cheapest copies first, so a card that needs several copies is priced up the listings. Items only the Points Shop sells are counted in points. Items sold both ways count in money, with a line showing what they'd cost in points instead.
- **Price history.** Each item's history is read once, then refreshed about once a month, a few items a day. The app keeps the last year in a compact form and works out the average over 30 days, 90 days, a year and all time, and the lowest and highest prices ever. Click an item's price in a game's details for its figures and a chart, and "Show full history" for every sale since the first.
- **Price spike and Price dip tags,** when today's cheapest listing is far from the average. Settings choose how far (25% by default) and by how much at least (5 cents by default).
- **Currency setting.** Prices can be shown in any currency Steam uses, converted with daily exchange rates.
- Settings: **Average price over** 30 days, 90 days, a year or all time.
- Value and To complete columns in the Games tab.
- The connection test checks the order book and both price history routes, and how much each one downloads.

### Changed

- Whether an item can be sold now comes from the Market itself, replacing the rule of thumb behind the Points and Not marketable tags.
- Emoticons and backgrounds Steam no longer hands out but that are for sale on the Market join their game's list and count toward completion. They're tagged Unobtainable again if nobody is selling them.
- The first price history read takes a few hours for a large collection, so the scan asks before starting it, with a time estimate. It can be paused anytime.

## [0.5.1] - 2026-10-22

### Fixed

- Emoticons and backgrounds from a game's card set count as available even when the Points Shop doesn't sell them, since crafting the badge gives them and they're on the Market. DEATHLOOP's twelve were tagged Limited in 0.5.0.
- Foil cards show the card's picture in a game's details.

## [0.5.0] - 2026-10-22

### Added

- **Completion.** Every game shows how much of its collection you have, and the overview shows your whole account: items complete, the average game, games started and complete, and card sets complete. Cards, foil cards, emoticons, backgrounds and every other Points Shop item count. Booster packs are listed but don't count.
- Cards count by badge level and the copies you hold. Each badge level uses one copy of every card, so a level 3 badge covers three of the five copies each card needs, and spare copies cover the rest. A game's cards are complete at badge level 5 plus the foil badge.
- **Game details.** Click a game in the Games tab to see every item, what you own, each item's tags and its price in points. The tab also has a sortable Complete column and a Complete filter.
- **Tags:** Unowned, Family shared, Points, Unobtainable, Limited/Event, Not marketable and Unverified.
- **Availability.** The app works out whether each item can still be had, from the Points Shop's dates, whether it's sold on its own or in a bundle, and Steam sale events. Items that can't be had any more are tagged Unobtainable and left out of the percentage. You can set any item's availability yourself in its game's details.
- Items you own that Steam no longer lists anywhere show up in their game's list, tagged Unobtainable. Owned items without an item number are matched by name and tagged Unverified.
- Setting: **Also need the cards in my inventory**, so a card set counts as complete only when you also hold one of each card.
- Steam Family games count toward the account figure when the Steam Family setting is on.

### Changed

- The next scan reads every game's Points Shop again, to learn which items can be bought on their own.

### Fixed

- Settings → Version history says it couldn't load the list of versions, with a Try again button, instead of saying no versions have been published.

## [0.4.2] - 2026-10-22

### Fixed

- Cards you own keep their exact Steam item numbers even in sets whose numbers have gaps, so every owned card lines up with the catalog. A scan on 0.4.1 matched 22 of 23 owned cards; the one miss was in a set like that. From now on, any game where you own a card the catalog doesn't match is read again on the next scan.

## [0.4.1] - 2026-10-22

### Fixed

- Achievement progress now loads. Steam only accepts that request as a form submission, which 0.4.0 didn't send.
- Requests to Steam's web pages and Market now identify as the browser built into the app. Steam was turning the Market away even though it works in a normal browser on the same connection.

### Removed

- The attempt to read Steam's own item list. Steam only shares it with game developers, so the app now always reads each game's card page.
- The attempt to read your emoticon list, which Steam doesn't offer to apps. Owned emoticons come from your inventory as before.

## [0.4.0] - 2026-10-22

### Added

- **Achievements.** Scans read your achievement progress for every owned game. The overview shows your average completion (counted the way Steam does, over games you've started), achievements unlocked and perfect games. The Games tab has a sortable Achievements column.
- **Owned games with items**, shown as "X of Y" on the overview, and a **With items** filter on the Games tab.
- Points Shop items that can't be bought directly, such as sale rewards and retired items, are now part of the catalog.

### Changed

- Opening the app while it's already running brings the open window forward instead of starting a second copy.
- If Steam's quick item list isn't available, the app tries it again a day later, and re-reads games it had to catalog the slow way once it works.

### Fixed

- Cards read from a game's card page now use Steam's own item numbers when you own any card from the set, so they match your inventory. Games cataloged this way in 0.3.0 are read again on the next scan.

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
