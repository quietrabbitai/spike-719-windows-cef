//! macOS pane host for the items.id=720 spike (THROWAWAY). The macOS counterpart of bwin.rs:
//!  * mode `a`: CEF creates the pane NSViews (set_as_child); we only place them and keep them BELOW the WKWebView.
//!  * mode `b-*-below`: we create one plain NSView per pane, wgpu gives it a CAMetalLayer, CEF paints into it.
//!  * every pane view sits below the (transparent) WKWebView in the window's content view.
//! Everything here runs on the main thread (sync Tauri commands and window events run there).
use std::ffi::{c_char, c_void};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use cef::{ImplBrowser, ImplBrowserHost, ImplRunContextMenuCallback, MenuId};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, sel, MainThreadMarker};
use objc2_app_kit::{
    NSMenu, NSMenuItem, NSPasteboard, NSPasteboardTypeString, NSView, NSWindow, NSWindowOrderingMode,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use tauri::Emitter;

use crate::bcef;
use crate::bgpu::{Cmd, Gpu};
use crate::bshared::with_slot;
use crate::cefapp::{self, POPUP_SLOT, SLOT_COUNT};
use crate::spikelog;

#[link(name = "objc")]
extern "C" {
    fn object_getClass(obj: *mut c_void) -> *mut c_void;
    fn class_getName(cls: *mut c_void) -> *const c_char;
    fn class_getSuperclass(cls: *mut c_void) -> *mut c_void;
    fn objc_allocateClassPair(sup: *mut c_void, name: *const c_char, extra: usize) -> *mut c_void;
    fn objc_registerClassPair(cls: *mut c_void);
    fn class_addMethod(cls: *mut c_void, name: *const c_void, imp: *const c_void, types: *const c_char) -> u8;
    fn class_getMethodImplementation(cls: *mut c_void, name: *const c_void) -> *const c_void;
    fn sel_registerName(name: *const c_char) -> *const c_void;
}

pub struct BState {
    /// NSWindow*, the content NSView* and the WKWebView* (all owned by AppKit, valid for the process lifetime).
    pub window: usize,
    pub content: usize,
    pub wk: usize,
    /// Windowless modes only.
    pub gpu: Option<Arc<Gpu>>,
    pub tx: Option<Mutex<Sender<Cmd>>>,
}

static STATE: OnceLock<BState> = OnceLock::new();

struct SendView(Retained<NSView>);
unsafe impl Send for SendView {}
unsafe impl Sync for SendView {}
static VIEWS: Mutex<[Option<SendView>; SLOT_COUNT]> = Mutex::new([const { None }; SLOT_COUNT]);

pub fn state() -> Option<&'static BState> {
    STATE.get()
}

fn mtm() -> MainThreadMarker {
    // Sync Tauri commands, window events and run_on_main_thread closures all run on the main thread.
    unsafe { MainThreadMarker::new_unchecked() }
}

fn content() -> Retained<NSView> {
    let st = state().expect("bmac not initialised");
    unsafe { Retained::retain(st.content as *mut NSView) }.expect("content view")
}

fn window() -> Retained<NSWindow> {
    let st = state().expect("bmac not initialised");
    unsafe { Retained::retain(st.window as *mut NSWindow) }.expect("window")
}

fn class_name(obj: *mut AnyObject) -> String {
    if obj.is_null() {
        return "nil".into();
    }
    unsafe { (*obj).class().name().to_string_lossy().into_owned() }
}

/// Finds the WKWebView (wry's subclass) among the content view's subviews.
fn find_webview(content: &NSView) -> Option<usize> {
    let subs = content.subviews();
    for v in subs.iter() {
        let n = class_name(&*v as *const NSView as *mut AnyObject);
        if n.contains("WebView") {
            return Some(&*v as *const NSView as usize);
        }
    }
    None
}

pub fn init(
    window_ptr: *mut c_void,
    content_ptr: *mut c_void,
    gpu: Option<(Arc<Gpu>, Sender<Cmd>)>,
) -> Result<(), String> {
    let content_v = unsafe { Retained::retain(content_ptr as *mut NSView) }.ok_or("null content view")?;
    let wk = find_webview(&content_v).ok_or("WKWebView not found in the content view")?;
    let (gpu, tx) = match gpu {
        Some((g, t)) => (Some(g), Some(Mutex::new(t))),
        None => (None, None),
    };
    let _ = STATE.set(BState {
        window: window_ptr as usize,
        content: content_ptr as usize,
        wk,
        gpu,
        tx,
    });
    Ok(())
}

