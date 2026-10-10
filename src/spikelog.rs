//! Tiny file logger + the CSV event log. No telemetry, no network: everything stays in the profile dir.
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static T0: OnceLock<Instant> = OnceLock::new();
static LOG: OnceLock<Mutex<File>> = OnceLock::new();
static EVENTS: OnceLock<Mutex<File>> = OnceLock::new();

pub fn now_ms() -> u128 {
    T0.get_or_init(Instant::now).elapsed().as_millis()
}

struct FileLogger;

impl log::Log for FileLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        if let Some(f) = LOG.get() {
            if let Ok(mut f) = f.lock() {
                let _ = writeln!(f, "{:>8} {:<5} {}", now_ms(), r.level(), r.args());
            }
        }
    }
    fn flush(&self) {}
}

static LOGGER: FileLogger = FileLogger;

pub fn init(dir: &Path) {
    let _ = T0.get_or_init(Instant::now);
    let _ = std::fs::create_dir_all(dir);
    let open = |name: &str| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(name))
    };
    if let Ok(f) = open("spike.log") {
        let _ = LOG.set(Mutex::new(f));
    }
    if let Ok(mut f) = open("events.csv") {
        let _ = writeln!(f, "t_ms,kind,detail");
        let _ = EVENTS.set(Mutex::new(f));
    }
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
}

/// One row in events.csv (markers, layout/modal changes, SetWindowPos timings). Aligns with cpu.csv by t_ms.
pub fn event(kind: &str, detail: &str) {
    if let Some(f) = EVENTS.get() {
        if let Ok(mut f) = f.lock() {
            let _ = writeln!(f, "{},{},\"{}\"", now_ms(), kind, detail.replace('"', "'"));
        }
    }
}
