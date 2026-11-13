# Tests that need a person (2026-11-11)

## Results (2026-11-13)

Filled in by the user with a controller, Steam started normally.

| Test | Result |
|---|---|
| 1a TEST EA7, EA app closed | **All yes:** Home opens the overlay, Resume works, controller works, F12 screenshot, Play again within ~20 s and the EA app gone. |
| 1b TEST EA7, EA app open | All yes (signed in again without a prompt), **except F12: no screenshot** (once; not retried). |
| 2 TEST EA6 (Play pressed in an EA app that Steam started) | **Works:** game starts, Home opens the overlay, Steam shows running until the EA app is exited (expected). Unlike EA R4, where the game was asked for with a link from outside. |
| 3 Stop, shipped Alice shortcut | The game keeps running (expected). |
| 3 Stop, TEST EA7 | Ends the game, but only after **about a minute** of hanging. |
| 4a TEST UB1 FC3 exe, Ubisoft Connect closed | Resume, controller, quit → Play all fine, but **Home: the overlay flashes for a second and disappears** (sounds play, no usable UI). |
| 4b Same, Ubisoft Connect open | **All yes.** |
| 5 Far Cry 4, AC IV through their exe | **All yes.** The admin prompt (prerequisites) came once; gone on the second launch. AC IV needed the password once. |
| 6 TEST EA3 Resume prototype | **Doesn't work:** overlay sounds but nothing appears; Resume doesn't bring the game back. |
| 6 TEST EA4 Overlay window prototype | **Unreliable:** the overlay sometimes appears; it works with the controller, but the game's controller input is broken afterwards (focus not given back). |
| 7 Cloud saves, optional tests | Not done. |

Conclusions:

- **EA:** `EALauncher.exe` + `--restart-launcher` is the way (tracking, overlay, Resume, Play
  again). Open: the minute-long Stop, and the one missed F12 screenshot.
- **Ubisoft:** the game's own exe works when Ubisoft Connect is already running; when the game
  has to start Ubisoft Connect itself, the overlay breaks (likely Ubisoft Connect's window or
  its own overlay taking over as it starts). A helper could start Ubisoft Connect through
  `explorer.exe` first, wait until it is up, then start the exe. Untested.
- **Prototypes:** dropped; the in-tree routes make them unnecessary.

What the agents couldn't check themselves: the controller's Home button, Big Picture's
"Resume game", the controller layout, UAC prompts, passwords, and real save games. Every
shortcut below is in Steam on the test machine (account folder `userdata\<id>`); start
them from Big Picture with a controller connected. The Steam id after each name starts it
from outside too (`steam://rungameid/<id>`).

Before you start:

