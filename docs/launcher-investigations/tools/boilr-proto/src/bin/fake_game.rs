//! A stand-in game for testing boilr-proto: one plain 800x600 window titled "Fake Game"
//! with a green client area. Closes when its window closes.
#![windows_subsystem = "windows"]

use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::CreateSolidBrush,
        Foundation::COLORREF,
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
};

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_DESTROY {
            PostQuitMessage(0);
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

fn main() {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("FakeGameWindow"),
            hbrBackground: CreateSolidBrush(COLORREF(0x0040A040)),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&class);
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("FakeGameWindow"),
            w!("Fake Game"),
            WS_OVERLAPPEDWINDOW,
            200,
            150,
            800,
            600,
            None,
            None,
            instance,
            None,
        ) else {
            return;
        };
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
