//! State shared between CEF's callback threads, the Tauri main thread and the render thread (windowless modes).
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use crate::cefapp::SLOT_COUNT;

pub enum Frame {
    /// BGRA8 pixels from `on_paint` (what Linux ships).
    Software { px: Vec<u8>, w: u32, h: u32 },
    /// A texture imported from CEF's D3D11 shared handle inside `on_accelerated_paint`.
    Gpu { tex: wgpu::Texture },
}

#[derive(Default)]
pub struct PopupWidget {
    /// CEF `<select>`-style popup surface (PET_POPUP), in view DIPs.
    pub visible: bool,
    pub rect: (i32, i32, i32, i32),
    pub frame: Option<PopupFrame>,
}

pub struct PopupFrame {
    pub frame: Frame,
    pub dirty: bool,
}

pub struct Slot {
    pub active: bool,
    /// Newest view frame not yet drawn.
    pub frame: Option<Frame>,
    pub popup: PopupWidget,
    /// Target physical pixel size of the pane (its child window's client size).
    pub pix: (u32, u32),
    /// Device scale factor (monitor DPI / 96).
    pub scale: f32,
    pub last_paint: Option<Instant>,
    /// Gaps between CEF paints (ms), drained once per second by the render thread for frames.csv.
    pub gaps_ms: Vec<f32>,
    pub paints: u64,
    pub import_ok: u64,
    pub import_err: u64,
    pub selected_text: String,
    pub cursor_css: &'static str,
    /// Raw HCURSOR for the native-input (above) placement.
    pub hcursor: isize,
}

impl Slot {
    pub const fn new() -> Self {
        Self {
            active: false,
            frame: None,
            popup: PopupWidget {
                visible: false,
                rect: (0, 0, 0, 0),
                frame: None,
            },
            pix: (0, 0),
            scale: 1.0,
            last_paint: None,
            gaps_ms: Vec::new(),
            paints: 0,
            import_ok: 0,
            import_err: 0,
            selected_text: String::new(),
            cursor_css: "default",
            hcursor: 0,
        }
    }
}

impl Default for Slot {
    fn default() -> Self {
        Self::new()
    }
}

pub static SLOT: [Mutex<Slot>; SLOT_COUNT] = [const { Mutex::new(Slot::new()) }; SLOT_COUNT];

/// The shared wgpu device, set before any browser is created so `on_accelerated_paint` can import into it.
pub static DEVICE: OnceLock<wgpu::Device> = OnceLock::new();

pub fn with_slot<R>(i: usize, f: impl FnOnce(&mut Slot) -> R) -> Option<R> {
    SLOT.get(i)
        .and_then(|m| m.lock().ok())
        .map(|mut s| f(&mut s))
}

/// CEF's view rect in DIPs for a slot.
pub fn view_dip(i: usize) -> (i32, i32) {
    with_slot(i, |s| {
        let sc = if s.scale > 0.0 { s.scale } else { 1.0 };
        (
            ((s.pix.0 as f32 / sc).round() as i32).max(1),
            ((s.pix.1 as f32 / sc).round() as i32).max(1),
        )
    })
    .unwrap_or((1, 1))
}
