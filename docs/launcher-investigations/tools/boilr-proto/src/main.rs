//! boilr-proto: a throwaway prototype for two ideas around Steam's overlay and "Resume game"
//! for non-Steam shortcuts whose game runs outside Steam's process tree.
//!
//! `boilr-proto.exe --mode resume|overlay --game-folder "<folder>" --link "<link>"`
//!
//! Common: opens the link with `explorer.exe "<link>"`, waits up to 10 minutes for a process
//! whose exe is under the game folder, then runs the mode until no such process has run for
//! 5 seconds. Everything goes to `proto.log` next to the exe.
//!
//! - `resume`: a tiny top-level window. When it is activated (what Steam's "Resume game"
//!   should do), the game's main window is brought to the front instead.
//! - `overlay`: GlosSI-style. A topmost, transparent, click-through window over the game's
//!   monitor that Direct3D 11 presents into every frame, so Steam's overlay (loaded into this
//!   process because Steam started it) can draw over the game. When the overlay opens, the
//!   window becomes clickable and takes focus; when it closes, focus goes back to the game.
//!   Includes the resume behaviour.
//!
//! Extra flags: `--fg-hook` / `--no-fg-hook` (default: on in overlay mode, off in resume mode)
//! patches `GetForegroundWindow` in GameOverlayRenderer64.dll's import table so the overlay
//! believes our window is in front while the game is (GlosSI's `keepControllerConfig`).
//! `--tint` (overlay mode) clears to a faint red instead of fully transparent, for checking
//! the window. `--resume-size <px>` sets the resume window's size (default 1).
//! `--self-test-iat` checks the import-table hook on this exe itself and exits.
#![windows_subsystem = "windows"]

use std::{
    collections::HashSet,
    ffi::c_void,
    fs::File,
    io::Write,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU64, AtomicUsize, Ordering},
        Mutex, OnceLock, RwLock,
    },
    thread::sleep,
    time::{Duration, Instant},
};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use windows::{
    core::{s, w, Interface, PCWSTR},
    Win32::{
        Foundation::{BOOL, COLORREF, HMODULE, HWND, LPARAM, LRESULT, RECT, TRUE, WPARAM},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            Direct3D11::{
                D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext,
                ID3D11RenderTargetView, ID3D11Texture2D, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                D3D11_SDK_VERSION,
            },
            Dwm::{DwmExtendFrameIntoClientArea, DwmGetWindowAttribute, DWMWA_CLOAKED},
            Dxgi::{
                Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_MODE_DESC, DXGI_SAMPLE_DESC},
                IDXGISwapChain, DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_CHAIN_FLAG,
                DXGI_SWAP_EFFECT_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
            },
            Gdi::{
                CreateSolidBrush, GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO,
                MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
            },
        },
        System::{
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            Memory::{VirtualProtect, PAGE_PROTECTION_FLAGS, PAGE_READWRITE},
            SystemInformation::GetLocalTime,
            Threading::{AttachThreadInput, GetCurrentThreadId},
        },
        UI::{
            Controls::MARGINS,
            HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, MapVirtualKeyW, SendInput, SetActiveWindow, SetFocus, INPUT,
                INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VK_MENU,
                VK_SHIFT, VK_TAB,
            },
            WindowsAndMessaging::*,
        },
    },
};

// ---------------------------------------------------------------------------------------
// Timings

const START_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const EXIT_GRACE: Duration = Duration::from_secs(5);
const SCAN_EVERY: Duration = Duration::from_millis(500);
/// How long after an activation the hand-over to the game happens. In overlay mode Steam may
/// activate our window because it is opening the overlay; the overlay detector needs a few
/// frames to notice, so the hand-over waits and is cancelled if the overlay opened.
const HANDOVER_DELAY_RESUME: Duration = Duration::from_millis(50);
const HANDOVER_DELAY_OVERLAY: Duration = Duration::from_millis(250);
const HANDOVER_MIN_GAP: Duration = Duration::from_millis(500);

// ---------------------------------------------------------------------------------------
// Shared state (window procedure, process watcher thread, GetForegroundWindow hook)

static LOG: Mutex<Option<File>> = Mutex::new(None);
static STARTED: OnceLock<Instant> = OnceLock::new();
static GAME_PIDS: RwLock<Vec<u32>> = RwLock::new(Vec::new());
static DONE: AtomicBool = AtomicBool::new(false);
static OVERLAY_MODE: AtomicBool = AtomicBool::new(false);
static OVERLAY_OPEN: AtomicBool = AtomicBool::new(false);
static OUR_HWND: AtomicIsize = AtomicIsize::new(0);
static PENDING_HANDOVER: Mutex<Option<Instant>> = Mutex::new(None);
static REAL_GFW: AtomicUsize = AtomicUsize::new(0);
static FG_HOOK_CALLS: AtomicU64 = AtomicU64::new(0);
static FG_HOOK_SUBSTITUTED: AtomicU64 = AtomicU64::new(0);

fn log(message: &str) {
    let t = unsafe { GetLocalTime() };
    let since = STARTED.get().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
    let line = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} [+{:8.3}s] {}\n",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds, since, message
    );
    if let Ok(mut guard) = LOG.lock() {
        if let Some(file) = guard.as_mut() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }
}

