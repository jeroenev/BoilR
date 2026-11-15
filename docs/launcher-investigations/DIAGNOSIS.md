# Diagnosis: EA app and Ubisoft Connect games launched from Steam

All test scenarios run between 2026-11-11 and 2026-11-14, with their results, and every issue
found along the way. Compiled on 2026-11-15 from [ea-app.md](ea-app.md),
[ubisoft-connect.md](ubisoft-connect.md) and [user-tests.md](user-tests.md), which hold the
full details (logs, process trees, timings). Test IDs match those files.

**Test machine:** Windows 11 Home build 26300; Steam build 1792618285 (desktop and Big
Picture, controller); EA app 13.822.0.6335; Ubisoft Connect 175.1.13416.
**Games:** Alice: Madness Returns (EA); Far Cry 3, Far Cry 4, Assassin's Creed IV (Ubisoft).

**Who ran it:** *Agent* = run by Claude through scripts (overlay checked with Shift+Tab and a
screenshot, or by Steam attaching its overlay to the game). *User* = run by hand with a
controller (Home button, Resume, saves, prompts).

**Results:** Pass · Fail · Partial (works with a catch) · Dead end (approach abandoned) ·
Invalid (result came from a faulty test setup; see issue I-3).

## Outcome

| Goal | EA app (Alice) | Ubisoft Connect (Far Cry 3) |
|---|---|---|
| Game is found by BoilR | Pass, via the EA app's installer data (#638) | Pass (already worked) |
| Game starts from Steam | Pass: `EALauncher.exe` + `origin2://` link, non-empty StartDir (#638) | Pass |
| Steam shows "running" exactly while the game runs | Pass with launch and wait (#645) | Pass with launch and wait (#645) |
| Steam overlay, Big Picture Resume | Pass when the EA app was closed, or always with `steam_overlay` | Pass when Ubisoft Connect was closed, or always with `steam_overlay` |
| Play button back after quitting | ~7–9 s | ~18 s (BoilR started Ubisoft Connect) / ~7 s (it was open) |
| Cloud saves survive | Pass (one real save, R5-5) | Pass (one real save, R5-5, R8-4) |
| Final check with shortcuts imported by BoilR | Pass (2026-11-14) | Pass (2026-11-14) |

## EA app scenarios

### Round 1 (2026-11-11): which program opens the link

| ID | Scenario | EA app | Result | By |
|---|---|---|---|---|
| 1 | `EADesktop.exe "origin2://game/launch?offerIds=alice2_dd"` | running | **Fail**: the game never starts; EADesktop ignores the link (BoilR's first EA app shortcut) | Agent |
| 2 | `EALauncher.exe "origin2://..."` | closed | **Pass**: game and EA app start | Agent |
| 3 | `EALauncher.exe "origin2://..."` | running | **Pass** | Agent |
| 4 | As 3, with Steam's environment variables set | running | **Pass**; the variables change nothing | Agent |
| 5 | `Link2EA.exe "link2ea://launchgame/alice2_dd?platform=origin"` | — | **Fail**: "invalid launch parameters (platform)", EA error dialog | Agent |
| 6 | The game exe `AliceMadnessReturns.exe` directly | — | **Fail**: exits at once (code 100010), the EA app restarts it (DRM) | Agent |

### Round 1 (2026-11-11): through Steam

| ID | Shortcut | EA app | Result | By |
|---|---|---|---|---|
| 7 | `EADesktop.exe`, empty StartDir | running | **Fail**: `OS Error 123` before anything runs (issue I-1) | Agent |
| 8 | `EALauncher.exe`, empty StartDir | running | **Fail**: same `OS Error 123` | Agent |
| 9 | `EALauncher.exe`, StartDir = its folder | running | **Partial**: game starts, but Steam shows it stopped within seconds; no overlay, Resume does nothing | Agent |
| 10 | As 9 | closed | **Invalid**: "Create Process: Access is denied", launch error 9. Caused by the job object (I-3); passes as R2 | Agent |
| 11 | PowerShell wrapper that opens the link and waits while the game runs | running | **Partial**: running status right; no overlay; Resume does nothing | Agent |
| 12 | EA app started by Steam, then Play pressed in it | closed | **Invalid**: Access is denied (I-3); passes as user test 2 | Agent |
| 13 | BoilR helper through `explorer.exe`, link unquoted | closed | **Fail**: nothing happens; Explorer ignores an unquoted `origin2://` link (I-6) | Agent |
| 14 | BoilR helper through `explorer.exe`, link quoted | closed | **Partial**: running status right, but no overlay (EA app outside Steam) | Agent |
| 15 | BoilR helper with `EALauncher.exe` | closed | **Invalid**: Access is denied (I-3); passes as R5 | Agent |

### Round 2 (2026-11-11): Steam started normally

| ID | Scenario | EA app | Result | By |
|---|---|---|---|---|
| R1 | Job probe shortcut | — | Steam uses **no** job object; Steam's overlay DLL loads even with the overlay off | Agent |
| R2 | `EALauncher.exe` + link (= 10) | closed | **Pass with overlay**, but Steam shows "running" until the EA app quits | Agent |
| R3 | Same, overlay off | closed | Pass (no overlay, as set) | Agent |
| R4 | EA app started by Steam, game asked for through `explorer.exe` | closed | **Fail**: game runs but isn't tracked, no overlay | Agent |
| R5 | BoilR helper with `EALauncher.exe` (= 15) | closed | **Pass with overlay**; shortcut stays "running" until the EA app quits | Agent |
| R6 | `EALauncher.exe` + link (= 9) | open | Partial, as 9 | Agent |
| R7 | Helper through `explorer.exe` (then shipped) | open | Partial: status right, no overlay | Agent |
| R8 | EA in-game overlay setting on | closed | Invalid (job); the EA setting plays no role | Agent |
| R9 | Start the game exe with the EA app's `EA*` environment variables (login tokens) | open/closed | **Pass, overlay included**, but it reuses EA login tokens: rejected as a fallback | Agent |
| R10 | `link2ea://...?platform=steam` | open | **Dead end**: the EA app wants a Steam entitlement | Agent |
| R11 | Inject Steam's overlay DLL into a running game | — | **Dead end**: loads, no overlay | Agent |

### Rounds 3–4 (2026-11-11): helper with `--restart-launcher` (TEST EA7)

| ID | Scenario | EA app | Result | By |
|---|---|---|---|---|
| R12 | TEST EA7, BoilR before fix cddaa47 | closed | **Fail**: BoilR never closed the EA app after the game; its folder is a symbolic link (I-7) | Agent |
| R13 | Same | open | **Fail**: open EA app reused, as 9 (same bug) | Agent |
| R14 | TEST EA7, fixed | closed | **Pass**: tracked, overlay, Play again 16 s after the game | Agent |
| R15 | Same | open | **Pass**: open EA app closed and restarted signed in; overlay; Play again 17 s after | Agent |
| R16 | Cloud sync timing | — | EA app logs "Post game cloud sync completed successfully" 0.8–1.5 s after the game: a usable signal | Agent |
| R17 | How to quit the EA app cleanly | — | `WM_CLOSE` to its window or tray Exit quit it in ~1 s; not during the post-game sync (dialog EC:10609) | Agent |

### User tests (2026-11-13), controller, Steam started normally

| ID | Scenario | Result | By |
|---|---|---|---|
| 1a | TEST EA7, EA app closed | **Pass**: Home overlay, Resume, F12, EA app gone. Play only after ~20 s. Controller layout not really checked | User |
| 1b | TEST EA7, EA app open | **Partial**: as 1a, signed in again without a prompt, but **F12 screenshot failed** once | User |
| 2 | TEST EA6: Play pressed in an EA app that Steam started | **Pass**: game starts, Home overlay; "running" until the EA app exits (expected) | User |
| 3 | Steam's Stop, the then-shipped shortcut | Pass: the game keeps running (expected) | User |
| 3 | Steam's Stop, TEST EA7 | **Partial**: ends the game, but only after **~1 minute** of hanging (I-11) | User |
| 6 | TEST EA3, Resume-forwarding prototype | **Dead end**: Resume doesn't bring the game back | User |
| 6 | TEST EA4, overlay-window prototype | **Dead end**: overlay unreliable; game loses controller focus afterwards | User |

### Round 5 (2026-11-13): the implemented routes

| ID | Scenario | EA app | Result | By |
|---|---|---|---|---|
| R18 | TEST EA8, default | closed | **Pass**: waits for the cloud sync, closes the EA app; Play again 9.1 s after the game | Agent |
| R19 | TEST EA8, default | open | **Partial (expected)**: EA app takes the game over; no overlay; Play again 7.1 s | Agent |
| R20 | TEST EA9, `--restart-launcher` | open | **Pass**: EA app closed first, then as R18 | Agent |
| R21 | Close request while a game runs | open | EA app says "Close your game, first" and stays: BoilR only asks after the game | Agent |
| R5-1 | TEST EA8 | closed | **Pass**: Home overlay, Resume, Play again ~7 s | User |
| R5-2 | TEST EA8 | open | **Pass (expected)**: game starts, Play again within 10 s, no overlay | User |
| R5-5 | Real save in Alice, TEST EA8 | closed | **Pass**: saved at 14:36, EA app closed without a sync dialog | User |
| R5-6 | Real shortcuts imported by BoilR (2026-11-14) | — | **Pass** | User |

## Ubisoft Connect scenarios

### Round 1 (2026-11-11)

| ID | Scenario | Ubisoft Connect | Result | By |
|---|---|---|---|---|
| 2 | Plain shortcut `upc.exe "uplay://launch/46/0"` | closed | **Partial**: game, overlay and Resume work… | Agent |
| 3 | …after quitting the game | (started by Steam) | …but Steam shows "running" until Ubisoft Connect is closed | Agent |
| 4 | Quit Ubisoft Connect from the tray while the game runs | running | Refused: "Shutdown aborted due to game running" | Agent |
| 5 | `taskkill /IM upc.exe` (no `/F`) | running | **Fail**: only hides it in the tray (I-8) | Agent |
| 6 | Search for a quit switch in Ubisoft Connect's programs | — | None exists | Agent |
| 7 | Cloud-save lines in its log | — | None: Ubisoft Connect logs no sync end (I-9) | Agent |
| 8 | BoilR helper with `--restart-launcher` | either | **Pass** (user-confirmed): overlay, status, Resume; Play again ~15–18 s | Agent + user |

### Round 2 (2026-11-11): Steam started normally

| ID | Scenario | Ubisoft Connect | Result | By |
|---|---|---|---|---|
| U1 | Plain shortcut | running | **Fail**: Steam shows Play before the game is in its menu; no overlay | Agent |
| U2a | Helper, default | running | **Partial**: status right, no overlay | Agent |
| U2b | Helper, default | closed | **Fail**: overlay works but Steam **never** returns to Play (fixed in round 5) | Agent |
| U3a | Steam starts `bin\farcry3.exe` directly | closed | **Pass**: tracked, overlay, Play again 3 s after | Agent |
| U3b | Same | running | **Pass** | Agent |
| U4 FC4 | Steam starts `FarCry4.exe` | running | **Pass** after a UAC prompt (prerequisite installers) | Agent |
| U4 AC4 | Steam starts `AC4BFSP.exe` | running | **Fail** (unattended): UAC prompt, then "Re-enter your password" | Agent |
| U4 WD/ACO | Watch_Dogs, AC Origins exes | running | **Fail**: "Game needs download"; not registered as installed | Agent |
| U5 | Where Ubisoft Connect keeps exe paths | — | Encrypted cache `configurations.vdat` (AES-GCM, key protected with DPAPI) | Agent |
| U6 | Then-shipped helper, `--restart-launcher` | running | **Pass**: tracked (`farcry3_d3d11.exe`), overlay; Play again 18 s | Agent |
| U7a | `closebehavior: CloseBehavior_Close` + `WM_CLOSE` | running | **Pass**: clean quit in ~6 s | Agent |
| U7b | `WM_QUERYENDSESSION`/`WM_ENDSESSION` | running | Fail: no effect | Agent |
| U7c | Tray → Quit via UI Automation | running | Pass but fragile (moves the mouse) | Agent |
| U7 game | U7a while a game runs | — | Refused, leaves a dialog: only ask after the game | Agent |
| U8 | Ubisoft Connect disk activity after the game | — | Baseline only (no save made) | Agent |

### User tests (2026-11-13), controller

| ID | Scenario | Result | By |
|---|---|---|---|
| 4a | TEST UB1, Far Cry 3's exe, Ubisoft Connect closed | **Partial**: Resume, quit → Play fine, but **the overlay flashes and disappears** | User |
| 4b | Same, Ubisoft Connect open | **Pass** | User |
| 5 | Far Cry 4 and AC IV through their exe | **Pass**; admin prompt once (gone on 2nd launch), AC IV password once | User |

### Rounds 5–9 (2026-11-13/14): implemented routes and tuning

| ID | Scenario | Ubisoft Connect | Result | By |
|---|---|---|---|---|
| U9 | TEST UB3, default helper (BoilR ends Ubisoft Connect after 15 s) | closed | **Pass**: overlay; Play again 24 s after (U2b: never) | Agent |
| U10 | TEST UB8, game's own program, Ubisoft Connect opened first | closed | **Pass** for the agent: overlay attached; Play again 7 s | Agent |
| U11 | Same | open | **Pass** | Agent |
| U12 | TEST UB9, Far Cry 4's own program | open | **Pass**, no admin prompt | Agent |
| R5-3 | TEST UB8 | closed | **Fail**: Ubisoft Connect asks to restart and sign in again; game started before it had signed in (I-10) | User |
| R5-4 | TEST UB11, default helper with clean quit | closed | **Pass**: overlay, Play again ~30 s, its X button still hides it afterwards | User |
| R5-5 | Real save in Far Cry 3, TEST UB11 | closed | **Pass**: Ubisoft Connect showed it uploaded within 2 s; no conflict | User |
| — | Far Cry 3 inside a level | — | **Fail** at first: overlay works in the main menu only; once no overlay, once a crash (I-12) | User |
| U13 | Clean quit: setting changed while closed, close request 5 s after the game | closed | **Pass**: quit after 7.5 s; Play again 12 s | Agent |
| U14 | TEST UB10, helper through `explorer.exe` | closed | **Fail**: no overlay (Ubisoft Connect outside Steam) | Agent |
| U15 | TEST UB11 as BoilR does it (setting change + close request after 15 s) | closed | **Pass**: setting restored; Play again 28.5 s | Agent |
| U16 | TEST UB12, own program, waits for Ubisoft Connect's sign-in line + 5 s (2 runs) | closed | **Pass**: no sign-in error, overlay (user saw it in the main menu); Play again ~8 s | Agent + user |
| U17 | TEST UB12 with 1 s instead of 5 s after sign-in (3 runs) | closed | **Fail**: no overlay for the user; run 3 hit the race of R5-3. Reverted to 5 s | Agent + user |
| U18 | TEST UB11 with a 5 s wait before the quit | closed | **Pass**: Play again 18.2 s (was ~28 s). Kept | Agent |
| R8-1 | TEST UB12, Far Cry 3's own program | closed | **Partial**: no sign-in error, but **overlay unreliable** in menu and level | User |
| R8-2 | Same, Ubisoft's in-game overlay off | closed | **Fail**: overlay still broken, but no crash | User |
| R8-3 | TEST UB13 (Far Cry 4), TEST UB14 (AC IV), own program | closed | **Pass**, overlay in a level; AC IV asked for admin rights and password again | User |
| R8-4 | TEST UB11, default helper with the 5 s quit, real save | closed | **Pass**: overlay in a level, Play again ~20 s, no save conflict | User |
| R9-1 | TEST UB15, Far Cry 3 through `bin\farcry3.exe` (not the updater) | closed | **Pass**: overlay in menu and level; Play again; Ubisoft Connect stays | Agent + user |
| R5-6 | Real shortcuts imported by BoilR (2026-11-14) | — | **Pass** for Far Cry 3; AC IV still asks for admin rights and password through the default shortcut too, so that's the game's own behaviour | User |

## Issues found

| # | Issue | Symptom | Cause | Resolution |
|---|---|---|---|---|
| I-1 | Empty StartDir | Shortcut "does nothing"; `OS Error 123` in `gameprocess_log.txt` | Steam passes StartDir to `CreateProcess`, which rejects an empty folder | Fixed: StartDir = launcher's folder on Windows (#638) |
| I-2 | Wrong EA program | Game never starts from the shortcut | `EADesktop.exe` ignores `origin2://` links | Fixed: use the registered `origin2://` handler, `EALauncher.exe` (#638) |
| I-3 | "Access is denied" from the EA app | EA app can't start games under Steam | **Test setup:** Steam restarted from T3 Code's terminal inherited its Windows job object, which forbids breakaway | Not a real-world issue. Restart Steam via `explorer.exe` when testing. An Explorer-based workaround built on this wrong conclusion was later removed |
| I-4 | Far Cry 3 "not tracked" | Game appeared missing from Steam's log | Searched for `farcry3.exe`; the real process is `farcry3_d3d11.exe` | Test-method error |
| I-5 | Overlay "not loaded" in Far Cry 3 | Module list showed no overlay DLL | 64-bit PowerShell can't list a 32-bit game's modules | Test-method error; use 32-bit PowerShell |
| I-6 | Explorer ignored the link | Nothing happened | Unquoted `origin2://...?offerIds=` argument | Fixed by quoting; route later dropped |
| I-7 | EA app never closed after the game | Steam kept showing "running" | EA app folder is a symbolic link; Windows reports the real path | Fixed (cddaa47): folders also match by real path |
| I-8 | Ubisoft Connect won't quit | Close request only hides it in the tray | Default `closebehavior: CloseBehavior_Minimize` | Fixed: BoilR sets it to Close while it runs Ubisoft Connect, then restores it |
| I-9 | No Ubisoft cloud-sync signal | Can't tell when uploads are done | Ubisoft Connect logs nothing about syncs | Fixed wait (15 s, then 5 s); one real save verified (R5-5, R8-4) |
| I-10 | Ubisoft sign-in race | "Ownership connection is not set up (11-5002)", sign-in prompt | Game started before Ubisoft Connect had signed in | Fixed: wait for the sign-in log line + 5 s (U16). Only used by the own-program route, which isn't in #645 |
| I-11 | Steam's Stop hangs ~1 min | Stop takes a minute to end an EA game | Probably the EA app's 60 s shutdown timeout | Open; not confirmed |
| I-12 | Far Cry 3 overlay breaks in a level | Overlay missing, once a crash | Own-program route started `FC3Updater.exe`, and the game restarts itself through Ubisoft Connect | Fixed: Far Cry 3 keeps the link route (R8-4, R9-1) |
| I-13 | PowerShell mangled launch options | Test shortcuts got truncated arguments | Windows PowerShell 5.1 strips embedded quotes when passing arguments to native programs | Test-method error; write values from a file |
| I-14 | Linux CI failed | `test_Ubuntu` failed on Windows path joins | `Path` on Linux doesn't split `\` | Fixed (3235947): Windows paths handled as text |
| I-15 | AC IV prompts | Admin rights and password asked each launch | Ubisoft Connect retrying prerequisite installers + its password check | The game's own behaviour, also with a plain shortcut |
| I-16 | One F12 screenshot failed | No screenshot (1b) | Unknown | Open; seen once, not retried |

## Not tested

- How long a large cloud upload takes (only small saves were tested).
- Steam Input layouts for these shortcuts, playtime, friends' "In game" status.
- Launchers that need a sign-in or update first (the helper waits 10 minutes).
- Other EA games (anticheat, Denuvo, own launchers); old Origin `.mfst` games.
- Watch_Dogs and AC Origins after "Locate installed game"; Ubisoft+ titles.
- Two games from the same launcher at once.
- Linux/Proton (out of scope: those shortcuts are unchanged).

## What made it into the upstream PRs

- [#638](https://github.com/PhilipK/BoilR/pull/638): I-1, I-2 (finding and starting EA app games).
- [#645](https://github.com/PhilipK/BoilR/pull/645): launch and wait, EA and Ubisoft defaults, `steam_overlay` settings, I-7, I-8, I-9.
- Kept on this fork only: the Ubisoft own-program route (`start_game_exe`, catalogue decryption; I-10, I-12), the Explorer route, the prototypes, and these notes.
