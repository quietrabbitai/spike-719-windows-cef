//! CEF process bootstrap, App/Client handlers and the two-pane registry.
//! The bootstrap shape (api_hash first, execute_process before anything else, SoftNavigationDetection
//! workaround) is copied from quietrabbit-core's cloud_chat_gpu_pane/bootstrap.rs.
use std::path::Path;
use std::sync::Mutex;

use cef::{self, args::Args, *};

use crate::cli::{CefLoop, Cli, Runtime};

pub const PANE_COUNT: usize = 2;

/// Runs CEF's subprocess entry point when this invocation is a CEF-spawned helper. Returns true when the
/// caller must return from main() immediately.
pub fn dispatch_subprocess(sandbox_info: *mut u8) -> bool {
    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
    let args = Args::new();
    let cmd = args
        .as_cmd_line()
        .expect("CEF could not parse the command line");
    let type_switch = cef::CefString::from("type");
    let is_browser_process = cmd.has_switch(Some(&type_switch)) != 1;

    let mut app = AppBuilder::build(SpikeApp);
    let ret = cef::execute_process(Some(args.as_main_args()), Some(&mut app), sandbox_info);

    if is_browser_process {
        assert_eq!(
            ret, -1,
            "execute_process returned {ret} for the browser process"
        );
        false
    } else {
        true
    }
}

/// Initialises CEF in the browser process. `root_cache` is the profile's `cef` dir (a clean directory, never
/// Jason's own browser data).
pub fn initialize(cli: &Cli, root_cache: &Path, sandbox_info: *mut u8) -> bool {
    let args = Args::new();
    let mut app = AppBuilder::build(SpikeApp);
    let root = root_cache.to_string_lossy().into_owned();
    let log_file = root_cache
        .parent()
        .unwrap_or(root_cache)
        .join("cef.log")
        .to_string_lossy()
        .into_owned();
    let mt = cli.cef_loop == CefLoop::Mt;
    let settings = Settings {
        windowless_rendering_enabled: cli.mode.windowless() as _,
        multi_threaded_message_loop: mt as _,
        external_message_pump: (!mt) as _,
        // decisions.id=860: sandbox ENABLED. Only the explicit diagnostic flag turns it off.
        no_sandbox: cli.diag_no_sandbox as _,
        root_cache_path: CefString::from(root.as_str()),
        log_file: CefString::from(log_file.as_str()),
        log_severity: LogSeverity::INFO,
        ..Default::default()
    };
    let ok = cef::initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        sandbox_info,
    );
    log::info!(
        "cef::initialize -> {ok} (mt_loop={mt}, no_sandbox={})",
        cli.diag_no_sandbox
    );
    ok == 1
}

// ---------------------------------------------------------------- App

#[derive(Clone)]
pub struct SpikeApp;

wrap_app! {
    pub(crate) struct AppBuilder {
        app: SpikeApp,
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
            // SPA soft-navigation crash workaround proven on Linux (items.id=192): every provider is an SPA.
            // No other disable-features value is passed by this spike, so no merge is needed.
            sw("disable-features", Some("SoftNavigationDetection"));
            // Items.id=719 condition 6: nothing contacts anything beyond the three provider sites and the
            // local page. No telemetry, no background networking, no component updater, no sync.
            sw("disable-background-networking", None);
            sw("disable-component-update", None);
            sw("disable-sync", None);
            sw("disable-domain-reliability", None);
            sw("no-pings", None);
            sw("metrics-recording-only", None);
        }
    }
}

impl AppBuilder {
    pub(crate) fn build(app: SpikeApp) -> cef::App {
        Self::new(app)
    }
}

// ---------------------------------------------------------------- Client / panes

static PANES: Mutex<[Option<Browser>; PANE_COUNT]> = Mutex::new([None, None]);

#[derive(Clone)]
pub struct PaneLife {
    pub pane: usize,
}

