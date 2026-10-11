//! Approach B renderer: one wgpu (D3D12) device, one swapchain per pane child window, drawn from a dedicated
//! render thread at 60 Hz. The same thread drives CEF's external begin frames (as pane_host.rs does on Linux),
//! so CEF only paints when asked.
use std::io::Write;
use std::num::NonZeroIsize;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::bshared::{Frame, SLOT};
use crate::cefapp::{self, SLOT_COUNT};
use crate::spikelog;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Gpu {
    pub fn new() -> Result<Gpu, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            flags: wgpu::InstanceFlags::empty(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("request_adapter: {e}"))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .map_err(|e| format!("request_device: {e}"))?;

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spike bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spike shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spike layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spike pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Gpu {
            instance,
            adapter,
            device,
            queue,
            pipeline,
            bgl,
            sampler,
        })
    }

    pub fn info_json(&self) -> serde_json::Value {
        let i = self.adapter.get_info();
        serde_json::json!({
            "wgpu_backend": format!("{:?}", i.backend),
            "wgpu_adapter": i.name,
            "wgpu_driver": i.driver,
            "wgpu_driver_info": i.driver_info,
            "wgpu_device_type": format!("{:?}", i.device_type),
        })
    }

    fn bind(&self, tex: &wgpu::Texture) -> wgpu::BindGroup {
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Uploads software pixels into (a possibly recreated) texture and returns the bind group.
    fn upload(
        &self,
        cache: &mut Option<(wgpu::Texture, (u32, u32))>,
        px: &[u8],
        w: u32,
        h: u32,
    ) -> wgpu::BindGroup {
        if cache.as_ref().map(|c| c.1) != Some((w, h)) {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("spike cef sw texture"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            *cache = Some((tex, (w, h)));
        }
        let tex = &cache.as_ref().unwrap().0;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            px,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.bind(tex)
    }
}

pub enum Cmd {
    Add { slot: usize, hwnd: isize },
    Remove { slot: usize },
}

#[derive(Default)]
struct PaneSurf {
    surface: Option<wgpu::Surface<'static>>,
    cfg_size: (u32, u32),
    sw_cache: Option<(wgpu::Texture, (u32, u32))>,
    view_bind: Option<wgpu::BindGroup>,
    popup_cache: Option<(wgpu::Texture, (u32, u32))>,
    popup_bind: Option<wgpu::BindGroup>,
    need_draw: bool,
    present_ms: Vec<f32>,
}

fn create_surface(gpu: &Gpu, hwnd: isize) -> Result<wgpu::Surface<'static>, String> {
    use wgpu::rwh::{RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle};
    let nz = NonZeroIsize::new(hwnd).ok_or("null hwnd")?;
    unsafe {
        gpu.instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(RawDisplayHandle::Windows(WindowsDisplayHandle::new())),
                raw_window_handle: RawWindowHandle::Win32(Win32WindowHandle::new(nz)),
            })
    }
    .map_err(|e| format!("create_surface: {e}"))
}

fn configure(gpu: &Gpu, ps: &mut PaneSurf, size: (u32, u32)) {
    let Some(surface) = ps.surface.as_ref() else {
        return;
    };
    let Some(mut cfg) = surface.get_default_config(&gpu.adapter, size.0, size.1) else {
        log::warn!("surface has no default config");
        return;
    };
    cfg.format = FORMAT;
    cfg.present_mode = wgpu::PresentMode::Fifo;
    cfg.desired_maximum_frame_latency = 1;
    surface.configure(&gpu.device, &cfg);
    ps.cfg_size = size;
    ps.need_draw = true;
}

fn pct(v: &mut [f32], p: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[((v.len() as f32 - 1.0) * p) as usize]
}

