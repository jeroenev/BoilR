# EA app: launching from Steam, playing status, overlay

Tested on 2026-11-11 with **Alice: Madness Returns** (EA app 13.822.0.6335, Steam build
1792618285, Windows 11). See the [overview](README.md) for how Steam tracks shortcuts and
where the logs are.

## Result

| | Status |
|---|---|
| Game starts from Steam, EA app open or closed | Works (BoilR wait helper, link through `explorer.exe`) |
| Steam shows the game as running, returns to Play after it | Works (wait helper) |
| Steam overlay in the game (Home button) | **No way found**: chime, no overlay |
| Big Picture "Resume game" | **Doesn't work**: does nothing |

The overlay needs the game in Steam's process tree, and the EA app refuses to start games
while it runs inside that tree (scenarios 9 and 10).

## The game and the EA app

| | |
|---|---|
| Install folder | `D:\OriginLibrary\Alice Madness Returns\` (Windows uninstall entry `InstallLocation`, `{93A3AB24-36E8-41BA-80C6-CCEC237836DC}`) |
| Installer data | `__Installer\installerdata.xml`, `<contentID>alice2_dd</contentID>` |
| Game exe | `Game\Alice2\Binaries\Win32\AliceMadnessReturns.exe` (32-bit), from `<runtime><launcher><filePath>` with the registry prefix `[HKEY_LOCAL_MACHINE\SOFTWARE\Spicy Horse\Alice Madness Returns\Install Dir]` |
| Launch link | `origin2://game/launch?offerIds=alice2_dd` (content id works as the offer id) |
| Steam app id of BoilR's shortcut | 4055294683, game id 17417358039161241600 (kept across exe changes by the `owned-libraries` identity) |

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
-launchHelperArgument=<link>` and exits. The game is started by `EADesktop.exe` (its parent),
together with `OriginLegacyCompatibility.exe`.

## Scenarios

Each row: what was run, how, and what happened. "Direct" means started by hand from a shell,
not by Steam.

### Which program opens the link

| # | Command | How | Result |
|---|---|---|---|
| 1 | `EADesktop.exe "origin2://game/launch?offerIds=alice2_dd"` | Direct, EA app running | **Game doesn't start** (60 s). EADesktop ignores the link. This was BoilR's first EA app shortcut (commit f58d193). |
| 2 | `EALauncher.exe "origin2://..."` | Direct, EA app closed | **Game starts.** EA app starts too. |
| 3 | `EALauncher.exe "origin2://..."` | Direct, EA app running | **Game starts.** |
| 4 | `EALauncher.exe "origin2://..."` with Steam's variables (`SteamAppId=0`, `SteamGameId`, `SteamClientLaunch=1`, `SteamEnv=1`, `SteamOverlayGameId`, `SteamPath`) | Direct, EA app running | **Game starts.** The variables change nothing. |
| 5 | `Link2EA.exe "link2ea://launchgame/alice2_dd?platform=origin"` | Direct | **Refused.** `Link2EA.log`: `"emsg":"invalid launch parameters (platform)"`; the user sees an EA dialog "Check that your game's installed correctly". Link2EA only takes games bought on Steam or Epic (`platform=steam`/`epic`). Not tried: other offer ids, other platforms. |
| 6 | `AliceMadnessReturns.exe` (the game exe itself), working dir = its folder | Direct | **Game exits at once** and the EA app starts it again under a new process id (EA DRM). So Steam cannot start this game's exe itself and keep it. |

### Through Steam (BoilR's shortcut or a test shortcut)

| # | Shortcut (Exe / StartDir / LaunchOptions) | EA app before | Result |
|---|---|---|---|
| 7 | `EADesktop.exe` / *(empty)* / `"origin2://..."` | running | **Fails before starting:** `Failed running GameID ... (OS Error 123)`. Empty StartDir (see overview). Several attempts 17:00–17:13. |
| 8 | `EALauncher.exe` / *(empty)* / `"origin2://..."` | running | **Same OS Error 123.** (The exe fix alone wasn't enough.) |
| 9 | `EALauncher.exe` / `"...\EA Desktop\EA Desktop"` / `"origin2://..."` | running | **Game starts.** Steam tracks `EALauncher.exe` and `EALaunchHelper.exe` only; both exit within seconds; the game (parent `EADesktop.exe`) is not tracked. **Playing status ends at once**: Play shows again, pressing it starts a second launch. **Home:** chime, no overlay. **Resume:** nothing. |
| 10 | Same as 9 | **closed** | **EA app error, game doesn't start.** Steam tracks `EALauncher` → `EALaunchHelper` → `EADesktop.exe` (+ `EACefSubProcess`, `EALocalHostSvc`, `IGOProxy32`). `EADesktop.log`: `[PROCESS] Create Process: Access is denied. \| exe=[...\AliceMadnessReturns.exe]` then `Game launch error: [9] (LaunchFailed)`. The EA app then stays inside Steam's tree (Steam shows the shortcut running) until it is closed. **So with BoilR's direct EALauncher shortcut, EA games only started from Steam while the EA app was already open.** |
| 11 | Test wrapper: `powershell.exe` / its folder / `-File launch-and-wait.ps1 -Link "origin2://..." -Folder "<game folder>"` (opens the link with `Start-Process`, waits while a process from the game folder runs) | running | **Game starts. Playing status correct:** Steam shows running (Stop button) for the whole game and returns to Play within seconds of quitting. **Home:** chime, no overlay. **Resume** is shown but does nothing (the tracked process has no window). |
| 12 | `EADesktop.exe` (no arguments) started by a wrapper from Steam; then **Play pressed inside the EA app** | **closed** | **Same "Access is denied", launch error 9**, twice. This is the 2024 community workaround (add the EA app to Steam, start games from it); it no longer works. |
| 13 | BoilR `--launch-and-wait --launcher "C:\WINDOWS\explorer.exe" ...`, link passed **unquoted** (`Command::arg`) | closed | **Nothing happens**: explorer.exe ignores an unquoted `origin2://...?offerIds=...` link. Reproduced directly: `explorer.exe origin2://...` does nothing, `explorer.exe "origin2://..."` starts the EA app. |
| 14 | BoilR `--launch-and-wait --launcher "C:\WINDOWS\explorer.exe" --link "origin2://..."`, link passed **quoted** (shipped) | closed | **Works.** Steam tracks only BoilR and `explorer.exe`; the EA app starts outside Steam's tree; game running after 6 s. Playing status correct; BoilR exits about 5–10 s after the game. No overlay. |
| 15 | BoilR `--launch-and-wait --launcher "...\EALauncher.exe" ...` | closed | **Access is denied** (as 10): BoilR's child EALauncher starts the EA app inside Steam's tree. Hence explorer.exe (14). |

