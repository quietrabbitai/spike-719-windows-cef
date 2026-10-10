//! CEF's Windows sandbox hook. The cef crate wraps this on macOS only (src/sandbox.rs there); on Windows the
//! sys bindings merely accept a `windows_sandbox_info` pointer. These two functions live in cef_sandbox.lib
//! (build.rs links it) and have C linkage (cef_sandbox_win.h).
//!
//! Must be called at the very top of main(), before anything else in the process, and the returned pointer is
//! handed to execute_process() and initialize(). If it cannot be created the spike fails closed
//! (decisions.id=860), unless --diag-no-sandbox is passed.
use std::ffi::c_void;

extern "C" {
    fn cef_sandbox_info_create() -> *mut c_void;
    #[allow(dead_code)]
    fn cef_sandbox_info_destroy(sandbox_info: *mut c_void);
}

pub fn create() -> *mut u8 {
    // SAFETY: plain call into cef_sandbox.lib; no arguments, returns an owned opaque pointer.
    unsafe { cef_sandbox_info_create() as *mut u8 }
}
