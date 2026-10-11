//! Approach B: windowless (off-screen) CEF handlers, browser creation, and the input-forwarding functions that
//! both input paths share (the DOM hit-layer IPC commands in host.rs, and the native WndProc in bwin.rs).
//! The render-handler shape follows quietrabbit-core's cloud_chat_gpu_pane/render.rs.
use std::path::Path;
use std::sync::OnceLock;
use std::time::Instant;

use cef::sys::HCURSOR;
use cef::Frame as CefFrame;
use cef::*;
use tauri::{AppHandle, Emitter};

use crate::bshared::{self, with_slot, Frame, PopupFrame, DEVICE};
use crate::cefapp::{self, POPUP_SLOT};
use crate::cli::{Cli, Mode, Runtime};

/// Set once at startup; lets CEF-thread callbacks emit events to the webview and hop to the main thread.
pub static APP: OnceLock<AppHandle> = OnceLock::new();

#[derive(Clone)]
pub struct BSlot {
    pub slot: usize,
}

fn record_paint(slot: usize) {
    with_slot(slot, |s| {
        let now = Instant::now();
        if let Some(prev) = s.last_paint {
            s.gaps_ms
                .push(now.duration_since(prev).as_secs_f32() * 1000.0);
        }
        s.last_paint = Some(now);
        s.paints += 1;
    });
}

// ------------------------------------------------------------------ RenderHandler

wrap_render_handler! {
    pub(crate) struct BRenderBuilder {
        handler: BSlot,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(rect) = rect {
                let (w, h) = bshared::view_dip(self.handler.slot);
                rect.x = 0;
                rect.y = 0;
                rect.width = w;
                rect.height = h;
            }
        }

        fn screen_info(
            &self,
            _browser: Option<&mut Browser>,
            screen_info: Option<&mut ScreenInfo>,
        ) -> ::std::os::raw::c_int {
            let Some(si) = screen_info else { return false as _ };
            si.device_scale_factor = with_slot(self.handler.slot, |s| s.scale).unwrap_or(1.0);
            true as _
        }

        fn screen_point(
            &self,
            _browser: Option<&mut Browser>,
            view_x: ::std::os::raw::c_int,
            view_y: ::std::os::raw::c_int,
            screen_x: Option<&mut ::std::os::raw::c_int>,
            screen_y: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let (Some(sx), Some(sy)) = (screen_x, screen_y) else { return false as _ };
            let scale = with_slot(self.handler.slot, |s| s.scale).unwrap_or(1.0);
            let (ox, oy) = crate::bwin::slot_origin(self.handler.slot);
            *sx = ox + (view_x as f32 * scale) as i32;
            *sy = oy + (view_y as f32 * scale) as i32;
            true as _
        }

        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: ::std::os::raw::c_int) {
            with_slot(self.handler.slot, |s| {
                s.popup.visible = show != 0;
                if show == 0 {
                    s.popup.frame = None;
                }
            });
        }

        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&Rect>) {
            if let Some(r) = rect {
                with_slot(self.handler.slot, |s| s.popup.rect = (r.x, r.y, r.width, r.height));
            }
        }

        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            if buffer.is_null() || width <= 0 || height <= 0 {
                return;
            }
            let px = unsafe { std::slice::from_raw_parts(buffer, (width * height * 4) as usize) }.to_vec();
            let frame = Frame::Software { px, w: width as u32, h: height as u32 };
            if type_ == PaintElementType::POPUP {
                with_slot(self.handler.slot, |s| {
                    s.popup.frame = Some(PopupFrame { frame, dirty: true });
                });
            } else {
                record_paint(self.handler.slot);
                with_slot(self.handler.slot, |s| s.frame = Some(frame));
            }
        }

        fn on_accelerated_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            info: Option<&AcceleratedPaintInfo>,
        ) {
            use cef::osr_texture_import::shared_texture_handle::SharedTextureHandle;
            let (Some(info), Some(device)) = (info, DEVICE.get()) else { return };
            // Import inside the callback: the shared handle is only guaranteed valid for its duration.
            match SharedTextureHandle::new(info).import_texture(device) {
                Ok(tex) => {
                    let slot = self.handler.slot;
                    if type_ == PaintElementType::POPUP {
                        with_slot(slot, |s| {
                            s.popup.frame = Some(PopupFrame { frame: Frame::Gpu { tex }, dirty: true });
                        });
                    } else {
                        record_paint(slot);
                        with_slot(slot, |s| {
                            s.import_ok += 1;
                            s.frame = Some(Frame::Gpu { tex });
                        });
                    }
                }
                Err(e) => {
                    with_slot(self.handler.slot, |s| s.import_err += 1);
                    log::warn!("slot {}: shared texture import failed: {e:?}", self.handler.slot);
                }
            }
        }

        fn on_text_selection_changed(
            &self,
            _browser: Option<&mut Browser>,
            selected_text: Option<&CefString>,
            _selected_range: Option<&Range>,
        ) {
            let text = selected_text.map(|s| s.to_string()).unwrap_or_default();
            with_slot(self.handler.slot, |s| s.selected_text = text);
        }
    }
}

