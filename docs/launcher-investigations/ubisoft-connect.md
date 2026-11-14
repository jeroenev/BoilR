# Ubisoft Connect: launching from Steam, playing status, overlay

Tested on 2026-11-11 with **Far Cry 3** (plus Far Cry 4 and AC IV in round 2), Ubisoft Connect
175.1.13416, Steam build 1792618285, Windows 11. See the [overview](README.md) for how Steam
tracks shortcuts and where the logs are.

> **Note (round 2).** Round 1 ran while Steam had been restarted from an IDE terminal and sat
> in the IDE's job object (see the [overview](README.md#testing-pitfall-steam-started-from-a-terminal)).
> Ubisoft Connect was not visibly affected: round 2 repeated the key cases with Steam started
> normally and got the same results. Far Cry 3's game process is **`farcry3_d3d11.exe`**, not
> `farcry3.exe`, which explains round 1's "not tracked" puzzle (U6).

## Result

| | Status |
|---|---|
| Game starts from Steam | Works |
| Steam overlay, Big Picture "Resume game" | **Works when Steam starts the game's own exe** (U3, U4; setting `start_game_exe` since round 5, U10–U12), or when Steam starts Ubisoft Connect: by default when it was closed (round 5, U9), always with the setting `steam_overlay` |
| Steam shows the game as running, returns to Play after it | Game's own exe: ~7 s after the game (BoilR's 5 s check included). Link, Ubisoft Connect started by this launch: ~24 s (BoilR waits 15 s, then ends it; U9). Link, Ubisoft Connect open: ~7 s |

