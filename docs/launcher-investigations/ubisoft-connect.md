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

## Still open

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
  round 5: BoilR ends a Ubisoft Connect it started (U9). The 15 s wait before that is a guess
  without a cloud-save signal (U8); a real save upload is still untested.
