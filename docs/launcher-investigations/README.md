# Launcher investigations: Steam playing status and overlay

Findings from testing how Steam on Windows launches and tracks games that another launcher
starts, done on 2026-11-11 to make EA app and Ubisoft Connect games behave in Steam. The
per-launcher reports hold every scenario that was tried, with what worked and what didn't:

- [EA app](ea-app.md): Alice: Madness Returns.
- [Ubisoft Connect](ubisoft-connect.md): Far Cry 3 (plus Far Cry 4 and AC IV in round 2).
- [Tests that need a person](user-tests.md): controller, UAC, passwords, save games.

What BoilR does as a result: `boilr_core::launch_and_wait` (the `--launch-and-wait` mode of
both BoilR binaries), used by the EA and Ubisoft sources on Windows. See
[What BoilR ships](#what-boilr-ships) below, with the settings and their trade-offs.

## Test machine

| | Version |
|---|---|
| Windows | 11 Home, build 26300 |
| Steam client | build 1792618285 (desktop and Big Picture, controller connected) |
| EA app | 13.822.0.6335, `C:\Program Files\Electronic Arts\EA Desktop\EA Desktop\` |
| Ubisoft Connect | 175.1.13416, `C:\Program Files (x86)\Ubisoft\Ubisoft Game Launcher\` |
| BoilR | Tauri app (2.0.0-beta.2), branches `launch-and-wait` / `owned-libraries` |

## How Steam treats a non-Steam shortcut (Windows)

Facts established by testing; Valve documents none of this.

- **App id.** Steam uses the `appid` stored in `shortcuts.vdf`, not one computed from exe and
  name. `steam://rungameid/<id>` starts a shortcut, with `<id> = (appid << 32) | 0x02000000`.
  An id Steam doesn't know gives "Game configuration unavailable" (`AppError_9` in
  `console_log.txt`).
- **Shortcuts are read at startup.** Steam must be closed while `shortcuts.vdf` is written
  (BoilR does that), and only sees the change after it starts again.
- **StartDir must not be empty.** Steam passes StartDir to `CreateProcess` as the working
  directory; an empty one fails before anything runs: `Failed running GameID <id>: <command>
  (OS Error 123)` in `gameprocess_log.txt`, `AppError_46` in `console_log.txt`. Error 123 is
  `ERROR_INVALID_NAME`; reproduced with a direct `CreateProcessW` call with `lpCurrentDirectory
  = ""`. Running the same command by hand works, because a shell always has a working
  directory, which hides this.
- **Launch options keep their quotes.** Steam builds the command line as
  `<Exe> <LaunchOptions>` with the quotes intact (`"C:\...\EALauncher.exe"
  "origin2://..."`). (An early test seemed to show Steam dropping quotes; the cause was
  Windows PowerShell 5.1 mangling embedded quotes when passing arguments to `python`, which
  wrote truncated launch options into the test shortcut.)
- **Playing status = process tree.** Steam logs `AppID <id> adding PID <pid> as a tracked
  process "<command>"` for the program it starts and every program that program starts,
  recursively, and `no longer tracking PID <pid>, exit code <n>` when they exit. The shortcut
  shows as running (Stop button) while any tracked process is alive. A program started by a
  process outside that tree (for example a launcher that was already running) is not tracked,
  even if the tree asked for it.
- **Steam's Stop** ends all tracked processes (they log exit code 1).
- **Overlay and Resume.** The controller's Home button shows the in-game overlay only in a game
  Steam started (in its tree). Otherwise the Steam chime plays and nothing appears, or Big
  Picture opens; there, "Resume game" does nothing when the tracked process has no window (for
  example a waiting helper).
- **How the overlay gets in.** Steam loads `gameoverlayrenderer64.dll` / `gameoverlayrenderer.dll`
  into what it starts, and it follows into child processes. It is loaded **even when the
  shortcut's overlay is turned off** (Steam then only sets `SteamNoOverlayUI=1`). A game can
  get the DLL without being logged as tracked (Far Cry 4, EA round 2 R4); in Far Cry 4 the
  overlay still worked, in R4 it didn't. Loading the DLL into a game that already runs does
  nothing (EA R11).
