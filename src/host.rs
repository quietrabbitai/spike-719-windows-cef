//! The Tauri side: one main window (WebView2) + IPC commands + the mode-A pane layout.
//! Layout travels as FRACTIONS of the webview's client area (like PaneRectFraction in quietrabbit-core),
//! so a window resize re-lays-out the panes in Rust without waiting on a JS round trip.
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cef::{ImplBrowser, ImplBrowserHost};
use serde::{Deserialize, Serialize};
use tauri::{RunEvent, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use windows::core::BOOL;
use windows::core::HSTRING;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, HGDIOBJ, RGN_DIFF,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetAncestor, GetClassNameW, GetClientRect, GetWindowRect, IsWindowVisible,
    MessageBoxW, SetWindowPos, ShowWindow, GA_PARENT, HWND_TOP, MB_ICONERROR, MB_OK,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNA,
};

use crate::cefapp;
use crate::cli::{self, CefLoop, Cli, ModalPolicy, Mode, ZOrder};
use crate::spikelog;
use crate::winenv;

#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
pub struct Frac {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

pub struct Shared {
    layout: Mutex<Vec<Frac>>,
    modal: Mutex<Option<Frac>>,
    parent: AtomicIsize,
    zorder_logged: std::sync::atomic::AtomicBool,
    port: u16,
    profile: PathBuf,
}

fn hwnd(v: isize) -> HWND {
    HWND(v as *mut std::ffi::c_void)
}

pub fn fatal(msg: &str) -> ! {
    log::error!("FATAL: {msg}");
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(msg),
            &HSTRING::from("spike719"),
            MB_OK | MB_ICONERROR,
        );
    }
    std::process::exit(2)
}

static ENUM_PARENT: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn child_cb(h: HWND, lp: LPARAM) -> BOOL {
    // Direct children only; EnumChildWindows walks the z-order top to bottom.
    if GetAncestor(h, GA_PARENT) == hwnd(ENUM_PARENT.load(Ordering::Relaxed)) {
        let v = &mut *(lp.0 as *mut Vec<String>);
        let mut cls = [0u16; 64];
        let n = GetClassNameW(h, &mut cls).max(0) as usize;
        let mut r = RECT::default();
        let _ = GetWindowRect(h, &mut r);
        v.push(format!(
            "{:#x} class={} visible={} rect=({},{},{},{})",
            h.0 as isize,
            String::from_utf16_lossy(&cls[..n]),
            IsWindowVisible(h).as_bool(),
            r.left,
            r.top,
            r.right,
            r.bottom
        ));
    }
    BOOL(1)
}

/// Logs the parent's direct children in z-order (top first) to events.csv: the evidence for the stacking finding.
fn log_zorder(parent: isize, label: &str) {
    ENUM_PARENT.store(parent, Ordering::Relaxed);
    let mut v: Vec<String> = Vec::new();
    unsafe {
        let _ = EnumChildWindows(
            Some(hwnd(parent)),
            Some(child_cb),
            LPARAM(&mut v as *mut _ as isize),
        );
    }
    spikelog::event("zorder", &format!("{label}: {}", v.join(" | ")));
}

fn client_size(parent: isize) -> (i32, i32) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd(parent), &mut r);
    }
    (r.right - r.left, r.bottom - r.top)
}

fn px(f: &Frac, cw: i32, ch: i32) -> (i32, i32, i32, i32) {
    (
        (f.x * cw as f64).round() as i32,
        (f.y * ch as f64).round() as i32,
        (f.w * cw as f64).round() as i32,
        (f.h * ch as f64).round() as i32,
    )
}

