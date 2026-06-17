use std::ffi::c_void;
use std::ptr;
use std::panic::{self, AssertUnwindSafe};

use khronos_egl as egl;

pub type GetProcAddr = unsafe extern "system" fn(*const u8) -> *const c_void;

pub struct EglState {
    pub instance: egl::DynamicInstance<egl::EGL1_5>,
    pub display: egl::Display,
    pub config: egl::Config,
    pub context: egl::Context,
    pub get_proc_address: GetProcAddr,
}

impl EglState {
    pub fn init(wayland_display: *mut c_void, verbose: u8) -> Self {
        let gl_versions: &[(i32, i32)] = &[
            (4, 6), (4, 5), (4, 4), (4, 3), (4, 2), (4, 1), (4, 0),
            (3, 3), (3, 2), (3, 1), (3, 0),
        ];
        let gles_versions: &[(i32, i32)] = &[
            (3, 2), (3, 1), (3, 0), (2, 0),
        ];

        // Try each EGL library: GLVND first, then vendor-specific libs
        let lib_names: &[&str] = &["libEGL.so.1", "libEGL_nvidia.so.0", "libEGL_mesa.so.0"];

        for &lib_name in lib_names {
            if verbose > 0 {
                log_info!("Trying EGL library: {}", lib_name);
            }
            let lib = match unsafe { libloading::Library::new(lib_name) } {
                Ok(l) => l,
                Err(e) => {
                    if verbose > 0 { log_info!("  failed to load: {}", e); }
                    continue;
                },
            };
            let get_proc_addr: GetProcAddr = match unsafe {
                lib.get(b"eglGetProcAddress")
            } {
                Ok(f) => *f,
                Err(e) => {
                    if verbose > 0 { log_info!("  no eglGetProcAddress: {}", e); }
                    continue;
                },
            };
            let instance = match unsafe {
                egl::DynamicInstance::<egl::EGL1_5>::load_required_from(lib)
            } {
                Ok(i) => i,
                Err(e) => {
                    if verbose > 0 { log_info!("  not EGL 1.5: {}", e); }
                    continue;
                },
            };

            let init_result = Self::try_init_with_instance(
                &instance, wayland_display, verbose,
                gl_versions, gles_versions,
            );
            if let Some((display, config, context)) = init_result {
                instance.make_current(display, None, None, Some(context))
                    .expect("Failed to make EGL context current");
                gl::load_with(|name| {
                    let name_str = name.to_string();
                    let ptr = instance.get_proc_address(&name_str);
                    ptr.map_or(ptr::null::<c_void>() as *const c_void, |p| p as *const c_void)
                });
                return Self { instance, display, config, context, get_proc_address: get_proc_addr };
            }
        }

        log_error!(
            "No usable EGL display found.\n\
             This system has NVIDIA GPU which lacks Mesa DRI driver support.\n\
             Ensure the NVIDIA EGL driver is active:\n\
               __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/10_nvidia.json\n\
             Or use DRI_PRIME=1 to prefer the AMD iGPU.");

        panic!("Failed to create any EGL context");
    }