fn is_game_pid(pid: u32) -> bool {
    GAME_PIDS
        .read()
        .map(|pids| pids.contains(&pid))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------------------
// Arguments

#[derive(Debug)]
struct Args {
    overlay: bool,
    game_folder: PathBuf,
    link: String,
    fg_hook: bool,
    tint: bool,
    resume_size: i32,
    self_test_iat: bool,
    /// Test only: pretend the overlay is open from `start` s for `duration` s after start.
    test_overlay: Option<(f64, f64)>,
}

fn parse_args() -> Result<Args, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let has = |name: &str| args.iter().any(|a| a == name);
    let self_test_iat = has("--self-test-iat");
    let mode = value("--mode").unwrap_or_default();
    let overlay = match mode.as_str() {
        "overlay" => true,
        "resume" => false,
        _ if self_test_iat => false,
        other => return Err(format!("--mode must be resume or overlay, got {other:?}")),
    };
    let game_folder = PathBuf::from(value("--game-folder").unwrap_or_default());
    let link = value("--link").unwrap_or_default();
    if !self_test_iat && (game_folder.as_os_str().is_empty() || link.is_empty()) {
        return Err("--game-folder and --link are required".into());
    }
    let fg_hook = if has("--no-fg-hook") {
        false
    } else {
        has("--fg-hook") || overlay
    };
    let resume_size = value("--resume-size")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1);
    Ok(Args {
        overlay,
        game_folder,
        link,
        fg_hook,
        tint: has("--tint"),
        resume_size,
        self_test_iat,
        test_overlay: value("--test-overlay").and_then(|v| {
            let (a, b) = v.split_once(':')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        }),
    })
}

// ---------------------------------------------------------------------------------------
// Process watcher (runs on its own thread so a slow process scan never drops frames)

fn quoted(value: &str) -> String {
    let value = value.replace('"', "");
    format!("\"{}\"", value.trim_end_matches(['\\', '/']))
}

fn comparable(path: &Path) -> String {
    path.to_string_lossy()
        .to_lowercase()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_string()
}

fn is_in(exe: &Path, folder: &str) -> bool {
    comparable(exe)
        .strip_prefix(folder)
        .is_some_and(|rest| rest.starts_with('\\'))
}

fn scan(system: &mut System, folder: &str) -> Vec<(u32, String)> {
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let mut found: Vec<(u32, String)> = system
        .processes()
        .iter()
        .filter_map(|(pid, p)| {
            let exe = p.exe()?;
            is_in(exe, folder).then(|| (pid.as_u32(), exe.display().to_string()))
        })
        .collect();
    found.sort();
    found
}

fn watch(game_folder: PathBuf, link: String) {
    let folder = comparable(&game_folder);
    let mut system = System::new();
    let mut found = scan(&mut system, &folder);
    if !found.is_empty() {
        log(&format!("watcher: the game is already running: {found:?}; not opening the link"));
    } else {
        let mut command = Command::new("explorer.exe");
        command.raw_arg(quoted(&link));
        match command.spawn() {
            Ok(child) => log(&format!(
                "watcher: started explorer.exe {} (pid {})",
                quoted(&link),
                child.id()
            )),
            Err(err) => {
                log(&format!("watcher: could not start explorer.exe: {err}"));
                DONE.store(true, Ordering::SeqCst);
                return;
            }
        }
        let waiting = Instant::now();
        loop {
            if DONE.load(Ordering::SeqCst) {
                return;
            }
            sleep(SCAN_EVERY);
            found = scan(&mut system, &folder);
            if !found.is_empty() {
                log(&format!(
                    "watcher: game detected after {:.1}s: {found:?}",
                    waiting.elapsed().as_secs_f64()
                ));
                break;
            }
            if waiting.elapsed() > START_TIMEOUT {
                log("watcher: the game did not start within 10 minutes; giving up");
                DONE.store(true, Ordering::SeqCst);
                return;
            }
        }
    }
    let mut known: HashSet<u32> = found.iter().map(|(pid, _)| *pid).collect();
    if let Ok(mut pids) = GAME_PIDS.write() {
        *pids = known.iter().copied().collect();
    }
    let mut last_seen = Instant::now();
    loop {
        if DONE.load(Ordering::SeqCst) {
            return;
        }
        sleep(SCAN_EVERY);
        found = scan(&mut system, &folder);
        let now: HashSet<u32> = found.iter().map(|(pid, _)| *pid).collect();
        for (pid, exe) in &found {
            if !known.contains(pid) {
                log(&format!("watcher: game process appeared: {pid} {exe}"));
            }
        }
        for pid in known.difference(&now) {
            log(&format!("watcher: game process gone: {pid}"));
        }
        known = now;
        if let Ok(mut pids) = GAME_PIDS.write() {
            *pids = known.iter().copied().collect();
        }
        if !known.is_empty() {
            last_seen = Instant::now();
        } else if last_seen.elapsed() >= EXIT_GRACE {
            log("watcher: no game process for 5 s; the game closed");
            DONE.store(true, Ordering::SeqCst);
            return;
        }
    }
}

// ---------------------------------------------------------------------------------------
// Window helpers

fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn describe(hwnd: HWND) -> String {
    if hwnd.0.is_null() {
        return "(none)".into();
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    format!(
        "{:#x} pid {} {:?} class {:?}",
        hwnd.0 as usize,
        pid,
        window_text(hwnd),
        class_name(hwnd)
    )
}

struct Candidate {
    hwnd: HWND,
    area: i64,
}

extern "system" fn collect_game_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        let list = &mut *(lparam.0 as *mut Vec<Candidate>);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if !is_game_pid(pid) || !IsWindowVisible(hwnd).as_bool() {
            return TRUE;
        }
        if GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.0.is_null()) {
            return TRUE;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            4,
        );
        if cloaked != 0 {
            return TRUE;
        }
        let rect = if IsIconic(hwnd).as_bool() {
            let mut placement = WINDOWPLACEMENT {
                length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                ..Default::default()
            };
            let _ = GetWindowPlacement(hwnd, &mut placement);
            placement.rcNormalPosition
        } else {
            let mut r = RECT::default();
            let _ = GetWindowRect(hwnd, &mut r);
            r
        };
        let area = (rect.right - rect.left) as i64 * (rect.bottom - rect.top) as i64;
        list.push(Candidate { hwnd, area });
        TRUE
    }
}

