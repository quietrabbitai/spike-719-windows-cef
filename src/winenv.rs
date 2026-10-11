//! Environment capture (env.json), the sandbox self-check, and the 1 Hz process-tree CPU sampler.
//! Everything is written to the profile directory only. Nothing is sent anywhere.
use std::ffi::c_void;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, RECT};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIDevice, IDXGIFactory1};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, IsTokenRestricted,
    TokenIntegrityLevel, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::JobObjects::IsProcessInJob;
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, GetDpiForMonitor, GetThreadDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    DPI_AWARENESS_CONTEXT_SYSTEM_AWARE, DPI_AWARENESS_CONTEXT_UNAWARE, MDT_EFFECTIVE_DPI,
};

use crate::cli::Cli;

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

fn os_version() -> String {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("cmd")
        .args(["/c", "ver"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|e| format!("unknown ({e})"))
}

fn dpi_awareness() -> &'static str {
    // Must run on the thread that owns the Tauri window (the main thread).
    unsafe {
        let ctx = GetThreadDpiAwarenessContext();
        let eq = |c| AreDpiAwarenessContextsEqual(ctx, c).as_bool();
        if eq(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) {
            "PER_MONITOR_AWARE_V2"
        } else if eq(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE) {
            "PER_MONITOR_AWARE"
        } else if eq(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE) {
            "SYSTEM_AWARE"
        } else if eq(DPI_AWARENESS_CONTEXT_UNAWARE) {
            "UNAWARE"
        } else {
            "OTHER"
        }
    }
}

unsafe extern "system" fn monitor_cb(m: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
    let out = &mut *(lp.0 as *mut Vec<serde_json::Value>);
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(m, &mut mi).as_bool() {
        let (mut dx, mut dy) = (0u32, 0u32);
        let _ = GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        out.push(serde_json::json!({
            "primary": mi.dwFlags & 1 == 1,
            "left": mi.rcMonitor.left, "top": mi.rcMonitor.top,
            "width_px": mi.rcMonitor.right - mi.rcMonitor.left,
            "height_px": mi.rcMonitor.bottom - mi.rcMonitor.top,
            "effective_dpi": dx,
            "scale_percent": (dx as f64 / 96.0 * 100.0).round(),
        }));
    }
    BOOL(1)
}

fn monitors() -> Vec<serde_json::Value> {
    let mut v: Vec<serde_json::Value> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(monitor_cb),
            LPARAM(&mut v as *mut _ as isize),
        );
    }
    v
}

fn gpus() -> Vec<serde_json::Value> {
    let mut v = Vec::new();
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
            return vec![serde_json::json!({"error": "CreateDXGIFactory1 failed"})];
        };
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            if let Ok(d) = adapter.GetDesc1() {
                // Driver version as the four 16-bit parts, e.g. 31.0.15.5222.
                let driver = adapter
                    .CheckInterfaceSupport(&IDXGIDevice::IID)
                    .map(|q| {
                        format!(
                            "{}.{}.{}.{}",
                            (q >> 48) & 0xffff,
                            (q >> 32) & 0xffff,
                            (q >> 16) & 0xffff,
                            q & 0xffff
                        )
                    })
                    .unwrap_or_else(|e| format!("unavailable ({e})"));
                v.push(serde_json::json!({
                    "name": wide_to_string(&d.Description),
                    "vendor_id": format!("{:#06x}", d.VendorId),
                    "device_id": format!("{:#06x}", d.DeviceId),
                    "dedicated_vram_mib": d.DedicatedVideoMemory / (1024 * 1024),
                    "software_adapter": d.Flags & 2 != 0,
                    "driver_version": driver,
                }));
            }
            i += 1;
        }
    }
    v
}

pub fn write_env_json(profile: &Path, cli: &Cli, sandbox: &str, port: u16) {
    let j = serde_json::json!({
        "spike": "items.id=719",
        "mode": cli.mode.as_str(),
        "cef_loop": format!("{:?}", cli.cef_loop),
        "runtime_style": format!("{:?}", cli.runtime),
        "a_modal_policy": format!("{:?}", cli.modal_policy),
        "sandbox": sandbox,
        "cef_crate": "151.1.0",
        "os": os_version(),
        "process_dpi_awareness": dpi_awareness(),
        "monitors": monitors(),
        "gpus": gpus(),
        "wgpu_backend": "n/a in stage 1 (mode a does not use wgpu)",
        "local_test_server": format!("http://127.0.0.1:{port}/ (loopback only)"),
        "profile_dir": profile.to_string_lossy(),
        "contacts": ["127.0.0.1 local test page", "https://claude.ai", "https://chatgpt.com", "https://gemini.google.com"],
    });
    let _ = std::fs::write(
        profile.join("env.json"),
        serde_json::to_string_pretty(&j).unwrap_or_default(),
    );
}

// ------------------------------------------------------------ process tree

struct Proc {
    pid: u32,
    ppid: u32,
    exe: String,
}

