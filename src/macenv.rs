//! Environment capture for the macOS spike: env.json in the profile dir. Nothing is sent anywhere.
use std::path::Path;

use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;

use crate::cli::Cli;

fn cmd(program: &str, args: &[&str]) -> String {
    std::process::Command::new(program)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|e| format!("unknown ({e})"))
}

fn screens() -> Vec<serde_json::Value> {
    let Some(mtm) = MainThreadMarker::new() else { return vec![] };
    NSScreen::screens(mtm)
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let f = s.frame();
            serde_json::json!({
                "index": i,
                "name": s.localizedName().to_string(),
                "frame_pt": [f.origin.x, f.origin.y, f.size.width, f.size.height],
                "backing_scale_factor": s.backingScaleFactor(),
                "max_fps": s.maximumFramesPerSecond(),
            })
        })
        .collect()
}

pub fn write_env_json(profile: &Path, cli: &Cli, sandbox: &str, port: u16) {
    let v = serde_json::json!({
        "spike": "720 macOS",
        "macos": cmd("sw_vers", &["-productVersion"]),
        "macos_build": cmd("sw_vers", &["-buildVersion"]),
        "model": cmd("sysctl", &["-n", "hw.model"]),
        "cpu": cmd("sysctl", &["-n", "machdep.cpu.brand_string"]),
        "mem_bytes": cmd("sysctl", &["-n", "hw.memsize"]),
        "screens": screens(),
        "cli": format!("{cli:?}"),
        "sandbox": sandbox,
        "local_port": port,
        "wgpu_crate": "30.0.1 (this spike's Cargo.lock; production pins 30.0.0)",
        "cef_crate": "151.1.0+151.3.12",
    });
    let _ = std::fs::write(profile.join("env.json"), serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// Adds keys to env.json (the GPU adapter info arrives after the first write).
pub fn merge_env(profile: &Path, extra: serde_json::Value) {
    let p = profile.join("env.json");
    let mut base: serde_json::Value = std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    let _ = std::fs::write(&p, serde_json::to_string_pretty(&base).unwrap_or_default());
}

/// Top-left origin (points, global top-left coordinates as tao's logical positions) of the first screen whose
/// backing scale factor is >= 2, for `--screen=virtual2x`.
pub fn first_2x_screen_origin() -> Option<(f64, f64)> {
    let mtm = MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm);
    let primary_h = screens.iter().next()?.frame().size.height;
    for s in screens.iter() {
        if s.backingScaleFactor() >= 1.99 {
            let f = s.frame();
            return Some((f.origin.x, primary_h - (f.origin.y + f.size.height)));
        }
    }
    None
}
