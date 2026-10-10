//! items.id=719 spike: Windows pane-host test bed. THROWAWAY. Never merged into quietrabbit-core.
pub mod cefapp;
pub mod cli;
pub mod localserver;
pub mod spikelog;

#[cfg(windows)]
pub mod host;
#[cfg(windows)]
pub mod winenv;
#[cfg(windows)]
pub mod winsandbox;

#[cfg(windows)]
pub fn run() {
    // The sandbox info must exist before anything else touches the process (cef_sandbox_win.h), and the same
    // exe serves as browser and helper processes, so this runs on every launch.
    let diag = std::env::args().any(|a| a == "--diag-no-sandbox");
    let sandbox = if diag {
        std::ptr::null_mut()
    } else {
        winsandbox::create()
    };
    if cefapp::dispatch_subprocess(sandbox) {
        return;
    }
    let parsed = match cli::parse() {
        Ok(c) => c,
        Err(e) => host::fatal(&e),
    };
    spikelog::init(&parsed.profile);
    log::info!("spike719 starting: {parsed:?}");
    if sandbox.is_null() && !diag {
        // decisions.id=860: fail closed, no silent downgrade.
        host::fatal("Chromium sandbox could not be created; refusing to run unsandboxed");
    }
    cli::init(parsed.clone());
    host::run(parsed, sandbox);
}

#[cfg(not(windows))]
pub fn run() {
    eprintln!("spike719 is a Windows-only test bed");
}
