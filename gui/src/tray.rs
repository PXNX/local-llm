//! A Windows notification-area icon for the llama.cpp router: open the window, unload the model,
//! stop the router or quit, without looking for a console the router doesn't have.

use std::sync::OnceLock;
use std::sync::Mutex;

use eframe::egui;

/// Shared with the tray window's message loop, which lives for the whole process.
struct Shared {
    ctx: egui::Context,
    tip: Mutex<String>,
    /// The router the menu's unload/stop act on.
    url: Mutex<String>,
}

static SHARED: OnceLock<&'static Shared> = OnceLock::new();

pub struct Tray {
    shared: &'static Shared,
}

impl Tray {
    /// Creates the icon in a background thread; a failure just leaves the app without a tray icon.
    pub fn new(ctx: egui::Context) -> Self {
        let shared = SHARED.get_or_init(|| {
            Box::leak(Box::new(Shared {
                ctx,
                tip: Mutex::new("local-llm".into()),
                url: Mutex::new(crate::llamacpp::DEFAULT_URL.into()),
            }))
        });
        std::thread::Builder::new().name("tray".into()).spawn(windows_impl::run).ok();
        Self { shared }
    }

    pub fn set_tip(&self, tip: impl Into<String>) {
        let mut t = self.shared.tip.lock().unwrap();
        let tip = tip.into();
        if *t != tip {
            *t = tip;
        }
    }

    /// The endpoint the menu's Unload/Stop act on (the Coding page's current router).
    pub fn set_router(&self, url: &str) {
        let mut u = self.shared.url.lock().unwrap();
        if *u != url {
            *u = url.to_owned();
        }
    }
}

fn shared() -> Option<&'static Shared> {
    SHARED.get().copied()
}