/// The area the WKWebView's page actually lays out in: the window's contentLayoutRect (the content view minus the
/// title-bar band). Found with Jason's first click test: the content view is 800 pt tall but the page's innerHeight
/// is 768, because Tauri's window lets the content view run under the title bar. Pane geometry from the page's
/// fractions MUST be applied to this rect, not to the full content view (the first version used the full view and
/// every pane click landed ~30 pt above its target).
fn layout_rect() -> NSRect {
    window().contentLayoutRect()
}

/// Size (points) of the page's layout area.
pub fn content_size() -> (f64, f64) {
    let r = layout_rect();
    (r.size.width, r.size.height)
}

/// Frame in content-view coordinates (origin bottom-left) from a top-left based point rect in the page's space.
fn flip(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    let r = layout_rect();
    NSRect::new(NSPoint::new(r.origin.x + x, r.origin.y + r.size.height - y - h), NSSize::new(w, h))
}

/// Window frame (Cocoa coords, points) and the screen AppKit says it is on: instrumentation.
pub fn window_where() -> String {
    let w = window();
    let f = w.frame();
    let scr = w.screen().map(|s| format!("{} scale={}", s.localizedName(), s.backingScaleFactor())).unwrap_or("none".into());
    format!("window frame=({:.0},{:.0},{:.0},{:.0}) screen={scr}", f.origin.x, f.origin.y, f.size.width, f.size.height)
}

/// `--screen=virtual2x`: moves the NSWindow straight onto the first 2x screen (Tauri's set_position is queued to the
/// event loop, which setup() is blocking). Returns false when there is no 2x screen.
pub fn move_window_to_2x_screen() -> bool {
    for s in objc2_app_kit::NSScreen::screens(mtm()).iter() {
        if s.backingScaleFactor() >= 1.99 {
            let f = s.frame();
            window().setFrameTopLeftPoint(NSPoint::new(f.origin.x + 60.0, f.origin.y + f.size.height - 60.0));
            return true;
        }
    }
    false
}

/// Diagnostics only (control experiment).
pub fn hide_webview() {
    wk_view().setHidden(true);
}

pub fn scale() -> f32 {
    window().backingScaleFactor() as f32
}

fn wk_view() -> Retained<NSView> {
    let st = state().expect("bmac not initialised");
    unsafe { Retained::retain(st.wk as *mut NSView) }.expect("wk")
}

/// Puts `view` directly below the WKWebView (above every pane view that was inserted earlier).
fn below_webview(view: &NSView) {
    let c = content();
    let wk = wk_view();
    c.addSubview_positioned_relativeTo(view, NSWindowOrderingMode::Below, Some(&wk));
}

// ------------------------------------------------------------------ windowless panes (b-*-below)

/// Creates the NSView + wgpu surface for a slot. Main thread. `x,y,w,h` are points, top-left origin.
pub fn create_pane_view(slot: usize, x: f64, y: f64, w: f64, h: f64) -> Result<(), String> {
    let st = state().ok_or("bmac not initialised")?;
    let gpu = st.gpu.as_ref().ok_or("no gpu in this mode")?;
    let frame = flip(x, y, w.max(1.0), h.max(1.0));
    let view = NSView::initWithFrame(mtm().alloc::<NSView>(), frame);
    below_webview(&view);
    let surface = crate::bgpu::create_surface_for_view(gpu, Retained::as_ptr(&view) as *mut c_void)?;
    if let Some(tx) = &st.tx {
        let _ = tx.lock().unwrap().send(Cmd::AddSurface { slot, surface });
    }
    let sc = scale();
    with_slot(slot, |s| {
        s.pix = (((w * sc as f64).round().max(1.0)) as u32, ((h * sc as f64).round().max(1.0)) as u32);
        s.scale = sc;
        s.active = true;
    });
    if let Ok(mut v) = VIEWS.lock() {
        v[slot] = Some(SendView(view));
    }
    spikelog::event("pane_view", &format!("slot={slot} pts=({x:.1},{y:.1},{w:.1},{h:.1}) scale={sc}"));
    Ok(())
}