/// The game's main window: the largest visible, uncloaked, unowned top-level window of a
/// process from the game folder.
fn find_game_window() -> Option<HWND> {
    let mut list: Vec<Candidate> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(collect_game_window),
            LPARAM(&mut list as *mut Vec<Candidate> as isize),
        );
    }
    list.into_iter().max_by_key(|c| c.area).map(|c| c.hwnd)
}

fn is_foreground(target: HWND) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        fg == target || GetAncestor(fg, GA_ROOTOWNER) == target
    }
}

fn press_alt() {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_MENU,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let inputs = [key(Default::default()), key(KEYEVENTF_KEYUP)];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

/// Brings `target` to the front, escalating: plain SetForegroundWindow, then attaching to
/// the foreground thread's input, then a simulated Alt press. Logs each step.
fn force_foreground(target: HWND, label: &str, own_window: bool) -> bool {
    unsafe {
        let before = GetForegroundWindow();
        if is_foreground(target) {
            log(&format!("focus {label}: already in front"));
            return true;
        }
        if IsIconic(target).as_bool() {
            let _ = ShowWindow(target, SW_RESTORE);
            log(&format!("focus {label}: was minimized, restored"));
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(target, Some(&mut pid));
        let allow = AllowSetForegroundWindow(pid);

        let ok1 = SetForegroundWindow(target).as_bool();
        sleep(Duration::from_millis(40));
        if is_foreground(target) {
            log(&format!(
                "focus {label}: SetForegroundWindow ok (returned {ok1}, AllowSetForegroundWindow {allow:?}, was {})",
                describe(before)
            ));
            return true;
        }

        let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let me = GetCurrentThreadId();
        let attached = fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
        let _ = BringWindowToTop(target);
        let ok2 = SetForegroundWindow(target).as_bool();
        if own_window {
            let _ = SetActiveWindow(target);
            let _ = SetFocus(target);
        }
        if attached {
            let _ = AttachThreadInput(me, fg_thread, false);
        }
        sleep(Duration::from_millis(40));
        if is_foreground(target) {
            log(&format!(
                "focus {label}: ok after AttachThreadInput (attached {attached}, returned {ok2}; first try returned {ok1})"
            ));
            return true;
        }

        press_alt();
        let ok3 = SetForegroundWindow(target).as_bool();
        if own_window {
            let _ = SetFocus(target);
        }
        sleep(Duration::from_millis(40));
        let done = is_foreground(target);
        log(&format!(
            "focus {label}: {} after Alt press (returns {ok1}/{ok2}/{ok3}); foreground now {}",
            if done { "ok" } else { "FAILED" },
            describe(GetForegroundWindow())
        ));
        done
    }
}

/// Resume: hand the foreground to the game's main window.
fn hand_over(reason: &str) {
    match find_game_window() {
        None => log(&format!(
            "hand-over ({reason}): no game window found (game pids {:?})",
            GAME_PIDS.read().map(|p| p.clone()).unwrap_or_default()
        )),
        Some(game) => {
            log(&format!("hand-over ({reason}): game window {}", describe(game)));
            force_foreground(game, "game", false);
            // Windows sometimes reports the switch and then reverts it; check once more.
            sleep(Duration::from_millis(150));
            if !is_foreground(game) {
                log(&format!(
                    "hand-over ({reason}): did not stick (foreground {}); retrying",
                    describe(unsafe { GetForegroundWindow() })
                ));
                force_foreground(game, "game (retry)", false);
            }
        }
    }
}

fn schedule_hand_over(source: &str) {
    if OVERLAY_OPEN.load(Ordering::SeqCst) {
        log(&format!("{source}: overlay is open, keeping focus"));
        return;
    }
    let delay = if OVERLAY_MODE.load(Ordering::SeqCst) {
        HANDOVER_DELAY_OVERLAY
    } else {
        HANDOVER_DELAY_RESUME
    };
    if let Ok(mut pending) = PENDING_HANDOVER.lock() {
        if pending.is_none() {
            *pending = Some(Instant::now() + delay);
            log(&format!("{source}: hand-over to the game in {delay:?}"));
        }
    }
}

fn hiword(v: usize) -> usize {
    (v >> 16) & 0xffff
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_ACTIVATEAPP => {
                log(&format!(
                    "WM_ACTIVATEAPP active={} other thread {}",
                    wp.0 != 0,
                    lp.0
                ));
                if wp.0 != 0 {
                    schedule_hand_over("WM_ACTIVATEAPP");
                }
            }
            WM_ACTIVATE => {
                let state = (wp.0 & 0xffff) as u32;
                let other = HWND(lp.0 as *mut c_void);
                log(&format!(
                    "WM_ACTIVATE state={} minimized={} other {}",
                    match state {
                        WA_ACTIVE => "active",
                        WA_CLICKACTIVE => "clickactive",
                        _ => "inactive",
                    },
                    hiword(wp.0) != 0,
                    describe(other)
                ));
                if state != WA_INACTIVE {
                    schedule_hand_over("WM_ACTIVATE");
                }
            }
            WM_SETFOCUS => log("WM_SETFOCUS"),
            WM_KILLFOCUS => log("WM_KILLFOCUS"),
            WM_SHOWWINDOW => log(&format!("WM_SHOWWINDOW show={} status={}", wp.0, lp.0)),
            WM_SYSCOMMAND => log(&format!("WM_SYSCOMMAND {:#x}", wp.0 & 0xfff0)),
            WM_MOUSEACTIVATE => log("WM_MOUSEACTIVATE"),
            WM_CLOSE => {
                log("WM_CLOSE: stopping");
                DONE.store(true, Ordering::SeqCst);
                return LRESULT(0);
            }
            WM_ERASEBKGND if OVERLAY_MODE.load(Ordering::SeqCst) => return LRESULT(1),
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

fn monitor_rect(monitor: HMONITOR) -> RECT {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let _ = GetMonitorInfoW(monitor, &mut info);
    }
    info.rcMonitor
}

fn primary_monitor() -> HMONITOR {
    unsafe { MonitorFromWindow(HWND::default(), MONITOR_DEFAULTTOPRIMARY) }
}

fn ex_style_names(ex: u32) -> String {
    let mut names = vec![];
    for (bit, name) in [
        (WS_EX_TOPMOST.0, "TOPMOST"),
        (WS_EX_LAYERED.0, "LAYERED"),
        (WS_EX_TRANSPARENT.0, "TRANSPARENT"),
        (WS_EX_NOACTIVATE.0, "NOACTIVATE"),
        (WS_EX_APPWINDOW.0, "APPWINDOW"),
        (WS_EX_TOOLWINDOW.0, "TOOLWINDOW"),
    ] {
        if ex & bit != 0 {
            names.push(name);
        }
    }
    names.join("|")
}

fn create_window(args: &Args) -> Result<HWND, String> {
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("BoilRProtoWindow"),
            hbrBackground: if args.overlay {
                Default::default()
            } else {
                CreateSolidBrush(COLORREF(0))
            },
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err("RegisterClassW failed".into());
        }
        let screen = monitor_rect(primary_monitor());
        let (ex, style, rect, title): (WINDOW_EX_STYLE, WINDOW_STYLE, RECT, PCWSTR) = if args
            .overlay
        {
            // GlosSI: popup, monitor size minus 1 px (a window exactly the monitor's size
            // made the background black for them with DPI awareness on).
            let r = RECT {
                left: screen.left,
                top: screen.top,
                right: screen.right - 1,
                bottom: screen.bottom - 1,
            };
            (
                WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT,
                WS_POPUP,
                r,
                w!("BoilR overlay (proto)"),
            )
        } else {
            // A real (non-tool) top-level window with a taskbar button, so Steam has a
            // window to activate. 1x1 px in the primary monitor's bottom-right corner, not
            // topmost: a game in front covers it.
            let n = args.resume_size;
            let r = RECT {
                left: screen.right - n,
                top: screen.bottom - n,
                right: screen.right,
                bottom: screen.bottom,
            };
            (WS_EX_APPWINDOW, WS_POPUP, r, w!("BoilR game (proto)"))
        };
        let hwnd = CreateWindowExW(
            ex,
            w!("BoilRProtoWindow"),
            title,
            style,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            instance,
            None,
        )
        .map_err(|e| e.to_string())?;
        if args.overlay {
            SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)
                .map_err(|e| format!("SetLayeredWindowAttributes: {e}"))?;
            let margins = MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            };
            DwmExtendFrameIntoClientArea(hwnd, &margins)
                .map_err(|e| format!("DwmExtendFrameIntoClientArea: {e}"))?;
        }
        // Shown without activation: the game, starting later, keeps the focus it gets.
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        log(&format!(
            "window created: {} rect ({},{})-({},{}) style WS_POPUP ex {} (shown with SW_SHOWNOACTIVATE)",
            describe(hwnd),
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            ex_style_names(ex.0)
        ));
        Ok(hwnd)
    }
}

