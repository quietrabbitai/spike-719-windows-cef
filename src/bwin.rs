//! Approach B Win32 side: the child windows that carry each pane's swapchain, the native input path used by the
//! "above" placement, the Win32 context menu, the clipboard, and the host-managed popup window.
use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use cef::{
    ImplBrowser, ImplBrowserHost, ImplRunContextMenuCallback, KeyEventType, MenuId, MouseButtonType,
};
use tauri::Emitter;
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
    VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetClientRect, GetWindowLongPtrW, LoadCursorW, RegisterClassW, SetCursor, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, TrackPopupMenuEx, GWLP_USERDATA, HCURSOR, HTCLIENT,
    HWND_BOTTOM, HWND_TOP, IDC_ARROW, MF_GRAYED, MF_STRING, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_CHAR, WM_ERASEBKGND, WM_KEYDOWN, WM_KEYUP,
    WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SETFOCUS, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WNDCLASSW, WS_CHILD, WS_VISIBLE,
};

use crate::bcef;
use crate::bgpu::Cmd;
use crate::bshared::with_slot;
use crate::cefapp::{POPUP_SLOT, SLOT_COUNT};

/// Not exported by windows-rs's WindowsAndMessaging module (it lives in the Controls namespace).
const WM_MOUSELEAVE: u32 = 0x02A3;

pub struct BState {
    pub parent: isize,
    /// true = pane windows above the WebView2 child with native input; false = below a transparent WebView2 with
    /// the DOM hit-layer input path.
    pub above: bool,
    pub tx: Mutex<Sender<Cmd>>,
}

static STATE: OnceLock<BState> = OnceLock::new();
static HWNDS: [AtomicIsize; SLOT_COUNT] = [const { AtomicIsize::new(0) }; SLOT_COUNT];

pub fn state() -> Option<&'static BState> {
    STATE.get()
}

fn hw(v: isize) -> HWND {
    HWND(v as *mut c_void)
}

pub fn init(parent: isize, above: bool, tx: Sender<Cmd>) {
    let _ = STATE.set(BState {
        parent,
        above,
        tx: Mutex::new(tx),
    });
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: w!("Spike719Pane"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
}

pub fn dpi_scale(parent: isize) -> f32 {
    let d = unsafe { GetDpiForWindow(hw(parent)) };
    if d == 0 {
        1.0
    } else {
        d as f32 / 96.0
    }
}

pub fn client_size(parent: isize) -> (i32, i32) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(hw(parent), &mut r);
    }
    (r.right - r.left, r.bottom - r.top)
}

pub fn create_child(slot: usize, x: i32, y: i32, w: i32, h: i32) -> Result<isize, String> {
    let st = state().ok_or("bwin not initialised")?;
    unsafe {
        let hinst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let h = CreateWindowExW(
            Default::default(),
            w!("Spike719Pane"),
            w!(""),
            WS_CHILD | WS_VISIBLE,
            x,
            y,
            w.max(1),
            h.max(1),
            Some(hw(st.parent)),
            None,
            Some(hinst.into()),
            None,
        )
        .map_err(|e| e.to_string())?;
        SetWindowLongPtrW(h, GWLP_USERDATA, slot as isize + 1);
        HWNDS[slot].store(h.0 as isize, Ordering::Relaxed);
        Ok(h.0 as isize)
    }
}

pub fn slot_hwnd(slot: usize) -> Option<isize> {
    let v = HWNDS[slot].load(Ordering::Relaxed);
    (v != 0).then_some(v)
}

/// Screen coordinates of the top-left of a slot's client area.
pub fn slot_origin(slot: usize) -> (i32, i32) {
    let Some(h) = slot_hwnd(slot) else {
        return (0, 0);
    };
    let mut p = POINT { x: 0, y: 0 };
    unsafe {
        let _ = ClientToScreen(hw(h), &mut p);
    }
    (p.x, p.y)
}