fn pane_view(slot: usize) -> Option<Retained<NSView>> {
    if let Some(v) = VIEWS.lock().ok().and_then(|v| v[slot].as_ref().map(|s| s.0.clone())) {
        return Some(v);
    }
    // Mode a only: the NSView CEF created. (A windowless browser's window_handle is the parent view: never use it.)
    if state()?.gpu.is_some() {
        return None;
    }
    let p = cefapp::pane_view(slot)?;
    unsafe { Retained::retain(p as *mut NSView) }
}

/// Moves/resizes a slot's view (points, top-left origin) and, for windowless panes, tells CEF the new pixel size.
pub fn place(slot: usize, x: f64, y: f64, w: f64, h: f64, windowless: bool) {
    let Some(view) = pane_view(slot) else { return };
    let (w, h) = (w.max(1.0), h.max(1.0));
    view.setFrame(flip(x, y, w, h));
    below_webview(&view);
    let sc = scale();
    if !windowless {
        let sig = ((x * 10.0) as i64, (y * 10.0) as i64, (w * 10.0) as i64, (h * 10.0) as i64);
        let changed = with_slot(slot, |s| {
            let c = s.pix != (sig.2 as u32, sig.3 as u32) || (s.scale - sc).abs() > f32::EPSILON;
            s.pix = (sig.2 as u32, sig.3 as u32);
            s.scale = sc;
            c
        })
        .unwrap_or(false);
        if changed {
            spikelog::event("place_windowed", &format!("slot={slot} pts=({x:.1},{y:.1},{w:.1},{h:.1}) scale={sc}"));
        }
        return;
    }
    let pix = (((w * sc as f64).round().max(1.0)) as u32, ((h * sc as f64).round().max(1.0)) as u32);
    let changed = with_slot(slot, |s| {
        let c = s.pix != pix || (s.scale - sc).abs() > f32::EPSILON;
        s.pix = pix;
        s.scale = sc;
        s.active = true;
        if c {
            s.resize_pending = Some((Instant::now(), pix));
        }
        c
    })
    .unwrap_or(false);
    if changed {
        spikelog::event(
            "place",
            &format!("slot={slot} pts=({x:.1},{y:.1},{w:.1},{h:.1}) scale={sc} pix={}x{}", pix.0, pix.1),
        );
        if let Some(host) = cefapp::pane_browser(slot).and_then(|b| b.host()) {
            host.notify_screen_info_changed();
            host.was_resized();
        }
    }
}

/// Screen origin of the pane's top-left in points (CEF's GetScreenPoint is in DIP on macOS).
pub fn slot_origin(slot: usize) -> (i32, i32) {
    let Some(view) = pane_view(slot) else { return (0, 0) };
    let Some(win) = view.window() else { return (0, 0) };
    // Top-left of the view in window coordinates (unflipped), converted to screen coordinates, then flipped
    // against the primary screen height to get a top-left based screen y.
    let b = view.bounds();
    let in_win = view.convertPoint_toView(NSPoint::new(0.0, b.size.height), None);
    let scr = win.convertPointToScreen(in_win);
    let screen_h = objc2_app_kit::NSScreen::screens(mtm())
        .iter()
        .next()
        .map(|s| s.frame().size.height)
        .unwrap_or(0.0);
    (scr.x as i32, (screen_h - scr.y) as i32)
}

// ------------------------------------------------------------------ hit-test passthrough (mode a)
// A transparent WKWebView above the CEF views would swallow every mouse hit. For mode `a` the WKWebView instance
// is switched (isa swizzle, this one object only) to a runtime subclass whose hitTest: returns nil over the pane
// rectangles, so AppKit continues to the CEF view below it. While an HTML overlay is open the WKWebView takes
// everything. COMPROMISE: a runtime subclass of wry's WKWebView.

