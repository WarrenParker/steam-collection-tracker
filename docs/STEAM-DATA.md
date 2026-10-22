# Steam data sources

What the app reads from Steam, where it comes from, and what each connection test has shown. Update this whenever a test report shows something new.

## Sources

| Data | Where it comes from | Needs | What tests showed |
|---|---|---|---|
| Profile | `ISteamUser/GetPlayerSummaries/v2` | API key | Works. `communityvisibilitystate` 3 means public. (0.1.0) |
| Owned games | `IPlayerService/GetOwnedGames/v1` | API key; Game details public | Works: 177 games. A private library answers with an empty `response`. (0.1.0) |
| Badges and level | `IPlayerService/GetBadges/v1` | API key | Works. Game badges have an `appid`; foil badges have `border_color: 1`. (0.1.0) |
| Card set and owned cards | `steamcommunity.com/profiles/{id}/gamecards/{appid}` (HTML) | Nothing | Works without sign-in. Cards are `class="badge_card_set_card …"`; owned quantities are in `badge_card_set_text_qty`. (0.1.0) |
| Badges page | `steamcommunity.com/profiles/{id}/badges` (HTML) | Nothing | Works without sign-in. Rows are `class="badge_row …"`, with card drops remaining in `badge_title_stats_drops`. (0.1.0) |
| Inventory | `steamcommunity.com/inventory/{id}/753/6` | Sign-in | **Refused without sign-in** (429 on the first request). (0.1.0) |
| Inventory, signed in | `IEconService/GetInventoryItemsWithDescriptions/v1` with a sign-in token, or the community inventory with the sign-in cookie | Sign-in | Not tested yet. (0.2.0) |
| Market items per game | `steamcommunity.com/market/search/render?norender=1&category_753_Game[]=tag_app_{appid}` | Sign-in | **Refused without sign-in** (429 on the first request). (0.1.0) |
| Price history | `steamcommunity.com/market/pricehistory` (signed in), or the `var line1` graph data in an item's market page | Sign-in | Not tested yet. (0.2.0) |
| Points Shop catalog | `ILoyaltyRewardsService/QueryRewardItems/v1?appids[0]={appid}` | Nothing | Works without a key: 29 items for Counter-Strike 2. (0.1.0) |
| Points Shop items owned | `IPlayerService/GetProfileItemsOwned/v1` | Sign-in token | 401 with an API key, as expected. Signed in: not tested yet. |
| Steam Family library | `IFamilyGroupsService/GetFamilyGroupForUser` and `GetSharedLibraryApps` | Sign-in token | 401 with an API key, as expected. Signed in: not tested yet. |
| Sign-in token | `store.steampowered.com/pointssummary/ajaxgetasyncconfig` (`data.webapi_token`) | Sign-in cookie | Not tested yet. The fallback is the token inside the `steamLoginSecure` cookie. |

## Rate limits

- Steam limits the Community pages, the inventory and the Market per internet connection. Asking again after a refusal (HTTP 429) can make the block longer.
- The app spaces requests out: 0.3 s for the Web API, 3.5 s for Community pages, 4 s for the inventory and Market, and 1 s for the store.
- After a 429 from anything except the Web API, the app doesn't send that kind of request again for 30 minutes, tracked separately for signed-in and anonymous requests. The Web API is retried after 5 and 15 seconds.
- Version 0.1.0 retried refused Community requests, which made a test take nearly 3 minutes. Version 0.2.0 fixed that.

## Formats worth knowing

### Inventory (app 753, context 6)

- `assets` and `descriptions` are joined on `classid` and `instanceid`. `amount` is a string, and gems come as one stack.
- An item's type is its `item_class` tag: 2 is a trading card, 3 a profile background, 4 an emoticon, 5 a booster pack and 7 Steam Gems.
- Foil cards also have the tag `cardborder_1`. The game is the `Game` tag (`app_{appid}`), or `market_fee_app`.
- Pages hold up to 2,000 items. When `more_items` is 1, pass `last_assetid` as `start_assetid` to get the next page.

### Points Shop definitions

Each definition has these fields:

- `defid`, `appid`, `point_cost` (a string) and `active`
- `community_item_class` and `community_item_type`
- `timestamp_available` and `timestamp_available_end`, where 0 means no limit
- `community_item_data`, with `item_name`, `item_title`, `item_image_large`, `item_image_small` and `animated`

`active` and the availability timestamps are the starting point for the Limited and Unobtainable tags.

### Badges

- Each badge has `badgeid`, `level`, `xp`, `scarcity` and `completion_time`.
- Game badges also have `appid` and `border_color`.
- The response itself also has `player_level`, `player_xp` and the XP needed for the next level.
