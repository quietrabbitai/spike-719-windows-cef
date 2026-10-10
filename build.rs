fn main() {
    tauri_build::build();

    // CEF's Windows sandbox: the browser exe must link cef_sandbox.lib and pass
    // cef_sandbox_info_create() to cef_execute_process/cef_initialize. The cef crate has no wrapper
    // for this, so src/winsandbox.rs declares the two extern fns by hand and this script links the lib.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        match std::env::var("DEP_CEF_DLL_WRAPPER_CEF_DIR") {
            Ok(dir) => println!("cargo::rustc-link-search=native={dir}"),
            Err(_) => println!(
                "cargo::warning=DEP_CEF_DLL_WRAPPER_CEF_DIR not set; cef_sandbox.lib may not be found"
            ),
        }
        println!("cargo::rustc-link-lib=static=cef_sandbox");
        // System libraries cef_sandbox.lib depends on (CEF's cef_variables.cmake lists these).
        for lib in [
            "dbghelp", "psapi", "version", "winmm", "advapi32", "ole32", "oleaut32",
        ] {
            println!("cargo::rustc-link-lib=dylib={lib}");
        }
    }
}