static HIT_RECTS: Mutex<Vec<(f64, f64, f64, f64)>> = Mutex::new(Vec::new());
static MODAL: AtomicBool = AtomicBool::new(false);
static SUPER_HIT: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static HIT_LOGGED: AtomicU32 = AtomicU32::new(0);
static HIT_CALLS: AtomicI64 = AtomicI64::new(0);

extern "C" fn hit_test(this: *mut c_void, sel: *const c_void, p: NSPoint) -> *mut c_void {
    let n = HIT_CALLS.fetch_add(1, Ordering::Relaxed);
    // Diagnostics: the first few calls and every 200th, with the rectangles compared against.
    if n < 6 || n % 200 == 0 {
        let rs = HIT_RECTS.lock().map(|r| format!("{r:?}")).unwrap_or_default();
        spikelog::event("hittest_call", &format!("n={n} point=({:.1},{:.1}) modal={} rects={rs}", p.x, p.y, MODAL.load(Ordering::Relaxed)));
    }
    if !MODAL.load(Ordering::Relaxed) {
        if let Ok(r) = HIT_RECTS.lock() {
            if let Some((i, _)) = r
                .iter()
                .enumerate()
                .find(|(_, &(x, y, w, h))| p.x >= x && p.x < x + w && p.y >= y && p.y < y + h)
            {
                // Instrumentation: hit-test coordinates (points, content view, bottom-left origin). Rate limited.
                if HIT_LOGGED.load(Ordering::Relaxed) < 40 && n % 25 == 0 {
                    HIT_LOGGED.fetch_add(1, Ordering::Relaxed);
                    spikelog::event("hittest", &format!("passthrough pane={i} point=({:.1},{:.1})", p.x, p.y));
                }
                // Diagnostics: which view does AppKit pick once this override steps aside? (re-entrancy guarded)
                static IN_PROBE: AtomicBool = AtomicBool::new(false);
                if HIT_LOGGED.load(Ordering::Relaxed) < 12 && !IN_PROBE.swap(true, Ordering::Relaxed) {
                    drop(r);
                    if let Some(st) = state() {
                        let content = st.content as *mut AnyObject;
                        let frame: NSRect = unsafe { msg_send![content, frame] };
                        let pt = NSPoint::new(p.x + frame.origin.x, p.y + frame.origin.y);
                        let winner: *mut AnyObject = unsafe { msg_send![content, hitTest: pt] };
                        // Walk up from the winner to show the chain (class names), 4 levels.
                        let mut chain = Vec::new();
                        let mut cur = winner;
                        for _ in 0..4 {
                            if cur.is_null() { break; }
                            chain.push(class_name(cur));
                            cur = unsafe { msg_send![cur, superview] };
                        }
                        let w = window();
                        let key = w.isKeyWindow();
                        let fr: *mut AnyObject = unsafe { msg_send![&*w, firstResponder] };
                        spikelog::event("hittest_winner", &format!("point=({:.1},{:.1}) winner chain: {} | window key={key} main={} firstResponder={}", p.x, p.y, chain.join(" < "), w.isMainWindow(), class_name(fr)));
                    }
                    HIT_LOGGED.fetch_add(1, Ordering::Relaxed);
                    IN_PROBE.store(false, Ordering::Relaxed);
                }
                return null_mut();
            }
        }
    }
    let f: extern "C" fn(*mut c_void, *const c_void, NSPoint) -> *mut c_void =
        unsafe { std::mem::transmute(SUPER_HIT.load(Ordering::Relaxed)) };
    f(this, sel, p)
}