## Why the EA app refuses to start games under Steam (unconfirmed)

`Create Process: Access is denied` is what `CreateProcess` returns when a process inside a
Windows job object asks for `CREATE_BREAKAWAY_FROM_JOB` and the job doesn't allow breakaway
([Microsoft docs](https://learn.microsoft.com/windows/win32/procthread/process-creation-flags)).
The guess: Steam puts what it launches in a job, and the EA app starts games with breakaway.
Neither half is confirmed. Ways to check:

- Process Explorer: `EADesktop.exe` started from Steam → Properties → Job tab (is there a job,
  does it allow breakaway?).
- Process Monitor: filter on `EADesktop.exe`, the failing `Process Create`, and its result.

If it is the job, a way around it would have to give the EA app a job that allows breakaway,
or get the game created in Steam's tree some other way; neither is known to be possible.

## Still open

- Confirm or refute the job object theory (above).
- Whether the EA app's own in-game overlay setting (EA app → Settings) changes anything for
  Steam's overlay; one 2024 thread said it had to be on. Not tested.
- `Link2EA.exe` with `platform=steam` or an EA offer id (`Origin.OFR.50.xxxx`) instead of the
  content id. Not tested; Link2EA is meant for Steam/Epic purchases.
- EA games whose exe Steam can start directly (no DRM relaunch): Steam would then track the
  game and show the overlay. Only Alice was tried (scenario 6).
- Games found through old Origin `.mfst` manifests still get `EADesktop.exe` +
  `link2ea://launchgame/<id>` (or Origin's `origin2://`) and no wait helper; untested, likely
  broken the same way (Link2EA handles `link2ea://`, not EADesktop).

## Not yet tried (review of 2026-11-11)

Gaps in what was tested:

- Scenario 14 has a row for "EA app closed" only. "EA app already open" with the shipped
  helper works (user-confirmed), but has no logged run.
- Only one EA game (Alice) was tried. Newer EA games (EA anticheat, Denuvo, their own
  launchers such as the Sims 4 or Battlefield ones) may hand over to other programs or run
  from outside the install folder.

Options not tried, cheapest first:

1. **Steam's overlay off for the shortcut, then scenario 10 again** (`AllowOverlay` = 0 in
   `shortcuts.vdf`). "Access is denied" has a second possible cause besides the job object:
   Steam loads its overlay into what it starts and hooks process creation so the overlay
   follows into child programs (this is probably how it reached `farcry3.exe`, see the
   Ubisoft report). The hook may clash with how the EA app creates games, or with the EA
   app's own overlay injector (`IGOProxy32`, which was running in scenario 10). If the
   launch works with the overlay off, the job is not the cause; then try the EA app's
   in-game overlay off with Steam's on. Even overlay-off is a partial win: the EA app inside
   Steam's tree would make Steam track the game itself (status and Resume without the
   helper).
2. **Check the job directly.** With a process that Steam started running, call
   `IsProcessInJob` and `QueryInformationJobObject` (`JobObjectExtendedLimitInformation`)
   from PowerShell: is there a job, does it set `JOB_OBJECT_LIMIT_BREAKAWAY_OK` or
   `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK`? If breakaway is allowed, BoilR could start the EA
   app so that it can launch games; if not, nested jobs are the only idea left.
3. **Copy what the EA app gives the game.** Scenario 6 started the bare exe. The EA app
   starts it with its own command line and environment (some values likely login tokens
   that expire). Read both from the running game (command line via `Win32_Process`,
   environment from its PEB), then start the exe with them, first by hand, then from Steam.
4. **Link2EA** with `platform=steam` or an `Origin.OFR.50.xxxx` offer id (low odds, see
   above).
5. Cross-launcher ideas (Resume forwarding, an overlay window of BoilR's own, injecting the
   overlay into a running game) are in the [overview](README.md#not-yet-tried).