// ------------------------------------------------------------------ DisplayHandler (cursor)

fn css_cursor(t: &CursorType) -> &'static str {
    if *t == CursorType::HAND {
        "pointer"
    } else if *t == CursorType::IBEAM {
        "text"
    } else if *t == CursorType::CROSS {
        "crosshair"
    } else if *t == CursorType::WAIT {
        "wait"
    } else if *t == CursorType::HELP {
        "help"
    } else {
        "default"
    }
}

wrap_display_handler! {
    pub(crate) struct BDisplayBuilder {
        handler: BSlot,
    }

    impl DisplayHandler {
        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            cursor: HCURSOR,
            type_: CursorType,
            _custom_cursor_info: Option<&CursorInfo>,
        ) -> ::std::os::raw::c_int {
            let css = css_cursor(&type_);
            let slot = self.handler.slot;
            let changed = with_slot(slot, |s| {
                let c = s.cursor_css != css;
                s.cursor_css = css;
                s.hcursor = cursor as isize;
                c
            })
            .unwrap_or(false);
            if changed {
                if let Some(app) = APP.get() {
                    let _ = app.emit("pane-cursor", serde_json::json!({ "slot": slot, "css": css }));
                }
            }
            true as _
        }
    }
}

// ------------------------------------------------------------------ ContextMenuHandler

wrap_context_menu_handler! {
    pub(crate) struct BContextMenuBuilder {
        handler: BSlot,
    }

    impl ContextMenuHandler {
        fn run_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut CefFrame>,
            params: Option<&mut ContextMenuParams>,
            _model: Option<&mut MenuModel>,
            callback: Option<&mut RunContextMenuCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(params), Some(callback)) = (params, callback) else { return 0 };
            let sel = CefString::from(&params.selection_text()).to_string();
            let (x, y) = (params.xcoord(), params.ycoord());
            let editable = params.is_editable() != 0;
            let callback = callback.clone();
            let slot = self.handler.slot;
            let Some(app) = APP.get() else { return 0 };
            // The Win32 menu must run on the thread that owns the parent window.
            let _ = app.run_on_main_thread(move || {
                crate::bwin::show_context_menu(slot, x, y, editable, !sel.is_empty(), callback);
            });
            1
        }
    }
}

// ------------------------------------------------------------------ Client + LifeSpan

