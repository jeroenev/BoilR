# boilr-proto: Resume and overlay prototype

A throwaway Windows exe for testing two ideas from the launcher investigation's "Not yet tried" list
(`docs/launcher-investigations/README.md`, options 1 and 2) for games that run outside Steam's process tree,
such as EA games opened through `explorer.exe "origin2://..."`.

```
boilr-proto.exe --mode resume|overlay --game-folder "<install folder>" --link "<link>"
```

Everything is logged with timestamps to `proto.log` next to the exe (this folder). The log is appended to, and
each run starts with a `=====` line.

## Shared behaviour (both modes)

- Opens the link the way BoilR's `launch_and_wait` does: `explorer.exe "<link>"`, quoted with `raw_arg`.
- A watcher thread (sysinfo, every 0.5 s) waits up to 10 minutes for a process whose exe is under the game
  folder. It then keeps the list of game PIDs current and ends the program once none has run for 5 s. If the
  game is already running, the link is not opened again.
- At start it logs the Steam environment variables and whether `GameOverlayRenderer64.dll` is loaded into the
  process.

## Mode `resume`

- Creates a real top-level window. Choices (all logged):
  - `WS_POPUP` with `WS_EX_APPWINDOW`, so it is not a tool window and has a taskbar button.
  - Not topmost, 1x1 px in the bottom-right corner of the primary monitor. The game covers it.
  - Black background, titled `BoilR game (proto)`.
  - The size can be changed with `--resume-size <px>`.
- The window is shown with `SW_SHOWNOACTIVATE`, so it doesn't take focus when it appears or when the game
  starts. This was verified: the window that was in front stayed in front.
- When the window is activated (`WM_ACTIVATEAPP`/`WM_ACTIVATE` to active, which is what Steam's "Resume game"
  should cause), it finds the game's main window after 50 ms and brings it to the front:
  - The game's main window is the largest visible, uncloaked, unowned top-level window that belongs to a
    game-folder PID.
  - The hand-over escalates through:
    1. `AllowSetForegroundWindow` + `SetForegroundWindow`.
    2. `AttachThreadInput` to the foreground thread + `BringWindowToTop` + `SetForegroundWindow`.
    3. A simulated Alt press + `SetForegroundWindow`.
  - A minimized game is restored first.
  - After 150 ms it checks again and retries once if the switch didn't stick.
- Safety net: if the window is ever in front without an activation message, a hand-over runs about 1 s later.
- Hand-overs are throttled to one per 500 ms. A second one within that time is postponed, not dropped.
- Also logged: `WM_SETFOCUS`/`WM_KILLFOCUS`, `WM_SHOWWINDOW`, `WM_SYSCOMMAND` and `WM_MOUSEACTIVATE`. If Steam
  restores or shows the window in some other way, the log shows which message it was.

## Mode `overlay` (GlosSI-style)

- A borderless `WS_POPUP` covering the primary monitor, 1 px smaller in width and height as GlosSI did. Its
  extended style is `WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT`. It also gets
  `SetLayeredWindowAttributes(alpha 255)` and `DwmExtendFrameIntoClientArea(-1)`, which makes the window
  per-pixel transparent through DWM.
- Direct3D 11 renders into it every frame: a hardware device, a `B8G8R8A8` swap chain with `DISCARD` and 1
  buffer, `ClearRenderTargetView(0,0,0,0)` and `Present(1)`, at about 60 fps. Steam's overlay hooks DXGI
  `Present` in this process, so it can draw into this window. The swap chain uses the classic hwnd-based
  model, not DirectComposition, on purpose: Steam's overlay reads the target window from the swap chain, and a
  composition swap chain has none.
- Once a game window exists, the overlay window follows the game's monitor (`MonitorFromWindow`).
- `WS_EX_NOACTIVATE` is not used. Our window must be able to take focus when the overlay opens, and Steam must
  be able to activate it for Resume. It is only shown without activation.
- **Overlay detection.** It uses two sources, both logged separately, and treats the overlay as open if
  either says so:
  1. GlosSI's method: `PeekMessage(PM_NOREMOVE)` once per frame. While the overlay is open, Steam's message
     hook sets the message field to 0. Three zeroed peeks in a row mean open; three non-zero messages ≥1000 in
     a row mean closed.
  2. `GameOverlayRenderer64.dll`'s own exports: `SteamOverlayIsUsingKeyboard/Mouse/Gamepad`, plus
     `BOverlayNeedsPresent`, which is logged only. These exports were found in the DLL's export table on this
     machine. Their signatures are assumed to be `bool fn()`.
- **When the overlay opens:**
  1. `WS_EX_TRANSPARENT` is removed, so the window catches clicks.
  2. The current foreground window is remembered.
  3. Our window is forced to the front with the same escalation as above, plus `SetActiveWindow`/`SetFocus`.
  4. Pending hand-overs are cancelled.
- **When it closes:** `WS_EX_TRANSPARENT` is restored and focus goes back to the remembered game window, or to
  the game's main window.
