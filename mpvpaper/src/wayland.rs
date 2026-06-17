use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

use wayland_client::protocol::{wl_callback, wl_compositor, wl_output, wl_region, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

extern "C" {
    fn wl_egl_window_create(surface: *mut c_void, width: i32, height: i32) -> *mut c_void;
    fn wl_egl_window_resize(window: *mut c_void, width: i32, height: i32, dx: i32, dy: i32);
}

#[link(name = "wayland-egl")]
extern "C" {}

use crate::cli::SurfaceLayer;

use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

pub struct DisplayOutput {
    pub wl_name: u32,
    pub wl_output: wl_output::WlOutput,
    pub name: Option<String>,
    pub identifier: Option<String>,
    pub surface: Option<wl_surface::WlSurface>,
    pub layer_surface: Option<ZwlrLayerSurfaceV1>,
    pub egl_window: Option<*mut c_void>,
    pub egl_surface: Option<*mut c_void>,
    pub width: u32,
    pub height: u32,
    pub scale: i32,
    pub frame_callback: Option<wl_callback::WlCallback>,
    pub redraw_needed: bool,
}

impl DisplayOutput {
    fn new(wl_name: u32, wl_output: wl_output::WlOutput) -> Self {
        Self {
            wl_name,
            wl_output,
            name: None,
            identifier: None,
            surface: None,
            layer_surface: None,
            egl_window: None,
            egl_surface: None,
            width: 0,
            height: 0,
            scale: 1,
            frame_callback: None,
            redraw_needed: false,
        }
    }
}

pub struct WaylandState {
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub layer_shell: Option<ZwlrLayerShellV1>,
    pub outputs: Vec<DisplayOutput>,
    pub monitor: String,
    pub surface_layer: SurfaceLayer,
    pub show_outputs: bool,
    pub verbose: u8,
    pub halt_info: Arc<HaltInfo>,
    pub egl_initialized: bool,
}

pub struct HaltInfo {
    pub stop_render_loop: AtomicBool,
    pub frame_ready: AtomicBool,
    pub is_paused: AtomicI32,
    pub auto_pause: AtomicBool,
    pub auto_stop: AtomicBool,
}

impl Default for HaltInfo {
    fn default() -> Self {
        Self {
            stop_render_loop: AtomicBool::new(false),
            frame_ready: AtomicBool::new(false),
            is_paused: AtomicI32::new(0),
            auto_pause: AtomicBool::new(false),
            auto_stop: AtomicBool::new(false),
        }
    }
}

impl WaylandState {
    pub fn new(monitor: String, surface_layer: SurfaceLayer, show_outputs: bool, verbose: u8, halt_info: Arc<HaltInfo>) -> Self {
        Self {
            compositor: None,
            layer_shell: None,
            outputs: Vec::new(),
            monitor,
            surface_layer,
            show_outputs,
            verbose,
            halt_info,
            egl_initialized: false,
        }
    }

    fn find_output_by_wl_output(&self, wl_output: &wl_output::WlOutput) -> Option<usize> {
        self.outputs.iter().position(|o| o.wl_output == *wl_output)
    }

    fn find_output_by_name(&self, wl_name: u32) -> Option<usize> {
        self.outputs.iter().position(|o| o.wl_name == wl_name)
    }

    fn output_matches_monitor(&self, name: &Option<String>, identifier: &Option<String>) -> bool {
        let monitor = &self.monitor;
        if monitor.is_empty() {
            return false;
        }
        if monitor == "*" || monitor == "ALL" || monitor == "All" || monitor == "all" {
            return true;
        }
        if let Some(n) = name {
            if monitor.contains(n.as_str()) {
                return true;
            }
        }
        if let Some(id) = identifier {
            if !id.is_empty() && monitor.contains(id.as_str()) {
                return true;
            }
        }
        false
    }

    fn create_layer_surface(&mut self, idx: usize, qh: &QueueHandle<Self>) {
        let compositor = self.compositor.as_ref().unwrap();
        let layer_shell = self.layer_shell.as_ref().unwrap();
        let output = &mut self.outputs[idx];

        let surface = compositor.create_surface(qh, ());
        let input_region = compositor.create_region(qh, ());
        surface.set_input_region(Some(&input_region));
        input_region.destroy();

        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            Some(&output.wl_output),
            self.surface_layer.to_wlr_layer(),
            "mpvpaper".to_string(),
            qh,
            (),
        );

        layer_surface.set_size(0, 0);
        layer_surface.set_anchor(
            zwlr_layer_surface_v1::Anchor::Top
                | zwlr_layer_surface_v1::Anchor::Right
                | zwlr_layer_surface_v1::Anchor::Bottom
                | zwlr_layer_surface_v1::Anchor::Left,
        );
        layer_surface.set_exclusive_zone(-1);

        output.surface = Some(surface);
        output.layer_surface = Some(layer_surface);

        if let Some(ref surface) = output.surface {
            surface.commit();
        }
    }
}

// --- Registry ---
impl Dispatch<wl_registry::WlRegistry, ()> for WaylandState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, .. } => match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind::<wl_compositor::WlCompositor, _, _>(name, 4, qh, ()));
                }
                "wl_output" => {
                    let wl_output = registry.bind::<wl_output::WlOutput, _, _>(name, 4, qh, ());
                    state.outputs.push(DisplayOutput::new(name, wl_output));
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(registry.bind::<ZwlrLayerShellV1, _, _>(name, 1, qh, ()));
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(idx) = state.find_output_by_name(name) {
                    let output = &state.outputs[idx];
                    if state.verbose > 0 {
                        log_info!(
                            "Destroying output {} ({})",
                            output.name.as_deref().unwrap_or(""),
                            output.identifier.as_deref().unwrap_or("")
                        );
                    }
                    state.outputs.swap_remove(idx);
                }
            }
            _ => {}
        }
    }
}

