# Changelog

All notable changes to Steam Collection Tracker are listed here, newest first. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [semantic versioning](https://semver.org). Versions starting with 0 are early development.

## [0.8.3] - 2026-10-24

### Fixed

- Pausing a scan that's waiting for Steam stays paused after the app restarts, instead of carrying on by itself.
- Changing an item's availability from its panel keeps your place in the Items list, instead of jumping back to the top.

### Security

- Updates are only downloaded from the app's own GitHub releases, and a download far larger than the app is refused.
- Links the app opens are checked properly: only https pages on Steam and GitHub, with no user name, password or port.
- Every build checks the app's Rust and npm packages against known security advisories.

## [0.8.2] - 2026-10-23

### Fixed

- Waits read "2 hours" rather than "1 hour 60 minutes".
- Sorting items by name puts emoticons in order of the word inside their colons, and numbers in number order.
- In the Items list, the first arrow key press starts from the rows on screen instead of jumping back to the top.

## [0.8.1] - 2026-10-23

### Fixed

- A scan that was waiting for Steam when the app closed carries on by itself after the app opens again, once the wait is over.
- Under Cheapest to finish, costs that include points no longer run past the card's edge.
- Emoticons use Steam's larger image, so they're sharp in the art grid and item panel.
- An item without price history says so in its panel, instead of showing a red "slow down" note.
- The Games tab's Badge column keeps "Level 5" on one line, and By kind on the dashboard has room for "not counted".
- An item's panel names its kind in the singular, like "Foil card".

### Changed

- Pages use more of a wide screen and sit in the middle of the window.

## [0.8.0] - 2026-10-23

### Added

- **Dashboard.** The first tab leads with three cards: how complete your collection is, what your items are worth and what finishing would cost, each with its change since the previous day. Below them: the games closest to complete, the cheapest games to finish, good times to sell (your items listed well above their average) and to buy (items you need listed well below it), recent changes and completion by kind. Click any card or row to go straight to it. Every figure the overview used to show is under "All the figures".
- **Recent changes.** After each scan that reads your inventory, items you gained, sold, traded or used up are listed by day. The first scan after updating sets the starting point.
- The account's completion, value and cost to complete are kept once a day, for the trend on the dashboard.
- Settings: **Open on** chooses the tab the app starts on, and **Show a game's items as** a list or an art grid. The theme picker shows a preview of each theme, and Settings lists the keyboard shortcuts.

### Changed

- Every theme now carries through panels, menus and cards, with its own shadows and a soft accent glow on dark and light.
- Foil items are edged with the foil sheen everywhere.

## [0.7.0] - 2026-10-22

### Added

- **Items tab:** every item of every game in one list, with its art, game, kind, how much of it you have, today's price, the average price and tags. Sort by any column, filter by owned or missing, kind, availability, game, tags and price range, and search by item or game name. The list only draws the rows on screen, so tens of thousands of items scroll smoothly.
- **Item panel:** click any item for its art, what you have, its tags and availability, its prices and chart, and buttons to open it on the Market, open the game's Points Shop or copy its name. With the panel open, the arrow keys step through the list.
- **Search** with Ctrl+K or /: games and items by name. Arrow keys move, Enter opens.
- **Art grid** in a game's details: every item's art, grouped by kind. Items you have are in full colour and missing ones greyed out, foils are edged with the foil sheen, and cards show their copies, like 3/5. Switch between List and Art, and the app remembers which.
- Keyboard shortcuts: Esc closes the topmost panel. In an item's panel, M opens the Market, P the Points Shop and C copies the name.

### Changed

- Clicking an item in a game's details opens its panel, which has the price details that used to open below the item.

### Fixed

- **Price history reads again.** Steam stopped answering the app's price history requests while signed in, yet answered the same request without sign-in, so it seems to limit price history per account. Price history is now read without sign-in, which Steam allows; the older signed-in route is only tried when an answer can't be read.
- A Market pace slowed by refusals speeds back up step by step after 50 answers in a row, until it's back to about 480 requests an hour.
- Waits after refusals are kept apart for signed-in and anonymous requests.

## [0.6.3] - 2026-10-22

### Changed

- **Order books are read when you open a game.** A card that needs several copies is priced by walking up the Market's listings, and that needs the card's order book. Scans used to read over a thousand of them on a large account; now a game reads its own when you open it, with progress shown in its details. Until then, its cost to complete uses each card's cheapest listing and is marked as estimated.
- **Price history reads the most useful items first:** items you still need in games that count, since their usual price decides cost to complete and whether a far-off listing counts, then items you own, then everything else.
- Time estimates for price history follow the app's current Market pace.

### Fixed

- **The app waits longer each time Steam refuses in a row,** 30 minutes, then 1, 2 and 4 hours, instead of asking again every 30 minutes. Asking during a refusal seemed to keep it going: one retry every half hour was refused for hours. The wait is remembered when the app restarts.

## [0.6.2] - 2026-10-22

### Changed

- **Long Market reads go slower so Steam doesn't take a break.** Steam paused a scan for 30 minutes after about 885 Market requests in an hour. Short bursts still go at the usual pace, so clicks and small scans stay quick, but long reads now send about 480 requests an hour, one every 7.5 seconds. If Steam still refuses, the app slows down by half again and remembers it.
- Opening a game, the Games tab and the overview is faster: completion and prices are worked out once and reused until something changes.
- The To complete column puts points on their own line, and sorting it from high to low starts with the games that cost the most money.
- The Prices settings give their descriptions more room.

### Security

- The app window only loads images from Steam's image servers.
- Exchange rates and Market image ids are checked before they're used.

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
