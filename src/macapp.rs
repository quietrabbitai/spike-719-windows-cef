//! macOS process plumbing for the items.id=720 spike (THROWAWAY):
//!  * `patch_nsapplication`: makes NSApp satisfy CEF's CefAppProtocol (isHandlingSendEvent /
//!    setHandlingSendEvent:) by adding the methods to the NSApplication base class at runtime, so tao's TaoApp
//!    (a subclass created later by tauri) inherits them. COMPROMISE: production would need a tao change or a
//!    tauri-runtime-cef style bootstrap.
//!  * the external message pump: macOS has no multi_threaded_message_loop, CEF's UI thread is the main thread,
//!    so `do_message_loop_work` is called from libdispatch's main queue (serviced in the common run-loop modes,
//!    so it keeps running during live resize, window drags and menu tracking).
//!  * the CEF `App` for the browser process (command-line switches + BrowserProcessHandler).
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use cef::{self, *};

use crate::spikelog;

// ---------------------------------------------------------------- raw Objective-C runtime (kept raw on purpose:
// no dependency on objc2's wrapper API for the few calls that must exist before tao starts)

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn objc_getProtocol(name: *const c_char) -> *mut c_void;
    fn objc_allocateProtocol(name: *const c_char) -> *mut c_void;
    fn objc_registerProtocol(p: *mut c_void);
    fn protocol_addProtocol(p: *mut c_void, other: *const c_void);
    fn protocol_addMethodDescription(
        p: *mut c_void,
        name: *const c_void,
        types: *const c_char,
        required: u8,
        instance: u8,
    );
    fn class_addMethod(cls: *mut c_void, name: *const c_void, imp: *const c_void, types: *const c_char) -> u8;
    fn class_addProtocol(cls: *mut c_void, p: *const c_void) -> u8;
    fn class_conformsToProtocol(cls: *mut c_void, p: *const c_void) -> u8;
    fn class_getInstanceMethod(cls: *mut c_void, name: *const c_void) -> *mut c_void;
    fn method_getImplementation(m: *mut c_void) -> *const c_void;
    fn method_setImplementation(m: *mut c_void, imp: *const c_void) -> *const c_void;
    fn sel_registerName(name: *const c_char) -> *const c_void;
}

static HANDLING_SEND_EVENT: AtomicBool = AtomicBool::new(false);
static ORIG_SEND_EVENT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

extern "C" fn is_handling_send_event(_this: *mut c_void, _sel: *const c_void) -> u8 {
    HANDLING_SEND_EVENT.load(Ordering::Relaxed) as u8
}

extern "C" fn set_handling_send_event(_this: *mut c_void, _sel: *const c_void, v: u8) {
    HANDLING_SEND_EVENT.store(v != 0, Ordering::Relaxed);
}

/// Replacement for -[NSApplication sendEvent:]: sets the flag for the duration of the dispatch (what
/// CrAppControlProtocol implementers do), then calls the original IMP. tao's TaoApp calls super, so it lands here.
extern "C" fn send_event(this: *mut c_void, sel: *const c_void, event: *mut c_void) {
    let prev = HANDLING_SEND_EVENT.swap(true, Ordering::Relaxed);
    let orig = ORIG_SEND_EVENT.load(Ordering::Relaxed);
    if !orig.is_null() {
        let f: extern "C" fn(*mut c_void, *const c_void, *mut c_void) = unsafe { std::mem::transmute(orig) };
        f(this, sel, event);
    }
    HANDLING_SEND_EVENT.store(prev, Ordering::Relaxed);
}

unsafe fn sel(name: &CStr) -> *const c_void {
    sel_registerName(name.as_ptr())
}

/// Returns (protocol, was_new). Prefers the protocol object that libcef itself registered (call this after the
/// framework is loaded); allocates an equivalent one otherwise.
unsafe fn ensure_protocol(
    name: &CStr,
    parent: Option<*mut c_void>,
    method_sel: &CStr,
    method_types: &CStr,
) -> (*mut c_void, bool) {
    let existing = objc_getProtocol(name.as_ptr());
    if !existing.is_null() {
        return (existing, false);
    }
    let p = objc_allocateProtocol(name.as_ptr());
    if let Some(par) = parent {
        protocol_addProtocol(p, par);
    }
    protocol_addMethodDescription(p, sel(method_sel), method_types.as_ptr(), 1, 1);
    objc_registerProtocol(p);
    (p, true)
}