- **Steam uses no job object** for what it starts (EA round 2 R1).
- **Environment.** Steam sets `SteamAppId`, `SteamGameId`, `SteamClientLaunch`, `SteamEnv`,
  `SteamOverlayGameId` and others for what it starts. Passing those to the EA launcher by hand
  changed nothing (see the EA report).

## Logs and tools

| What | Where |
|---|---|
| Steam: processes it tracks, launch failures | `C:\Program Files (x86)\Steam\logs\gameprocess_log.txt` |
| Steam: launch steps (`GameAction ... LaunchApp`), app errors | `...\Steam\logs\console_log.txt` |
| Steam: sign-in state | `...\Steam\logs\connection_log.txt` |
| EA app | `%LOCALAPPDATA%\Electronic Arts\EA Desktop\Logs\` (`EADesktop.log`, `EALauncher.log`, `EALaunchHelper.log`, `Link2EA.log`) |
| Ubisoft Connect | `C:\Program Files (x86)\Ubisoft\Ubisoft Game Launcher\logs\launcher_log.txt` |
| BoilR's wait helper | `%APPDATA%\boilr\launch-and-wait.log` |

Ways of testing that worked:

- **Launch through Steam from a script:** `Start-Process 'steam://rungameid/<id>'`, then read
  the new lines of `gameprocess_log.txt`.
- **See who started what:** poll `Get-CimInstance Win32_Process` for `ProcessId`,
  `ParentProcessId` and `CommandLine` while a launch runs (short-lived processes like
  `EALauncher.exe` are easy to miss).
- **Test shortcuts:** close Steam (`steam.exe -shutdown`), add or edit an entry in
  `shortcuts.vdf`, start Steam. [`tools/add_shortcut.py`](tools/add_shortcut.py) reads and
  writes the binary VDF (it round-trips a 1043-entry file byte for byte) and adds an entry.
  Pass the values from a file or from Python, not through PowerShell 5.1 arguments (see the
  quotes note above). Back up `shortcuts.vdf` first.
- **Find a shortcut's app id:** read `appid` (little-endian u32 before `AppName`) from
  `shortcuts.vdf`, or the number in its artwork file names in `userdata\<user>\config\grid\`.

- **Check the overlay without a controller:** send Shift+Tab to the game (`SendKeys`) and take
  a screenshot (`System.Drawing` `CopyFromScreen`); works in windowed and most borderless
  games, not reliably in exclusive fullscreen.
- **Check a 32-bit game's modules from 32-bit PowerShell**
  (`C:\Windows\SysWOW64\WindowsPowerShell\v1.0\powershell.exe`): from 64-bit PowerShell,
  `Process.Modules` only shows the WOW64 modules, which is why round 1 missed Steam's overlay
  in Far Cry 3.
- **Job check:** [`tools/job-probe.ps1`](tools/job-probe.ps1), run by a Steam shortcut
  (`powershell.exe -NoProfile -ExecutionPolicy Bypass -File job-probe.ps1 -Tag <name>`), writes
  `probe-<name>.txt` next to itself: whether it runs in a job, the job's limit flags and
  members, whether Steam's overlay DLL is loaded, and whether `CREATE_BREAKAWAY_FROM_JOB` works.
- **Ubisoft game configurations:** [`tools/ubisoft-vdat-decrypt.ps1`](tools/ubisoft-vdat-decrypt.ps1)
  `-Out <file>` decrypts `configurations.vdat`; [`tools/ubisoft-configs.py`](tools/ubisoft-configs.py)
  prints the `start_game` block of the given game ids.
- **Resume / overlay window prototype:** [`tools/boilr-proto/`](tools/boilr-proto/) (standalone
  cargo project; see its README).

### Testing pitfall: Steam started from a terminal

Programs started from an IDE's terminal (here: T3 Code, whose logon script puts its whole
process tree in a Windows job object without `JOB_OBJECT_LIMIT_BREAKAWAY_OK`) inherit that
job. Restarting Steam from such a terminal (`steam.exe -shutdown`, then `Start-Process
steam.exe`) puts Steam and **everything it launches** in that job. The EA app then fails to
start games (`Create Process: Access is denied`, because it uses `CREATE_BREAKAWAY_FROM_JOB`),
which round 1 took for Steam's doing. Users never see this. To restart Steam (or start a
launcher) from a terminal for tests, use `explorer.exe "C:\Program Files (x86)\Steam\steam.exe"`:
Explorer starts it outside the job.

## What BoilR ships

Branch `launch-and-wait` (on top of `ea-app-installed-games`), as of round 5 (2026-11-13):

- On Windows, EA games found by their installer data and Ubisoft games found in the registry
  get a shortcut that runs BoilR itself (`"<BoilR.exe>" --launch-and-wait --game-folder
  "<install folder>" --launcher "<program>" ...`, StartDir = BoilR's folder). BoilR (either
  binary, before its single-instance lock or any window) starts `<program>` as its own child,
  waits up to 10 minutes for a program from the install folder, then stays alive until none
  has run for 5 seconds. Steam tracks BoilR, so the game shows as running and Play doesn't
  start it twice.
- **A launcher BoilR started is closed after the game; one that was open is left alone.**
  Before launching, BoilR checks whether a program from the launcher's folder runs.
  - Closed: the launcher starts inside Steam's tree, so the game gets the overlay and Resume.
    After the game BoilR waits for cloud saves, then closes the launcher, and Steam returns to
    Play.
  - Open: the launcher takes the link over, outside Steam's tree. Status right, no overlay,
    nothing closed.
- **EA** (`--launcher EALauncher.exe --link origin2://...`): BoilR reads the EA app's
  `EADesktop.log` to wait until the post-game cloud sync is done (`--sync-log`), then asks
  the EA app's "EA" window to close (`--close-window`), and only ends its programs if that
  fails. Play again ~9 s after the game (was ~16 s).