/// Starts the render thread. `rx` carries surface add/remove commands from the main thread.
pub fn spawn(gpu: Arc<Gpu>, rx: Receiver<Cmd>, profile: PathBuf) {
    std::thread::Builder::new()
        .name("spike-render".into())
        .spawn(move || {
            let mut panes: Vec<PaneSurf> = (0..SLOT_COUNT).map(|_| PaneSurf::default()).collect();
            let mut csv = std::fs::File::create(profile.join("frames.csv")).ok();
            if let Some(f) = csv.as_mut() {
                let _ = writeln!(
                    f,
                    "t_ms,slot,cef_paints_per_s,paint_gap_avg_ms,paint_gap_p95_ms,paint_gap_max_ms,present_avg_ms,present_max_ms,import_ok,import_err"
                );
            }
            let frame_dt = Duration::from_micros(16_667);
            let mut next = Instant::now();
            let mut last_stats = Instant::now();
            loop {
                // Commands
                loop {
                    match rx.try_recv() {
                        Ok(Cmd::Add { slot, hwnd }) => match create_surface(&gpu, hwnd) {
                            Ok(s) => {
                                panes[slot] = PaneSurf {
                                    surface: Some(s),
                                    need_draw: true,
                                    ..Default::default()
                                };
                                log::info!("render: surface added for slot {slot}");
                            }
                            Err(e) => log::error!("render: slot {slot}: {e}"),
                        },
                        Ok(Cmd::Remove { slot }) => {
                            panes[slot] = PaneSurf::default();
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }

                for (i, ps) in panes.iter_mut().enumerate() {
                    if ps.surface.is_none() {
                        continue;
                    }
                    // Ask CEF for the next frame (external_begin_frame_enabled), like pane_host.rs does.
                    if let Some(h) = cefapp::pane_browser(i).and_then(|b| cef::ImplBrowser::host(&b)) {
                        cef::ImplBrowserHost::send_external_begin_frame(&h);
                    }

                    // Size
                    let want = crate::bshared::with_slot(i, |s| s.pix).unwrap_or((0, 0));
                    if want.0 > 0 && want.1 > 0 && want != ps.cfg_size {
                        configure(&gpu, ps, want);
                    }

                    // New frames
                    let (view, popup_new) = {
                        let mut s = SLOT[i].lock().unwrap();
                        let v = s.frame.take();
                        let p = s.popup.frame.as_mut().filter(|p| p.dirty).map(|p| {
                            p.dirty = false;
                            match &p.frame {
                                Frame::Software { px, w, h } => Frame::Software {
                                    px: px.clone(),
                                    w: *w,
                                    h: *h,
                                },
                                Frame::Gpu { tex } => Frame::Gpu { tex: tex.clone() },
                            }
                        });
                        (v, p)
                    };
                    if let Some(f) = view {
                        ps.view_bind = Some(match f {
                            Frame::Software { px, w, h } => gpu.upload(&mut ps.sw_cache, &px, w, h),
                            Frame::Gpu { tex } => gpu.bind(&tex),
                        });
                        ps.need_draw = true;
                    }
                    if let Some(f) = popup_new {
                        ps.popup_bind = Some(match f {
                            Frame::Software { px, w, h } => {
                                gpu.upload(&mut ps.popup_cache, &px, w, h)
                            }
                            Frame::Gpu { tex } => gpu.bind(&tex),
                        });
                        ps.need_draw = true;
                    }
                    let (popup_visible, popup_rect, scale) =
                        crate::bshared::with_slot(i, |s| (s.popup.visible, s.popup.rect, s.scale))
                            .unwrap_or((false, (0, 0, 0, 0), 1.0));

                    if !ps.need_draw || ps.view_bind.is_none() {
                        continue;
                    }
                    let t = Instant::now();
                    let st = match ps.surface.as_ref().unwrap().get_current_texture() {
                        wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                        wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                            ps.cfg_size = (0, 0); // force reconfigure next tick
                            continue;
                        }
                        _ => continue,
                    };
                    let view = st.texture.create_view(&wgpu::TextureViewDescriptor::default());
                    let mut enc = gpu
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                    {
                        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: None,
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &view,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        rp.set_pipeline(&gpu.pipeline);
                        rp.set_bind_group(0, ps.view_bind.as_ref(), &[]);
                        rp.draw(0..3, 0..1);
                        if popup_visible {
                            if let Some(pb) = ps.popup_bind.as_ref() {
                                let (cw, ch) = (ps.cfg_size.0 as f32, ps.cfg_size.1 as f32);
                                let x = (popup_rect.0 as f32 * scale).clamp(0.0, cw - 1.0);
                                let y = (popup_rect.1 as f32 * scale).clamp(0.0, ch - 1.0);
                                let w = (popup_rect.2 as f32 * scale).min(cw - x).max(1.0);
                                let h = (popup_rect.3 as f32 * scale).min(ch - y).max(1.0);
                                rp.set_viewport(x, y, w, h, 0.0, 1.0);
                                rp.set_scissor_rect(x as u32, y as u32, w as u32, h as u32);
                                rp.set_bind_group(0, Some(pb), &[]);
                                rp.draw(0..3, 0..1);
                            }
                        }
                    }
                    gpu.queue.submit(Some(enc.finish()));
                    gpu.queue.present(st);
                    ps.need_draw = false;
                    ps.present_ms.push(t.elapsed().as_secs_f32() * 1000.0);
                }

                if last_stats.elapsed() >= Duration::from_secs(1) {
                    last_stats = Instant::now();
                    for (i, ps) in panes.iter_mut().enumerate() {
                        if ps.surface.is_none() {
                            continue;
                        }
                        let (mut gaps, paints, ok, err) = crate::bshared::with_slot(i, |s| {
                            let g = std::mem::take(&mut s.gaps_ms);
                            let n = s.paints;
                            s.paints = 0;
                            (g, n, s.import_ok, s.import_err)
                        })
                        .unwrap_or_default();
                        let avg = if gaps.is_empty() { 0.0 } else { gaps.iter().sum::<f32>() / gaps.len() as f32 };
                        let p95 = pct(&mut gaps, 0.95);
                        let max = gaps.iter().cloned().fold(0.0, f32::max);
                        let pavg = if ps.present_ms.is_empty() {
                            0.0
                        } else {
                            ps.present_ms.iter().sum::<f32>() / ps.present_ms.len() as f32
                        };
                        let pmax = ps.present_ms.iter().cloned().fold(0.0, f32::max);
                        ps.present_ms.clear();
                        if let Some(f) = csv.as_mut() {
                            let _ = writeln!(
                                f,
                                "{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{},{}",
                                spikelog::now_ms(), i, paints, avg, p95, max, pavg, pmax, ok, err
                            );
                            let _ = f.flush();
                        }
                    }
                }

                next += frame_dt;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now;
                }
            }
        })
        .expect("spawn render thread");
}
