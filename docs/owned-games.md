# Owned games in Steam

BoilR can sign in to game stores itself and add every game an account owns to Steam,
installed or not. No Playnite or Heroic is needed. A game that is not installed gets a
shortcut that installs it. Once the game is installed, the shortcut becomes the real game and
keeps its Steam app id, so artwork, playtime and collections stay.

The approach follows Playnite's library plugins
(github.com/JosefNemec/PlayniteExtensions, `source/Libraries`, and Jeshibu's `EaLibrary`).
They talk to the same store services.

## Pieces

| Piece | Where | What it does |
|---|---|---|
| Store clients | `crates/boilr-stores` | Sign-in, session storage, owned-game lists. One module per store. No Steam, no UI. |
| Account platforms | `src/platforms/accounts` | One `GamesPlatform` per store (`epic_account`, `gog_account`, ...). They turn the cached library into shortcuts. |
| Identity and sticky app ids | `crates/boilr-core/src/sync/synchronization.rs` | Each shortcut gets `DevkitGameID = boilr:<store>:<id>`. A sync keeps the app id Steam already has for that identity. |
| Login window | `apps/boilr-tauri/src/accounts.rs` | A private webview per store. When the store lands on its post-login page, BoilR stops the navigation, reads the cookies and the URL, and finishes the sign-in. |
| Accounts screen | `apps/boilr-tauri/src/components/AccountsView.tsx` | Sign in, sign in with your own browser (paste), refresh, sign out, choose what not-installed games open. |
| egui | `AccountPlatform::render_ui` | Paste sign-in for the stores that support it. EA and Battle.net need the login window, so they need the Tauri app. |

## Stores

| Store | Sign-in | Owned list | Not installed: shortcut opens |
|---|---|---|---|
| Epic | Login window, or paste the `authorizationCode` | library-service items, then the catalog. Skips DLC and Unreal assets. | Epic Games Launcher with `apps/<ns>:<catalog>:<app>?action=launch&silent=true`, Heroic on Linux, or the store page |
| GOG | Login window, or paste the URL after login | `embed.gog.com/account/getFilteredProducts` | GOG Galaxy `goggalaxy://openGameView/<id>`, Heroic on Linux, or the store page |
| Amazon | Login window, or paste the URL after login. Registers a device, as the Amazon Games app does. | Entitlements service | Amazon Games `amazon-games://install/<id>`, Heroic on Linux, or the Prime Gaming page |
| EA | Login window only | `getPreloadedOwnedGames` GraphQL. Skips games owned through a linked Steam or Epic account. | EA app `origin2://game/launch/?offerIds=<content id>`, or ea.com |
| Battle.net | Login window only | `account.battle.net/api/games-and-subs` and classic games | `Battle.net.exe --game=<internal id>`, or the download page |
| Humble | Login window, or paste the `_simpleauth_sess` cookie | Orders API: DRM-free games with a download | The order's download page |
| itch.io | Paste an API key | `api.itch.io/profile/owned-keys` | itch app `itch://install?game_id=<id>&launch`, or the game page |

Each store module documents its endpoints and the reference code they come from.

## What happens on a sync

1. An account platform reads its cached library. If the cache is older than 6 hours and the
   account is signed in, it fetches it again first. A failed fetch keeps the old cache, so a
   network problem never removes games from Steam.
2. Installed-game platforms (Epic, Legendary, GOG, Amazon, Heroic, itch, EA) report their
   games with the same identity.
3. `accounts::drop_owned_duplicates` drops owned entries whose game is installed.
4. The sync writes the shortcuts. A game whose identity is already in `shortcuts.vdf` keeps
   that app id, even though its target changed.
5. Owned games go into the Steam collection "Not installed" when collections are on.

## Storage

Each store has a folder at `<config folder>/stores/<store>/`:
- `session.json` holds tokens or cookies, written with mode 0600 on Unix.
- `library.json` holds the last owned list.

Sign-out deletes the folder.

## Not verified yet

None of the store clients have been run against real accounts. The parsers are tested against
fixtures made from the reference code's models. These need a real account, and some need
Windows:

- **Every sign-in, end to end:**
  - Epic's captcha in the login window. It uses the launcher's user agent, as Playnite does.
  - Amazon's sign-in page in a webview.
  - Which EA and Battle.net cookies keep the session alive.
- **Steam honours a kept app id:** the one in `shortcuts.vdf` after the exe changes, across
  restarts and property edits. Check on Windows and Linux.
- **Windows install links:**
  - Whether Epic `?action=launch` on a game that is not installed offers the install. The
    fallback is `store/library`.
  - Whether `amazon-games://install` works while the Amazon client is not running.
  - EA `origin2://` with a content id.
  - Battle.net `--game=`.
- **Matching an installed copy to its owned entry:**
  - EA: whether the `.mfst` id is the account's offer id.
  - itch: whether butler's receipt carries the game id. Older installs without it simply
    don't match.
- **Heroic on Linux:** used only when it is signed in to the same store. It finds the game
  in its own library.

## Not covered

- **Ubisoft Connect:** Playnite only reads the launcher's local cache. BoilR's Ubisoft
  platform already does that for installed games.
- **Xbox / Game Pass:** Xbox has no ownership API. Playnite lists play history.
- **Steam:** native.
- **Linux without Heroic:** Epic, GOG, Amazon, EA and Battle.net have no native client, so
  an owned game's shortcut opens the store page. BoilR does not download games itself.