wrap_life_span_handler! {
    pub(crate) struct LifeSpanBuilder {
        handler: PaneLife,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser else { return; };
            log::info!("pane {} browser created", self.handler.pane);
            if let Ok(mut p) = PANES.lock() {
                p[self.handler.pane] = Some(browser.clone());
            }
        }

        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            log::info!("pane {} browser closed", self.handler.pane);
            if let Ok(mut p) = PANES.lock() {
                p[self.handler.pane] = None;
            }
        }
    }
}

wrap_client! {
    pub(crate) struct ClientBuilder {
        life_span_handler: LifeSpanHandler,
    }

    impl Client {
        fn life_span_handler(&self) -> Option<cef::LifeSpanHandler> {
            Some(self.life_span_handler.clone())
        }
    }
}

#[cfg(windows)]
fn child_window_info(parent: isize, b: (i32, i32, i32, i32), runtime: Runtime) -> WindowInfo {
    let rect = Rect {
        x: b.0,
        y: b.1,
        width: b.2,
        height: b.3,
    };
    let mut wi =
        WindowInfo::default().set_as_child(cef::sys::HWND(parent as *mut cef::sys::HWND__), &rect);
    wi.runtime_style = match runtime {
        Runtime::Alloy => RuntimeStyle::ALLOY,
        Runtime::Chrome => RuntimeStyle::CHROME,
    };
    wi
}

#[cfg(not(windows))]
fn child_window_info(_parent: isize, _b: (i32, i32, i32, i32), _runtime: Runtime) -> WindowInfo {
    unimplemented!("windowed CEF children are Windows-only in this spike")
}

/// Mode A: asks CEF to create pane `idx` as a native child window of `parent` (the Tauri window's HWND).
/// Each pane gets its own RequestContext with its own on-disk cache dir (cookie-jar isolation, criterion 8).
/// Returns false when CEF refused the creation request.
pub fn create_windowed_pane(
    cli: &Cli,
    idx: usize,
    url: &str,
    parent: isize,
    bounds: (i32, i32, i32, i32),
    cache_root: &Path,
) -> bool {
    let wi = child_window_info(parent, bounds, cli.runtime);
    let mut client = ClientBuilder::new(LifeSpanBuilder::new(PaneLife { pane: idx }));
    let cache = cache_root.join(format!("pane{idx}"));
    let ctx_settings = RequestContextSettings {
        cache_path: CefString::from(cache.to_string_lossy().as_ref()),
        persist_session_cookies: 1,
        ..Default::default()
    };
    let mut ctx = request_context_create_context(Some(&ctx_settings), None);
    if ctx.is_none() {
        log::warn!("pane {idx}: request_context_create_context returned None");
    }
    let url = CefString::from(url);
    let created = browser_host_create_browser(
        Some(&wi),
        Some(&mut client),
        Some(&url),
        Some(&BrowserSettings::default()),
        None,
        ctx.as_mut(),
    );
    log::info!("pane {idx}: browser_host_create_browser -> {created}");
    created != 0
}

pub fn pane_browser(idx: usize) -> Option<Browser> {
    PANES.lock().ok().and_then(|p| p[idx].clone())
}

/// The native HWND CEF created for a windowed pane (mode A), once on_after_created has run.
#[cfg(windows)]
pub fn pane_hwnd(idx: usize) -> Option<isize> {
    let b = pane_browser(idx)?;
    Some(b.host()?.window_handle().0 as isize)
}

pub fn navigate_pane(idx: usize, url: &str) -> bool {
    let Some(b) = pane_browser(idx) else {
        return false;
    };
    let Some(f) = b.main_frame() else {
        return false;
    };
    f.load_url(Some(&CefString::from(url)));
    true
}

pub fn close_all_panes() {
    for i in 0..PANE_COUNT {
        if let Some(b) = pane_browser(i) {
            if let Some(h) = b.host() {
                h.close_browser(true as _);
            }
        }
    }
}
