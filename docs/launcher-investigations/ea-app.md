# EA app: launching from Steam, playing status, overlay

Tested on 2026-11-11 with **Alice: Madness Returns** (EA app 13.822.0.6335, Steam build
1792618285, Windows 11). See the [overview](README.md) for how Steam tracks shortcuts and
where the logs are.

> **Correction (round 2).** Round 1 below concluded that the EA app refuses to start games
> from inside Steam's tree ("Access is denied"). That came from the test setup: Steam had been
> restarted from an IDE terminal and inherited the IDE's Windows job object, which forbids
> breakaway (see the [overview](README.md#testing-pitfall-steam-started-from-a-terminal)).
> With Steam started normally, the EA app starts games from inside Steam's tree, Steam tracks
> them and **the Steam overlay works** (round 2, R2 and R5). Rows affected by the job are
> marked *(job)*.

## Result

| | Status |
|---|---|
| Game starts from Steam, EA app open or closed | Works (shipped wait helper with `EALauncher.exe`) |
| Steam shows the game as running, returns to Play after it | Works: ~9 s after the game when BoilR started the EA app (it waits for the cloud sync, then closes the EA app), ~7 s when the EA app was open (round 5) |
| Steam overlay in the game | **Works when the EA app was closed** (shipped since round 5, R18), or always with the setting `steam_overlay` (R20). Not when an open EA app takes the game over (R19) |
| Big Picture "Resume game" | Works where the overlay does (user, 2026-11-13, TEST EA7, [results](user-tests.md#results-2026-11-13)) |

The catch of the in-tree route: the EA app then stays in Steam's tree after the game, so Steam
shows the game as running until the EA app is quit (as Ubisoft Connect, see that report). Since
round 5 BoilR closes an EA app it started itself, once the EA app's log says its post-game
cloud sync is done, by asking its window to close. With `steam_overlay` it also closes an open
EA app before the game ([settings and trade-offs](README.md#settings-and-their-trade-offs)).

## The game and the EA app

| | |
|---|---|
| Install folder | `D:\OriginLibrary\Alice Madness Returns\` (Windows uninstall entry `InstallLocation`, `{93A3AB24-36E8-41BA-80C6-CCEC237836DC}`) |
| Installer data | `__Installer\installerdata.xml`, `<contentID>alice2_dd</contentID>` |
| Game exe | `Game\Alice2\Binaries\Win32\AliceMadnessReturns.exe` (32-bit), from `<runtime><launcher><filePath>` with the registry prefix `[HKEY_LOCAL_MACHINE\SOFTWARE\Spicy Horse\Alice Madness Returns\Install Dir]` |
| Launch link | `origin2://game/launch?offerIds=alice2_dd` (content id works as the offer id); the EA app logs offer id `OFB-EAST:57154` |
| Steam app id of BoilR's shortcut | 4055294683, game id 17417358039161241600 (kept across exe changes by the `owned-libraries` identity) |
| EA app settings | `%LOCALAPPDATA%\Electronic Arts\EA Desktop\user_<id>.ini` (`user.igoenabled` = the EA in-game overlay) |

Link handlers registered in `HKEY_CLASSES_ROOT\<scheme>\shell\open\command`:

| Scheme | Command |
|---|---|
| `origin2`, `origin` | `C:\Program Files\Electronic Arts\EA Desktop\EA Desktop\EALauncher.exe "%1"` (path not quoted) |
| `link2ea` | `"...\EA Desktop\Link2EA.exe" "%1" "%2" ... "%9"` |
| `ealink` | `"...\EA Desktop\legacyPM\OriginLegacyCLI.exe" "%1" -wait` |
| `eadm` | missing (BoilR's old exe lookup relied on it) |

What happens on `EALauncher.exe "origin2://..."` (from `EALauncher.log`,
`EALaunchHelper.log`): EALauncher starts `EALaunchHelper.exe <link>` and exits within a second
or two. EALaunchHelper hands the link to the running EA app, or, when it isn't running, starts
`EADesktop.exe -updater_call -ls=LaunchHelper -gameSdkPort=3216 -silent
-launchHelperArgument=<link>` and exits. The game is started by `EADesktop.exe` (its parent,
`createProcess ... breakaway=[true]` in `EADesktop.log`; the game gets a job of its own),
together with `OriginLegacyCompatibility.exe`. The game gets **no arguments**, but about 25
`EA*` environment variables (see R9).

`EABackgroundService.exe` (a Windows service) lives in the same folder as `EADesktop.exe`.

## Round 1 scenarios

Each row: what was run, how, and what happened. "Direct" means started by hand from a shell,
not by Steam. In round 1, Steam had been restarted from the IDE terminal (job).

### Which program opens the link

| # | Command | How | Result |
|---|---|---|---|
| 1 | `EADesktop.exe "origin2://game/launch?offerIds=alice2_dd"` | Direct, EA app running | **Game doesn't start** (60 s). EADesktop ignores the link. This was BoilR's first EA app shortcut (commit f58d193). |
| 2 | `EALauncher.exe "origin2://..."` | Direct, EA app closed | **Game starts.** EA app starts too. |
| 3 | `EALauncher.exe "origin2://..."` | Direct, EA app running | **Game starts.** |
| 4 | `EALauncher.exe "origin2://..."` with Steam's variables (`SteamAppId=0`, `SteamGameId`, `SteamClientLaunch=1`, `SteamEnv=1`, `SteamOverlayGameId`, `SteamPath`) | Direct, EA app running | **Game starts.** The variables change nothing. |
| 5 | `Link2EA.exe "link2ea://launchgame/alice2_dd?platform=origin"` | Direct | **Refused.** `Link2EA.log`: `"emsg":"invalid launch parameters (platform)"`; the user sees an EA dialog "Check that your game's installed correctly". See R10 for `platform=steam`. |
| 6 | `AliceMadnessReturns.exe` (the game exe itself), working dir = its folder | Direct | **Game exits at once** (exit code 100010) and the EA app starts it again under a new process id (EA DRM). Without the EA environment variables, Steam can't start this exe and keep it (see R9). |

### Through Steam (BoilR's shortcut or a test shortcut)

| # | Shortcut (Exe / StartDir / LaunchOptions) | EA app before | Result |
|---|---|---|---|
| 7 | `EADesktop.exe` / *(empty)* / `"origin2://..."` | running | **Fails before starting:** `Failed running GameID ... (OS Error 123)`. Empty StartDir (see overview). Several attempts 17:00–17:13. |
| 8 | `EALauncher.exe` / *(empty)* / `"origin2://..."` | running | **Same OS Error 123.** (The exe fix alone wasn't enough.) |
| 9 | `EALauncher.exe` / `"...\EA Desktop\EA Desktop"` / `"origin2://..."` | running | **Game starts.** Steam tracks `EALauncher.exe` and `EALaunchHelper.exe` only; both exit within seconds; the game (parent `EADesktop.exe`) is not tracked. **Playing status ends at once**: Play shows again, pressing it starts a second launch. **Home:** chime, no overlay. **Resume:** nothing. Reconfirmed without the job (R6). |
| 10 | Same as 9 *(job)* | **closed** | **EA app error, game doesn't start.** Steam tracks `EALauncher` → `EALaunchHelper` → `EADesktop.exe` (+ `EACefSubProcess`, `EALocalHostSvc`, `IGOProxy32`). `EADesktop.log`: `[PROCESS] Create Process: Access is denied. \| exe=[...\AliceMadnessReturns.exe]` then `Game launch error: [9] (LaunchFailed)`. **Caused by the job; works without it (R2).** |
| 11 | Test wrapper: `powershell.exe` / its folder / `-File launch-and-wait.ps1 -Link "origin2://..." -Folder "<game folder>"` (opens the link with `Start-Process`, waits while a process from the game folder runs) | running | **Game starts. Playing status correct:** Steam shows running (Stop button) for the whole game and returns to Play within seconds of quitting. **Home:** chime, no overlay. **Resume** is shown but does nothing (the tracked process has no window). |
| 12 | `EADesktop.exe` (no arguments) started by a wrapper from Steam; then **Play pressed inside the EA app** *(job)* | **closed** | **Same "Access is denied", launch error 9**, twice. Without the job it works: user test 2 (TEST EA6) pressed Play in an EA app that Steam started: game starts, Home opens the overlay, Steam shows running until the EA app exits. |
| 13 | BoilR `--launch-and-wait --launcher "C:\WINDOWS\explorer.exe" ...`, link passed **unquoted** (`Command::arg`) | closed | **Nothing happens**: explorer.exe ignores an unquoted `origin2://...?offerIds=...` link. Reproduced directly: `explorer.exe origin2://...` does nothing, `explorer.exe "origin2://..."` starts the EA app. |
| 14 | BoilR `--launch-and-wait --launcher "C:\WINDOWS\explorer.exe" --link "origin2://..."`, link passed **quoted** (shipped) | closed | **Works.** Steam tracks only BoilR and `explorer.exe`; the EA app starts outside Steam's tree; game running after 6 s. Playing status correct; BoilR exits about 5–10 s after the game. No overlay. With the EA app open: R7. |
| 15 | BoilR `--launch-and-wait --launcher "...\EALauncher.exe" ...` *(job)* | closed | **Access is denied** (as 10). **Works without the job, with overlay (R5).** |

## Round 2 scenarios (Steam started normally)

Steam restarted through `explorer.exe` (outside any job). Every EA app instance started by a
Steam shortcut or through `explorer.exe`. Overlay checked with Shift+Tab plus a screenshot;
the controller's Home button and Resume are still for the user.

| # | What | EA app before | Result |
|---|---|---|---|
| R1 | Job probe: a PowerShell shortcut that queries its own job (`IsProcessInJob`, `QueryInformationJobObject`) and tries `CREATE_BREAKAWAY_FROM_JOB`, with Steam's overlay on and off | — | **Steam uses no job.** Probe in no job, breakaway works. (Under the IDE-started Steam: in the IDE's job, `LimitFlags = 0`, breakaway fails with error 5.) **`gameoverlayrenderer64.dll` is loaded even with the shortcut's overlay turned off**; Steam only adds `SteamNoOverlayUI=1`. |
| R2 | TEST EA1: `EALauncher.exe "origin2://..."`, overlay on (= scenario 10) | closed | **Works, with overlay.** Game after 6 s, `createProcess ... breakaway=[true]` succeeds. `gameprocess_log`: `adding PID ... AliceMadnessReturns.exe` (parent EADesktop, tracked). `gameoverlayrenderer.dll` in the game, **Shift+Tab opened the Steam overlay**. After the game: **Steam keeps the shortcut running until the EA app is closed** (EADesktop and helpers are tracked). |
| R3 | TEST EA2: same, overlay off | closed | Same: game starts and is tracked; no overlay (as set). |
| R4 | EADesktop.exe (no arguments) as a Steam child, then the game asked for with `explorer.exe "origin2://..."` (instead of pressing Play) | closed | Game starts (parent the in-tree EADesktop), Steam's overlay DLL and FPS counter in it, but **Steam doesn't log the game as tracked** and Shift+Tab opens nothing. Pressing Play inside the EA app instead works, overlay included (user test 2, TEST EA6). |
| R5 | BoilR `--launcher "...\EALauncher.exe"` (no `--restart-launcher`) as a Steam child (= scenario 15) | closed | **Works, with overlay.** Game after 5 s; Steam tracks BoilR → EALauncher → EALaunchHelper → EADesktop → game. Shift+Tab overlay. After the game, BoilR exits, but the shortcut stays running until the EA app is closed. |
| R6 | TEST EA1 (= scenario 9) | open (via Explorer) | As scenario 9: game in 3 s, not tracked, Steam back to Play after 5 s, no overlay. |
| R7 | Shipped shortcut (explorer.exe route) | open (via Explorer) | **Works.** Game after 3 s; Steam tracks BoilR and `explorer.exe` only; Steam back to Play ~2 s after the game. No overlay. |
| R8 | EA app in-game overlay setting `user.igoenabled` 0 → 1, scenario 10 again *(job)* | closed | No difference (still the job's "Access is denied"). The EA overlay setting plays no role. |
| R9 | What the EA app gives the game: command line and environment (read from the 32-bit PEB) | — | **No arguments.** Environment adds about 25 `EA*` variables: three login JWTs (valid 3 days), `EALaunchCode`, `EARtPLaunchCode`, `OriginSessionKey`, `ContentId`, `EALicenseToken` (offer id), `EALaunchOwner=EA`, `EALsxPort=3216`, and others. Starting the exe directly **with these variables** keeps it running as the same process (no DRM relaunch), with the EA app open or even closed; the same captured values worked 6+ times over 10 minutes. **Through Steam** (a PowerShell wrapper that sets them and starts the exe): tracked, **Steam overlay works**. Works by reusing EA login tokens, so it's a fallback at best; the in-tree EA app (R2/R5) gives the same without it. |
| R10 | `Link2EA.exe "link2ea://launchgame/alice2_dd?platform=steam"`, and with the offer id `OFB-EAST:57154` | open | **Refused** both: the EA app looks for a Steam entitlement (`No active entitlement found`, `EntitlementNotFound`, `LaunchFailed`). Dead end. |
| R11 | Load `GameOverlayRenderer.dll` into a running Alice (`CreateRemoteThread` + `LoadLibraryW`) | — | DLL loads, game keeps running, **no overlay** (no FPS counter, Shift+Tab nothing). Dead end. |

## Rounds 3 and 4: the helper with `--restart-launcher`

Shortcut TEST EA7: BoilR `--launch-and-wait --game-folder "D:\OriginLibrary\Alice Madness
Returns" --launcher "...\EA Desktop\EALauncher.exe" --link "origin2://..." --restart-launcher`.
Steam started normally.

| # | What | EA app before | Result |
|---|---|---|---|
| R12 | TEST EA7, BoilR before commit cddaa47 | closed | Game after 5 s, tracked, Shift+Tab overlay. **But BoilR ended nothing after the game**: `EA Desktop\EA Desktop` is a **symbolic link** to `...\EA Desktop\13.822.0.6335-1793732195\EA Desktop`, Windows reports the programs' paths through the real folder, and BoilR compared against the link. Steam kept showing the game running until the EA app was closed by hand. Fixed in cddaa47 (folders also match through their real path). |
| R13 | Same | open | Because of the same bug, the open EA app was reused: scenario 9 (not tracked, no overlay). Ending the EA app by hand before the launch and 15 s after the game gave the expected result, and EALauncher restarted it **signed in, no prompt**. |
| R14 | TEST EA7, fixed BoilR, game quit from its own menu | closed | **Works.** Game after 5 s, tracked, Shift+Tab overlay (the first try during the intro video showed only the FPS counter). `Post game cloud sync completed successfully` ~1 s after the game; `closing the launcher (7 programs)` (EADesktop, EALocalHostSvc, 5 EACefSubProcess) 14 s after that; Steam back to Play **16 s** after the game. `EABackgroundService` untouched (BoilR can't read a service's path, so it never matches). |
| R15 | Same | open (via Explorer, signed in) | **Works.** `closing the launcher (7 programs)` before the launch, EALauncher restarts it signed in inside Steam's tree, game after 5 s, tracked, Shift+Tab overlay. Back to Play **17 s** after the game, again after the cloud sync line. |
| R16 | Cloud saves | — | Alice syncs (cloud id `50391_`). The EA app logs `Post game cloud sync completed successfully` (`game.cloud.succ`) 0.8–1.5 s after the game exits, well before BoilR's 5 s + 10 s. Nothing had changed between runs, so a real upload's duration is untested. A real "finished" signal exists here, unlike Ubisoft Connect. |
| R17 | Quitting the EA app properly | running, no game | No quit switch in the exe files, no close-to-tray setting in the ini. **`WM_CLOSE` to the main window and the tray menu's Exit both quit it cleanly in ~1 s**, also when Steam started it silently. `WM_CLOSE` during the post-game sync opens a "Saving your progress to the cloud... EC:10609" dialog that keeps the app open until someone clicks "Skip sync & close", so only ask after the sync line. |

So for EA, `--restart-launcher` with `EALauncher.exe` gives what Ubisoft's `steam_overlay`
gives: tracking, the Steam overlay, and Play again ~16 s after the game. Same risks as for
Ubisoft (a download in the EA app is cut off; a second EA game loses its launcher). Better
than Ubisoft: the cloud-sync line could replace the fixed grace, and `WM_CLOSE` could replace
ending the processes.

## Round 5: what BoilR ships now (2026-11-13)

The implemented routes, through test shortcuts with exactly the arguments BoilR now writes
(TEST EA8: default; TEST EA9: with `--restart-launcher`). Game ended by the script (as a
crash would); Steam started normally. The overlay was checked by Steam starting
`gameoverlayui64.exe -pid <game>` and `gameoverlayrenderer.dll` in the game, not by Shift+Tab.

| # | What | EA app before | Result |
|---|---|---|---|
| R18 | TEST EA8 (default) | closed | Log: `this launch starts the launcher`. Game after 5 s, tracked, overlay DLL and overlay UI attached. After the game: `the launcher's cloud sync is done` (already logged during the 5 s exit check), `asking the launcher to close`, `the launcher closed after 2.0s`. **Steam back to Play 9.1 s after the game** (R14: 16 s). |
| R19 | TEST EA8 (default) | open (via Explorer) | `the launcher is open; it takes the game over and stays open`. Game after 3 s, not in Steam's tree, no overlay. Steam back to Play 7.1 s after the game; EA app still open. |
| R20 | TEST EA9 (`--restart-launcher`) | open | Open EA app asked to close: gone after 2 s. Then as R18: overlay, Play again 9.1 s after the game, EA app closed. |
| R21 | Close request to the "EA" window while a game runs | open, game running | The EA app shows **"Close your game, first"** and stays open. BoilR only asks after the game. |

The EA app's top-level windows (Qt): two titled "EA" (`Qt5152QWindowOwnDCIcon`), plus tray,
power and IME windows. A close request to the "EA" windows quits it in 1–2 s once no game
and no sync runs. The log has `Cloud Sync entry point post game termination` when the sync
after a game starts and `ProcessCoordinator::handleTerminalCloudSyncStatus` when it ends
("Post game cloud sync completed successfully"); BoilR waits up to 10 s for the first and up
to 60 s for the second, and reads only what the log got since the game started.

The minute-long Stop (TEST EA7, user tests): a close request makes the EA app log `Starting
shutdown timeout, duration[60s]`. Steam's Stop probably asks the same way while the game
runs, and the EA app waits out that timeout. Not confirmed.

## Still open

- Controller checks on TEST EA7 and TEST EA6 passed (user, 2026-11-13: Home overlay, Resume,
  Play again). Still odd: Steam's Stop on TEST EA7 hangs about a minute before the game
  closes (see round 5 for a likely cause), and one F12 screenshot failed with the EA app open
  beforehand. TEST EA8 passed the controller checks (user, round 5: Home overlay, Resume, Play
  again after ~7 s; with the EA app open no overlay, as expected), and a real save (14:36)
  synced before BoilR closed the EA app, without a dialog.
- How long a real cloud upload takes (play, let the game save, quit; R16).
- Only one EA game (Alice) was tried. Newer EA games (EA anticheat, Denuvo, their own
  launchers such as the Sims 4 or Battlefield ones) may hand over to other programs or run
  from outside the install folder.
- Games found through old Origin `.mfst` manifests still get `EADesktop.exe` +
  `link2ea://launchgame/<id>` (or Origin's `origin2://`) and no wait helper; untested, likely
  broken (Link2EA handles `link2ea://`, not EADesktop). None installed on the test machine.
- Cross-launcher ideas (Resume forwarding, an overlay window of BoilR's own) are in the
  [overview](README.md#not-yet-tried); both prototypes failed the user tests (test 6) and are
  dropped. They would only have mattered when the EA app is already running and must not be
  restarted.
