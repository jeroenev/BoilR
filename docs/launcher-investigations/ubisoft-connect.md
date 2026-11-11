# Ubisoft Connect: launching from Steam, playing status, overlay

Tested on 2026-11-11 with **Far Cry 3** (Ubisoft Connect 175.1.13416, Steam build
1792618285, Windows 11). See the [overview](README.md) for how Steam tracks shortcuts and
where the logs are.

## Result

| | Status |
|---|---|
| Game starts from Steam | Works |
| Steam overlay (Home button), Big Picture "Resume game" | **Works when Steam started Ubisoft Connect** (setting `steam_overlay`) |
| Steam shows the game as running, returns to Play after it | Works with `steam_overlay` (BoilR closes Ubisoft Connect after the game); without it, see scenarios 1 and 4 |

## The game and Ubisoft Connect

| | |
|---|---|
| Game id | 46 (`HKLM\SOFTWARE\WOW6432Node\Ubisoft\Launcher\Installs\46`) |
| Install folder | registry `InstallDir` = `D:/UplayLibrary/Far Cry 3/` (forward slashes) |
| Game exe | `D:\UplayLibrary\Far Cry 3\bin\farcry3.exe` |
| Launch link | `uplay://launch/46/0` |
| BoilR's shortcut before the helper | `"...\Ubisoft Game Launcher\upc.exe"` / StartDir `...\Ubisoft Game Launcher` / `"uplay://launch/46/0"` |
| Steam app id of that shortcut | 3855989155, game id 16561347314489229312 |

Processes: `upc.exe` (Ubisoft Connect), `UplayWebCore.exe` (its browser, several),
`UplayService.exe`, `UbisoftGameLauncher.exe` (starts the game), `UbisoftExtension.exe`,
`UplayCrashReporter.exe`, all directly in `...\Ubisoft Game Launcher\`. Games can be installed
below that folder (`...\Ubisoft Game Launcher\games\`), so BoilR only ends programs directly
in the folder.

## Scenarios

| # | What | Ubisoft Connect before | Result |
|---|---|---|---|
| 1 | BoilR's plain shortcut (`upc.exe "uplay://launch/46/0"`) from Steam | **running** | **Not tested directly.** Expected, from how Steam tracks processes: the new `upc.exe` hands the link to the running one and exits, so Steam shows the game as stopped at once and no overlay (like EA scenario 9). Worth confirming. |
| 2 | Same shortcut from Steam (`steam://rungameid/16561347314489229312`) | **closed** | **Everything works: overlay, Resume, running status.** Steam tracks `upc.exe` and several `UplayWebCore.exe` and `UplayService.exe` processes. The game started after ~40 s via `UbisoftGameLauncher.exe`. Odd: Steam's log lists no `farcry3.exe` and `UbisoftGameLauncher.exe`'s parent (already exited) was not in the log, and `tasklist /m GameOverlayRenderer.dll` found nothing in the game, yet the overlay and Resume work (user-confirmed). |
| 3 | After quitting Far Cry 3 in scenario 2 | (started by Steam) | **Steam keeps showing the game as running** until Ubisoft Connect closes (Steam tracks Ubisoft Connect). Stopping it from Steam ended all tracked processes (`no longer tracking PID ..., exit code 1`). |
| 4 | Quitting Ubisoft Connect from its tray while the game runs | running | `launcher_log.txt`: `Received shutdown request, reason: 1 restart? false` → `Shutdown aborted due to game running`, then `Received shutdown message (quit)`. Ubisoft Connect protects a running game. |
| 5 | `taskkill /IM upc.exe` (window close request, no `/F`) | running, outside Steam | **Doesn't quit**: "SUCCESS: Sent termination signal", Ubisoft Connect keeps running (hidden in the tray), nothing in its log. |
| 6 | Looking for a quit switch: strings in `upc.exe`, `UplayService.exe`, `UbisoftGameLauncher*.exe`, `uplay_r1/r2*.dll`, `libcef.dll` | — | **None found** (no `-quit`, `--shutdown`, `uplay://quit`, ...). The only hit was a CEF `shutdown` function. So BoilR ends the processes. |
| 7 | Cloud saves after the game | — | `launcher_log.txt` has no cloud-save lines. After the game: `Game process ended.`, ~1 s later `Play session (1) successfully deleted for game 46`, `Gear will not be performed`, then quiet. No signal to wait for, hence a fixed grace period. |
| 8 | BoilR `--launch-and-wait --game-folder "D:/UplayLibrary/Far Cry 3" --launcher "...\upc.exe" --link "uplay://launch/46/0" --restart-launcher` from Steam (shipped, setting `steam_overlay` on) | running or closed | **Works** (user-confirmed: overlay opens, game shows running, Resume works). Helper log: `closing the launcher (7 programs)`, `the game started after 9.08s`, `the game closed`, `closing the launcher (7 programs)` 20 s later. With the first timings (10 s exit + 20 s cloud-save grace) Steam returned to Play ~30 s after quitting; shortened to 5 s + 10 s (~15 s). |