**Best approach found: Steam starts the game's exe directly** (round 2, U3/U4). Far Cry 3 and
4 start Ubisoft Connect themselves when needed, Steam tracks the game, the overlay works,
Steam returns to Play 3 s after the game, and Ubisoft Connect is never ended (when the game
starts it, Steam doesn't count it). It needs each game's exe path, which Ubisoft Connect keeps
in an encrypted cache (see [Exe paths](#exe-paths)). Shipped since round 5 as the opt-in
setting `start_game_exe`, with Ubisoft Connect opened first through Explorer (see round 5).

## The game and Ubisoft Connect

| | |
|---|---|
| Game id | 46 (`HKLM\SOFTWARE\WOW6432Node\Ubisoft\Launcher\Installs\46`) |
| Install folder | registry `InstallDir` = `D:/UplayLibrary/Far Cry 3/` (forward slashes) |
| Game exe | `bin\farcry3.exe` (stub) / `bin\FC3Updater.exe` (what Ubisoft Connect starts) → real game `bin\farcry3_d3d11.exe` (32-bit) |
| Launch link | `uplay://launch/46/0` |
| BoilR's shortcut before the helper | `"...\Ubisoft Game Launcher\upc.exe"` / StartDir `...\Ubisoft Game Launcher` / `"uplay://launch/46/0"` |
| Steam app id of that shortcut | 3855989155, game id 16561347314489229312 |

How Far Cry 3 starts (every run): Ubisoft Connect or Steam runs `bin\FC3Updater.exe` or
`bin\farcry3.exe` → a `farcry3_d3d11.exe` that lives < 1 s → `UbisoftGameLauncher.exe
-gamelauncher_wait_handle ... -upc_uplay_id 46 ...` → `FC3Updater.exe -language=English` → the
real `farcry3_d3d11.exe -language=English`. All parent → child. When the stub starts and
Ubisoft Connect isn't running, `UbisoftGameLauncher.exe` starts `upc.exe -upc_uplay_id 46
-upc_desktop_mode -upc_game_starter`; Steam doesn't track that one.

Processes of Ubisoft Connect: `upc.exe`, `UplayWebCore.exe` (its browser, several),
`UplayService.exe`, `UbisoftGameLauncher.exe` (starts the game), `UbisoftExtension.exe`,
`UplayCrashReporter.exe`, all directly in `...\Ubisoft Game Launcher\`. Games can be installed
below that folder (`...\Ubisoft Game Launcher\games\`), so BoilR only ends programs directly
in the folder.

## Round 1 scenarios

| # | What | Ubisoft Connect before | Result |
|---|---|---|---|
| 1 | BoilR's plain shortcut (`upc.exe "uplay://launch/46/0"`) from Steam | **running** | Not tested in round 1; confirmed in round 2 (U1). |
| 2 | Same shortcut from Steam (`steam://rungameid/16561347314489229312`) | **closed** | **Everything works: overlay, Resume, running status.** Steam tracks `upc.exe` and several `UplayWebCore.exe` and `UplayService.exe` processes. The game started after ~40 s via `UbisoftGameLauncher.exe`. (The "game not in Steam's log" was a search for the wrong exe name, and a 64-bit module listing, which can't see a 32-bit game's modules: U6.) |
| 3 | After quitting Far Cry 3 in scenario 2 | (started by Steam) | **Steam keeps showing the game as running** until Ubisoft Connect closes (Steam tracks Ubisoft Connect). Stopping it from Steam ended all tracked processes (`no longer tracking PID ..., exit code 1`). |
| 4 | Quitting Ubisoft Connect from its tray while the game runs | running | `launcher_log.txt`: `Received shutdown request, reason: 1 restart? false` → `Shutdown aborted due to game running`, then `Received shutdown message (quit)`. Ubisoft Connect protects a running game. |
| 5 | `taskkill /IM upc.exe` (window close request, no `/F`) | running, outside Steam | **Doesn't quit** with the default setting: "SUCCESS: Sent termination signal", Ubisoft Connect keeps running (hidden in the tray). See U7a for the setting that changes this. |
| 6 | Looking for a quit switch: strings in `upc.exe`, `UplayService.exe`, `UbisoftGameLauncher*.exe`, `uplay_r1/r2*.dll`, `libcef.dll` | — | **None found** (no `-quit`, `--shutdown`, `uplay://quit`, ...). |
| 7 | Cloud saves after the game | — | `launcher_log.txt` has no cloud-save lines. After the game: `Game process ended.`, ~1 s later `Play session (1) successfully deleted for game 46`, `Gear will not be performed`, then quiet. |
| 8 | BoilR `--launch-and-wait --game-folder "D:/UplayLibrary/Far Cry 3" --launcher "...\upc.exe" --link "uplay://launch/46/0" --restart-launcher` from Steam (shipped, setting `steam_overlay` on) | running or closed | **Works** (user-confirmed: overlay opens, game shows running, Resume works). Steam returns to Play ~15–18 s after quitting (5 s exit + 10 s cloud-save grace + ending Ubisoft Connect). Reconfirmed in U6. |

## Round 2 scenarios (Steam started normally)

Ubisoft Connect and games started only through Steam shortcuts or `explorer.exe`. Overlay
checked with Shift+Tab plus a screenshot; the controller's Home button and Resume are still
for the user.

| # | What | UC before | Result |
|---|---|---|---|
| U1 | Plain shortcut `upc.exe "uplay://launch/46/0"` (= scenario 1) | running | **As expected:** the new `upc.exe` hands the link over and exits after 3 s; Steam shows Play before the game is even in its menu. Game not tracked, no Steam overlay (Ubisoft's own `overlay.dll` only). |
| U2a | Shipped helper, **default** (`steam_overlay` off: `--launcher upc.exe`, no `--restart-launcher`) | running | Status right, no overlay. Steam tracks BoilR and the short-lived `upc.exe`; game after 3 s; Steam back to Play 7 s after the game. |
| U2b | Same | **closed** | **Overlay works, but Steam never returns to Play**: `upc.exe` starts inside Steam's tree, the game (`farcry3_d3d11.exe`) is tracked, Shift+Tab opens the overlay; after the game the helper exits, but the shortcut stays running until Ubisoft Connect is quit. |
| U3a | Steam starts `bin\farcry3.exe` directly | closed | **Everything works.** Tracked: `farcry3.exe` → `UbisoftGameLauncher.exe` → `FC3Updater.exe` → `farcry3_d3d11.exe`. The game launcher started Ubisoft Connect itself, **untracked**. No login prompt (saved login). Overlay DLL, Shift+Tab overlay. Steam back to Play **3 s** after the game; Ubisoft Connect stays in the tray. |
| U3b | Same | running | Same, works. The running Ubisoft Connect hands the game to a `UbisoftGameLauncher.exe` that is a child of `farcry3.exe` (tracked). Back to Play 3 s after the game. |
| U4 FC4 | Steam starts `bin\FarCry4.exe` directly (2 runs) | running | **Works**, after a **UAC prompt on both agent launches** (the user saw it only on their first launch, and round 5's U12 had none, so it goes away once the prerequisites are in place): Ubisoft Connect retries Far Cry 4's prerequisite installers, which fail (`One or more installers failed for game 420`), then the game starts anyway. The real game process is **not logged as tracked**, yet Steam's overlay is loaded in it (FPS counter, Shift+Tab overlay in windowed mode) and the status is right: the stub and the game launcher stay alive until the game exits. Back to Play 3 s after the game. |
| U4 AC4 | Steam starts `AC4BFSP.exe` directly | running | Blocked by Ubisoft Connect: UAC prompt (installers), then **"Re-enter your password to play this game"**. Status "running" the whole time; ended by hand. |
| U4 WD, ACO | Steam starts `watch_dogs.exe` / `ACOrigins.exe` | running | `Game needs download`: Ubisoft Connect doesn't know these as installed (not in the registry's `Installs`). Fail fast (Play again after 4–11 s). |
| U5 | Where the exe paths are | — | See [Exe paths](#exe-paths). |
| U6 | Shipped "Far Cry 3" shortcut (`--restart-launcher`) | running | Works: `closing the launcher (9 programs)`, game after 9 s, **`farcry3_d3d11.exe` logged as tracked**, overlay DLL (32-bit module listing), Shift+Tab overlay. Back to Play 18 s after the game. |
| U7a | Setting `closebehavior: CloseBehavior_Close` in `%LOCALAPPDATA%\Ubisoft Game Launcher\settings.yaml` (default `CloseBehavior_Minimize`), then `WM_CLOSE` to the `Chrome_WidgetWin_1` "Ubisoft Connect" window | running | **Clean quit in ~6 s**, all processes gone, log `Received shutdown message (quit), returnCode: 0`. Works on the hidden window too. Ubisoft Connect rewrites `settings.yaml` on exit, so change it while it's closed. |
| U7b | `WM_QUERYENDSESSION` + `WM_ENDSESSION` to all upc.exe windows | running | No effect. |
| U7c | Tray icon → right-click → Quit, through UI Automation and keys | running | Works with default settings (quit in 6 s), but moves the mouse and needs the desktop: fragile. |
| U7 game | U7a with a game running | started by the game | Protection honoured (`Shutdown aborted due to game running`), but a "Cancel / Force Quit" dialog stays on screen after the game exits. **Only ask Ubisoft Connect to quit after the game has exited.** |
| U8 | `upc.exe` I/O counters around the game's exit | — | Within 1 s after the game: ~1.5 MB read/write burst; a 75 KB write ~11 s later; then only a 5 s heartbeat. No save had been made, so this is the baseline, not an upload. |

## Exe paths

Ubisoft Connect keeps each game's launch data in
`%LOCALAPPDATA%\Ubisoft Game Launcher\cache\configuration\configurations.vdat` (the old plain
`cache\configuration\configurations` in the launcher folder is from 2023 and no longer
updated).

- **Format:** protobuf `{1: {1: header{1:1, 2:{1:1, 2:"<keyId>", 3:nonce(12), 4:tag(16)}}, 2:
  ciphertext}}`, **AES-256-GCM**. The key is a DPAPI blob (current user, no entropy) in
  `%LOCALAPPDATA%\Ubisoft Game Launcher\store\<keyId>.bin`. The plaintext is the old container:
  repeated `{1: product id, 2: id, 3: YAML}`.
- **YAML:** `root.start_game.online.executables[]` (and `offline`), each with `path`
  (`relative:` or `register:`), `working_directory` (`register:` + optional `append:`),
  `arguments` / `arguments_register` / `play_arguments`. Registry paths are written as
  `HKEY_LOCAL_MACHINE\SOFTWARE\Ubisoft\...`; read them in the 32-bit view (`WOW6432Node`).
- **Rule:** working dir = the value named by `working_directory.register` (+ `append`); exe =
  working dir + `path.relative`, or + the value named by `path.register`. Take the first
  executable.

| Id | Game | From the config | Checked against |
|---|---|---|---|
| 46 | Far Cry 3 | wd `...\Ubisoft\Far Cry 3\InstallDir`; exe from `HKCU\SOFTWARE\Ubisoft\Far Cry 3\ExecutableName` = `bin/FC3Updater.exe` | what Ubisoft Connect ran (`bin\farcry3.exe` works too, U3) |
| 273 | AC IV Black Flag | wd `Installs\273\InstallDir`; `AC4BFSP.exe` | `UbisoftGameLauncher.exe -upc_exe_path` |
| 420 | Far Cry 4 | wd `Installs\420\InstallDir` + `bin`; `FarCry4.exe` | `-upc_exe_path`, `-upc_working_directory` |
| 274 | Watch_Dogs | `HKLM\...\Watch_Dogs\Exe`, wd `Installs\274\InstallDir` + `bin\` | not registered here |
| 3539 | AC Origins | `ACOrigins.exe`, `arguments: /appid:{AppId}` (placeholder) | not registered here |

Without decryption, `UbisoftGameLauncher.exe`'s command line carries `-upc_exe_path`,
`-upc_working_directory`, `-upc_arguments` (base64), but only once a game has started.

## Risks of `steam_overlay` (why it is off by default)

- Ubisoft Connect is ended, not asked to quit: a running download stops, and a cloud-save
  upload slower than the grace period (15 s since round 5) could be cut off. U7a/U7c show
  proper quits are possible, but need a changed `settings.yaml` or UI automation. Since round
  5 the same applies, without the setting, to a Ubisoft Connect that BoilR started itself.
- Before a launch, BoilR ends a running Ubisoft Connect unless a program from *this* game's
  folder runs. Another Ubisoft game running at the time would lose its launcher.
- If Ubisoft Connect needs a login or a game update first, the helper waits up to 10 minutes
  for the game before it gives up (Steam then shows Play; Ubisoft Connect stays open).

## Round 5: what BoilR ships now (2026-11-13)

Test shortcuts with exactly the arguments BoilR now writes. Game ended by the script; Steam
started normally; overlay checked by `gameoverlayui64.exe -pid <game>` and the overlay DLL in
the game, not by Shift+Tab.

| # | What | UC before | Result |
|---|---|---|---|
| U9 | TEST UB3: the default helper (`upc.exe` + link), with the installed BoilR | closed | `this launch starts the launcher; it is closed after the game`. Game after 9 s, overlay DLL and overlay UI. After the game, 15 s wait, `ending the launcher (7 programs)`; **Steam back to Play 24 s after the game** (U2b: never). |
| U10 | TEST UB8: Far Cry 3's own program (`FC3Updater.exe`, from the catalogue), `--open-first upc.exe` | closed | `opening the launcher first`, Ubisoft Connect's window after 3 s, game started 3 s later. Tracked: `FC3Updater.exe` → `farcry3_d3d11.exe`, then `UbisoftGameLauncher.exe` → `FC3Updater.exe` → `farcry3_d3d11.exe` (the game restarts itself through Ubisoft Connect, still inside the tree). Overlay attached. Steam back to Play 7 s after the game; Ubisoft Connect stays open. |
| U11 | Same | open | Game after 3 s, overlay, Play again 7 s after the game. |
| U12 | TEST UB9: Far Cry 4's own program | open | Game at once, 64-bit overlay DLL, overlay UI attached. Play again 7 s after the game. No admin prompt. |

`catalogue.rs` decrypts `configurations.vdat` with Windows' own DPAPI and CNG AES-GCM (no new
crates) and finds on this machine: 46 → `bin\FC3Updater.exe` (start in the install folder),
420 → `bin\FarCry4.exe`, 273 → `AC4BFSP.exe`. AC Origins is skipped (its program needs
`/appid:{AppId}`); Watch_Dogs isn't found (its registry entries are missing).

Whether opening Ubisoft Connect first fixes the overlay that flashed and vanished in the user
test (4a) needs the controller's Home button: see [user tests](user-tests.md).

## Round 6: how to close a Ubisoft Connect that BoilR started (2026-11-13)

Round 5 ends a Ubisoft Connect it started, 15 s after the game, without any setting turned on.
That stops a running download and could cut off an upload. Two alternatives were tested.

| # | What | UC before | Result |
|---|---|---|---|
| U13 | (b) Clean quit: `closebehavior: CloseBehavior_Close` set in `settings.yaml` while Ubisoft Connect was closed, TEST UB2 (Steam starts `upc.exe` with the link), game ended, then a close request to the "Ubisoft Connect" windows 5 s later | closed | Overlay as usual. Close request to 3 windows; log `Received shutdown request, reason: 1`, `Received shutdown message (quit), returnCode: 0`; all programs gone **7.5 s** later; **Steam back to Play 12 s after the game** (forced: 24 s). Ubisoft Connect wrote `settings.yaml` back on exit with the changed value, so restoring must happen after it exits. |
| U14 | (c) TEST UB10: BoilR with `--launcher explorer.exe --link uplay://launch/46/0` | closed | Ubisoft Connect starts outside Steam's tree. **No Steam overlay** (no `gameoverlayrenderer.dll` in the game, Ubisoft's own overlay only). Steam back to Play 7 s after the game; Ubisoft Connect stays open. |
| U15 | (b) as BoilR does it now: TEST UB11 (`--close-window "Ubisoft Connect" --setting-file ...settings.yaml --setting-from "closebehavior: CloseBehavior_Minimize" --setting-to "closebehavior: CloseBehavior_Close"`) | closed | `changed the launcher's setting for this launch`, overlay as usual, after the game 15 s wait, `asking the launcher to close`, quit after 6 s (`returnCode: 0`), `changed the launcher's setting back` (file back to `CloseBehavior_Minimize`). **Steam back to Play 28.5 s after the game.** |

Chosen: (b), with the 15 s wait kept (the user, 2026-11-13).

**Does a clean quit wait for cloud saves?** Probably not.
- An agent read Ubisoft Connect's memory (its `upc.exe` and `UplayService.exe` are packed by a
  custom packer: `.text`/`.data` empty on disk, a 17 MB `.ubx2` section at entropy 7.97).
  `biz\misc\Shutdown.cpp` has two outcomes, `Shutdown aborted due to game running` and
  `Proceeding to shutdown`; the shutdown messages have one failure,
  `MessageShutdownFailedGameRunning`, and none for a sync. No quit-time sync text exists.
  Medium-high confidence (absence of strings, not read code).
- Uploads run at game end: `CloudSaveSyncGameEnd.cpp`, `CloudSaveUpload.cpp`,
  `BusCloudSave.cpp` (`Synchronization job on game end should be completed.`), usage fields
  `When = START|END`, `Direction = UPLOAD|DOWNLOAD`. Successful syncs aren't logged at all.
- Next to `CloudSaveSyncGameStart.cpp`: `Fail to get a timestamp from upload marker`, `Failed
  to delete upload marker`, `*.upload`, `*.delete`. So `N.save.upload` most likely marks a
  slot that changed and isn't confirmed uploaded, and the next game start's sync handles it:
  a cut-off upload would be retried then (inferred, not confirmed).
- Online (no official answer): players wait at least 10 s before closing Ubisoft Connect;
  others also find stray 0-byte `.save.upload` files; Playnite closes Ubisoft Connect after a
  game with `taskkill`, then `taskkill /f` after 3 s; GOG Galaxy's plugin uses `taskkill`
  without `/f`. No reports of saves lost that way. Sources:
  [Shadows thread](https://steamcommunity.com/app/3159330/discussions/1/591767099278427310),
  [Playnite UplayClient.cs](https://github.com/JosefNemec/PlayniteExtensions/blob/master/source/Libraries/UplayLibrary/UplayClient.cs),
  [GOG Galaxy plugin](https://github.com/FriendsOfGalaxy/galaxy-integration-uplay/blob/master/src/plugin.py).

Earlier notes, before the analysis:
- Ubisoft Connect logs no sync start or end around a game's exit (`Game process ended`, `Play
  session ... deleted`, nothing else), and its programs carry no readable shutdown strings
  (packed).
- Its save folder (`<launcher>\savegames\<user id>\<game id>\`) still holds an empty
  `1.save.upload` next to AC IV's `1.save`, both written during the user's AC IV test
  (08:36). Ubisoft Connect was quit cleanly 42 s after that game (08:38:56, `reason: 1`, the
  same quit as U13), and has run several times since; the marker stayed. So either the
  marker doesn't mean "upload pending", or nothing uploaded it, even with a clean quit. It
  can't serve as a signal to wait for.
- Only a test with a real save can tell: play, save, quit, close Ubisoft Connect cleanly, and
  check on another computer (or after clearing the local save) whether the save arrived.

## Round 7: starting the game's own program too early (2026-11-13)

The user's R5-3 (TEST UB8, Ubisoft Connect closed) failed with Ubisoft Connect asking to
restart and sign in again. Its log for both runs: Ubisoft Connect started, its window
appeared within a second, BoilR started the game 3 s later, and the game connected **at the
same second as** `AccountStartupUser.cpp (238) User: <id>` (signed in), as `product id 0`.
20–30 s later: `Demux will be closed due to 12`, `Ownership connection is not set up (11-5002)`,
`Shell recovery page ... {"retryOwnershipInit"...}`. The agent's U10 had worked by timing:
there the game connected one second after the sign-in line. Normally the account's ownership
data loads 1–2 s after that line (`OwnershipSaveToCache.cpp ... Could not read ownership
cache`).

Fix: `--open-first` now waits for a line in the launcher's log (`--open-first-log
<launcher>\logs\launcher_log.txt --open-first-ready "AccountStartupUser.cpp"`, up to 5 minutes,
so a sign-in by the user fits), then 5 s more, instead of waiting for the window.

| # | What | UC before | Result |
|---|---|---|---|
| U16 | TEST UB12 (Far Cry 3's own program, new arguments), 2 runs | closed | `the launcher was ready after 7.0s`; game started 5 s later and connected as `product id 46`; no ownership error, no recovery page. Overlay DLL and overlay UI attached; the user saw the overlay work (main menu). Steam back to Play ~8 s after the game; Ubisoft Connect stays open. |

## Round 8: shorter waits (2026-11-13)

Tried, at the user's request: start the game's own program 1 s after the sign-in line
instead of 5 s, and close a Ubisoft Connect BoilR started 5 s after the game instead of 15 s.

| # | What | UC before | Result |
|---|---|---|---|
| U17 | TEST UB12 with 1 s after sign-in, 3 runs | closed | Runs 1 and 2: game connected as `product id 46` right after ownership loaded, no error, Steam's overlay DLL in the game, **but the user saw no overlay**. Run 3: the game connected as `product id 0` at the same second as ownership loaded (the pattern of the failed R5-3), and got through by timing. **Dropped: back to 5 s**, which worked for the user in round 7. Ubisoft Connect is evidently still starting up 1 s after signing in, which breaks Steam's overlay without an error (compare user test 4a). |
| U18 | TEST UB11 with 5 s before the quit | closed | `asking the launcher to close` 5 s after the game, quit after 6 s (`returnCode: 0`), setting back to `CloseBehavior_Minimize`. **Steam back to Play 18.2 s after the game** (was ~28 s). Kept: the user's Far Cry 3 save showed as uploaded within 2 s of the game closing (R5-5). |

## Still open

- AC IV asks for admin rights and the Ubisoft password on every launch tested, through the
  default shortcut too (user, 2026-11-14). Whether it does the same when started from Ubisoft
  Connect itself is unchecked; if so, nothing in BoilR can avoid it.

- ~~**Far Cry 3's own program**~~: solved in round 9. The catalogue's program for Far Cry 3
  is `FC3Updater.exe` (registry `ExecutableName`), which is also the entry's `evil_updater`.
  It starts the game, which then restarts itself through Ubisoft Connect (two
  `farcry3_d3d11.exe`, U10), and Steam's overlay was unreliable that way (user, R8-1; also
  with Ubisoft's in-game overlay off, R8-2). Through `bin\farcry3.exe` (TEST UB15) the game
  starts once and the overlay worked in the main menu and in a level (user, R9-1). The
  catalogue doesn't name `farcry3.exe`, so BoilR can't find it; instead, a game whose program
  is its own `evil_updater` keeps the link (`catalogue.rs`), and Far Cry 3 uses the default
  shortcut, which works fully (R8-4). A per-game override (46 → `bin\farcry3.exe`) would give
  Far Cry 3 the own-program route too; not done, as it is game-specific knowledge in BoilR.
- The earlier Far Cry 3 in-game crash (R5-5, TEST UB11) didn't come back in R8-4.

- Controller checks (user, 2026-11-13, [results](user-tests.md#results-2026-11-13)): the
  direct exe works fully for Far Cry 3, Far Cry 4 and AC IV **when Ubisoft Connect is already
  running**. With Ubisoft Connect closed, Far Cry 3's overlay flashes and disappears (Resume
  and status fine). Round 5 opens Ubisoft Connect through `explorer.exe` first (U10); the
  Home button on that route is untested.
- Far Cry 4's admin prompt came only once. Watch_Dogs and AC Origins after "Locate
  installed game" are untested.
- Games where the direct exe doesn't work: Ubisoft+ titles, games with Denuvo or anti-cheat,
  games whose exe is a separate launcher.
- A real cloud-save signal: rerun U8 after saving in the game.
- The default helper with Ubisoft Connect closed (U2b) left Steam showing "running". Fixed in
  round 5: BoilR ends a Ubisoft Connect it started (U9); since round 6 it asks it to quit
  instead (U15). The 15 s wait before that is a guess without a cloud-save signal (U8, round
  6); a real save upload is still untested.