// ---------------------------------------------------------------------------------------
// GetForegroundWindow hook in GameOverlayRenderer64.dll's import table

unsafe extern "system" fn hooked_get_foreground_window() -> HWND {
    FG_HOOK_CALLS.fetch_add(1, Ordering::Relaxed);
    let real: unsafe extern "system" fn() -> HWND =
        std::mem::transmute(REAL_GFW.load(Ordering::SeqCst));
    let fg = real();
    if fg.0.is_null() {
        return fg;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(fg, Some(&mut pid));
    let ours = OUR_HWND.load(Ordering::SeqCst);
    if ours != 0 && is_game_pid(pid) {
        FG_HOOK_SUBSTITUTED.fetch_add(1, Ordering::Relaxed);
        return HWND(ours as *mut c_void);
    }
    fg
}

unsafe fn cstr_at(p: *const u8) -> String {
    let mut n = 0;
    while *p.add(n) != 0 && n < 512 {
        n += 1;
    }
    String::from_utf8_lossy(std::slice::from_raw_parts(p, n)).into_owned()
}

/// Replaces `func` (imported by name from `dll`) in `module`'s import address table with
/// `new`. Returns the previous pointer.
unsafe fn patch_iat(module: HMODULE, dll: &str, func: &str, new: usize) -> Result<usize, String> {
    let base = module.0 as *const u8;
    let nt = base.add(*(base.add(0x3c) as *const u32) as usize);
    if *(nt as *const u32) != 0x4550 {
        return Err("no PE header".into());
    }
    let opt = nt.add(24);
    if *(opt as *const u16) != 0x20b {
        return Err("not a 64-bit image".into());
    }
    // Data directory 1 (imports) of a PE32+ optional header.
    let import_rva = *(opt.add(112 + 8) as *const u32) as usize;
    if import_rva == 0 {
        return Err("no import table".into());
    }
    let mut desc = base.add(import_rva);
    loop {
        let original_thunk = *(desc as *const u32) as usize;
        let name = *(desc.add(12) as *const u32) as usize;
        let first_thunk = *(desc.add(16) as *const u32) as usize;
        if name == 0 {
            break;
        }
        if cstr_at(base.add(name)).eq_ignore_ascii_case(dll) {
            let names = base.add(if original_thunk != 0 { original_thunk } else { first_thunk })
                as *const u64;
            let slots = base.add(first_thunk) as *mut usize;
            let mut i = 0;
            loop {
                let entry = *names.add(i);
                if entry == 0 {
                    break;
                }
                if entry & (1 << 63) == 0 && cstr_at(base.add(entry as usize + 2)) == func {
                    let slot = slots.add(i);
                    let mut old = PAGE_PROTECTION_FLAGS(0);
                    VirtualProtect(slot as *const c_void, 8, PAGE_READWRITE, &mut old)
                        .map_err(|e| format!("VirtualProtect: {e}"))?;
                    let previous = *slot;
                    *slot = new;
                    let mut ignored = PAGE_PROTECTION_FLAGS(0);
                    let _ = VirtualProtect(slot as *const c_void, 8, old, &mut ignored);
                    return Ok(previous);
                }
                i += 1;
            }
            return Err(format!("{func} is not imported by name from {dll}"));
        }
        desc = desc.add(20);
    }
    Err(format!("{dll} is not imported"))
}

fn real_get_foreground_window() -> usize {
    unsafe {
        GetModuleHandleW(w!("user32.dll"))
            .ok()
            .and_then(|user32| GetProcAddress(user32, s!("GetForegroundWindow")))
            .map(|f| f as usize)
            .unwrap_or(0)
    }
}

fn install_fg_hook(overlay_dll: HMODULE) -> bool {
    REAL_GFW.store(real_get_foreground_window(), Ordering::SeqCst);
    match unsafe {
        patch_iat(
            overlay_dll,
            "USER32.dll",
            "GetForegroundWindow",
            hooked_get_foreground_window as *const () as usize,
        )
    } {
        Ok(previous) => {
            if previous != REAL_GFW.load(Ordering::SeqCst) {
                // Already redirected by someone; chain to what was there.
                REAL_GFW.store(previous, Ordering::SeqCst);
            }
            log(&format!(
                "fg-hook: GetForegroundWindow in GameOverlayRenderer64.dll now returns our window while a game window is in front (previous {previous:#x})"
            ));
            true
        }
        Err(err) => {
            log(&format!("fg-hook: could not hook: {err}"));
            false
        }
    }
}

/// `--self-test-iat`: hooks GetForegroundWindow in this exe's own import table and checks
/// that a call goes through the hook.
fn self_test_iat() {
    let real = real_get_foreground_window();
    REAL_GFW.store(real, Ordering::SeqCst);
    let me = unsafe { GetModuleHandleW(None).unwrap_or_default() };
    for dll in ["user32.dll", "USER32.dll"] {
        match unsafe { patch_iat(me, dll, "GetForegroundWindow", hooked_get_foreground_window as *const () as usize) } {
            Ok(previous) => {
                let fg = unsafe { GetForegroundWindow() };
                let calls = FG_HOOK_CALLS.load(Ordering::SeqCst);
                log(&format!(
                    "self-test-iat: patched via {dll:?}, previous {previous:#x} (real {real:#x}); call returned {} and the hook saw {calls} call(s): {}",
                    describe(fg),
                    if calls >= 1 { "PASS" } else { "FAIL" }
                ));
                unsafe {
                    let _ = patch_iat(me, dll, "GetForegroundWindow", previous);
                }
                return;
            }
            Err(err) => log(&format!("self-test-iat: {dll}: {err}")),
        }
    }
    log("self-test-iat: FAIL (this exe does not import GetForegroundWindow through an import table)");
}

// ---------------------------------------------------------------------------------------
// Steam overlay state

type BoolFn = unsafe extern "C" fn() -> bool;

struct OverlayExports {
    keyboard: Option<BoolFn>,
    mouse: Option<BoolFn>,
    gamepad: Option<BoolFn>,
    needs_present: Option<BoolFn>,
}

impl OverlayExports {
    fn resolve(module: HMODULE) -> Self {
        let get = |name: windows::core::PCSTR| unsafe {
            GetProcAddress(module, name).map(|f| std::mem::transmute::<_, BoolFn>(f))
        };
        let e = OverlayExports {
            keyboard: get(s!("SteamOverlayIsUsingKeyboard")),
            mouse: get(s!("SteamOverlayIsUsingMouse")),
            gamepad: get(s!("SteamOverlayIsUsingGamepad")),
            needs_present: get(s!("BOverlayNeedsPresent")),
        };
        log(&format!(
            "overlay exports: IsUsingKeyboard {} IsUsingMouse {} IsUsingGamepad {} BOverlayNeedsPresent {}",
            e.keyboard.is_some(),
            e.mouse.is_some(),
            e.gamepad.is_some(),
            e.needs_present.is_some()
        ));
        e
    }

    /// (keyboard, mouse, gamepad, needs_present)
    fn read(&self) -> (bool, bool, bool, bool) {
        let call = |f: Option<BoolFn>| f.is_some_and(|f| unsafe { f() });
        (
            call(self.keyboard),
            call(self.mouse),
            call(self.gamepad),
            call(self.needs_present),
        )
    }
}

/// GlosSI's SteamOverlayDetector: while the overlay is open, Steam's message hook sets the
/// `message` field of (non-input) messages to 0. Three zeroed peeks in a row = open; three
/// non-zero messages >= 1000 in a row = closed. Messages 1..999 are ignored.
#[derive(Default)]
struct GlossiDetector {
    open: bool,
    count: u32,
}

impl GlossiDetector {
    fn update(&mut self) -> Option<bool> {
        let mut msg = MSG::default();
        if !unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) }.as_bool() {
            return None;
        }
        if msg.message > 0 && msg.message < 1000 {
            return None;
        }
        if msg.message == 0 && !self.open {
            self.count += 1;
            if self.count >= 3 {
                self.count = 0;
                self.open = true;
                return Some(true);
            }
        }
        if msg.message != 0 && self.open {
            self.count += 1;
            if self.count >= 3 {
                self.count = 0;
                self.open = false;
                return Some(false);
            }
        }
        None
    }
}