/// Mode A: place each CEF child window over its slot. Runs on the main thread (IPC command / window event).
fn apply_layout(sh: &Shared) {
    let parent = sh.parent.load(Ordering::Relaxed);
    if parent == 0 {
        return;
    }
    let (cw, ch) = client_size(parent);
    let layout = sh.layout.lock().map(|l| l.clone()).unwrap_or_default();
    if !sh.zorder_logged.load(Ordering::Relaxed)
        && (0..cefapp::PANE_COUNT).all(|i| cefapp::pane_hwnd(i).is_some())
    {
        sh.zorder_logged.store(true, Ordering::Relaxed);
        log_zorder(parent, "before first layout (creation order)");
    }
    let modal = sh.modal.lock().ok().and_then(|m| *m);
    let policy = cli::get().modal_policy;
    for (i, f) in layout.iter().enumerate().take(cefapp::PANE_COUNT) {
        let Some(h) = cefapp::pane_hwnd(i) else {
            continue;
        };
        let (x, y, w, hh) = px(f, cw, ch);
        let t = Instant::now();
        unsafe {
            if cli::get().a_zorder == ZOrder::Raise {
                let _ = SetWindowPos(hwnd(h), Some(HWND_TOP), x, y, w, hh, SWP_NOACTIVATE);
            } else {
                let _ = SetWindowPos(hwnd(h), None, x, y, w, hh, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            match (policy, modal) {
                (ModalPolicy::Hide, Some(_)) => {
                    let _ = ShowWindow(hwnd(h), SW_HIDE);
                }
                (ModalPolicy::Hole, Some(m)) => {
                    let (mx, my, mw, mh) = px(&m, cw, ch);
                    let _ = ShowWindow(hwnd(h), SW_SHOWNA);
                    let full = CreateRectRgn(0, 0, w, hh);
                    let hole = CreateRectRgn(mx - x, my - y, mx - x + mw, my - y + mh);
                    CombineRgn(Some(full), Some(full), Some(hole), RGN_DIFF);
                    let _ = DeleteObject(HGDIOBJ(hole.0));
                    SetWindowRgn(hwnd(h), Some(full), true);
                }
                _ => {
                    let _ = ShowWindow(hwnd(h), SW_SHOWNA);
                    SetWindowRgn(hwnd(h), None, true);
                }
            }
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        if ms > 4.0 {
            spikelog::event("setwindowpos_slow_ms", &format!("pane={i} {ms:.1}"));
        }
    }
}

// ------------------------------------------------------------------ IPC

#[tauri::command]
fn set_layout(sh: State<'_, Arc<Shared>>, panes: Vec<Frac>) {
    if let Ok(mut l) = sh.layout.lock() {
        *l = panes;
    }
    apply_layout(&sh);
}

#[tauri::command]
fn set_modal(sh: State<'_, Arc<Shared>>, open: bool, rect: Option<Frac>) {
    if let Ok(mut m) = sh.modal.lock() {
        *m = if open { rect } else { None };
    }
    spikelog::event("modal", if open { "open" } else { "close" });
    apply_layout(&sh);
}

#[tauri::command]
fn mark(label: String) {
    spikelog::event("mark", &label);
}

#[tauri::command]
fn navigate_pane(sh: State<'_, Arc<Shared>>, pane: usize, url: String) -> Result<(), String> {
    let allowed = [
        "https://claude.ai/",
        "https://chatgpt.com/",
        "https://gemini.google.com/",
    ];
    let local = format!("http://127.0.0.1:{}/", sh.port);
    if !(allowed.contains(&url.as_str()) || url.starts_with(&local)) || pane >= cefapp::PANE_COUNT {
        return Err("url or pane not allowed".into());
    }
    spikelog::event("navigate", &format!("pane={pane} {url}"));
    if cefapp::navigate_pane(pane, &url) {
        Ok(())
    } else {
        Err("pane browser not ready".into())
    }
}

#[derive(Serialize)]
struct Info {
    mode: &'static str,
    port: u16,
    profile: String,
    sandbox: &'static str,
}

#[tauri::command]
fn info(sh: State<'_, Arc<Shared>>) -> Info {
    let c = cli::get();
    Info {
        mode: c.mode.as_str(),
        port: sh.port,
        profile: sh.profile.to_string_lossy().into_owned(),
        sandbox: if c.diag_no_sandbox {
            "DISABLED (diagnostic)"
        } else {
            "enabled"
        },
    }
}

// ------------------------------------------------------------------ run

pub fn run(cli: Cli, sandbox_info: *mut u8) {
    let sandbox_addr = sandbox_info as usize;
    let port = crate::localserver::start();
    let shared = Arc::new(Shared {
        layout: Mutex::new(vec![
            Frac {
                x: 0.06,
                y: 0.14,
                w: 0.44,
                h: 0.84,
            },
            Frac {
                x: 0.52,
                y: 0.14,
                w: 0.46,
                h: 0.84,
            },
        ]),
        modal: Mutex::new(None),
        parent: AtomicIsize::new(0),
        zorder_logged: std::sync::atomic::AtomicBool::new(false),
        port,
        profile: cli.profile.clone(),
    });
    let sh_setup = shared.clone();
    let sh_events = shared.clone();

    let app = tauri::Builder::default()
        .manage(shared.clone())
        .invoke_handler(tauri::generate_handler![
            set_layout,
            set_modal,
            mark,
            navigate_pane,
            info
        ])
        .setup(move |app| {
            let cli = cli::get();
            if !cli.mode.implemented() {
                fatal(&format!(
                    "mode {} is not built yet (stage 1 ships --mode=a only)",
                    cli.mode.as_str()
                ));
            }
            let sandbox = if cli.diag_no_sandbox {
                "DISABLED (diagnostic)"
            } else {
                "enabled"
            };
            let title = format!(
                "Spike 719 | mode={} | loop={:?} | runtime={:?} | sandbox={}",
                cli.mode.as_str(),
                cli.cef_loop,
                cli.runtime,
                sandbox
            );
            let win = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title(title)
                .inner_size(1280.0, 800.0)
                .min_inner_size(900.0, 600.0)
                .transparent(cli.mode.below())
                // WebView2 gets its own user-data folder inside the spike profile: never Edge's or Jason's.
                .data_directory(sh_setup.profile.join("webview2"))
                .build()?;
            let parent = win.hwnd()?.0 as isize;
            sh_setup.parent.store(parent, Ordering::Relaxed);

            winenv::write_env_json(&sh_setup.profile, cli, sandbox, port);

            let root_cache = sh_setup.profile.join("cef");
            // Hypothesis from the first run's log (init failed on every first start after the profile folder was
            // deleted, then succeeded on the next start): create the cache dirs up front.
            let _ = std::fs::create_dir_all(&root_cache);
            let _ = std::fs::create_dir_all(sh_setup.profile.join("webview2"));
            if !cefapp::initialize(cli, &root_cache, sandbox_addr as *mut u8) {
                let code = cef::get_exit_code();
                if code == 24 || code == 38 {
                    // Chromium relaunched/handed over to another copy (e.g. de-elevated); that copy is the real one.
                    log::info!("cef::initialize handed over (exit code {code}); this process exits");
                    std::process::exit(0);
                }
                let meaning = match code {
                    21 => "PROFILE_IN_USE: another process holds the profile folder",
                    24 => "NORMAL_EXIT_PROCESS_NOTIFIED: handed over to an already-running instance",
                    _ => "see cef_resultcode_t",
                };
                let others = winenv::other_instances();
                fatal(&format!(
                    "cef::initialize failed: exit code {code} ({meaning}). Other spike719.exe processes (pid:parent): {others}"
                ));
            }
            if cli.cef_loop == CefLoop::Pump {
                // Pump CEF from the Tauri main thread (same thread as the parent window).
                let handle = app.handle().clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(8));
                    let _ = handle.run_on_main_thread(cef::do_message_loop_work);
                });
            }

            // Create both panes over their initial slots.
            let (cw, ch) = client_size(parent);
            let layout = sh_setup
                .layout
                .lock()
                .map(|l| l.clone())
                .unwrap_or_default();
            for (i, f) in layout.iter().enumerate().take(cefapp::PANE_COUNT) {
                let url = format!("http://127.0.0.1:{port}/?pane={i}");
                let ok =
                    cefapp::create_windowed_pane(cli, i, &url, parent, px(f, cw, ch), &root_cache);
                if !ok {
                    fatal(&format!("CEF refused to create pane {i}"));
                }
            }
            // Pane HWNDs appear asynchronously: re-apply the layout until both exist, then log the stacking.
            {
                let handle = app.handle().clone();
                let sh = sh_setup.clone();
                std::thread::spawn(move || {
                    for _ in 0..40 {
                        std::thread::sleep(Duration::from_millis(250));
                        let sh2 = sh.clone();
                        let _ = handle.run_on_main_thread(move || apply_layout(&sh2));
                    }
                    let sh3 = sh.clone();
                    let _ = handle.run_on_main_thread(move || {
                        log_zorder(sh3.parent.load(Ordering::Relaxed), "after layout");
                    });
                });
            }
            winenv::start_sampler(sh_setup.profile.clone());
            Ok(())
        })
        .on_window_event(move |window, event| match event {
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                apply_layout(&sh_events);
            }
            WindowEvent::Moved(_) => {
                // CEF asks hosts to tell it when the window moves so popups/menus can dismiss.
                for i in 0..cefapp::PANE_COUNT {
                    if let Some(h) = cefapp::pane_browser(i).and_then(|b| b.host()) {
                        h.notify_move_or_resize_started();
                    }
                }
                let _ = window;
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|e| fatal(&format!("tauri build failed: {e}")));

    app.run(|_app, event| {
        if let RunEvent::Exit = event {
            cefapp::close_all_panes();
            std::thread::sleep(Duration::from_millis(300));
            cef::shutdown();
        }
    });
}

#[allow(dead_code)]
fn _unused(_: Mode) {}