#[cfg(windows)]
mod windows_impl {
    use super::shared;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW, FindWindowW,
        GetCursorPos, GetMessageW, GetSystemMetrics, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR, LoadIconW, LoadImageW,
        MF_SEPARATOR, MF_STRING, MSG, PostMessageW, RegisterClassW, SM_CXSMICON, SW_RESTORE, SetForegroundWindow, SetTimer,
        ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE, WM_CONTEXTMENU,
        WM_DESTROY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
    };

    const CLASS_NAME: &[u16] = &[108, 111, 99, 97, 108, 45, 108, 108, 109, 45, 116, 114, 97, 121, 0];
    /// Title of the hidden tray window (different from the app's "local-llm", so FindWindowW gets the app).
    const TRAY_TITLE: &[u16] = &[108, 111, 99, 97, 108, 45, 108, 108, 109, 45, 116, 114, 97, 121, 0];
    const APP_TITLE: &[u16] = &[108, 111, 99, 97, 108, 45, 108, 108, 109, 0];
    const CALLBACK: u32 = WM_APP + 1;
    const TIMER: usize = 1;
    const ICON_ID: u32 = 1;
    const MENU_OPEN: usize = 1;
    const MENU_UNLOAD: usize = 2;
    const MENU_STOP: usize = 3;
    const MENU_QUIT: usize = 4;

    pub fn run() {
        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinst,
                lpszClassName: CLASS_NAME.as_ptr(),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return;
            }
            let hwnd = CreateWindowExW(
                0,
                CLASS_NAME.as_ptr(),
                TRAY_TITLE.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinst,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return;
            }
            let data = icon_data(hwnd);
            if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
                return;
            }
            SetTimer(hwnd, TIMER, 1000, None);
            let mut msg = MSG::default();
            // GetMessageW returns -1 on error, 0 on WM_QUIT
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            CALLBACK => {
                // versions before NOTIFYICON_VERSION_4: the mouse message is the low word of lparam
                let event = lparam as u32;
                if event == WM_LBUTTONUP || event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                    unsafe { menu(hwnd) };
                }
                0
            }
            WM_TIMER => {
                unsafe { refresh_tip(hwnd) };
                0
            }
            WM_DESTROY => {
                unsafe {
                    let data = icon_data(hwnd);
                    Shell_NotifyIconW(NIM_DELETE, &data);
                }
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    unsafe fn menu(hwnd: HWND) {
        let text = |s: &str| -> Vec<u16> { s.encode_utf16().chain(Some(0)).collect() };
        let open = text("Open local-llm");
        let unload = text("Unload llama.cpp model");
        let stop = text("Stop llama.cpp router");
        let quit = text("Quit local-llm");
        unsafe {
            let hmenu = CreatePopupMenu();
            AppendMenuW(hmenu, MF_STRING, MENU_OPEN, open.as_ptr());
            AppendMenuW(hmenu, MF_STRING, MENU_UNLOAD, unload.as_ptr());
            AppendMenuW(hmenu, MF_STRING, MENU_STOP, stop.as_ptr());
            AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
            AppendMenuW(hmenu, MF_STRING, MENU_QUIT, quit.as_ptr());
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            // without this the menu stays open when the user clicks elsewhere
            SetForegroundWindow(hwnd);
            let cmd = TrackPopupMenu(hmenu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, std::ptr::null());
            DestroyMenu(hmenu);
            PostMessageW(hwnd, WM_NULL, 0, 0);
            match cmd as usize {
                MENU_OPEN => show_app(),
                MENU_UNLOAD => job(false),
                MENU_STOP => job(true),
                MENU_QUIT => {
                    if let Some(app) = app_window() {
                        ShowWindow(app, SW_RESTORE);
                        PostMessageW(app, WM_CLOSE, 0, 0);
                    }
                }
                _ => {}
            }
        }
    }

    /// The app's own window (not the hidden tray window), by its title.
    fn app_window() -> Option<HWND> {
        let hwnd = unsafe { FindWindowW(std::ptr::null(), APP_TITLE.as_ptr()) };
        (!hwnd.is_null()).then_some(hwnd)
    }

    /// Brings the app to the front - also when it is minimized and draws no frames.
    fn show_app() {
        if let Some(app) = app_window() {
            unsafe {
                ShowWindow(app, SW_RESTORE);
                SetForegroundWindow(app);
            }
        }
    }

    /// Unloads (or stops) the router in a worker thread: the UI may be minimized, and the menu must
    /// not block the message loop.
    fn job(stop: bool) {
        let Some(s) = shared() else { return };
        let url = s.url.lock().unwrap().clone();
        std::thread::spawn(move || {
            let _ = if stop { crate::llamacpp::stop(&url) } else { crate::llamacpp::unload_all(&url) };
            s.ctx.request_repaint();
        });
    }

    unsafe fn refresh_tip(hwnd: HWND) {
        let tip = shared().map(|s| s.tip.lock().unwrap().clone()).unwrap_or_default();
        let mut data = icon_data(hwnd);
        copy(&mut data.szTip, &tip);
        data.uFlags = NIF_TIP;
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: CALLBACK,
            ..Default::default()
        };
        let tip = shared().map(|s| s.tip.lock().unwrap().clone()).unwrap_or_default();
        copy(&mut data.szTip, &tip);
        data.hIcon = icon();
        data
    }

    /// The exe's icon (winresource id 1), falling back to the generic application icon. Loaded once:
    /// the tooltip refresh shouldn't leak a new HICON every second.
    #[allow(clippy::manual_dangling_ptr)] // MAKEINTRESOURCE(1): an integer, not a pointer
    fn icon() -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
        static ICON: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        *ICON.get_or_init(|| unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let size = GetSystemMetrics(SM_CXSMICON);
            let icon = LoadImageW(hinst, 1 as *const u16, IMAGE_ICON, size, size, LR_DEFAULTCOLOR);
            (if icon.is_null() { LoadIconW(hinst, IDI_APPLICATION) } else { icon }) as usize
        }) as windows_sys::Win32::UI::WindowsAndMessaging::HICON
    }

    /// Copies UTF-16 into a fixed Windows array, truncating and terminating it.
    fn copy(dst: &mut [u16], text: &str) {
        let mut w: Vec<u16> = text.encode_utf16().take(dst.len().saturating_sub(1)).collect();
        w.push(0);
        dst[..w.len()].copy_from_slice(&w);
    }
}

#[cfg(not(windows))]
mod windows_impl {
    pub fn run() {}
}