- **Ubisoft** (`--launcher upc.exe --link uplay://...`): Ubisoft Connect logs no end of its
  sync and can't be asked to quit (a close request only hides it), so BoilR waits 15 seconds
  and ends it. Play again ~24 s after the game. Before round 5, a Ubisoft Connect started this
  way stayed open and Steam showed the game running until it was quit (U2b).
- **Ubisoft, the game's own program** (setting `start_game_exe`): `--launcher "<game exe>"`
  without a link, `--start-in`, and `--open-first upc.exe`: when Ubisoft Connect isn't
  running, BoilR opens it through Explorer (outside Steam) and waits for its window, then
  starts the game. Ubisoft Connect is never closed; Play again ~7 s after the game. The exe
  comes from Ubisoft Connect's encrypted catalogue (`src/platforms/uplay/catalogue.rs`, see
  [Exe paths](ubisoft-connect.md#exe-paths)); games without one, or whose program needs
  arguments, keep the link.
- Existing shortcuts get the new behaviour from the installed BoilR as far as their arguments
  allow (the Ubisoft fix needs nothing more); the EA and `start_game_exe` routes need an
  import, which rewrites the shortcuts.
- Linux/Proton shortcuts are unchanged.

### Settings and their trade-offs

| Setting (`settings.toml` key) | Default | What it does | Trade-off |
|---|---|---|---|
| *(none)* close a launcher BoilR started | always on | Overlay whenever the launcher was closed; Steam returns to Play after the game | Closes a launcher the user didn't have open. A download started in it during the game stops (EA: asked to close, so the EA app may still object; Ubisoft: ended). A second game started from that launcher during the first one loses it. Not a setting because the alternative (opening the link through Explorer) gives no overlay and nothing else. |
| EA `steam_overlay` | off | Also close an **open** EA app before the game, so the overlay always reaches it | Stops the EA app's downloads; another EA game running at the time loses its launcher; the EA app restarts with each game (signed in, no prompt, R13/R15). |
| Ubisoft `steam_overlay` | off | Same for Ubisoft Connect | As EA, plus Ubisoft Connect can only be ended (no clean quit), and a cloud-save upload longer than 15 s is cut off. |
| Ubisoft `start_game_exe` | off | Steam starts the game's own program; overlay without ever closing Ubisoft Connect, Play ~7 s after the game | Reads Ubisoft Connect's encrypted catalogue cache (format can change without notice; decrypted with the user's own key). Proven on Far Cry 3, Far Cry 4 and AC IV only; newer games may refuse a direct start (Denuvo, Ubisoft+, games that relaunch through Ubisoft Connect). AC IV asked for the password once, Far Cry 4 for admin rights once (prerequisites). |

Kept as constants, not settings (`crates/boilr-core/src/launch_and_wait.rs`): 5 s before a
game counts as closed, 15 s for cloud saves without a log, 10 s for a logged sync to start
and 60 s for it to end, 15 s for a launcher to close. Settings would only move the balance
between a fast Play button and a cut-off upload, which users can't judge.

## Not yet tried

From a review of these reports on 2026-11-11. Launcher-specific items are at the end of the
[EA](ea-app.md#not-yet-tried-review-of-2026-11-11) and
[Ubisoft](ubisoft-connect.md#not-yet-tried-review-of-2026-11-11) reports.

Untested parts of the wait helper:

- **Steam's Stop** while BoilR waits: Steam ends BoilR only; the game, outside the tree,
  probably keeps running.
- **Other Steam features**: Steam Input (the shortcut's controller layout), playtime, "In
  game" for friends, F12 screenshots, game recording. The first three likely follow the
  running status; screenshots and recording need the overlay.
- **Harder cases**: the launcher asking for a login or an update first (10-minute limit),
  games that start a second launcher, games running a program from outside their install
  folder, two games from the same launcher at once.

Status after round 2: the EA app and Ubisoft Connect both give the real Steam overlay when
Steam starts them (EA) or the game's exe (Ubisoft); see the reports. Options 1 and 2 below
only matter where that isn't possible (EA app already running and not to be restarted). Both
exist as a prototype not yet tried through Steam ([`tools/boilr-proto`](tools/boilr-proto/),
`--mode resume|overlay`); option 3 was tried: dead end (EA R11).

Options:

1. **Resume forwarding.** "Resume game" does nothing because the tracked process (BoilR) has
   no window. If the helper owned a small window that, when Steam activates it, brings the
   game's window to the front, Resume would work even without the overlay (Home would still
   open Big Picture).
2. **An overlay window of BoilR's own** (what GlosSI did). Steam already loads its overlay
   into BoilR, because Steam starts it. A transparent, click-through, always-on-top window
   that BoilR keeps presenting (Direct3D) would let the overlay draw over the game. GlosSI
   being discontinued doesn't mean the technique stopped working. Limits: not over
   exclusive-fullscreen games, and input focus has to move to the window while the overlay
   is open.
3. **Loading the overlay into a running game** (`GameOverlayRenderer.dll` /
   `GameOverlayRenderer64.dll`). Only ruled out from research; never tried. Risky with
   anti-cheat and DRM, so last, and only in single-player games without anti-cheat.

## Research (2026-11-11)

Online search for a way to get Steam's overlay into launcher-started games:

- **Steam starts the launcher, the launcher starts the game.** Reported working for the EA app
  in 2024 Steam forum threads (Jedi: Survivor,
  [1](https://steamcommunity.com/app/1774580/discussions/0/4849903519807001192),
  [2](https://steamcommunity.com/app/1774580/discussions/0/4200238624228210446)) and for
  Ubisoft Connect in older threads
  ([Steam forum](https://steamcommunity.com/discussions/forum/1/592889669725736425/)). Fails
  today for the EA app; works for Ubisoft Connect (see the reports).
- **GloSC / [GlosSI](https://github.com/Alia5/GlosSI):** draws the overlay in its own
  transparent window that Steam starts, not in the game. Discontinued.
- **Injecting `GameOverlayRenderer64.dll`:** only at process start (for example a
  [Frida script](https://gist.github.com/9abb40556bba56e745a5e78e47797733) with `SteamGameId`
  set); nothing found that attaches to a running game. Steam still wouldn't track the game, so
  Resume wouldn't work, and injection risks DRM and anti-cheat.
- **Other tools:** Steam ROM Manager has the same problem
  ([#694](https://github.com/SteamGridDB/steam-rom-manager/issues/694), closed without a fix).
  Playnite and UWPHook only get the overlay into games they start themselves.
