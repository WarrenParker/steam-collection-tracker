# Steam data sources

What the app reads from Steam, where it comes from, and what each connection test has shown. Update this whenever a test report shows something new.

## Sources

| Data | Where it comes from | Needs | What tests showed |
|---|---|---|---|
| Profile | `ISteamUser/GetPlayerSummaries/v2` | API key | Works. `communityvisibilitystate` 3 means public. (0.1.0) |
| Owned games | `IPlayerService/GetOwnedGames/v1` | API key; Game details public | Works. A private library answers with an empty `response`. (0.1.0) |
| Badges and level | `IPlayerService/GetBadges/v1` | API key | Works. Game badges have an `appid`; foil badges have `border_color: 1`. (0.1.0) |
| Item definitions per game | `IQuestService/GetCommunityItemDefinitions/v1?appid={appid}` | A publisher key | **Refused** (403) without a key, with a user key and with the sign-in token: "Access is denied. Retrying will not help. Please verify your key= parameter." Only game developers' keys work, so the app doesn't use it. (0.4.0) |
| Which games have trading cards | `IStoreBrowseService/GetItems/v1?input_json=…` with `include_basic_info` | Nothing | **Works** without a key: 25 of 39 owned games checked have category 29 (Steam Trading Cards). Up to 100 games per request. (0.4.0) |
| Card set and owned cards | `steamcommunity.com/profiles/{id}/gamecards/{appid}` (HTML) | Nothing | **Works** without sign-in, and is the catalog route. Cards are `class="badge_card_set_card …"`; owned quantities are in `badge_card_set_text_qty`; "N of M, Series S" gives the card's place in the set. The place is not Steam's item number (a set's cards might be numbers 3 to 8), so the app takes the real numbers from cards the user owns. (0.4.0) |
| Badges page | `steamcommunity.com/profiles/{id}/badges` (HTML) | Nothing | Works without sign-in. Rows are `class="badge_row …"`, with card drops remaining in `badge_title_stats_drops`. (0.1.0) |
| Inventory | `steamcommunity.com/inventory/{id}/753/6` | Sign-in | **Refused without sign-in** (429 on the first request). (0.1.0) |
| Inventory, signed in | `IEconService/GetInventoryItemsWithDescriptions/v1` with the sign-in Web API token | Sign-in | **Works** and is fast (under 0.2 s). Same `assets`/`descriptions` layout as the community inventory, inside `response`. Includes Points Shop items such as animated avatars, frames and stickers. (0.2.0) |
| Market items per game | `steamcommunity.com/market/search/render?norender=1&category_753_Game[]=tag_app_{appid}` | A browser identity | **Works** from 0.4.1: 26 items for Counter-Strike 2, signed in, in 0.2 s. Before that the app was refused (429) even after nine hours, while the same URL worked in a browser on the same connection, so Steam was turning away the app's own identity. The app now sends the WebView2 browser identity and browser headers for Community and Market requests. (0.4.1) |
| Price history | `steamcommunity.com/market/pricehistory?appid=753&market_hash_name=…` | Sign-in | **Works** signed in: 5,572 data points for `730-:csgoa:`, from May 2013 to today, in hourly then daily points `[date, price, volume]`. (0.4.1) |
| Points Shop catalog | `ILoyaltyRewardsService/QueryRewardItems/v1?appids[0]={appid}` | Nothing | **Works** for several games per request (`appids[0]`…), 100 items per page with `next_cursor`. `include_direct_purchase_disabled=true` adds items that can't be bought directly. (0.4.0) Reading each batch with and without it (0.5.0): of 2,930 Points Shop items for the account's games, 2,826 are sold on their own, 79 only inside a bundle (Counter-Strike 2's extra 4 among them), and 25 neither: DEATHLOOP's card-set emoticons and backgrounds, and QuakeCon reward items. |
| Games with a Points Shop page | `ILoyaltyRewardsService/GetEligibleApps/v1` | Nothing | **Works**: 14,254 games and 122 sale events. `event_app` marks the sale events. (0.4.0) |
| Emoticons owned | `IPlayerService/GetEmoticonList/v1` | Not public | **Not available** (404 with the sign-in token). Owned emoticons come from the inventory, which includes Points Shop emoticons. (0.4.0) |
| Points Shop items owned | `IPlayerService/GetProfileItemsOwned/v1` | Sign-in token | **Works** signed in. The answer groups items into `animated_avatars`, `avatar_frames`, `mini_profile_backgrounds`, `profile_backgrounds` and so on. 401 with only an API key. (0.2.0) |
| Achievement progress | `IPlayerService/GetAchievementsProgress/v1` (POST) with `appids[N]` | Sign-in token | **Works** as a form POST with the sign-in token: 44 of 50 owned games checked have achievements. A GET answers 405, and the API key answers 401. (0.4.1) |
| Steam Family library | `IFamilyGroupsService/GetFamilyGroupForUser`, then `GetSharedLibraryApps` | Sign-in token | **Works** signed in: 193 shared games on the owner's account. The first returns `family_groupid` (or `is_not_member_of_any_group`); the second lists shared `apps` with their owners. 401 with only an API key. (0.2.0) |
| Sign-in token | `store.steampowered.com/pointssummary/ajaxgetasyncconfig` (`data.webapi_token`) | Sign-in cookie | **Works.** Steam sets the store cookie during sign-in, and the sign-in lasts about 24 hours before the app renews it. (0.2.0) |

## Rate limits

- Steam limits the Community pages, the inventory and the Market per internet connection. Asking again after a refusal (HTTP 429) can make the block longer.
- The app spaces requests out: 0.3 s for the Web API, 3.5 s for Community pages, 4 s for the inventory and Market, and 1 s for the store.
- After a 429 from anything except the Web API, the app doesn't send that kind of request again for 30 minutes, tracked separately for signed-in and anonymous requests. The Web API is retried after 5 and 15 seconds.
- Version 0.1.0 retried refused Community requests, which made a test take nearly 3 minutes. Version 0.2.0 fixed that.

## Formats worth knowing

### Inventory (app 753, context 6)

- Signed in, an item's `owner_actions` include "Turn into Gems" as `javascript:GetGooValue( '%contextid%', '%assetid%', {appid}, {item_type}, {border} )`. That gives the item's type number, which matches the item definitions. Not confirmed on a real inventory yet.

- Points Shop items (stickers, frames, animated avatars, mini profile backgrounds, and emoticons and backgrounds bought with points) have no "Turn into Gems" action, so they carry no type number. The app matches them to the catalog by name, and by number through the profile items list, which always has it.
- `assets` and `descriptions` are joined on `classid` and `instanceid`. `amount` is a string, and gems come as one stack.
- An item's type is its `item_class` tag: 2 is a trading card, 3 a profile background, 4 an emoticon, 5 a booster pack and 7 Steam Gems.
- Foil cards also have the tag `cardborder_1`. The game is the `Game` tag (`app_{appid}`), or `market_fee_app`.
- Pages hold up to 2,000 items. When `more_items` is 1, pass `last_assetid` as `start_assetid` to get the next page.

### Item definitions (`IQuestService/GetCommunityItemDefinitions`, developers only)

Each definition has `appid`, `item_class`, `item_type`, `item_name`, `item_title`, `item_description`, `item_image_small`, `item_image_large`, `item_series`, `active` and `deleted`, and for cards `item_image_composed` and `item_image_composed_foil`. Images can be file names inside `https://cdn.akamai.steamstatic.com/steamcommunity/public/images/items/{appid}/`. Item classes match the inventory's `item_class_N` tags: 1 badge, 2 card, 3 background, 4 emoticon, 5 booster, 11 sticker, 13 mini profile background, 14 avatar frame, 15 animated avatar.

Booster packs aren't community item definitions, so the app adds one per game with cards.

### Points Shop and trading card items

The Points Shop sells each game's trading-card emoticons and static backgrounds as well as its own items. Steam's partner documentation says buyers must own the game and have played it for at least two hours. Items bought with points can't be traded or sold on the market.

### Points Shop definitions

Each definition has these fields:

- `defid`, `appid`, `point_cost` (a string) and `active`
- `community_item_class` and `community_item_type`
- `timestamp_available` and `timestamp_available_end`, where 0 means no limit
- `community_item_data`, with `item_name`, `item_title`, `item_image_large`, `item_image_small` and `animated`

`active` and the availability timestamps are the starting point for the Limited and Unobtainable tags.

There's no field saying an item can't be bought on its own. The app asks for each batch of games twice, with and without `include_direct_purchase_disabled`; items only in the second answer are sale rewards or event items. A bundle lists the items inside it in `bundle_defids`, so items sold only in a bundle still count as for sale. Other fields include `type`, `quantity`, `usable_duration`, `bundle_discount` and `timestamp_free_until` (from the public SteamDatabase Protobufs, `LoyaltyRewardDefinition`).

### Badges

- Each badge has `badgeid`, `level`, `xp`, `scarcity` and `completion_time`.
- Game badges also have `appid` and `border_color`.
- The response itself also has `player_level`, `player_xp` and the XP needed for the next level.

## Cross-checks

Each catalog is checked against [SteamCardExchange](https://www.steamcardexchange.net) showcase pages, read through a browser.

**Oct 22, 2026 (0.3.0 scan).** Card set contents (cards, emoticons, backgrounds) matched for all 8 games checked:

| Game | Cards | Emoticons | Backgrounds |
|---|---|---|---|
| Half-Life 2 (220) | 8 | 5 | 5 |
| Counter-Strike 2 (730) | 5 | 11 | 4 |
| Civilization V (8930) | 8 | 6 | 6 |
| Kingdom Rush (246420) | 5 | 5 | 4 |
| Metro: Last Light Redux (287390) | 9 | 5 | 5 |
| Back to Bed (308040) | 6 | 6 | 4 |
| Disco Elysium (632470) | 14 | 7 | 3 |
| Vampire Survivors (1794680) | 5 | 5 | 3 |

**Gap:** items outside the card set that the Points Shop no longer lists are missing. Counter-Strike 2 has 75 more emoticons on SteamCardExchange (steam.tv drops such as FACEIT 2018), plus a few retired profile items.

**Oct 22, 2026 (0.5.0 completion).**

- Badge levels match the profile's badges page: Counter-Strike 2 and Back to Bed at level 5, 399670 at level 1, no foil badges, and 7 other badges.
- Owned card quantities on the card pages match the inventory for Back to Bed (regular and foil) and Kingdom Rush.
- SteamCardExchange's page for The Last of Us Part I (1888930) lists 9 cards, 6 emoticons, 9 backgrounds, 5 stickers, 2 avatar frames, 3 animated avatars and a startup movie, the same as the catalog. Its emoticons and backgrounds have Market prices and the rest don't, which matches the Points and Not marketable tags.
- The app's completion figures matched a separate calculation from the same database: 30 of 5,501 items complete and 17 of 194 games started with Steam Family games on, and Back to Bed at 68.2% (62.9% with "Also need the cards in my inventory").