/// Re-stack the pane windows: popup above panes; all above the WebView2 child (above) or all below it (below).
pub fn reorder() {
    let Some(st) = state() else { return };
    unsafe {
        if st.above {
            for slot in [1usize, 0, POPUP_SLOT] {
                if let Some(h) = slot_hwnd(slot) {
                    let _ = SetWindowPos(
                        hw(h),
                        Some(HWND_TOP),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
            }
        } else {
            for slot in [POPUP_SLOT, 0usize, 1] {
                if let Some(h) = slot_hwnd(slot) {
                    let _ = SetWindowPos(
                        hw(h),
                        Some(HWND_BOTTOM),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
            }
        }
    }
}

/// Moves/resizes a slot's window and tells CEF (physical pixels).
pub fn place(slot: usize, x: i32, y: i32, w: i32, h: i32) {
    let Some(st) = state() else { return };
    let Some(hn) = slot_hwnd(slot) else { return };
    let scale = dpi_scale(st.parent);
    let (w, h) = (w.max(1), h.max(1));
    unsafe {
        let _ = SetWindowPos(hw(hn), None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    let changed = with_slot(slot, |s| {
        let c = s.pix != (w as u32, h as u32) || (s.scale - scale).abs() > f32::EPSILON;
        s.pix = (w as u32, h as u32);
        s.scale = scale;
        s.active = true;
        c
    })
    .unwrap_or(false);
    if changed {
        if let Some(host) = crate::cefapp::pane_browser(slot).and_then(|b| b.host()) {
            host.notify_screen_info_changed();
            host.was_resized();
        }
    }
}

// ------------------------------------------------------------------ native input (above placement)

fn key_mods() -> u32 {
    use cef::sys::cef_event_flags_t as F;
    let down = |vk: i32| unsafe { GetKeyState(vk) < 0 };
    let mut f = 0u32;
    if down(VK_SHIFT.0 as i32) {
        f |= F::EVENTFLAG_SHIFT_DOWN.0 as u32;
    }
    if down(VK_CONTROL.0 as i32) {
        f |= F::EVENTFLAG_CONTROL_DOWN.0 as u32;
    }
    if down(VK_MENU.0 as i32) {
        f |= F::EVENTFLAG_ALT_DOWN.0 as u32;
    }
    f
}

fn mouse_mods(wp: usize) -> u32 {
    use cef::sys::cef_event_flags_t as F;
    let mut f = key_mods();
    if wp & 0x1 != 0 {
        f |= F::EVENTFLAG_LEFT_MOUSE_BUTTON.0 as u32;
    }
    if wp & 0x2 != 0 {
        f |= F::EVENTFLAG_RIGHT_MOUSE_BUTTON.0 as u32;
    }
    if wp & 0x10 != 0 {
        f |= F::EVENTFLAG_MIDDLE_MOUSE_BUTTON.0 as u32;
    }
    f
}

fn lp_xy(lp: LPARAM) -> (i32, i32) {
    (
        (lp.0 & 0xffff) as i16 as i32,
        ((lp.0 >> 16) & 0xffff) as i16 as i32,
    )
}

thread_local! {
    static TRACKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn native_input(slot: usize, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    let scale = with_slot(slot, |s| s.scale).unwrap_or(1.0);
    let dip = |v: i32| (v as f32 / scale) as i32;
    match msg {
        WM_MOUSEMOVE => {
            let (x, y) = lp_xy(lp);
            if !TRACKING.with(|t| t.replace(true)) {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    let _ = TrackMouseEvent(&mut tme);
                }
            }
            bcef::mouse_move(slot, dip(x), dip(y), false, mouse_mods(wp.0));
            Some(LRESULT(0))
        }
        WM_MOUSELEAVE => {
            TRACKING.with(|t| t.set(false));
            bcef::mouse_move(slot, 0, 0, true, key_mods());
            Some(LRESULT(0))
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP
        | WM_MBUTTONUP => {
            let (x, y) = lp_xy(lp);
            let (button, up) = match msg {
                WM_LBUTTONDOWN => (MouseButtonType::LEFT, false),
                WM_LBUTTONUP => (MouseButtonType::LEFT, true),
                WM_RBUTTONDOWN => (MouseButtonType::RIGHT, false),
                WM_RBUTTONUP => (MouseButtonType::RIGHT, true),
                WM_MBUTTONDOWN => (MouseButtonType::MIDDLE, false),
                _ => (MouseButtonType::MIDDLE, true),
            };
            unsafe {
                if !up {
                    let _ = SetFocus(Some(hwnd));
                    SetCapture(hwnd);
                } else {
                    let _ = ReleaseCapture();
                }
            }
            bcef::mouse_click(slot, dip(x), dip(y), button, up, 1, mouse_mods(wp.0));
            Some(LRESULT(0))
        }
        WM_MOUSEWHEEL => {
            let mut p = POINT {
                x: (lp.0 & 0xffff) as i16 as i32,
                y: ((lp.0 >> 16) & 0xffff) as i16 as i32,
            };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut p);
            }
            let delta = ((wp.0 >> 16) & 0xffff) as i16 as i32;
            bcef::mouse_wheel(slot, dip(p.x), dip(p.y), 0, delta / 3, key_mods());
            Some(LRESULT(0))
        }
        WM_SETFOCUS => {
            bcef::set_focus(slot, true);
            Some(LRESULT(0))
        }
        WM_KILLFOCUS => {
            bcef::set_focus(slot, false);
            Some(LRESULT(0))
        }
        WM_SETCURSOR => {
            if (lp.0 & 0xffff) as u32 == HTCLIENT {
                let hc = with_slot(slot, |s| s.hcursor).unwrap_or(0);
                if hc != 0 {
                    unsafe {
                        SetCursor(Some(HCURSOR(hc as *mut c_void)));
                    }
                    return Some(LRESULT(1));
                }
            }
            None
        }
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            let vk = wp.0 as i32;
            let mods = key_mods();
            let ctrl_only = unsafe {
                GetKeyState(VK_CONTROL.0 as i32) < 0 && GetKeyState(VK_MENU.0 as i32) >= 0
            };
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            if ctrl_only {
                let cmd = match vk as u8 {
                    b'C' => Some("copy"),
                    b'X' => Some("cut"),
                    b'V' => Some("paste"),
                    b'A' => Some("select_all"),
                    _ => None,
                };
                if let Some(c) = cmd {
                    if down {
                        bcef::edit_command(slot, c);
                    }
                    return Some(LRESULT(0));
                }
            }
            let kind = if down {
                KeyEventType::RAWKEYDOWN
            } else {
                KeyEventType::KEYUP
            };
            bcef::key(
                slot,
                kind,
                vk,
                lp.0 as i32,
                0,
                mods,
                msg == WM_SYSKEYDOWN || msg == WM_SYSKEYUP,
            );
            if msg == WM_SYSKEYDOWN || msg == WM_SYSKEYUP {
                None
            } else {
                Some(LRESULT(0))
            }
        }
        WM_CHAR => {
            let ch = wp.0 as u16;
            if ch >= 0x20 || ch == 9 || ch == 13 {
                bcef::key(
                    slot,
                    KeyEventType::CHAR,
                    ch as i32,
                    lp.0 as i32,
                    ch,
                    key_mods(),
                    false,
                );
            }
            Some(LRESULT(0))
        }
        _ => None,
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let ud = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if msg == WM_ERASEBKGND {
        return LRESULT(1);
    }
    if ud > 0 && state().map(|s| s.above).unwrap_or(false) {
        if let Some(r) = native_input((ud - 1) as usize, hwnd, msg, wp, lp) {
            return r;
        }
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

// ------------------------------------------------------------------ clipboard + context menu

pub fn write_clipboard(text: &str) {
    let Some(st) = state() else { return };
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if OpenClipboard(Some(hw(st.parent))).is_err() {
            log::warn!("OpenClipboard failed");
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) {
            let p = GlobalLock(hmem) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                let _ = GlobalUnlock(hmem);
                if SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(hmem.0))).is_err() {
                    log::warn!("SetClipboardData failed");
                }
            }
        }
        let _ = CloseClipboard();
    }
}

/// The Win32 stand-in for the Linux gtk::Menu (items.id=379). Runs on the main thread.
pub fn show_context_menu(
    slot: usize,
    x: i32,
    y: i32,
    editable: bool,
    has_selection: bool,
    callback: cef::RunContextMenuCallback,
) {
    let Some(st) = state() else {
        callback.cancel();
        return;
    };
    let scale = with_slot(slot, |s| s.scale).unwrap_or(1.0);
    let (ox, oy) = slot_origin(slot);
    let (sx, sy) = (
        ox + (x as f32 * scale) as i32,
        oy + (y as f32 * scale) as i32,
    );
    let chosen = unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            callback.cancel();
            return;
        };
        let on = |b: bool| if b { MF_STRING } else { MF_STRING | MF_GRAYED };
        let _ = AppendMenuW(menu, on(has_selection), 1, &HSTRING::from("Copy"));
        let _ = AppendMenuW(
            menu,
            on(editable && has_selection),
            2,
            &HSTRING::from("Cut"),
        );
        let _ = AppendMenuW(menu, on(editable), 3, &HSTRING::from("Paste"));
        let _ = AppendMenuW(menu, on(true), 4, &HSTRING::from("Select all"));
        let _ = SetForegroundWindow(hw(st.parent));
        let id = TrackPopupMenuEx(
            menu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            sx,
            sy,
            hw(st.parent),
            None,
        );
        let _ = DestroyMenu(menu);
        id.0
    };
    log::info!("context menu choice: {chosen}");
    match chosen {
        1 => {
            // Selection-cache copy path (stand-in for the clipboard route that Linux lacks): write the cached text.
            bcef::edit_command(slot, "copy");
            callback.cancel();
        }
        2 => callback.cont(MenuId::CUT.get_raw(), cef::EventFlags::default()),
        3 => callback.cont(MenuId::PASTE.get_raw(), cef::EventFlags::default()),
        4 => callback.cont(MenuId::SELECT_ALL.get_raw(), cef::EventFlags::default()),
        _ => callback.cancel(),
    }
}

// ------------------------------------------------------------------ host-managed popup window (slot 2)

const POPUP_W: f32 = 480.0;
const POPUP_H: f32 = 640.0;

pub fn popup_opened() {
    let Some(app) = bcef::APP.get() else { return };
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(st) = state() else { return };
        let scale = dpi_scale(st.parent);
        let (cw, ch) = client_size(st.parent);
        let pw = ((POPUP_W * scale) as i32).min(cw - 20).max(200);
        let ph = ((POPUP_H * scale) as i32).min(ch - 20).max(200);
        let (x, y) = ((cw - pw) / 2, (ch - ph) / 2);
        match create_child(POPUP_SLOT, x, y, pw, ph) {
            Ok(h) => {
                with_slot(POPUP_SLOT, |s| {
                    s.pix = (pw as u32, ph as u32);
                    s.scale = scale;
                    s.active = true;
                });
                if let Ok(tx) = st.tx.lock() {
                    let _ = tx.send(Cmd::Add { slot: POPUP_SLOT, hwnd: h });
                }
                reorder();
                if let Some(host) = crate::cefapp::pane_browser(POPUP_SLOT).and_then(|b| b.host()) {
                    host.notify_screen_info_changed();
                    host.was_resized();
                    host.set_focus(true as _);
                }
                let _ = app2.emit(
                    "popup-rect",
                    serde_json::json!({ "x": x as f32 / scale, "y": y as f32 / scale, "w": pw as f32 / scale, "h": ph as f32 / scale }),
                );
                log::info!("popup window created {pw}x{ph} at {x},{y}");
            }
            Err(e) => log::error!("popup window creation failed: {e}"),
        }
    });
}

pub fn popup_closed() {
    let Some(app) = bcef::APP.get() else { return };
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(st) = state() {
            if let Ok(tx) = st.tx.lock() {
                let _ = tx.send(Cmd::Remove { slot: POPUP_SLOT });
            }
        }
        let _ = app2.emit("popup-rect", serde_json::Value::Null);
        let h = HWNDS[POPUP_SLOT].swap(0, Ordering::Relaxed);
        with_slot(POPUP_SLOT, |s| *s = Default::default());
        if h != 0 {
            // Let the render thread drop the swapchain before the window goes away.
            let app3 = app2.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                let _ = app3.run_on_main_thread(move || unsafe {
                    let _ = DestroyWindow(hw(h));
                });
            });
        }
    });
}