fn overlay_module() -> Option<HMODULE> {
    unsafe { GetModuleHandleW(w!("GameOverlayRenderer64.dll")).ok() }
}

// ---------------------------------------------------------------------------------------
// Direct3D 11

struct Renderer {
    _device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain,
    target: Option<ID3D11RenderTargetView>,
    device: ID3D11Device,
}

impl Renderer {
    fn new(hwnd: HWND, width: u32, height: u32) -> windows::core::Result<Self> {
        let desc = DXGI_SWAP_CHAIN_DESC {
            BufferDesc: DXGI_MODE_DESC {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                ..Default::default()
            },
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 1,
            OutputWindow: hwnd,
            Windowed: TRUE,
            SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
            Flags: 0,
        };
        let mut swap_chain = None;
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDeviceAndSwapChain(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&desc),
                Some(&mut swap_chain),
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }
        let device: ID3D11Device = device.ok_or_else(windows::core::Error::empty)?;
        let mut r = Renderer {
            _device: device.clone(),
            context: context.ok_or_else(windows::core::Error::empty)?,
            swap_chain: swap_chain.ok_or_else(windows::core::Error::empty)?,
            target: None,
            device,
        };
        r.make_target()?;
        Ok(r)
    }

    fn make_target(&mut self) -> windows::core::Result<()> {
        unsafe {
            let buffer: ID3D11Texture2D = self.swap_chain.GetBuffer(0)?;
            let mut view = None;
            self.device
                .CreateRenderTargetView(&buffer.cast::<windows::Win32::Graphics::Direct3D11::ID3D11Resource>()?, None, Some(&mut view))?;
            self.target = view;
        }
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> windows::core::Result<()> {
        self.target = None;
        unsafe {
            self.context.OMSetRenderTargets(None, None);
            self.swap_chain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
        }
        self.make_target()
    }

    fn frame(&self, color: &[f32; 4]) -> windows::core::HRESULT {
        unsafe {
            if let Some(target) = &self.target {
                self.context
                    .OMSetRenderTargets(Some(&[Some(target.clone())]), None);
                self.context.ClearRenderTargetView(target, color);
            }
            self.swap_chain.Present(1, DXGI_PRESENT(0))
        }
    }
}