pub fn install_hit_passthrough() -> Result<(), String> {
    let st = state().ok_or("bmac not initialised")?;
    unsafe {
        let wk = st.wk as *mut c_void;
        // wry already KVO-observes the WKWebView, so the instance's class is a runtime "NSKVONotifying_..." subclass.
        // Swapping isa onto a subclass of that breaks KVO (first attempt aborted in NSDynamicProperties). Instead add
        // hitTest: to wry's own class (the first non-KVO class up the chain): only this app's one WKWebView is an instance.
        let mut target = object_getClass(wk);
        let mut chain = Vec::new();
        loop {
            let name = std::ffi::CStr::from_ptr(class_getName(target)).to_string_lossy().into_owned();
            chain.push(name.clone());
            if !name.starts_with("NSKVONotifying_") {
                break;
            }
            target = class_getSuperclass(target);
            if target.is_null() {
                return Err("no non-KVO class found".into());
            }
        }
        let s = sel_registerName(c"hitTest:".as_ptr());
        let sup = class_getSuperclass(target);
        let super_imp = class_getMethodImplementation(sup, s);
        if class_addMethod(target, s, hit_test as *const c_void, c"@@:{CGPoint=dd}".as_ptr()) == 0 {
            return Err(format!("class {} already implements hitTest:", chain.last().unwrap()));
        }
        SUPER_HIT.store(super_imp as *mut c_void, Ordering::Relaxed);
        spikelog::event("hit_passthrough", &format!("hitTest: added to {} (instance chain: {})", chain.last().unwrap(), chain.join(" > ")));
    }
    Ok(())
}

/// Pane rectangles in content-view coordinates (points, bottom-left origin) that the WKWebView must not claim.
pub fn set_hit_rects(rects_top_left: &[(f64, f64, f64, f64)]) {
    let lr = layout_rect();
    if let Ok(mut r) = HIT_RECTS.lock() {
        *r = rects_top_left
            .iter()
            .map(|&(x, y, w, h)| (lr.origin.x + x, lr.origin.y + lr.size.height - y - h, w, h))
            .collect();
    }
}

pub fn set_modal(open: bool) {
    MODAL.store(open, Ordering::Relaxed);
}

// ------------------------------------------------------------------ view hierarchy evidence

/// Writes the window/content-view stacking (bottom to top) to events.csv: the evidence for the decisive check.
pub fn dump_hierarchy(label: &str) {
    let Some(_st) = state() else { return };
    let c = content();
    let win = window();
    let mut parts: Vec<String> = Vec::new();
    for (i, v) in c.subviews().iter().enumerate() {
        let ptr = &*v as *const NSView as *mut AnyObject;
        let f = v.frame();
        let layer: *mut AnyObject = unsafe { msg_send![ptr, layer] };
        parts.push(format!(
            "#{i} {} frame=({:.0},{:.0},{:.0},{:.0}) opaque={} hidden={} alpha={:.2} layer={}",
            class_name(ptr),
            f.origin.x,
            f.origin.y,
            f.size.width,
            f.size.height,
            v.isOpaque(),
            v.isHidden(),
            v.alphaValue(),
            class_name(layer),
        ));
    }
    spikelog::event(
        "hierarchy",
        &format!(
            "{label}: window opaque={} scale={} content=({:.0}x{:.0}pt) contentLayoutRect=({:.0},{:.0},{:.0}x{:.0}) | bottom-to-top: {}",
            win.isOpaque(),
            win.backingScaleFactor(),
            c.bounds().size.width,
            c.bounds().size.height,
            win.contentLayoutRect().origin.x,
            win.contentLayoutRect().origin.y,
            win.contentLayoutRect().size.width,
            win.contentLayoutRect().size.height,
            parts.join(" | ")
        ),
    );
}

// ------------------------------------------------------------------ clipboard + context menu

pub fn write_clipboard(text: &str) {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    let ok = unsafe { pb.setString_forType(&NSString::from_str(text), NSPasteboardTypeString) };
    log::info!("clipboard write ({} chars) ok={ok}", text.chars().count());
}

static MENU_CHOICE: AtomicI64 = AtomicI64::new(0);

extern "C" fn menu_pick(_this: *mut c_void, _sel: *const c_void, sender: *mut AnyObject) {
    let tag: isize = unsafe { msg_send![sender, tag] };
    MENU_CHOICE.store(tag as i64, Ordering::Relaxed);
}

fn menu_target() -> Retained<AnyObject> {
    static CLS: OnceLock<usize> = OnceLock::new();
    let cls = *CLS.get_or_init(|| unsafe {
        let sup = AnyClass::get(c"NSObject").map(|c| c as *const AnyClass as *mut c_void).unwrap_or(null_mut());
        let c = objc_allocateClassPair(sup, c"SpikeMenuTarget".as_ptr(), 0);
        class_addMethod(c, sel_registerName(c"pick:".as_ptr()), menu_pick as *const c_void, c"v@:@".as_ptr());
        objc_registerClassPair(c);
        c as usize
    });
    unsafe {
        let obj: *mut AnyObject = msg_send![cls as *mut AnyObject, new];
        Retained::from_raw(obj).expect("menu target")
    }
}