fn snapshot() -> Vec<Proc> {
    let mut out = Vec::new();
    unsafe {
        let Ok(h) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut e = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(h, &mut e).is_ok() {
            loop {
                out.push(Proc {
                    pid: e.th32ProcessID,
                    ppid: e.th32ParentProcessID,
                    exe: wide_to_string(&e.szExeFile),
                });
                if Process32NextW(h, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(h);
    }
    out
}

/// Own process plus all descendants (CEF helpers and the WebView2 host tree).
fn tree(all: &[Proc], root: u32) -> Vec<&Proc> {
    let mut ids = vec![root];
    let mut i = 0;
    while i < ids.len() {
        let parent = ids[i];
        for p in all {
            if p.ppid == parent && !ids.contains(&p.pid) {
                ids.push(p.pid);
            }
        }
        i += 1;
    }
    all.iter().filter(|p| ids.contains(&p.pid)).collect()
}

fn cpu_100ns(h: HANDLE) -> u64 {
    let (mut c, mut e, mut k, mut u) = Default::default();
    unsafe {
        if GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u).is_ok() {
            let f = |t: windows::Win32::Foundation::FILETIME| {
                ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
            };
            return f(k) + f(u);
        }
    }
    0
}

/// (integrity RID, restricted token). RID: 0x0000 untrusted, 0x1000 low, 0x2000 medium, 0x3000 high.
fn token_info(h: HANDLE) -> (Option<u32>, Option<bool>) {
    unsafe {
        let mut tok = HANDLE::default();
        if OpenProcessToken(h, TOKEN_QUERY, &mut tok).is_err() {
            return (None, None);
        }
        let mut buf = vec![0u8; 256];
        let mut ret = 0u32;
        let rid = if GetTokenInformation(
            tok,
            TokenIntegrityLevel,
            Some(buf.as_mut_ptr() as *mut c_void),
            buf.len() as u32,
            &mut ret,
        )
        .is_ok()
        {
            let tml = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
            let sid = tml.Label.Sid;
            let n = *GetSidSubAuthorityCount(sid);
            Some(*GetSidSubAuthority(sid, (n - 1) as u32))
        } else {
            None
        };
        // windows-rs maps this BOOL to Result: Ok = the token has restricting SIDs.
        let restricted = IsTokenRestricted(tok).is_ok();
        let _ = CloseHandle(tok);
        (rid, Some(restricted))
    }
}

/// Starts the sampler thread: cpu.csv every second, procs.csv (with the sandbox self-check) every 10 s.
pub fn start_sampler(profile: PathBuf) {
    std::thread::spawn(move || {
        let me = std::process::id();
        let ncpu = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1) as f64;
        let mut cpu = std::fs::File::create(profile.join("cpu.csv")).ok();
        let mut procs = std::fs::File::create(profile.join("procs.csv")).ok();
        if let Some(f) = cpu.as_mut() {
            let _ = writeln!(
                f,
                "t_ms,tree_cpu_percent_of_all_cores,webview2_cpu_percent,processes"
            );
        }
        if let Some(f) = procs.as_mut() {
            let _ = writeln!(
                f,
                "t_ms,pid,ppid,exe,integrity_rid_hex,restricted_token,in_job,cpu_seconds"
            );
        }
        let mut last: std::collections::HashMap<u32, u64> = Default::default();
        let mut last_t = Instant::now();
        let mut tick = 0u64;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let dt = last_t.elapsed().as_secs_f64();
            last_t = Instant::now();
            let all = snapshot();
            let t = tree(&all, me);
            let (mut total, mut wv2) = (0f64, 0f64);
            for p in &t {
                let Ok(h) =
                    (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, p.pid) })
                else {
                    continue;
                };
                let now = cpu_100ns(h);
                let prev = last.insert(p.pid, now).unwrap_or(now);
                let pct = (now.saturating_sub(prev)) as f64 / 1e7 / dt / ncpu * 100.0;
                total += pct;
                if p.exe.eq_ignore_ascii_case("msedgewebview2.exe") {
                    wv2 += pct;
                }
                if tick % 10 == 0 {
                    let (rid, restricted) = token_info(h);
                    let mut job = BOOL(0);
                    unsafe {
                        let _ = IsProcessInJob(h, None, &mut job);
                    }
                    if let Some(f) = procs.as_mut() {
                        let _ = writeln!(
                            f,
                            "{},{},{},{},{},{},{},{:.2}",
                            crate::spikelog::now_ms(),
                            p.pid,
                            p.ppid,
                            p.exe,
                            rid.map(|r| format!("{r:#06x}"))
                                .unwrap_or_else(|| "n/a".into()),
                            restricted
                                .map(|b| b.to_string())
                                .unwrap_or_else(|| "n/a".into()),
                            job.as_bool(),
                            now as f64 / 1e7
                        );
                    }
                }
                unsafe {
                    let _ = CloseHandle(h);
                }
            }
            if let Some(f) = cpu.as_mut() {
                let _ = writeln!(
                    f,
                    "{},{:.2},{:.2},{}",
                    crate::spikelog::now_ms(),
                    total,
                    wv2,
                    t.len()
                );
                let _ = f.flush();
            }
            if tick % 10 == 0 {
                if let Some(f) = procs.as_mut() {
                    let _ = f.flush();
                }
            }
            tick += 1;
        }
    });
}

/// Other processes with our exe name (pid:parent), excluding this one. Used only to explain an init failure.
pub fn other_instances() -> String {
    let me = std::process::id();
    let list: Vec<String> = snapshot()
        .iter()
        .filter(|p| p.exe.eq_ignore_ascii_case("spike719.exe") && p.pid != me)
        .map(|p| format!("{}:{}", p.pid, p.ppid))
        .collect();
    if list.is_empty() {
        "none".into()
    } else {
        list.join(", ")
    }
}
