//! macOS entry points (items.id=720 spike, THROWAWAY). The browser binary and the helper binary are separate
//! executables inside the .app bundle (see tools/bundle-mac.sh); the helper is copied into every
//! "<name> Helper*.app" under Contents/Frameworks.
use cef::library_loader::LibraryLoader;

use crate::{cefapp, cli, host_mac, macapp, spikelog};

/// Browser process.
pub fn browser_main() {
    let exe = std::env::current_exe().expect("current_exe");
    // 1. The CEF framework must be loaded before any other CEF call.
    let loader = LibraryLoader::new(&exe, false);
    if !loader.load() {
        eprintln!("spike720 FATAL: cannot load Chromium Embedded Framework (is this running from the .app bundle?)");
        std::process::exit(2);
    }
    let parsed = match cli::parse() {
        Ok(c) => c,
        Err(e) => host_mac::fatal(&e),
    };
    spikelog::init(&parsed.profile);
    // 2. NSApp must answer CefAppProtocol before tao/tauri creates it and before CEF's loop runs.
    macapp::patch_nsapplication();
    log::info!("spike720 starting: {parsed:?}");
    if parsed.diag_no_sandbox {
        log::warn!("DIAGNOSTIC: Chromium sandbox disabled by --diag-no-sandbox");
    }
    // Returns false for the browser process (it asserts -1 from execute_process).
    let _ = cefapp::dispatch_subprocess(std::ptr::null_mut());
    cli::init(parsed.clone());
    host_mac::run(parsed);
    // tauri's run() normally exits the process; keep the loader alive until here regardless.
    drop(loader);
}

/// Helper processes (GPU, renderer, plugin, alerts, utility). Order matters (CEF macOS docs, cef-rs cefsimple):
/// sandbox init first, then load the framework, then execute_process.
pub fn helper_main() {
    use cef::{args::Args, sandbox::Sandbox};
    let args = Args::new();
    let mut sandbox = Sandbox::new();
    sandbox.initialize(args.as_main_args());
    let exe = std::env::current_exe().expect("current_exe");
    let loader = LibraryLoader::new(&exe, true);
    if !loader.load() {
        eprintln!("spike720 helper FATAL: cannot load Chromium Embedded Framework");
        std::process::exit(2);
    }
    let _ = cefapp::dispatch_subprocess(std::ptr::null_mut());
    drop(loader);
    drop(sandbox);
}