// ---------------------------------------------------------------------------------------
// Main loops

fn pump_messages() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Runs a due hand-over, and catches our window being in front without an activation
/// message.
fn resume_tick(hwnd: HWND, last_hand_over: &mut Option<Instant>) {
    let due = PENDING_HANDOVER
        .lock()
        .ok()
        .and_then(|mut p| match *p {
            Some(at) if Instant::now() >= at => p.take(),
            _ => None,
        })
        .is_some();
    if due {
        if OVERLAY_OPEN.load(Ordering::SeqCst) {
            log("hand-over cancelled: the overlay opened");
        } else if let Some(at) = last_hand_over.filter(|t| t.elapsed() < HANDOVER_MIN_GAP) {
            log("hand-over postponed: one ran less than 500 ms ago");
            if let Ok(mut p) = PENDING_HANDOVER.lock() {
                *p = Some(at + HANDOVER_MIN_GAP);
            }
        } else {
            let fg = unsafe { GetForegroundWindow() };
            if fg != hwnd {
                log(&format!(
                    "note: our window is no longer in front at hand-over time (foreground {})",
                    describe(fg)
                ));
            }
            hand_over("our window was activated");
            *last_hand_over = Some(Instant::now());
        }
    }
    let pending = PENDING_HANDOVER.lock().map(|p| p.is_some()).unwrap_or(false);
    if !pending
        && !OVERLAY_OPEN.load(Ordering::SeqCst)
        && unsafe { GetForegroundWindow() } == hwnd
        && !last_hand_over.is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        && find_game_window().is_some()
    {
        schedule_hand_over("poll: our window is in front");
    }
}

fn run_resume(hwnd: HWND) {
    let mut last_hand_over = None;
    let mut last_status = Instant::now();
    while !DONE.load(Ordering::SeqCst) {
        unsafe {
            MsgWaitForMultipleObjects(None, false, 50, QS_ALLINPUT);
        }
        pump_messages();
        resume_tick(hwnd, &mut last_hand_over);
        if last_status.elapsed() > Duration::from_secs(30) {
            last_status = Instant::now();
            log(&format!(
                "status: foreground {}",
                describe(unsafe { GetForegroundWindow() })
            ));
        }
    }
}