wrap_life_span_handler! {
    pub(crate) struct BLifeSpanBuilder {
        handler: BSlot,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser else { return };
            log::info!("slot {} windowless browser created", self.handler.slot);
            cefapp::register(self.handler.slot, browser.clone());
            if self.handler.slot == POPUP_SLOT {
                crate::bwin::popup_opened();
            }
        }

        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            log::info!("slot {} windowless browser closed", self.handler.slot);
            cefapp::unregister(self.handler.slot);
            if self.handler.slot == POPUP_SLOT {
                crate::bwin::popup_closed();
            }
        }

        // window.open() popups (OAuth): keep them windowless and host-managed, as quietrabbit-core's
        // render.rs does on Linux; the host shows them in slot 2.
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut CefFrame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            window_info: Option<&mut WindowInfo>,
            client: Option<&mut Option<Client>>,
            settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            log::info!(
                "slot {} on_before_popup url={:?}",
                self.handler.slot,
                target_url.map(|u| u.to_string())
            );
            if self.handler.slot == POPUP_SLOT || cefapp::pane_browser(POPUP_SLOT).is_some() {
                return 1; // one popup at a time; nested ones are cancelled
            }
            let cli = crate::cli::get();
            if let Some(wi) = window_info {
                windowless_info(wi, cli);
            }
            if let Some(s) = settings {
                s.windowless_frame_rate = 60;
            }
            if let Some(slot) = client {
                *slot = Some(client_for(POPUP_SLOT));
            }
            0
        }
    }
}

wrap_client! {
    pub(crate) struct BClientBuilder {
        render_handler: RenderHandler,
        display_handler: DisplayHandler,
        life_span_handler: LifeSpanHandler,
        context_menu_handler: ContextMenuHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<cef::RenderHandler> {
            Some(self.render_handler.clone())
        }
        fn display_handler(&self) -> Option<cef::DisplayHandler> {
            Some(self.display_handler.clone())
        }
        fn life_span_handler(&self) -> Option<cef::LifeSpanHandler> {
            Some(self.life_span_handler.clone())
        }
        fn context_menu_handler(&self) -> Option<cef::ContextMenuHandler> {
            Some(self.context_menu_handler.clone())
        }
    }
}

fn client_for(slot: usize) -> Client {
    let h = BSlot { slot };
    BClientBuilder::new(
        BRenderBuilder::new(h.clone()),
        BDisplayBuilder::new(h.clone()),
        BLifeSpanBuilder::new(h.clone()),
        BContextMenuBuilder::new(h),
    )
}

fn windowless_info(wi: &mut WindowInfo, cli: &Cli) {
    wi.windowless_rendering_enabled = true as _;
    wi.shared_texture_enabled = matches!(cli.mode, Mode::BGpuAbove | Mode::BGpuBelow) as _;
    wi.external_begin_frame_enabled = true as _;
    wi.runtime_style = match cli.runtime {
        Runtime::Alloy => RuntimeStyle::ALLOY,
        Runtime::Chrome => RuntimeStyle::CHROME,
    };
    #[cfg(windows)]
    if let Some(st) = crate::bwin::state() {
        wi.parent_window = cef::sys::HWND(st.parent as *mut cef::sys::HWND__);
    }
}

/// Creates pane `idx` as a windowless browser.
pub fn create_windowless_pane(cli: &Cli, idx: usize, url: &str, cache_root: &Path) -> bool {
    let mut wi = WindowInfo::default();
    windowless_info(&mut wi, cli);
    let bs = BrowserSettings {
        windowless_frame_rate: 60,
        ..Default::default()
    };
    let mut client = client_for(idx);
    // Linux (Chrome runtime) cannot give a pane its own RequestContext (decisions.id=711); the Alloy runtime can.
    let mut ctx = if cli.runtime == Runtime::Alloy {
        let st = RequestContextSettings {
            cache_path: CefString::from(
                cache_root
                    .join(format!("pane{idx}"))
                    .to_string_lossy()
                    .as_ref(),
            ),
            persist_session_cookies: 1,
            ..Default::default()
        };
        request_context_create_context(Some(&st), None)
    } else {
        None
    };
    let url = CefString::from(url);
    let r = browser_host_create_browser(
        Some(&wi),
        Some(&mut client),
        Some(&url),
        Some(&bs),
        None,
        ctx.as_mut(),
    );
    log::info!("slot {idx}: windowless browser_host_create_browser -> {r}");
    r != 0
}

// ------------------------------------------------------------------ input (shared by both input paths)

fn host(slot: usize) -> Option<BrowserHost> {
    cefapp::pane_browser(slot).and_then(|b| b.host())
}