## Risks of `steam_overlay` (why it is off by default)

- Ubisoft Connect is ended, not asked to quit (scenarios 5, 6): a running download stops, and
  a cloud-save upload slower than the 10 s grace could be cut off. Nothing tells BoilR when
  the upload is done (scenario 7).
- Before a launch, BoilR ends a running Ubisoft Connect unless a program from *this* game's
  folder runs. Another Ubisoft game running at the time would lose its launcher.
- If Ubisoft Connect needs a login or a game update first, the helper waits up to 10 minutes
  for the game before it gives up (Steam then shows Play; Ubisoft Connect stays open).

## Still open

- Confirm scenario 1 (plain shortcut with Ubisoft Connect already open), and what the helper
  without `steam_overlay` gives there (expected: running status correct, no overlay).
- Why the overlay reaches `farcry3.exe` although Steam doesn't log it as tracked (scenario 2):
  maybe Steam injects through `UplayService.exe`/`UbisoftGameLauncher.exe`, or Ubisoft
  Connect's own Steam integration loads it. Matters for knowing which games it works for.
- A real signal that cloud saves are uploaded (other log files in `logs\`, network activity,
  `service_log.txt`), to replace the fixed grace period.
- Other Ubisoft games, especially ones with a separate launcher exe (the 5 s exit grace is
  meant to cover such hand-overs), and Ubisoft+ titles.

## Not yet tried (review of 2026-11-11)

Gaps in what ships:

- **The default (`steam_overlay` off) was never run.** BoilR then still starts `upc.exe` as
  its own child, only without closing Ubisoft Connect first. Expected: when Ubisoft Connect
  was closed, Steam starts it inside its tree, and after the game Steam keeps showing the
  game as running until Ubisoft Connect is closed (as scenario 3); when it was open, the new
  `upc.exe` hands the link over and exits, and the status is right. A likely fix: with the
  setting off, open the link through `explorer.exe`, as for EA (status always right, no
  overlay).

Options not tried, cheapest first:

1. **Steam starts the game's exe directly.** Done for EA (DRM relaunch, EA scenario 6), never
   for Ubisoft. Many Ubisoft games start Ubisoft Connect themselves for the login check and
   keep running in the same process. If Far Cry 3 does, Steam tracks the game itself:
   overlay, Resume and status without ending Ubisoft Connect. Open part: the exe path (the
   registry only has the install folder; Ubisoft Connect's cached game configurations in
   `...\Ubisoft Game Launcher\cache\configuration\` should have it).
2. **Quitting Ubisoft Connect properly** instead of ending its processes: a setting that
   makes closing the window quit instead of hiding to the tray, the log-off messages
   (`WM_QUERYENDSESSION` / `WM_ENDSESSION`) to its windows, or its tray menu's Quit through UI
   automation. A proper quit keeps its "game running" protection (scenario 4) and may finish
   cloud saves.
3. **A cloud-save signal from traffic:** wait until `upc.exe` stops sending data after the
   game (per-process I/O counters), instead of a fixed 10 s. Needs a run where the game
   saved.
4. Cross-launcher ideas are in the [overview](README.md#not-yet-tried).
