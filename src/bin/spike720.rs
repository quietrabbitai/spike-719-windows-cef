//! items.id=720: macOS browser-process binary (THROWAWAY). Must run from the .app bundle made by tools/bundle-mac.sh.
#[cfg(target_os = "macos")]
fn main() {
    spike719::macmain::browser_main();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("spike720 is the macOS binary; on Windows use spike719.exe");
}