/// The NSMenu stand-in for the Linux gtk::Menu (items.id=379). Runs on the main thread. `x`,`y` are view DIPs.
pub fn show_context_menu(
    slot: usize,
    x: i32,
    y: i32,
    editable: bool,
    has_selection: bool,
    callback: cef::RunContextMenuCallback,
) {
    let Some(view) = pane_view(slot) else {
        callback.cancel();
        return;
    };
    let mtm = mtm();
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    let target = menu_target();
    let add = |title: &str, tag: isize, enabled: bool| {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc::<NSMenuItem>(),
                &NSString::from_str(title),
                Some(sel!(pick:)),
                &NSString::from_str(""),
            )
        };
        item.setTag(tag);
        item.setEnabled(enabled);
        unsafe { item.setTarget(Some(&target)) };
        menu.addItem(&item);
    };
    add("Copy", 1, has_selection);
    add("Cut", 2, editable && has_selection);
    add("Paste", 3, editable);
    add("Select all", 4, true);
    MENU_CHOICE.store(0, Ordering::Relaxed);
    let h = view.bounds().size.height;
    // Blocks in a nested tracking loop; the common-mode pump keeps CEF alive meanwhile.
    menu.popUpMenuPositioningItem_atLocation_inView(None, NSPoint::new(x as f64, h - y as f64), Some(&view));
    let chosen = MENU_CHOICE.load(Ordering::Relaxed);
    log::info!("context menu choice: {chosen}");
    match chosen {
        1 => {
            bcef::edit_command(slot, "copy");
            callback.cancel();
        }
        2 => callback.cont(MenuId::CUT.get_raw() as i32, cef::EventFlags::default()),
        3 => callback.cont(MenuId::PASTE.get_raw() as i32, cef::EventFlags::default()),
        4 => callback.cont(MenuId::SELECT_ALL.get_raw() as i32, cef::EventFlags::default()),
        _ => callback.cancel(),
    }
}

// ------------------------------------------------------------------ host-managed popup (slot 2, windowless)

const POPUP_W: f64 = 480.0;
const POPUP_H: f64 = 640.0;

pub fn popup_opened() {
    let Some(app) = bcef::APP.get() else { return };
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let (cw, ch) = content_size();
        let pw = POPUP_W.min(cw - 20.0).max(200.0);
        let ph = POPUP_H.min(ch - 20.0).max(200.0);
        let (x, y) = ((cw - pw) / 2.0, (ch - ph) / 2.0);
        match create_pane_view(POPUP_SLOT, x, y, pw, ph) {
            Ok(()) => {
                if let Some(host) = cefapp::pane_browser(POPUP_SLOT).and_then(|b| b.host()) {
                    host.notify_screen_info_changed();
                    host.was_resized();
                    host.set_focus(true as _);
                }
                let _ = app2.emit("popup-rect", serde_json::json!({ "x": x, "y": y, "w": pw, "h": ph }));
                log::info!("popup view created {pw}x{ph} pt at {x},{y}");
            }
            Err(e) => log::error!("popup view creation failed: {e}"),
        }
    });
}

pub fn popup_closed() {
    let Some(app) = bcef::APP.get() else { return };
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(st) = state() {
            if let Some(tx) = &st.tx {
                let _ = tx.lock().unwrap().send(Cmd::Remove { slot: POPUP_SLOT });
            }
        }
        let _ = app2.emit("popup-rect", serde_json::Value::Null);
        let view = VIEWS.lock().ok().and_then(|mut v| v[POPUP_SLOT].take());
        with_slot(POPUP_SLOT, |s| *s = Default::default());
        if let Some(SendView(view)) = view {
            // Let the render thread drop the surface before the view goes away.
            let app3 = app2.clone();
            let holder = SendView(view);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                let _ = app3.run_on_main_thread(move || {
                    let h = holder; // capture the whole Send wrapper, not its field
                    h.0.removeFromSuperview();
                });
            });
        }
    });
}