    fn try_init_with_instance(
        instance: &egl::DynamicInstance<egl::EGL1_5>,
        wayland_display: *mut c_void,
        verbose: u8,
        gl_versions: &[(i32, i32)],
        gles_versions: &[(i32, i32)],
    ) -> Option<(egl::Display, egl::Config, egl::Context)> {
        let try_display = |d: egl::Display| -> Option<egl::Display> {
            if instance.initialize(d).is_err() {
                if verbose > 0 { log_info!("  initialize failed"); }
                return None;
            }
            let mut cfgs = Vec::with_capacity(64);
            if instance.choose_config(d, &[egl::NONE], &mut cfgs).is_ok() {
                if cfgs.is_empty() {
                    if verbose > 0 { log_info!("  choose_config OK but 0 configs"); }
                    let _ = instance.terminate(d);
                    None
                } else {
                    if verbose > 0 { log_info!("  choose_config OK with {} configs", cfgs.len()); }
                    Some(d)
                }
            } else {
                if verbose > 0 { log_info!("  choose_config failed"); }
                let _ = instance.terminate(d);
                None
            }
        };

        let display = 'disp: {
            // 1) get_platform_display with Wayland display (wrap in catch_unwind
            //    because khronos-egl panics on EGL_NO_DISPLAY + None error)
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                unsafe {
                    instance.get_platform_display(0x31D8, wayland_display, &[egl::ATTRIB_NONE])
                }
            }));
            match &result {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => { if verbose > 0 { log_info!("get_platform_display(Wayland) failed: {:?}", e); }}
                Err(_) => { if verbose > 0 { log_info!("get_platform_display(Wayland) panicked"); }}
            }
            if let Ok(Ok(d)) = result {
                if let Some(d) = try_display(d) {
                    if verbose > 0 { log_info!("Using EGL_PLATFORM_WAYLAND_KHR"); }
                    break 'disp d;
                }
            }

            // 2) eglGetDisplay with Wayland pointer (safe)
            if let Some(d) = unsafe { instance.get_display(wayland_display) } {
                if let Some(d) = try_display(d) {
                    if verbose > 0 { log_info!("Using eglGetDisplay(Wayland)"); }
                    break 'disp d;
                }
            } else if verbose > 0 {
                log_info!("eglGetDisplay(Wayland) returned None");
            }

            // 3) get_platform_display with null display
            for &(platform, label) in &[
                (0x31D8_u32, "EGL_PLATFORM_WAYLAND_KHR + null"),
                (0x31D7_u32, "EGL_PLATFORM_GBM_KHR + null"),
            ] {
                match unsafe { instance.get_platform_display(platform, std::ptr::null_mut(), &[egl::ATTRIB_NONE]) } {
                    Ok(d) => {
                        if let Some(d) = try_display(d) {
                            if verbose > 0 { log_info!("Using {}", label); }
                            break 'disp d;
                        }
                    }
                    Err(e) => if verbose > 0 { log_info!("{} failed: {:?}", label, e); }
                }
            }

            // 4) eglGetDisplay with EGL_DEFAULT_DISPLAY
            if let Some(d) = unsafe { instance.get_display(std::ptr::null_mut()) } {
                if let Some(d) = try_display(d) {
                    if verbose > 0 { log_info!("Using eglGetDisplay(EGL_DEFAULT_DISPLAY)"); }
                    break 'disp d;
                }
            } else if verbose > 0 {
                log_info!("eglGetDisplay(null) returned None");
            }

            return None;
        };

        let mut configs = Vec::with_capacity(8);
        let win_attribs = [
            egl::SURFACE_TYPE as i32, egl::WINDOW_BIT as i32,
            egl::RENDERABLE_TYPE as i32, egl::OPENGL_BIT as i32,
            egl::RED_SIZE as i32, 8,
            egl::GREEN_SIZE as i32, 8,
            egl::BLUE_SIZE as i32, 8,
            egl::ALPHA_SIZE as i32, 8,
            egl::NONE as i32,
        ];
        let _ = instance.choose_config(display, &win_attribs, &mut configs);
        if verbose > 0 {
            log_info!("Found {} configs", configs.len());
        }

        // Try config-based context creation: OpenGL first, then GLES
        for &(major, minor) in gl_versions {
            for &cfg in &configs {
                if instance.bind_api(egl::OPENGL_API).is_err() {
                    break;
                }
                let ctx_attribs = [
                    egl::CONTEXT_MAJOR_VERSION as i32, major,
                    egl::CONTEXT_MINOR_VERSION as i32, minor,
                    egl::NONE as i32,
                ];
                match instance.create_context(display, cfg, None, &ctx_attribs) {
                    Ok(ctx) => {
                        if verbose > 0 {
                            log_info!("EGL context created: {}.{}", major, minor);
                        }
                        return Some((display, cfg, ctx));
                    }
                    Err(_) => {}
                }
            }
        }

        // Try GLES if OpenGL failed
        for &(major, minor) in gles_versions {
            for &cfg in &configs {
                if instance.bind_api(egl::OPENGL_ES_API).is_err() {
                    break;
                }
                let ctx_attribs = [
                    egl::CONTEXT_MAJOR_VERSION as i32, major,
                    egl::CONTEXT_MINOR_VERSION as i32, minor,
                    egl::NONE as i32,
                ];
                match instance.create_context(display, cfg, None, &ctx_attribs) {
                    Ok(ctx) => {
                        if verbose > 0 {
                            log_info!("EGL GLES context created: {}.{}", major, minor);
                        }
                        return Some((display, cfg, ctx));
                    }
                    Err(_) => {}
                }
            }
        }

        // Fallback: try no-config context with OpenGL
        let null_cfg = unsafe { egl::Config::from_ptr(std::ptr::null_mut()) };
        for &(major, minor) in gl_versions {
            if instance.bind_api(egl::OPENGL_API).is_err() {
                break;
            }
            let ctx_attribs = [
                egl::CONTEXT_MAJOR_VERSION as i32, major,
                egl::CONTEXT_MINOR_VERSION as i32, minor,
                egl::NONE as i32,
            ];
            match instance.create_context(display, null_cfg, None, &ctx_attribs) {
                Ok(ctx) => {
                    if verbose > 0 {
                        log_info!("EGL no-config context created: {}.{}", major, minor);
                    }
                    return Some((display, null_cfg, ctx));
                }
                Err(_) => {}
            }
        }

        None
    }

    pub fn create_surface_for_egl_window(&self, egl_window: *mut c_void) -> Option<egl::Surface> {
        unsafe {
            self.instance
                .create_window_surface(self.display, self.config, egl_window, None)
                .ok()
        }
    }

    pub fn make_current(&self, surface: Option<egl::Surface>) -> bool {
        self.instance
            .make_current(self.display, surface, surface, Some(self.context))
            .is_ok()
    }

    pub fn swap_buffers(&self, surface: egl::Surface) -> bool {
        self.instance.swap_buffers(self.display, surface).is_ok()
    }

    pub fn swap_interval(&self, interval: i32) {
        let _ = self.instance.swap_interval(self.display, interval);
    }
}

impl Drop for EglState {
    fn drop(&mut self) {
        let _ = self.instance.destroy_context(self.display, self.context);
    }
}