- **Resume behaviour** is the same as in mode `resume`, delayed 250 ms. If Steam activated our window because
  it is opening the overlay, the hand-over is cancelled.
- **GlosSI's Shift+Tab workaround.** While another window is in front and Shift+Tab is held
  (`GetAsyncKeyState`), it posts `WM_KEYDOWN`/`WM_KEYUP` for Shift and Tab to our window, where Steam's message
  hook can see them.
- **`GetForegroundWindow` hook** (GlosSI's `keepControllerConfig`). It is on by default in overlay mode; turn
  it off with `--no-fg-hook`, or turn it on in resume mode with `--fg-hook`.
  - `GameOverlayRenderer64.dll` imports `USER32!GetForegroundWindow` through its import table (checked on
    this machine). The prototype patches that one import-table slot: no inline hook, and no other code in the
    process is affected.
  - The replacement returns our overlay window whenever the real foreground window belongs to a game PID.
    This should make Steam treat "the game" as focused, which matters for the controller Home button and for
    keeping the shortcut's controller layout.
  - Calls and substitutions are counted in the 10-second `present:` log lines.
- Test flags: `--tint` clears to 25% red so the window can be seen. `--test-overlay <start>:<seconds>` fakes
  an open overlay to exercise the click-through and focus switch without Steam.

### What GlosSI did (github.com/Alia5/GlosSI, `GlosSITarget/`)

- **`TargetWindow`:** an SFML (OpenGL) window, borderless, desktop size minus 1 px, `WS_POPUP`, and
  `DwmExtendFrameIntoClientArea(-1)`. It cleared to transparent each frame at the monitor's refresh rate.
- **Click-through:** `WS_EX_LAYERED|WS_EX_TRANSPARENT|WS_EX_TOPMOST|WS_EX_COMPOSITED`, with `TRANSPARENT` and
  `TOPMOST` removed while its own or Steam's overlay is open.
- **`SteamOverlayDetector`:** the zeroed-`PeekMessage` trick described above.
- **`onOverlayChanged(open)`:** focuses its own window and turns click-through off. On close it turns
  click-through back on and focuses the last foreground window.
- **`focusWindow`:** `AttachThreadInput` to the foreground thread, then `SetForegroundWindow`, `SetCapture`,
  `SetFocus`, `SetActiveWindow` and `EnableWindow`, then retries `SetForegroundWindow` for 20 ms.
- **`overlayHotkeyWorkaround`:** polls the overlay hotkey and posts key messages to its own window.
- **`keepControllerConfig`:** an inline (subhook) hook on `GetForegroundWindow` in its own process. It returns
  GlosSI's window while a window of the launched game is in front, so Steam keeps the shortcut's controller
  configuration.
- **HidHide and ViGEm (`InputRedirector`):** hid the real controller from the game and fed it a virtual one
  that carries Steam Input's output. This is not in the prototype. It is only needed when the game reads the
  physical controller directly, and HidHide is a separate driver.

## How it was built

- Rust, standalone cargo project in `..\proto-src`: `cargo build --release` with `%USERPROFILE%\.cargo\bin\cargo`.
- Dependencies: `windows` 0.58 and `sysinfo` 0.38.4.
- `#![windows_subsystem = "windows"]`, so Steam starting it shows no console.
- The same project builds `fake-game.exe` (`src\bin\fake_game.rs`), a plain 800x600 window titled `Fake Game`,
  which was copied to `..\fakegame\`.
- Test helper: `..\tools\wintest.ps1`. It activates, minimizes and closes windows, reports the foreground
  window, hit-tests with `WindowFromPoint`, and takes DPI-aware screenshots with `BitBlt(SRCCOPY|CAPTUREBLT)`.

## What was verified (without Steam, with the fake game)

| Check | Result |
|---|---|
| Opens the link through `explorer.exe` and detects the game | The fake game was detected about 0.5 s after the link was opened |
| Doesn't steal focus at start | The previously focused window stayed in front, in both modes |
| Resume: activating the proto window brings the game to the front | Yes: `SetForegroundWindow` succeeded on the first try |
| Resume with the game minimized | The game was restored and brought to the front |
| Exits about 5 s after the game closes | 4.8 to 5.1 s in every run; no processes left behind |
| Overlay window created, topmost, over the whole monitor | Yes (3839x2159 on the 3840x2160 screen) |
| Transparent | The screenshot matches the baseline pixel for pixel. With `--tint` the whole screen gets the red tint (R+64), so the window really composites with per-pixel alpha |
| Click-through | `WindowFromPoint` at the game's position returns the window underneath while closed, and our window while the overlay is "open" |
| Present runs | 59.9 fps (`present:` log lines) |
| Simulated overlay open/close (`--test-overlay`) | Open: the style lost `TRANSPARENT`, our window took the foreground (once after `AttachThreadInput`). Close: `TRANSPARENT` came back and the game got the foreground |
| Resume in overlay mode | Activating the overlay window handed the foreground to the game |
| Import-table hook mechanism (`--self-test-iat`, on this exe's own import table) | PASS: the call went through the hook |
| GlosSI detector without Steam | No false "open" in any run |

## Not verified: Steam behaviour

Nothing here ran under Steam, and the overlay DLL was never loaded. The following are still open:

- **Whether Big Picture's "Resume game" activates our window at all.** Steam may look for a window
  differently, for example the window the overlay hooked, the largest window, or one it can see in a list,
  and might ignore a 1x1 window. The log answers this: around the time you press Resume there should be
  `WM_ACTIVATEAPP active=true` / `WM_ACTIVATE state=active` lines, followed by `hand-over`. If no message
  appears at all, try a larger window by adding `--resume-size 200` to the shortcut's launch options.
- Whether `SteamOverlayIsUsing*` behave as assumed, and whether GlosSI's zeroed-message trick still works
  with today's Steam.
- Whether the `GetForegroundWindow` substitution makes the controller Home button open the overlay instead of
  Big Picture.

## Known limits

- **Exclusive fullscreen:** no window can sit over an exclusive-fullscreen game. Alice: Madness Returns
  (UE3/DX9) may run exclusive fullscreen by default, and taking focus for the overlay may minimize it. Set it
  to windowed (or borderless, if offered) for the overlay test.
- **Primary monitor first:** the overlay window starts on the primary monitor and moves to the game's monitor
  once the game has a window.
- **Main-thread focus calls:** focus changes run on the render/message thread. `AttachThreadInput` to a busy
  foreground thread once blocked for 2.5 s (fps dropped to 41 for that period). A real implementation should
  do focus work on a separate thread.
- **Closing the game focuses our window:** when the game window closes, Windows activates our topmost window
  (logged as `no game window found`). It is harmless, because the program exits 5 s later.
- **Taskbar button:** the overlay window has a taskbar or Alt-Tab entry. GlosSI could hide it with
  `WS_EX_TOOLWINDOW`, which is left out here so Steam sees a normal app window.
- **Scope:** no HidHide/ViGEm, no handling of Steam's Stop (Steam ends only this exe; the game keeps running),
  and no restore of the import-table hook. The process exits anyway.

## What to test through Steam

Both shortcuts start Alice: Madness Returns through the EA app with this exe. Before each test, note the end
of `proto.log`. After each test, send the new part of the log, plus Steam's `logs\gameprocess_log.txt` lines
if anything looks odd.

### TEST EA3 Resume proto

1. In Big Picture, start "TEST EA3 Resume proto". Wait until Alice is in the game or menu.
2. Press the controller's Home button. Expect Big Picture or the chime, as before. Then choose
   **Resume game**.
   - **Expected:** Alice comes back to the front.
   - **In the log:** `WM_ACTIVATE...active`, then `hand-over ... game window ...`, then
     `focus game: ... ok`.
3. Repeat once with Alice minimized (Alt-Tab away and minimize it first), then Resume.
4. Quit Alice from its menu. Steam should show the game as stopped within about 5 s, and the log should end
   with `exit`.
5. If Resume does nothing and the log shows no activation lines, say so. That means Steam doesn't activate
   this window; the next try is `--resume-size 200`.

### TEST EA4 Overlay window proto

Set Alice to windowed or borderless first, if it runs exclusive fullscreen.

1. Start "TEST EA4 Overlay window proto" (desktop Steam or Big Picture).
   - **First sign it works:** Steam's "Press Shift+Tab to access the Steam overlay" notification appears
     shortly after the start, drawn by our window. In the log: `GameOverlayRenderer64.dll is loaded`,
     `overlay exports: ... true`, and `fg-hook: GetForegroundWindow ... now returns our window`.
2. In Alice, press **Shift+Tab**.
   - **Expected:** the Steam overlay appears over Alice, and you can click and type in it.
   - **In the log:** `OVERLAY OPENED (detected by ...)` (which detector fired) and `focus our window ...`.
3. Close the overlay (Shift+Tab or Esc).
   - **Expected:** Alice gets focus back and responds to keyboard and controller.
   - **In the log:** `OVERLAY CLOSED` and `focus game ... ok`.
4. Press the controller's **Home** button in the game. Does the overlay open over Alice (the hook worked), or
   Big Picture or the chime? The `present:` lines show `fg-hook calls N (substituted M)`. M > 0 means Steam
   asked and got our window.
5. If Big Picture opened, choose **Resume game**. Alice should come to the front (same as EA3).
6. Optional: F12 screenshot while the overlay works (Steam captures our window, so the shot may come out
   transparent or black, not Alice).
7. Quit Alice. The program exits about 5 s later; the log ends with `exit`.

If the overlay doesn't appear but the log says the DLL is loaded and `present:` shows about 60 fps, the
transparent-window technique itself is the problem. Please send a phone photo of the screen too.
