//! Command line. `--key=value` form only (CEF parses the same command line and ignores unknown switches).
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Windowed CEF children parented to the Tauri window's HWND.
    A,
    /// Windowless CEF, software on_paint upload (what Linux ships), native surface above the webview.
    BSwAbove,
    /// Same, native surface below a transparent webview.
    BSwBelow,
    /// Windowless CEF, D3D11 shared-texture import, above the webview.
    BGpuAbove,
    /// Same, below a transparent webview.
    BGpuBelow,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::A => "a",
            Mode::BSwAbove => "b-sw-above",
            Mode::BSwBelow => "b-sw-below",
            Mode::BGpuAbove => "b-gpu-above",
            Mode::BGpuBelow => "b-gpu-below",
        }
    }
    pub fn parse(s: &str) -> Option<Mode> {
        Some(match s {
            "a" => Mode::A,
            "b-sw-above" => Mode::BSwAbove,
            "b-sw-below" => Mode::BSwBelow,
            "b-gpu-above" => Mode::BGpuAbove,
            "b-gpu-below" => Mode::BGpuBelow,
            _ => return None,
        })
    }
    pub fn windowless(self) -> bool {
        self != Mode::A
    }
    pub fn below(self) -> bool {
        matches!(self, Mode::BSwBelow | Mode::BGpuBelow)
    }
    /// STAGE 1 builds mode `a` only; the B modes arrive in stage 2.
    pub fn implemented(self) -> bool {
        self == Mode::A
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CefLoop {
    /// CEF runs its UI thread on its own thread (what Linux ships, decisions.id=700).
    Mt,
    /// CEF's work is pumped from the Tauri main thread (same thread as the parent window).
    Pump,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Runtime {
    Alloy,
    Chrome,
}

/// Report-only experiments for mode A's airspace problem. NOT candidate solutions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModalPolicy {
    None,
    Hide,
    Hole,
}

/// Mode A: whether the CEF child windows are raised above the WebView2 child (default) or left where creation
/// order put them. `keep` exists to document the default stacking (CI-run-5 finding: panes loaded but invisible).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ZOrder {
    Raise,
    Keep,
}

#[derive(Clone, Debug)]
pub struct Cli {
    pub a_zorder: ZOrder,
    pub size: (f64, f64),
    pub mode: Mode,
    pub cef_loop: CefLoop,
    pub runtime: Runtime,
    pub modal_policy: ModalPolicy,
    pub profile: PathBuf,
    /// Diagnostic only: turns the Chromium sandbox OFF. Stamped loudly in env.json and the title bar.
    pub diag_no_sandbox: bool,
    pub soak: bool,
}

static CLI: OnceLock<Cli> = OnceLock::new();

pub fn get() -> &'static Cli {
    CLI.get().expect("cli not initialised")
}

pub fn init(cli: Cli) {
    let _ = CLI.set(cli);
}

pub fn parse() -> Result<Cli, String> {
    let mut cli = Cli {
        a_zorder: ZOrder::Raise,
        size: (1280.0, 800.0),
        mode: Mode::A,
        cef_loop: CefLoop::Mt,
        runtime: Runtime::Alloy,
        modal_policy: ModalPolicy::None,
        profile: std::env::temp_dir().join("qr-spike-719"),
        diag_no_sandbox: false,
        soak: false,
    };
    let mut runtime_set = false;
    for arg in std::env::args().skip(1) {
        let (k, v) = match arg.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (arg.clone(), String::new()),
        };
        match k.as_str() {
            "--mode" => cli.mode = Mode::parse(&v).ok_or(format!("unknown --mode={v}"))?,
            "--cef-loop" => {
                cli.cef_loop = match v.as_str() {
                    "mt" => CefLoop::Mt,
                    "pump" => CefLoop::Pump,
                    _ => return Err(format!("unknown --cef-loop={v}")),
                }
            }
            "--runtime" => {
                runtime_set = true;
                cli.runtime = match v.as_str() {
                    "alloy" => Runtime::Alloy,
                    "chrome" => Runtime::Chrome,
                    _ => return Err(format!("unknown --runtime={v}")),
                }
            }
            "--a-modal-policy" => {
                cli.modal_policy = match v.as_str() {
                    "none" => ModalPolicy::None,
                    "hide" => ModalPolicy::Hide,
                    "hole" => ModalPolicy::Hole,
                    _ => return Err(format!("unknown --a-modal-policy={v}")),
                }
            }
            "--a-zorder" => {
                cli.a_zorder = match v.as_str() {
                    "raise" => ZOrder::Raise,
                    "keep" => ZOrder::Keep,
                    _ => return Err(format!("unknown --a-zorder={v}")),
                }
            }
            "--size" => {
                let (w, h) = v.split_once('x').ok_or(format!("bad --size={v}"))?;
                cli.size = (
                    w.parse().map_err(|_| format!("bad --size={v}"))?,
                    h.parse().map_err(|_| format!("bad --size={v}"))?,
                );
            }
            "--profile" => cli.profile = PathBuf::from(v),
            "--diag-no-sandbox" => cli.diag_no_sandbox = true,
            "--soak" => cli.soak = true,
            // Switches CEF/Chromium add to the browser process command line are not ours.
            _ => {}
        }
    }
    // CI smoke finding: explicitly requesting the Chrome runtime style for a windowless browser makes CEF 151 log
    // "Chrome style is not supported for this browser" and fall back, so Alloy is the honest default for B too.
    let _ = runtime_set;
    Ok(cli)
}
