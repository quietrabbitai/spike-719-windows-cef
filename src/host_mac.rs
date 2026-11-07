//! The Tauri side on macOS (items.id=720 spike, THROWAWAY): one main window with a TRANSPARENT WKWebView, the
//! IPC commands, and the pane layout. Same layout contract as host.rs (fractions of the content view), but the
//! panes are NSViews that sit BELOW the WKWebView in every mode.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cef::{ImplBrowser, ImplBrowserHost};
use serde::{Deserialize, Serialize};
use tauri::{RunEvent, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::bgpu::Gpu;
use crate::cefapp;
use crate::cli::{self, Cli, Mode, Screen};
use crate::{bmac, macapp, macenv, spikelog};

#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
pub struct Frac {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

pub struct Shared {
    layout: Mutex<Vec<Frac>>,
    port: u16,
    profile: PathBuf,
}

pub fn fatal(msg: &str) -> ! {
    log::error!("FATAL: {msg}");
    eprintln!("spike720 FATAL: {msg}");
    std::process::exit(2)
}

/// Re-lays-out the pane views from the stored fractions. Main thread.
fn apply_layout(sh: &Shared) {
    if bmac::state().is_none() {
        return;
    }
    let (cw, ch) = bmac::content_size();
    if cw <= 0.0 || ch <= 0.0 {
        return;
    }
    let layout = sh.layout.lock().map(|l| l.clone()).unwrap_or_default();
    let windowless = cli::get().mode.windowless();
    let mut rects = Vec::new();
    for (i, f) in layout.iter().enumerate().take(cefapp::PANE_COUNT) {
        let (x, y, w, h) = (f.x * cw, f.y * ch, f.w * cw, f.h * ch);
        bmac::place(i, x, y, w, h, windowless);
        rects.push((x, y, w, h));
    }
    if !windowless {
        bmac::set_hit_rects(&rects);
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
fn set_modal(sh: State<'_, Arc<Shared>>, open: bool, _rect: Option<Frac>) {
    spikelog::event("modal", if open { "open" } else { "close" });
    bmac::set_modal(open);
    if open {
        bmac::dump_hierarchy("modal open");
    }
    apply_layout(&sh);
}

#[tauri::command]
fn mark(label: String) {
    spikelog::event("mark", &label);
}

#[tauri::command]
fn navigate_pane(sh: State<'_, Arc<Shared>>, pane: usize, url: String) -> Result<(), String> {
    let allowed = ["https://claude.ai/", "https://chatgpt.com/", "https://gemini.google.com/"];
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

#[derive(Deserialize, Default, Clone, Copy)]
struct DomMods {
    shift: bool,
    ctrl: bool,
    alt: bool,
    #[serde(default)]
    meta: bool,
    buttons: u16,
}

fn cef_mods(m: &DomMods) -> u32 {
    use cef::sys::cef_event_flags_t as F;
    let mut f = 0u32;
    if m.shift {
        f |= F::EVENTFLAG_SHIFT_DOWN.0 as u32;
    }
    if m.ctrl {
        f |= F::EVENTFLAG_CONTROL_DOWN.0 as u32;
    }
    if m.alt {
        f |= F::EVENTFLAG_ALT_DOWN.0 as u32;
    }
    if m.meta {
        f |= F::EVENTFLAG_COMMAND_DOWN.0 as u32;
    }
    if m.buttons & 0b001 != 0 {
        f |= F::EVENTFLAG_LEFT_MOUSE_BUTTON.0 as u32;
    }
    if m.buttons & 0b010 != 0 {
        f |= F::EVENTFLAG_RIGHT_MOUSE_BUTTON.0 as u32;
    }
    if m.buttons & 0b100 != 0 {
        f |= F::EVENTFLAG_MIDDLE_MOUSE_BUTTON.0 as u32;
    }
    f
}

#[tauri::command]
fn fwd_mouse_move(slot: usize, x: f64, y: f64, leave: bool, mods: DomMods) {
    crate::bcef::mouse_move(slot, x as i32, y as i32, leave, cef_mods(&mods));
}

static CLICK_LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[tauri::command]
fn fwd_mouse_click(slot: usize, x: f64, y: f64, button: u8, up: bool, count: i32, mods: DomMods) {
    static ENTER_LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if ENTER_LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 10 {
        spikelog::event("fwd_click_enter", &format!("slot={slot} up={up} button={button} thread={:?}", std::thread::current().name()));
    }
    let b = match button {
        0 => cef::MouseButtonType::LEFT,
        1 => cef::MouseButtonType::MIDDLE,
        2 => cef::MouseButtonType::RIGHT,
        _ => return,
    };
    // Instrumentation (Retina pass): DOM css px == CEF view DIP; the pixel position is DIP * backing scale.
    if !up && CLICK_LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 60 {
        let sc = bmac::scale();
        spikelog::event(
            "click_coords",
            &format!(
                "slot={slot} dip=({x:.1},{y:.1}) px=({:.0},{:.0}) scale={sc}",
                x * sc as f64,
                y * sc as f64
            ),
        );
    }
    crate::bcef::mouse_click(slot, x as i32, y as i32, b, up, count.max(1), cef_mods(&mods));
}

#[tauri::command]
fn fwd_wheel(slot: usize, x: f64, y: f64, dx: f64, dy: f64, mods: DomMods) {
    // Instrumentation (scroll smoothness): wheel events received per second per slot, next to frames.csv paints/s.
    {
        static WHEEL: std::sync::Mutex<(Option<std::time::Instant>, [u32; 3])> = std::sync::Mutex::new((None, [0; 3]));
        if let Ok(mut w) = WHEEL.lock() {
            let now = std::time::Instant::now();
            let t0 = *w.0.get_or_insert(now);
            if slot < 3 {
                w.1[slot] += 1;
            }
            if now.duration_since(t0).as_millis() >= 1000 {
                spikelog::event("wheel_per_s", &format!("slot0={} slot1={} slot2={} last_dy={dy:.1}", w.1[0], w.1[1], w.1[2]));
                *w = (Some(now), [0; 3]);
            }
        }
    }
    // DOM deltas are positive for "scroll down/right"; CEF's are positive for "scroll up/left".
    crate::bcef::mouse_wheel(slot, x as i32, y as i32, (-dx) as i32, (-dy) as i32, cef_mods(&mods));
}

/// kVK_* virtual key codes by DOM KeyboardEvent.code (ANSI layout). CEF's mac native_key_code is NSEvent.keyCode.
fn kvk(code: &str) -> i32 {
    match code {
        "KeyA" => 0x00, "KeyS" => 0x01, "KeyD" => 0x02, "KeyF" => 0x03, "KeyH" => 0x04, "KeyG" => 0x05,
        "KeyZ" => 0x06, "KeyX" => 0x07, "KeyC" => 0x08, "KeyV" => 0x09, "KeyB" => 0x0B, "KeyQ" => 0x0C,
        "KeyW" => 0x0D, "KeyE" => 0x0E, "KeyR" => 0x0F, "KeyY" => 0x10, "KeyT" => 0x11,
        "Digit1" => 0x12, "Digit2" => 0x13, "Digit3" => 0x14, "Digit4" => 0x15, "Digit6" => 0x16,
        "Digit5" => 0x17, "Equal" => 0x18, "Digit9" => 0x19, "Digit7" => 0x1A, "Minus" => 0x1B,
        "Digit8" => 0x1C, "Digit0" => 0x1D, "BracketRight" => 0x1E, "KeyO" => 0x1F, "KeyU" => 0x20,
        "BracketLeft" => 0x21, "KeyI" => 0x22, "KeyP" => 0x23, "Enter" => 0x24, "KeyL" => 0x25,
        "KeyJ" => 0x26, "Quote" => 0x27, "KeyK" => 0x28, "Semicolon" => 0x29, "Backslash" => 0x2A,
        "Comma" => 0x2B, "Slash" => 0x2C, "KeyN" => 0x2D, "KeyM" => 0x2E, "Period" => 0x2F,
        "Tab" => 0x30, "Space" => 0x31, "Backquote" => 0x32, "Backspace" => 0x33, "Escape" => 0x35,
        "Delete" => 0x75, "Home" => 0x73, "End" => 0x77, "PageUp" => 0x74, "PageDown" => 0x79,
        "ArrowLeft" => 0x7B, "ArrowRight" => 0x7C, "ArrowDown" => 0x7D, "ArrowUp" => 0x7E,
        "ShiftLeft" => 0x38, "ShiftRight" => 0x3C, "ControlLeft" => 0x3B, "ControlRight" => 0x3E, "AltLeft" => 0x3A,
        "AltRight" => 0x3D, "MetaLeft" => 0x37, "MetaRight" => 0x36, "CapsLock" => 0x39,
        _ => 0,
    }
}

/// The `character` CEF needs on macOS for key down/up (NSEvent.characters). Found by experiment (items.id=720): with
/// character 0, CEF on macOS reports event.key "Unidentified" for letters AND turns every KEYUP into an extra keydown (the
/// page never sees a keyup). With the character on both the down and the up event the page gets one keydown (right key)
/// and one keyup, as native input does. Function keys use NSEvent's private-use codes.
fn key_character(key: &str) -> u16 {
    let mut it = key.chars();
    if let (Some(c), None) = (it.next(), it.next()) {
        let mut b = [0u16; 2];
        return c.encode_utf16(&mut b)[0];
    }
    match key {
        "Enter" => 13,
        "Tab" => 9,
        "Escape" => 27,
        "Backspace" => 0x7F,
        "ArrowUp" => 0xF700,
        "ArrowDown" => 0xF701,
        "ArrowLeft" => 0xF702,
        "ArrowRight" => 0xF703,
        "Delete" => 0xF728,
        "Home" => 0xF729,
        "End" => 0xF72B,
        "PageUp" => 0xF72C,
        "PageDown" => 0xF72D,
        _ => 0, // modifier keys and anything unmapped
    }
}

#[tauri::command]
fn fwd_key(slot: usize, kind: String, vk: i32, ch: u16, code: Option<String>, key: Option<String>, mods: DomMods) {
    // Cmd+C/X/V/A: a windowless browser gets no OS accelerators; Copy goes through the selection cache
    // (OnTextSelectionChanged), like Linux's items.id=369 path.
    if kind == "down" && mods.meta && !mods.alt {
        let cmd = match vk as u8 {
            b'C' => Some("copy"),
            b'X' => Some("cut"),
            b'V' => Some("paste"),
            b'A' => Some("select_all"),
            _ => None,
        };
        if let Some(c) = cmd {
            crate::bcef::edit_command(slot, c);
            return;
        }
    }
    let k = match kind.as_str() {
        "down" => cef::KeyEventType::RAWKEYDOWN,
        "up" => cef::KeyEventType::KEYUP,
        "char" => cef::KeyEventType::CHAR,
        "keydown" => cef::KeyEventType::KEYDOWN, // experiment only
        _ => return,
    };
    let native = kvk(code.as_deref().unwrap_or(""));
    // down/up carry the key's character (see key_character); a char event keeps the character the page sent.
    let ch = if kind == "char" { ch } else { key.as_deref().map(key_character).unwrap_or(ch) };
    crate::bcef::key(slot, k, vk, native, ch, cef_mods(&mods), false);
}

#[tauri::command]
fn fwd_focus(slot: usize, on: bool) {
    crate::bcef::set_focus(slot, on);
}

#[tauri::command]
fn fwd_ime(slot: usize, kind: String, text: String) {
    match kind.as_str() {
        "update" => crate::bcef::ime_update(slot, &text),
        "commit" => crate::bcef::ime_commit(slot, &text),
        _ => {}
    }
}

#[tauri::command]
fn close_popup() {
    if let Some(h) = cefapp::pane_browser(cefapp::POPUP_SLOT).and_then(|b| b.host()) {
        h.close_browser(true as _);
    }
}

#[derive(Serialize)]
struct Info {
    mode: &'static str,
    windowless: bool,
    below: bool,
    selftest: bool,
    port: u16,
    profile: String,
    sandbox: &'static str,
}

#[tauri::command]
fn info(sh: State<'_, Arc<Shared>>) -> Info {
    let c = cli::get();
    Info {
        mode: c.mode.as_str(),
        windowless: c.mode.windowless(),
        // On macOS every mode puts the panes below the transparent WKWebView, so the page always cuts holes.
        below: true,
        selftest: std::env::var_os("SPIKE720_SELFTEST").is_some(),
        port: sh.port,
        profile: sh.profile.to_string_lossy().into_owned(),
        sandbox: if c.diag_no_sandbox { "DISABLED (diagnostic)" } else { "enabled" },
    }
}

// ------------------------------------------------------------------ run

pub fn run(cli: Cli) {
    let port = crate::localserver::start();
    let shared = Arc::new(Shared {
        layout: Mutex::new(vec![
            Frac { x: 0.06, y: 0.14, w: 0.44, h: 0.84 },
            Frac { x: 0.52, y: 0.14, w: 0.46, h: 0.84 },
        ]),
        port,
        profile: cli.profile.clone(),
    });
    let sh_setup = shared.clone();
    let sh_events = shared.clone();

    let app = tauri::Builder::default()
        .manage(shared.clone())
        .invoke_handler(tauri::generate_handler![
            set_layout, set_modal, mark, navigate_pane, info, fwd_mouse_move, fwd_mouse_click, fwd_wheel,
            fwd_key, fwd_focus, fwd_ime, close_popup
        ])
        .setup(move |app| {
            let cli = cli::get();
            let sandbox = if cli.diag_no_sandbox { "DISABLED (diagnostic)" } else { "enabled" };
            let title = format!(
                "Spike 720 | mode={} | runtime={:?} | sandbox={}",
                cli.mode.as_str(),
                cli.runtime,
                sandbox
            );
            let mut wb = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title(title)
                .inner_size(cli.size.0, cli.size.1)
                .min_inner_size(640.0, 480.0)
                // Needs tauri/macos-private-api (COMPROMISE). Only the WKWebView needs to be transparent; the
                // panes are siblings underneath it.
                .transparent(true)
                // No persistent WKWebView data under ~/Library: the page only loads the bundled UI.
                .incognito(true);
            if cli.screen == Screen::Virtual2x {
                let (ox, oy) = macenv::first_2x_screen_origin()
                    .ok_or("--screen=virtual2x: no 2x display is attached (run tools/out/vdisplay first)")?;
                spikelog::event("screen_target", &format!("2x screen origin (top-left, pt) = ({ox},{oy})"));
                wb = wb.position(ox + 60.0, oy + 60.0);
            }
            let win = wb.build()?;
            let ns_window = win.ns_window()?;
            let ns_view = win.ns_view()?;

            let windowless = cli.mode.windowless();
            let gpu_pack = if windowless {
                let gpu = match Gpu::new() {
                    Ok(g) => Arc::new(g),
                    Err(e) => fatal(&format!("wgpu init failed: {e}")),
                };
                let _ = crate::bshared::DEVICE.set(gpu.device.clone());
                macenv::merge_env(&sh_setup.profile, gpu.info_json());
                let (tx, rx) = std::sync::mpsc::channel();
                Some((gpu, tx, rx))
            } else {
                None
            };
            if let Err(e) = bmac::init(
                ns_window,
                ns_view,
                gpu_pack.as_ref().map(|(g, t, _)| (g.clone(), t.clone())),
            ) {
                fatal(&format!("bmac init: {e}"));
            }
            if cli.screen == Screen::Virtual2x {
                let moved = bmac::move_window_to_2x_screen();
                spikelog::event("screen_target", &format!("moved window to a 2x screen: {moved}"));
                // The window's screen (and so its backing scale) can settle a moment after creation.
                let mut sc = bmac::scale();
                for _ in 0..20 {
                    if sc >= 1.99 {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                    sc = bmac::scale();
                }
                spikelog::event("screen_target", &format!("window backing scale after placement = {sc}; {}", bmac::window_where()));
                if sc < 1.99 {
                    fatal("--screen=virtual2x: the window did not land on a 2x display");
                }
            }
            macenv::write_env_json(&sh_setup.profile, cli, sandbox, port);
            bmac::dump_hierarchy("after window build");
            // Control experiment (mode a, diagnostics only): hide the WKWebView so CEF's views get events natively.
            if !windowless && std::env::var_os("SPIKE720_A_HIDE_WK").is_some() {
                bmac::hide_webview();
                spikelog::event("control", "WKWebView hidden (SPIKE720_A_HIDE_WK)");
            }
            if !windowless {
                if let Err(e) = bmac::install_hit_passthrough() {
                    fatal(&format!("hit passthrough: {e}"));
                }
            }
            let root_cache = sh_setup.profile.join("cef");
            let _ = std::fs::create_dir_all(&root_cache);

            let layout = sh_setup.layout.lock().map(|l| l.clone()).unwrap_or_default();
            let (cw, ch) = bmac::content_size();
            if let Some((gpu, _tx, rx)) = gpu_pack {
                let _ = crate::bcef::APP.set(app.handle().clone());
                for (i, f) in layout.iter().enumerate().take(cefapp::PANE_COUNT) {
                    if let Err(e) = bmac::create_pane_view(i, f.x * cw, f.y * ch, f.w * cw, f.h * ch) {
                        fatal(&format!("pane view {i}: {e}"));
                    }
                }
                crate::bgpu::spawn(gpu, rx, sh_setup.profile.clone());
            }
            bmac::dump_hierarchy("after pane views");

            if !cefapp::initialize(cli, &root_cache, std::ptr::null_mut()) {
                fatal(&format!("cef::initialize failed: exit code {}", cef::get_exit_code()));
            }
            macapp::start_pump();
            log::info!("{}", macapp::describe_nsapp());

            for (i, f) in layout.iter().enumerate().take(cefapp::PANE_COUNT) {
                // SPIKE720_PANE<i>_URL: start pane i on a provider page (needed for mode a with the WKWebView hidden, where the
                // toolbar dropdown is not visible). Same allowlist as navigate_pane; anything else is refused (fail closed).
                let url = match std::env::var(format!("SPIKE720_PANE{i}_URL")) {
                    Ok(u) if ["https://claude.ai/", "https://chatgpt.com/", "https://gemini.google.com/"].contains(&u.as_str()) => {
                        spikelog::event("start_url", &format!("pane={i} {u}"));
                        u
                    }
                    Ok(u) => fatal(&format!("SPIKE720_PANE{i}_URL={u} is not an allowed address")),
                    Err(_) => format!("http://127.0.0.1:{port}/?pane={i}"),
                };
                let ok = if windowless {
                    crate::bcef::create_windowless_pane(cli, i, &url, &root_cache)
                } else {
                    let (x, y, w, h) = ((f.x * cw) as i32, (f.y * ch) as i32, (f.w * cw) as i32, (f.h * ch) as i32);
                    cefapp::create_windowed_pane(cli, i, &url, ns_view as isize, (x, y, w, h), &root_cache)
                };
                if !ok {
                    fatal(&format!("CEF refused to create pane {i}"));
                }
            }
            // Self-test of the typing path (b-* modes): click the first input of pane 0 and type through the very
            // same functions the IPC commands call, then have the page report the value to the loopback server.
            if windowless && std::env::var_os("SPIKE720_SELFTEST").is_some() {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(6));
                    let _ = handle.run_on_main_thread(|| {
                        let m = DomMods::default();
                        fwd_focus(0, true);
                        fwd_mouse_click(0, 60.0, 42.0, 0, false, 1, m);
                        fwd_mouse_click(0, 60.0, 42.0, 0, true, 1, m);
                        for (ch, code) in [('h', "KeyH"), ('e', "KeyE"), ('l', "KeyL"), ('l', "KeyL"), ('o', "KeyO"), ('1', "Digit1")] {
                            let vk = ch.to_ascii_uppercase() as i32;
                            fwd_key(0, "down".into(), vk, 0, Some(code.into()), Some(ch.to_string()), m);
                            fwd_key(0, "char".into(), ch as i32, ch as u16, Some(code.into()), None, m);
                            fwd_key(0, "up".into(), vk, 0, Some(code.into()), Some(ch.to_string()), m);
                        }
                    });
                    std::thread::sleep(Duration::from_millis(800));
                    if let Some(f) = cefapp::pane_browser(0).and_then(|b| b.main_frame()) {
                        use cef::ImplFrame;
                        let js = "fetch('/selftest?first=' + encodeURIComponent(document.getElementById('first').value) + '&active=' + (document.activeElement && document.activeElement.id))";
                        f.execute_java_script(Some(&cef::CefString::from(js)), None, 0);
                    }
                });
            }
            // Key-mapping experiment (b-* modes, SPIKE720_KEYTEST): for several keys, send different down/char/up combinations
            // through fwd_key and mark each variant in events.csv; the test page logs every keydown it receives.
            if windowless && std::env::var_os("SPIKE720_KEYTEST").is_some() {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(6));
                    let m = DomMods::default();
                    let send = |h: &tauri::AppHandle, f: Box<dyn FnOnce() + Send>| { let _ = h.run_on_main_thread(f); };
                    send(&handle, Box::new(move || { fwd_focus(0, true); fwd_mouse_click(0, 60.0, 42.0, 0, false, 1, m); fwd_mouse_click(0, 60.0, 42.0, 0, true, 1, m); }));
                    std::thread::sleep(Duration::from_millis(500));
                    // (name, kind sequence, put the character on the down event?)
                    let variants: [(&str, &[&str], bool); 3] = [
                        ("down(ch)+up(ch)", &["down", "up"], true),
                        ("up(ch)-only", &["up"], true),
                        ("down(ch)+up(ch)+up(ch)", &["down", "up", "up"], true),
                    ];
                    let _unused: [(&str, &[&str], bool); 6] = [
                        ("down", &["down"], false),
                        ("down+up", &["down", "up"], false),
                        ("down+char+up", &["down", "char", "up"], false),
                        ("char", &["char"], false),
                        ("down(ch)+up", &["down", "up"], true),
                        ("down(ch)+char+up", &["down", "char", "up"], true),
                    ];
                    // (label, vk, ch, DOM code)
                    let keys: [(&str, i32, u16, &str); 3] = [("a", 65, 'a' as u16, "KeyA"), ("Enter", 13, 13, "Enter"), ("ArrowLeft", 37, 0xF702, "ArrowLeft")];
                    for (vname, kinds, ch_on_down) in variants {
                        for (label, vk, ch, code) in keys {
                            spikelog::event("keytest", &format!("variant={vname} key={label}"));
                            for kind in kinds {
                                let (kind, ch_arg) = (kind.to_string(), if *kind == "down" && !ch_on_down { 0u16 } else { ch });
                                send(&handle, Box::new(move || fwd_key(0, kind, vk, ch_arg, Some(code.into()), None, DomMods::default())));
                                std::thread::sleep(Duration::from_millis(40));
                            }
                            std::thread::sleep(Duration::from_millis(250));
                        }
                    }
                    spikelog::event("keytest", "done");
                });
            }
            // Self-test of scroll smoothness (b-* modes, SPIKE720_SELFTEST): a controlled wheel stream at 120 Hz into
            // pane 0 through fwd_wheel (the IPC path), alternating 50 events down / 50 up so content keeps moving. The
            // window is marked in events.csv; read paints/s and gaps from frames.csv for slot 0 in that window.
            if windowless && std::env::var_os("SPIKE720_SELFTEST").is_some() {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(10));
                    let hz: u64 = std::env::var("SPIKE720_SCROLL_HZ").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
                    let dy_mag: f64 = std::env::var("SPIKE720_SCROLL_DY").ok().and_then(|v| v.parse().ok()).unwrap_or(30.0);
                    spikelog::event("selftest_scroll", &format!("start {hz} Hz x 3 s, dy=+-{dy_mag}"));
                    let t0 = std::time::Instant::now();
                    let mut i = 0u32;
                    while t0.elapsed() < Duration::from_secs(3) {
                        let flip: u64 = std::env::var("SPIKE720_SCROLL_FLIP_EVENTS").ok().and_then(|v| v.parse().ok()).unwrap_or((hz / 2).max(1));
                        let dir = if flip == 0 || (i as u64 / flip) % 2 == 0 { dy_mag } else { -dy_mag };
                        let _ = handle.run_on_main_thread(move || fwd_wheel(0, 200.0, 400.0, 0.0, dir, DomMods::default()));
                        i += 1;
                        std::thread::sleep(Duration::from_micros(1_000_000 / hz));
                    }
                    spikelog::event("selftest_scroll", &format!("end events_sent={i}"));
                });
            }
            // Diagnostics/self-test: exit through Tauri's own exit path (what Cmd+Q does) without any signal.
            if let Some(secs) = std::env::var("SPIKE720_EXIT_AFTER").ok().and_then(|v| v.parse::<u64>().ok()) {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(secs));
                    log::info!("SPIKE720_EXIT_AFTER: requesting exit");
                    handle.exit(0);
                });
            }
            // Pane views appear asynchronously (mode a): re-apply the layout until both exist, then dump the stack.
            {
                let handle = app.handle().clone();
                let sh = sh_setup.clone();
                std::thread::spawn(move || {
                    for _ in 0..40 {
                        std::thread::sleep(Duration::from_millis(250));
                        let sh2 = sh.clone();
                        let _ = handle.run_on_main_thread(move || apply_layout(&sh2));
                    }
                    let _ = handle.run_on_main_thread(|| bmac::dump_hierarchy("after layout (10 s)"));
                });
            }
            Ok(())
        })
        .on_window_event(move |window, event| match event {
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if let WindowEvent::ScaleFactorChanged { scale_factor, .. } = event {
                    spikelog::event("scale_factor_changed", &format!("{scale_factor}"));
                }
                apply_layout(&sh_events);
            }
            WindowEvent::Moved(_) => {
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
            // First attempt (close, drain 500 ms, shutdown) crashed inside cef::shutdown on every exit
            // (SIGTRAP in the framework): the render thread kept calling into CEF and the browsers were not
            // confirmed closed. Order now: stop the render surfaces, close browsers, pump until on_before_close
            // has run for all of them, stop the pump, then shut CEF down.
            if let Some(st) = bmac::state() {
                if let Some(tx) = &st.tx {
                    for slot in 0..cefapp::SLOT_COUNT {
                        let _ = tx.lock().unwrap().send(crate::bgpu::Cmd::Remove { slot });
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(100));
            cefapp::close_all_panes();
            let closed = macapp::drain_until(|| (0..cefapp::SLOT_COUNT).all(|i| cefapp::pane_browser(i).is_none()), 3000);
            log::info!("shutdown: all browsers closed={closed}");
            macapp::drain(200);
            macapp::stop_pump();
            cef::shutdown();
            log::info!("shutdown: cef::shutdown returned");
        }
    });
}

#[allow(dead_code)]
fn _unused(_: Mode) {}
