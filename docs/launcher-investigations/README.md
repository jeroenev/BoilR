# Launcher investigations: Steam playing status and overlay

Findings from testing how Steam on Windows launches and tracks games that another launcher
starts, done on 2026-11-11 to make EA app and Ubisoft Connect games behave in Steam. The
per-launcher reports hold every scenario that was tried, with what worked and what didn't:

- [EA app](ea-app.md): Alice: Madness Returns.
- [Ubisoft Connect](ubisoft-connect.md): Far Cry 3.

What BoilR does as a result: `boilr_core::launch_and_wait` (the `--launch-and-wait` mode of
both BoilR binaries), used by the EA and Ubisoft sources on Windows. See
[What BoilR ships](#what-boilr-ships) below.

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

Checks that turned out unreliable: `tasklist /m GameOverlayRenderer.dll` and
`Process.Modules` did not show Steam's overlay in Far Cry 3 (32-bit), although the overlay
worked there.

## What BoilR ships

Branch `launch-and-wait` (on top of `ea-app-installed-games`):

- On Windows, EA games found by their installer data and Ubisoft games found in the registry
  get a shortcut that runs BoilR itself:
  `"<BoilR.exe>" --launch-and-wait --game-folder "<install folder>" --launcher "<program>"
  --link "<link>" [--restart-launcher]`, StartDir = BoilR's folder.
- BoilR (either binary, before its single-instance lock or any window) starts `<program>` with
  the quoted link as its own child, waits up to 10 minutes for a program from the install
  folder, then stays alive until none has run for 5 seconds. Steam tracks BoilR, so the game
  shows as running and Play doesn't start it twice.
- EA: `<program>` is `explorer.exe`, so the EA app starts outside Steam's tree (see the EA
  report for why). No Steam overlay.
- Ubisoft: `<program>` is `upc.exe`. With the Ubisoft setting `steam_overlay` (off by default),
  `--restart-launcher` ends Ubisoft Connect before the launch and 10 seconds after the game,
  so Steam starts it (overlay, Resume) and returns to Play.
- Linux/Proton shortcuts are unchanged.

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