// --- Output ---
impl Dispatch<wl_output::WlOutput, ()> for WaylandState {
    fn event(
        state: &mut Self,
        wl_output: &wl_output::WlOutput,
        event: wl_output::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let idx = match state.find_output_by_wl_output(wl_output) {
            Some(i) => i,
            None => return,
        };

        match event {
            wl_output::Event::Name { name } => {
                state.outputs[idx].name = Some(name);
            }
            wl_output::Event::Description { description } => {
                let identifier = if let Some(paren_pos) = description.rfind('(') {
                    let trimmed = &description[..paren_pos];
                    if !trimmed.is_empty() {
                        trimmed[..trimmed.len() - 1].to_string()
                    } else {
                        trimmed.to_string()
                    }
                } else {
                    description
                };
                state.outputs[idx].identifier = Some(identifier);
            }
            wl_output::Event::Scale { factor } => {
                state.outputs[idx].scale = factor;
            }
            wl_output::Event::Done => {
                let name = state.outputs[idx].name.clone();
                let identifier = state.outputs[idx].identifier.clone();
                let has_layer_surface = state.outputs[idx].layer_surface.is_some();
                let name_ok = state.output_matches_monitor(&name, &identifier);

                if name_ok && !has_layer_surface {
                    if state.verbose > 0 {
                        log_info!(
                            "Output {} ({}) selected",
                            name.as_deref().unwrap_or(""),
                            identifier.as_deref().unwrap_or("")
                        );
                    }
                    state.create_layer_surface(idx, qh);
                }

                if !name_ok || state.monitor.is_empty() {
                    if state.show_outputs {
                        log_info!(
                            "Output: {}  Identifier: {}",
                            name.as_deref().unwrap_or(""),
                            identifier.as_deref().unwrap_or("")
                        );
                    }
                    if !state.show_outputs {
                        state.outputs.swap_remove(idx);
                    }
                }
            }
            _ => {}
        }
    }
}

// --- Layer Surface ---
impl Dispatch<ZwlrLayerSurfaceV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let idx = match state.outputs.iter().position(|o| o.layer_surface.as_ref() == Some(layer_surface)) {
            Some(i) => i,
            None => return,
        };

        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                let output = &mut state.outputs[idx];
                output.width = width;
                output.height = height;
                layer_surface.ack_configure(serial);

                if let Some(ref surface) = output.surface {
                    surface.set_buffer_scale(output.scale);
                }

                if output.egl_window.is_none() && state.egl_initialized {
                    let scale = output.scale;
                    let w = output.width as i32 * scale;
                    let h = output.height as i32 * scale;

                    let surface_ref = output.surface.as_ref().unwrap();
                    unsafe {
                        let egl_window = wl_egl_window_create(
                            surface_ref.id().as_ptr() as *mut c_void,
                            w,
                            h,
                        );
                        output.egl_window = Some(egl_window);
                    }
                } else if let Some(egl_win) = output.egl_window {
                    let scale = output.scale;
                    unsafe {
                        wl_egl_window_resize(
                            egl_win,
                            output.width as i32 * scale,
                            output.height as i32 * scale,
                            0,
                            0,
                        );
                    }
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {
                let output = &state.outputs[idx];
                if state.verbose > 0 {
                    log_info!(
                        "Destroying output {} ({})",
                        output.name.as_deref().unwrap_or(""),
                        output.identifier.as_deref().unwrap_or("")
                    );
                }
                state.outputs.swap_remove(idx);
            }
            _ => {}
        }
    }
}

// --- Frame Callback ---
impl Dispatch<wl_callback::WlCallback, ()> for WaylandState {
    fn event(
        state: &mut Self,
        callback: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            let idx = match state.outputs.iter().position(|o| {
                o.frame_callback.as_ref().map(|cb| cb.id()) == Some(callback.id())
            }) {
                Some(i) => i,
                None => return,
            };

            state.outputs[idx].frame_callback = None;
            state.halt_info.frame_ready.store(true, Ordering::Release);

            // Signal main loop to render if a redraw was pending while the
            // compositor was busy with the previous buffer. This mirrors the
            // C version which calls render() directly in the done handler.
            if state.outputs[idx].redraw_needed {
                state.outputs[idx].redraw_needed = false;
                if let Some(&fd) = crate::WAKEUP_FD.get() {
                    let inc: u64 = 1;
                    unsafe { libc::write(fd, &inc as *const u64 as *const c_void, 8); }
                }
            }
        }
    }
}

// --- No-op Dispatch impls for protocol objects we don't handle events for ---
impl Dispatch<wl_compositor::WlCompositor, ()> for WaylandState {
    fn event(_: &mut Self, _: &wl_compositor::WlCompositor, _: wl_compositor::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwlrLayerShellV1, ()> for WaylandState {
    fn event(_: &mut Self, _: &ZwlrLayerShellV1, _: zwlr_layer_shell_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_surface::WlSurface, ()> for WaylandState {
    fn event(_: &mut Self, _: &wl_surface::WlSurface, _: wl_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_region::WlRegion, ()> for WaylandState {
    fn event(_: &mut Self, _: &wl_region::WlRegion, _: wl_region::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