/// Call after the CEF framework is loaded and before tauri/tao creates NSApp.
pub fn patch_nsapplication() {
    unsafe {
        let cls = objc_getClass(c"NSApplication".as_ptr());
        assert!(!cls.is_null(), "NSApplication class not found");
        let ok1 = class_addMethod(cls, sel(c"isHandlingSendEvent"), is_handling_send_event as *const c_void, c"B@:".as_ptr());
        let ok2 = class_addMethod(cls, sel(c"setHandlingSendEvent:"), set_handling_send_event as *const c_void, c"v@:B".as_ptr());
        let (cr, n1) = ensure_protocol(c"CrAppProtocol", None, c"isHandlingSendEvent", c"B@:");
        let (ctl, n2) = ensure_protocol(c"CrAppControlProtocol", Some(cr), c"setHandlingSendEvent:", c"v@:B");
        let (cef_p, n3) = ensure_protocol(c"CefAppProtocol", Some(ctl), c"isHandlingSendEvent", c"B@:");
        for p in [cr, ctl, cef_p] {
            class_addProtocol(cls, p);
        }
        let m = class_getInstanceMethod(cls, sel(c"sendEvent:"));
        let swizzled = if m.is_null() {
            false
        } else {
            let orig = method_getImplementation(m);
            ORIG_SEND_EVENT.store(orig as *mut c_void, Ordering::Relaxed);
            method_setImplementation(m, send_event as *const c_void);
            true
        };
        let conforms = class_conformsToProtocol(cls, cef_p) != 0;
        log::info!(
            "NSApplication patched: isHandlingSendEvent added={} setHandlingSendEvent added={} protocols new(cr,ctl,cef)=({n1},{n2},{n3}) sendEvent swizzled={swizzled} conforms_CefAppProtocol={conforms}",
            ok1 != 0,
            ok2 != 0
        );
    }
}

// ---------------------------------------------------------------- external message pump (libdispatch main queue)

extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(queue: *const c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn dispatch_after_f(when: u64, queue: *const c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn dispatch_time(when: u64, delta: i64) -> u64;
}

const DISPATCH_TIME_NOW: u64 = 0;
static IN_WORK: AtomicBool = AtomicBool::new(false);
static PUMP_RUNNING: AtomicBool = AtomicBool::new(false);
static PUMP_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn main_queue() -> *const c_void {
    unsafe { &_dispatch_main_q as *const c_void }
}

/// One call to CefDoMessageLoopWork, guarded against re-entry (menu tracking runs nested loops that service
/// this queue; cefclient's external pump has the same guard).
extern "C" fn pump_once(_ctx: *mut c_void) {
    if !PUMP_RUNNING.load(Ordering::Relaxed) {
        return;
    }
    if IN_WORK.swap(true, Ordering::Acquire) {
        return;
    }
    cef::do_message_loop_work();
    PUMP_CALLS.fetch_add(1, Ordering::Relaxed);
    IN_WORK.store(false, Ordering::Release);
}

/// 60 Hz heartbeat so a missed on_schedule_message_pump_work can never stall CEF.
extern "C" fn heartbeat(ctx: *mut c_void) {
    if !PUMP_RUNNING.load(Ordering::Relaxed) {
        return;
    }
    pump_once(ctx);
    unsafe {
        dispatch_after_f(dispatch_time(DISPATCH_TIME_NOW, 16_000_000), main_queue(), std::ptr::null_mut(), heartbeat);
    }
}

/// Must be called on the main thread after cef::initialize succeeded.
pub fn start_pump() {
    PUMP_RUNNING.store(true, Ordering::Relaxed);
    unsafe {
        dispatch_async_f(main_queue(), std::ptr::null_mut(), heartbeat);
    }
    log::info!("external message pump started (main queue, 16 ms heartbeat)");
}

pub fn stop_pump() {
    PUMP_RUNNING.store(false, Ordering::Relaxed);
}

pub fn pump_calls() -> u64 {
    PUMP_CALLS.load(Ordering::Relaxed)
}

/// Safe from any thread.
fn schedule(delay_ms: i64) {
    if !PUMP_RUNNING.load(Ordering::Relaxed) {
        return;
    }
    unsafe {
        if delay_ms <= 0 {
            dispatch_async_f(main_queue(), std::ptr::null_mut(), pump_once);
        } else {
            dispatch_after_f(
                dispatch_time(DISPATCH_TIME_NOW, delay_ms * 1_000_000),
                main_queue(),
                std::ptr::null_mut(),
                pump_once,
            );
        }
    }
}

/// Used on shutdown: pumps CEF until `done()` or `max_ms` (main thread). Returns whether `done()` became true.
pub fn drain_until(done: impl Fn() -> bool, max_ms: u64) -> bool {
    let t = std::time::Instant::now();
    while t.elapsed().as_millis() < max_ms as u128 {
        if done() {
            return true;
        }
        cef::do_message_loop_work();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    done()
}

/// Used on shutdown: drains CEF while browsers close (main thread).
pub fn drain(ms: u64) {
    let t = std::time::Instant::now();
    while t.elapsed().as_millis() < ms as u128 {
        cef::do_message_loop_work();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

// ---------------------------------------------------------------- CEF App (browser process)

wrap_browser_process_handler! {
    pub(crate) struct PumpHandlerBuilder {
        _unit: (),
    }

    impl BrowserProcessHandler {
        fn on_schedule_message_pump_work(&self, delay_ms: i64) {
            schedule(delay_ms);
        }
    }
}

wrap_app! {
    pub(crate) struct MacAppBuilder {
        handler: BrowserProcessHandler,
    }

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&cef::CefStringUtf16>,
            command_line: Option<&mut cef::CommandLine>,
        ) {
            let Some(command_line) = command_line else { return; };
            let sw = |name: &str, value: Option<&str>| {
                let n = cef::CefString::from(name);
                match value {
                    Some(v) => command_line.append_switch_with_value(Some(&n), Some(&v.into())),
                    None => command_line.append_switch(Some(&n)),
                }
            };
            // Same switches as the Windows bed (items.id=719): SPA soft-navigation crash workaround, and nothing
            // contacts anything beyond the provider sites and the local page.
            sw("disable-features", Some("SoftNavigationDetection"));
            sw("disable-background-networking", None);
            sw("disable-component-update", None);
            sw("disable-sync", None);
            sw("disable-domain-reliability", None);
            sw("no-pings", None);
            sw("metrics-recording-only", None);
            // macOS only. COMPROMISE: without this Chromium reads/creates its "Safe Storage" Keychain item, which
            // raises a Keychain access prompt (a macOS permission request, not allowed without asking Jason).
            // With the mock keychain, cookies are encrypted with a fixed key instead of a Keychain-held one.
            sw("use-mock-keychain", None);
        }

        fn browser_process_handler(&self) -> Option<cef::BrowserProcessHandler> {
            Some(self.handler.clone())
        }
    }
}

pub fn make_app() -> cef::App {
    let handler = PumpHandlerBuilder::new(());
    MacAppBuilder::new(handler)
}

/// Facts about NSApp for env.json / the log.
pub fn describe_nsapp() -> String {
    use objc2::runtime::AnyObject;
    use objc2::{msg_send, MainThreadMarker};
    let Some(_mtm) = MainThreadMarker::new() else {
        return "not on main thread".into();
    };
    unsafe {
        let cls = objc_getClass(c"NSApplication".as_ptr());
        let app: *mut AnyObject = msg_send![cls as *mut AnyObject, sharedApplication];
        if app.is_null() {
            return "NSApp is nil".into();
        }
        let name = (*app).class().name().to_string_lossy().into_owned();
        let p = objc_getProtocol(c"CefAppProtocol".as_ptr());
        let conforms = !p.is_null() && class_conformsToProtocol((*app).class() as *const _ as *mut c_void, p) != 0;
        // What Chromium actually does: send the messages. The selector check is what prevents an
        // unrecognized-selector crash; the protocol check is a DCHECK in debug builds only.
        let responds_get: bool = msg_send![app, respondsToSelector: objc2::sel!(isHandlingSendEvent)];
        let responds_set: bool = msg_send![app, respondsToSelector: objc2::sel!(setHandlingSendEvent:)];
        let conforms_msg: bool = if p.is_null() { false } else { msg_send![app, conformsToProtocol: p as *mut AnyObject] };
        let s = format!(
            "NSApp class={name} class_conformsToProtocol={conforms} [NSApp conformsToProtocol:]={conforms_msg} responds(isHandlingSendEvent)={responds_get} responds(setHandlingSendEvent:)={responds_set}"
        );
        spikelog::event("nsapp", &s);
        s
    }
}