struct OverlayLoop {
    hwnd: HWND,
    renderer: Renderer,
    monitor: HMONITOR,
    color: [f32; 4],
    glossi: GlossiDetector,
    exports: Option<OverlayExports>,
    fg_hook_wanted: bool,
    fg_hooked: bool,
    exports_state: (bool, bool, bool, bool),
    exports_open: bool,
    glossi_open: bool,
    remembered_foreground: HWND,
    hotkey_down: bool,
    test_overlay: Option<(f64, f64)>,
}

impl OverlayLoop {
    fn set_click_through(&self, click_through: bool) {
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32;
            let ex = if click_through {
                ex | WS_EX_TRANSPARENT.0
            } else {
                ex & !WS_EX_TRANSPARENT.0
            };
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex as isize);
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            log(&format!(
                "window ex style now {}",
                ex_style_names(GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32)
            ));
        }
    }

    fn overlay_changed(&mut self, open: bool, source: &str) {
        log(&format!("OVERLAY {} (detected by {source})", if open { "OPENED" } else { "CLOSED" }));
        if open {
            OVERLAY_OPEN.store(true, Ordering::SeqCst);
            if let Ok(mut p) = PENDING_HANDOVER.lock() {
                *p = None;
            }
            self.remembered_foreground = unsafe { GetForegroundWindow() };
            log(&format!("foreground before: {}", describe(self.remembered_foreground)));
            self.set_click_through(false);
            force_foreground(self.hwnd, "our window (overlay open)", true);
        } else {
            OVERLAY_OPEN.store(false, Ordering::SeqCst);
            self.set_click_through(true);
            let previous = self.remembered_foreground;
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(previous, Some(&mut pid)) };
            if !previous.0.is_null() && is_game_pid(pid) {
                force_foreground(previous, "game (overlay closed)", false);
            } else {
                hand_over("overlay closed");
            }
        }
    }

    fn detect(&mut self) {
        if let Some(open) = self.glossi.update() {
            log(&format!("GlosSI detector says overlay {}", if open { "open" } else { "closed" }));
            self.glossi_open = open;
        }
        if let Some(exports) = &self.exports {
            let state = exports.read();
            if state != self.exports_state {
                log(&format!(
                    "overlay exports changed: keyboard {} mouse {} gamepad {} needs_present {}",
                    state.0, state.1, state.2, state.3
                ));
                self.exports_state = state;
            }
            self.exports_open = state.0 || state.1 || state.2;
        }
        let since = STARTED.get().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        let test_open = self
            .test_overlay
            .is_some_and(|(start, duration)| since >= start && since < start + duration);
        let open = self.exports_open || self.glossi_open || test_open;
        if open != OVERLAY_OPEN.load(Ordering::SeqCst) {
            let source = match (self.exports_open, self.glossi_open) {
                (false, false) if self.test_overlay.is_some() => "--test-overlay",
                (true, true) => "exports + GlosSI",
                (true, false) => "exports",
                (false, true) => "GlosSI",
                (false, false) => "both now closed",
            };
            self.overlay_changed(open, source);
        }
    }

    /// GlosSI's overlayHotkeyWorkaround: when Shift+Tab is held while another window has
    /// the keyboard, post the keys to our window, where Steam's message hook can see them.
    fn hotkey_workaround(&mut self) {
        let down = unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) < 0 && GetAsyncKeyState(VK_TAB.0 as i32) < 0 };
        let ours_in_front = unsafe { GetForegroundWindow() } == self.hwnd;
        let post = |msg: u32, vk: u16, up: bool| unsafe {
            let scan = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as isize;
            let mut lp = 1 | (scan << 16);
            if up {
                lp |= (1 << 30) | (1 << 31);
            }
            let _ = PostMessageW(self.hwnd, msg, WPARAM(vk as usize), LPARAM(lp));
        };
        if down && !self.hotkey_down {
            self.hotkey_down = true;
            if !ours_in_front {
                log("Shift+Tab held while another window is in front: posting it to our window");
                post(WM_KEYDOWN, VK_SHIFT.0, false);
                post(WM_KEYDOWN, VK_TAB.0, false);
            }
        } else if !down && self.hotkey_down {
            self.hotkey_down = false;
            if !ours_in_front {
                post(WM_KEYUP, VK_TAB.0, true);
                post(WM_KEYUP, VK_SHIFT.0, true);
            }
        }
    }

    fn follow_game_monitor(&mut self) {
        let Some(game) = find_game_window() else { return };
        let monitor = unsafe { MonitorFromWindow(game, MONITOR_DEFAULTTONEAREST) };
        if monitor == self.monitor {
            return;
        }
        self.monitor = monitor;
        let r = monitor_rect(monitor);
        let (w, h) = (r.right - r.left - 1, r.bottom - r.top - 1);
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                r.left,
                r.top,
                w,
                h,
                SWP_NOACTIVATE,
            );
        }
        let result = self.renderer.resize(w as u32, h as u32);
        log(&format!(
            "moved to the game's monitor ({},{})-({},{}); resize {:?}",
            r.left, r.top, r.right, r.bottom, result
        ));
    }

    fn run(&mut self) {
        let mut last_hand_over = None;
        let mut frames: u64 = 0;
        let mut window_frames: u64 = 0;
        let mut window_start = Instant::now();
        let mut last_second = Instant::now();
        let mut failed_presents = 0u64;
        while !DONE.load(Ordering::SeqCst) {
            let frame_start = Instant::now();
            self.detect();
            pump_messages();
            self.hotkey_workaround();
            resume_tick(self.hwnd, &mut last_hand_over);

            let hr = self.renderer.frame(&self.color);
            if hr.is_err() {
                failed_presents += 1;
                if failed_presents <= 5 || failed_presents % 600 == 0 {
                    log(&format!("Present failed: {hr:?} ({failed_presents} so far)"));
                }
            }
            frames += 1;
            window_frames += 1;

            if last_second.elapsed() >= Duration::from_secs(1) {
                last_second = Instant::now();
                if self.exports.is_none() || (self.fg_hook_wanted && !self.fg_hooked) {
                    if let Some(module) = overlay_module() {
                        if self.exports.is_none() {
                            log("GameOverlayRenderer64.dll is loaded");
                            self.exports = Some(OverlayExports::resolve(module));
                        }
                        if self.fg_hook_wanted && !self.fg_hooked {
                            self.fg_hooked = install_fg_hook(module);
                            // Do not retry a failed hook every second.
                            self.fg_hook_wanted = self.fg_hooked;
                        }
                    }
                }
                self.follow_game_monitor();
            }
            if window_start.elapsed() >= Duration::from_secs(10) {
                let secs = window_start.elapsed().as_secs_f64();
                log(&format!(
                    "present: {frames} frames total, {:.1} fps over the last {secs:.0}s; fg-hook calls {} (substituted {}); foreground {}",
                    window_frames as f64 / secs,
                    FG_HOOK_CALLS.load(Ordering::Relaxed),
                    FG_HOOK_SUBSTITUTED.load(Ordering::Relaxed),
                    describe(unsafe { GetForegroundWindow() })
                ));
                window_frames = 0;
                window_start = Instant::now();
            }
            // Present(1) normally waits for vblank; cap at ~60 fps in case it does not.
            let spent = frame_start.elapsed();
            if spent < Duration::from_millis(15) {
                sleep(Duration::from_millis(16) - spent);
            }
        }
        log(&format!("overlay loop ended after {frames} frames"));
    }
}

