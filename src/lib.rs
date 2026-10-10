//! items.id=719 spike: Windows pane-host test bed. THROWAWAY. Never merged into quietrabbit-core.
pub mod cefapp;
pub mod cli;
pub mod localserver;
pub mod spikelog;

#[cfg(windows)]
pub mod host;
#[cfg(windows)]
pub mod winenv;

/// Entry point called by CEF's bootstrap.exe (the renamed spike719.exe) after it has created the Chromium
/// sandbox: this DLL is the client. The same exe/DLL pair is re-launched for every CEF helper process.
/// Signature per CEF's include/cef_sandbox_win.h.
#[cfg(windows)]
#[no_mangle]
pub extern "C" fn RunWinMain(
    _hinstance: *mut std::ffi::c_void,
    _cmd_line: *mut u16,
    _n_cmd_show: i32,
    sandbox_info: *mut std::ffi::c_void,
    _version_info: *mut std::ffi::c_void,
) -> i32 {
    run(sandbox_info as *mut u8);
    0
}

#[cfg(windows)]
fn run(sandbox: *mut u8) {
    if cefapp::dispatch_subprocess(sandbox) {
        return;
    }
    let parsed = match cli::parse() {
        Ok(c) => c,
        Err(e) => host::fatal(&e),
    };
    spikelog::init(&parsed.profile);
    log::info!("spike719 starting: {parsed:?}");
    if sandbox.is_null() && !parsed.diag_no_sandbox {
        // decisions.id=860: fail closed, no silent downgrade.
        host::fatal(
            "Chromium sandbox info was not provided by bootstrap.exe; refusing to run unsandboxed",
        );
    }
    cli::init(parsed.clone());
    host::run(parsed, sandbox);
}