- Steam must be started normally (autostart, Start menu, tray), not from a terminal (see
  [the testing pitfall](README.md#testing-pitfall-steam-started-from-a-terminal)).
- "EA app closed" = tray icon → Exit. "Ubisoft Connect closed" = tray icon → Quit.
- Logs to note: BoilR's `%APPDATA%\boilr\launch-and-wait.log`; Steam's
  `logs\gameprocess_log.txt`; for the EA app `%LOCALAPPDATA%\Electronic Arts\EA
  Desktop\Logs\EADesktop.log`.

Each test lists what to check. Write down for each: yes / no / what happened instead.

## 1. EA, the new candidate: helper + EALauncher + `--restart-launcher`

Shortcut **TEST EA7 helper EALauncher restart** (18059144350683627520). Agents confirmed:
game tracked, Shift+Tab overlay, Play again ~16 s after quitting (EA R14, R15).

1a. EA app closed. 1b. EA app open and signed in.

- [ ] Home in the game opens the Steam overlay (not Big Picture).
- [ ] From Big Picture (Home → Library, or Alt+Tab to Steam), "Resume game" returns to the game.
- [ ] The shortcut's controller layout works in the game.
- [ ] F12 makes a screenshot.
- [ ] Quit from the game's menu: within ~20 s Steam shows Play and the EA app is gone from the tray.
- [ ] 1b only: the EA app comes back signed in, without a prompt.

## 2. EA: pressing Play inside an EA app that Steam started

Shortcut **TEST EA6 EA app** (17531808751412051968). EA app closed first.

- [ ] Start the shortcut; in the EA app, press Play on Alice. Does the game start (or an EA error)?
- [ ] Home in the game: overlay?
- [ ] Steam shows running while the game runs; after quitting the game it stays "running" until you
  exit the EA app (expected).

## 3. EA: Steam's Stop button

Shortcut **Alice: Madness Returns** (17417358039161241600): the shipped one (EA app outside
Steam's tree).

- [ ] While the game runs, press Stop in Steam. Does the game keep running? (Expected: yes; Steam
  only ends BoilR.) Then quit the game normally.
- [ ] Same with **TEST EA7**: expected that Stop ends the game too (it's in Steam's tree), without
  saving. Only do this at a point where losing progress doesn't matter.

## 4. Ubisoft: Steam starts the game's own exe

Shortcut **TEST UB1 FC3 exe** (11664493484995248128). Agents confirmed: tracked, Shift+Tab
overlay, Play again ~3 s after the game, Ubisoft Connect never ended (Ubisoft U3).

4a. Ubisoft Connect closed. 4b. Ubisoft Connect open.

- [ ] Home opens the Steam overlay.
- [ ] "Resume game" returns to the game.
- [ ] The controller layout works.
- [ ] Quit from the game's menu: Steam shows Play within a few seconds; Ubisoft Connect stays in the tray.

## 5. Ubisoft: Far Cry 4 and AC IV through their own exe

- **TEST UB4 FC4 exe** (14235991564475170816): a UAC prompt appears on every launch (Ubisoft
  Connect retries Far Cry 4's prerequisite installers, which fail). Approve it and note what it
  installs (the prompt shows the program). Then the checks from 4.
- **TEST UB5 AC4 exe** (18433823278813413376): Ubisoft Connect asks to re-enter your password,
  and a UAC prompt appears. Enter the password, approve, then the checks from 4.
- [ ] Note whether the UAC prompt still appears on the second launch.

## 6. Prototypes for when the launcher can't be restarted

Both open the game through `explorer.exe` (EA app outside Steam's tree, so no real overlay)
and try to make up for it. Log: `proto.log` next to the shortcut's `boilr-proto.exe` (Steam
→ shortcut → Properties shows the folder). Source and README in
[`tools/boilr-proto`](tools/boilr-proto/).

- **TEST EA3 Resume proto** (12736291342372569088):
  - [ ] Home: Big Picture opens (expected).
  - [ ] "Resume game": does the game come back to the front? (The log shows `WM_ACTIVATE` lines if
    Steam activated the prototype's window.)
- **TEST EA4 Overlay window proto** (15462954371075014656): set Alice to windowed or borderless
  first (exclusive fullscreen covers everything).
  - [ ] Home: does the Steam overlay appear over the game?
  - [ ] Can you use it with the controller, and does the game get focus back after closing it?

## 7. Cloud saves

- [ ] **Far Cry 3**, shipped shortcut **Far Cry 3** (14672378224776839168): play until the game
  saves (a checkpoint or a safe house), quit, and note the time. Afterwards I compare
  Ubisoft Connect's log and BoilR's log (does the upload finish within the 10 s grace?).
- [ ] **Alice**, through **TEST EA7**: play until the game saves, quit, note the time. In
  `EADesktop.log`, `Post game cloud sync completed successfully` should come before BoilR's
  `closing the launcher` in its log.

## Optional

- Watch_Dogs and AC Origins: Ubisoft Connect doesn't know them as installed. Use "Locate
  installed game" in Ubisoft Connect, then try **TEST UB7 Watch_Dogs exe**
  (11403864303854419968) and **TEST UB6 ACOrigins exe** (14390057341372334080) with the checks
  from 4.
- Friends see "In game" and playtime goes up, for 1 and 4.