fn run_overlay(hwnd: HWND, args: &Args) -> Result<(), String> {
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
    }
    let (w, h) = (client.right as u32, client.bottom as u32);
    let renderer = Renderer::new(hwnd, w, h).map_err(|e| format!("Direct3D 11: {e}"))?;
    log(&format!(
        "Direct3D 11 device and swap chain created ({w}x{h}, B8G8R8A8, DISCARD, 1 buffer); clearing to {}",
        if args.tint { "faint red (--tint)" } else { "fully transparent" }
    ));
    let color = if args.tint {
        [0.25, 0.0, 0.0, 0.25] // premultiplied: 25 % red
    } else {
        [0.0, 0.0, 0.0, 0.0]
    };
    let mut overlay = OverlayLoop {
        hwnd,
        renderer,
        monitor: primary_monitor(),
        color,
        glossi: GlossiDetector::default(),
        exports: None,
        fg_hook_wanted: args.fg_hook,
        fg_hooked: false,
        exports_state: (false, false, false, false),
        exports_open: false,
        glossi_open: false,
        remembered_foreground: HWND::default(),
        hotkey_down: false,
        test_overlay: args.test_overlay,
    };
    overlay.run();
    Ok(())
}

fn main() {
    let _ = STARTED.set(Instant::now());
    let log_path = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("proto.log")))
        .unwrap_or_else(|| PathBuf::from("proto.log"));
    if let Ok(mut guard) = LOG.lock() {
        *guard = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .ok();
    }
    log("==================================================================");
    log(&format!("start: {:?}", std::env::args().collect::<Vec<_>>()));
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            log(&format!("bad arguments: {e}"));
            return;
        }
    };
    if args.self_test_iat {
        self_test_iat();
        return;
    }
    log(&format!("parsed: {args:?}"));
    for var in ["SteamAppId", "SteamGameId", "SteamOverlayGameId", "SteamClientLaunch"] {
        log(&format!("env {var}={:?}", std::env::var(var).ok()));
    }
    log(&format!(
        "GameOverlayRenderer64.dll loaded at start: {}",
        overlay_module().is_some()
    ));
    OVERLAY_MODE.store(args.overlay, Ordering::SeqCst);
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let watcher = {
        let folder = args.game_folder.clone();
        let link = args.link.clone();
        std::thread::spawn(move || watch(folder, link))
    };

    let hwnd = match create_window(&args) {
        Ok(h) => h,
        Err(e) => {
            log(&format!("could not create the window: {e}"));
            DONE.store(true, Ordering::SeqCst);
            let _ = watcher.join();
            return;
        }
    };
    OUR_HWND.store(hwnd.0 as isize, Ordering::SeqCst);

    if args.overlay {
        if let Err(e) = run_overlay(hwnd, &args) {
            log(&format!("overlay mode failed: {e}; falling back to resume only"));
            run_resume(hwnd);
        }
    } else {
        if args.fg_hook {
            match overlay_module() {
                Some(module) => {
                    install_fg_hook(module);
                }
                None => log("fg-hook: GameOverlayRenderer64.dll is not loaded; no hook"),
            }
        }
        run_resume(hwnd);
    }
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    DONE.store(true, Ordering::SeqCst);
    let _ = watcher.join();
    log("exit");
}
