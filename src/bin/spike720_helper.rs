//! items.id=720: macOS CEF helper binary (GPU, renderer, plugin, alerts, utility). Copied by tools/bundle-mac.sh
//! into every "spike720 Helper*.app" under Contents/Frameworks. THROWAWAY.
#[cfg(target_os = "macos")]
fn main() {
    spike719::macmain::helper_main();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("spike720_helper is the macOS helper binary");
}