pub fn mouse_move(slot: usize, x: i32, y: i32, leave: bool, mods: u32) {
    if let Some(h) = host(slot) {
        h.send_mouse_move_event(
            Some(&MouseEvent {
                x,
                y,
                modifiers: mods,
            }),
            leave as _,
        );
    }
}

/// Last button-down, for click counting. DOM PointerEvents carry no click count and the native path only sees
/// separate WM_*BUTTONDOWN messages, so double/triple clicks (word/line selection) are counted here for both.
type LastDown = Option<(Instant, usize, i32, i32, u32, i32)>;
static LAST_DOWN: std::sync::Mutex<LastDown> = std::sync::Mutex::new(None);

pub fn mouse_click(
    slot: usize,
    x: i32,
    y: i32,
    button: MouseButtonType,
    up: bool,
    _count: i32,
    mods: u32,
) {
    let b = button.get_raw() as u32;
    let count = if let Ok(mut last) = LAST_DOWN.lock() {
        if !up {
            let c = match *last {
                Some((t, s, lx, ly, lb, lc))
                    if s == slot
                        && lb == b
                        && t.elapsed().as_millis() <= 500
                        && (lx - x).abs() <= 4
                        && (ly - y).abs() <= 4 =>
                {
                    (lc % 3) + 1
                }
                _ => 1,
            };
            *last = Some((Instant::now(), slot, x, y, b, c));
            c
        } else {
            last.map(|l| l.5).unwrap_or(1)
        }
    } else {
        1
    };
    if let Some(h) = host(slot) {
        h.send_mouse_click_event(
            Some(&MouseEvent {
                x,
                y,
                modifiers: mods,
            }),
            button,
            up as _,
            count,
        );
    }
}

pub fn mouse_wheel(slot: usize, x: i32, y: i32, dx: i32, dy: i32, mods: u32) {
    if let Some(h) = host(slot) {
        h.send_mouse_wheel_event(
            Some(&MouseEvent {
                x,
                y,
                modifiers: mods,
            }),
            dx,
            dy,
        );
    }
}

pub fn set_focus(slot: usize, on: bool) {
    if let Some(h) = host(slot) {
        h.set_focus(on as _);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn key(
    slot: usize,
    kind: KeyEventType,
    vk: i32,
    native: i32,
    ch: u16,
    mods: u32,
    system: bool,
) {
    if let Some(h) = host(slot) {
        let ev = KeyEvent {
            size: std::mem::size_of::<KeyEvent>(),
            type_: kind,
            modifiers: mods,
            windows_key_code: vk,
            native_key_code: native,
            is_system_key: system as _,
            character: ch,
            unmodified_character: ch,
            focus_on_editable_field: 0,
        };
        h.send_key_event(Some(&ev));
    }
}

pub fn ime_update(slot: usize, text: &str) {
    if let Some(h) = host(slot) {
        let n = text.encode_utf16().count() as u32;
        h.ime_set_composition(
            Some(&CefString::from(text)),
            None,
            None,
            Some(&Range { from: n, to: n }),
        );
    }
}

pub fn ime_commit(slot: usize, text: &str) {
    if let Some(h) = host(slot) {
        h.ime_commit_text(Some(&CefString::from(text)), None, 0);
    }
}

/// Edit commands that a windowless browser does not get from OS accelerators (Ctrl+C/X/V/A).
/// Copy goes through the selection cache (OnTextSelectionChanged) like Linux's items.id=369 path.
pub fn edit_command(slot: usize, cmd: &str) {
    match cmd {
        "copy" => {
            let text = with_slot(slot, |s| s.selected_text.clone()).unwrap_or_default();
            if !text.is_empty() {
                crate::bwin::write_clipboard(&text);
            }
        }
        _ => {
            let Some(f) = cefapp::pane_browser(slot).and_then(|b| b.main_frame()) else {
                return;
            };
            match cmd {
                "cut" => {
                    let text = with_slot(slot, |s| s.selected_text.clone()).unwrap_or_default();
                    if !text.is_empty() {
                        crate::bwin::write_clipboard(&text);
                    }
                    f.cut();
                }
                "paste" => f.paste(),
                "select_all" => f.select_all(),
                _ => {}
            }
        }
    }
}
