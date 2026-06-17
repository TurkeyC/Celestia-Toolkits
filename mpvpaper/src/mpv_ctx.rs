use std::ffi::{c_void, CStr, CString};
use std::ptr;

use libmpv_sys::*;

use crate::egl::GetProcAddr;

pub struct MpvContext {
    pub handle: *mut mpv_handle,
}

unsafe impl Send for MpvContext {}
unsafe impl Sync for MpvContext {}

impl MpvContext {
    pub fn new() -> Self {
        let handle = unsafe { mpv_create() };
        if handle.is_null() {
            log_error!("Failed creating mpv context");
            std::process::exit(1);
        }
        Self { handle }
    }

    pub fn set_option_string(&self, name: &str, value: &str) {
        let name_c = CString::new(name).unwrap();
        let value_c = CString::new(value).unwrap();
        unsafe {
            mpv_set_option_string(self.handle, name_c.as_ptr(), value_c.as_ptr());
        }
    }

    pub fn initialize(&self) {
        let err = unsafe { mpv_initialize(self.handle) };
        if err < 0 {
            log_error!("Failed to init mpv, {}", mpv_error_str(err));
            std::process::exit(1);
        }
    }

    pub fn get_property_string(&self, name: &str) -> Option<String> {
        let name_c = CString::new(name).unwrap();
        unsafe {
            let val = mpv_get_property_string(self.handle, name_c.as_ptr());
            if val.is_null() {
                None
            } else {
                let s = CStr::from_ptr(val).to_string_lossy().into_owned();
                mpv_free(val as *mut c_void);
                Some(s)
            }
        }
    }

    pub fn get_property_flag(&self, name: &str) -> Option<i32> {
        let name_c = CString::new(name).unwrap();
        let mut val: i32 = 0;
        let err = unsafe {
            mpv_get_property(
                self.handle,
                name_c.as_ptr(),
                mpv_format_MPV_FORMAT_FLAG,
                &mut val as *mut i32 as *mut c_void,
            )
        };
        if err >= 0 { Some(val) } else { None }
    }

    pub fn command(&self, args: &[&str]) -> i32 {
        let c_args: Vec<CString> = args.iter().map(|a| CString::new(*a).unwrap()).collect();
        let mut ptrs: Vec<*const i8> = c_args.iter().map(|a| a.as_ptr()).collect();
        ptrs.push(ptr::null());
        unsafe { mpv_command(self.handle, ptrs.as_mut_ptr()) }
    }

    pub fn command_async(&self, args: &[&str]) {
        let c_args: Vec<CString> = args.iter().map(|a| CString::new(*a).unwrap()).collect();
        let mut ptrs: Vec<*const i8> = c_args.iter().map(|a| a.as_ptr()).collect();
        ptrs.push(ptr::null());
        unsafe {
            mpv_command_async(self.handle, 0, ptrs.as_mut_ptr());
        }
    }

    pub fn observe_property(&self, reply_userdata: u64, name: &str, format: u32) {
        let name_c = CString::new(name).unwrap();
        unsafe {
            mpv_observe_property(self.handle, reply_userdata, name_c.as_ptr(), format);
        }
    }

    pub fn unobserve_property(&self, reply_userdata: u64) {
        unsafe {
            mpv_unobserve_property(self.handle, reply_userdata);
        }
    }

    pub fn wait_event(&self, timeout: f64) -> *mut mpv_event {
        unsafe { mpv_wait_event(self.handle, timeout) }
    }

    pub fn load_config_file(&self, path: &str) {
        let path_c = CString::new(path).unwrap();
        unsafe {
            mpv_load_config_file(self.handle, path_c.as_ptr());
        }
    }

    pub fn set_init_options(&self, slideshow_time: u32, mpv_options_config: Option<&str>) {
        self.set_option_string("input-default-bindings", "yes");
        self.set_option_string("input-terminal", "yes");
        self.set_option_string("terminal", "yes");
        self.set_option_string("config", "yes");
        self.set_option_string("background-color", "#00000000");

        if slideshow_time != 0 {
            self.set_option_string("loop", "yes");
            self.set_option_string("loop-playlist", "yes");
        }

        if let Some(config_content) = mpv_options_config {
            if !config_content.is_empty() {
                let config_path = format!("/tmp/mpvpaper_{}.config", std::process::id());
                if std::fs::write(&config_path, config_content).is_ok() {
                    self.load_config_file(&config_path);
                    let _ = std::fs::remove_file(&config_path);
                }
            }
        }
    }

    pub fn force_libmpv_vo(&self, verbose: u8) {
        if let Some(vo) = self.get_property_string("options/vo") {
            if vo != "libmpv" {
                if !vo.is_empty() && verbose > 0 {
                    log_warning!("mpvpaper does not support any other vo than \"libmpv\"");
                }
                self.set_option_string("vo", "libmpv");
            }
        }
    }

    pub fn load_media(&self, video_path: &str) {
        let err = if let Some(list_path) = video_path.strip_prefix("--playlist=") {
            self.command(&["loadlist", list_path])
        } else {
            self.command(&["loadfile", video_path])
        };

        if err < 0 {
            log_error!("Failed to load file, {}", mpv_error_str(err));
            std::process::exit(1);
        }
    }

    pub fn wait_for_file_loaded(&self, verbose: u8, video_path: &str) {
        loop {
            let event = self.wait_event(1.0);
            if event.is_null() {
                continue;
            }
            let event = unsafe { &*event };
            if event.event_id == mpv_event_id_MPV_EVENT_FILE_LOADED {
                break;
            }
        }
        if verbose > 0 {
            log_info!("Loaded {}", video_path);
        }
    }
}

impl Drop for MpvContext {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                mpv_terminate_destroy(self.handle);
            }
        }
    }
}

pub struct MpvRenderContext {
    pub handle: *mut mpv_render_context,
}

unsafe impl Send for MpvRenderContext {}
unsafe impl Sync for MpvRenderContext {}

impl MpvRenderContext {
    pub fn new(mpv: &MpvContext, wl_display: *mut c_void, get_proc_addr: GetProcAddr) -> Self {
        let api_type = CString::new("opengl").unwrap();

        let gl_init_params = mpv_opengl_init_params {
            get_proc_address: Some(get_proc_address_mpv),
            get_proc_address_ctx: get_proc_addr as *mut c_void,
            extra_exts: ptr::null(),
        };

        let mut params = [
            mpv_render_param {
                type_: mpv_render_param_type_MPV_RENDER_PARAM_WL_DISPLAY,
                data: wl_display,
            },
            mpv_render_param {
                type_: mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
                data: api_type.as_ptr() as *mut c_void,
            },
            mpv_render_param {
                type_: mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: &gl_init_params as *const _ as *mut c_void,
            },
            mpv_render_param {
                type_: mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
                data: ptr::null_mut(),
            },
        ];

        let mut ctx: *mut mpv_render_context = ptr::null_mut();
        let err = unsafe {
            mpv_render_context_create(&mut ctx, mpv.handle, params.as_mut_ptr())
        };
        if err < 0 {
            log_error!("Failed to initialize mpv GL context, {}", mpv_error_str(err));
            std::process::exit(1);
        }

        Self { handle: ctx }
    }

    pub fn set_update_callback(&self, callback: extern "C" fn(*mut c_void), ctx: *mut c_void) {
        unsafe {
            mpv_render_context_set_update_callback(self.handle, Some(callback), ctx);
        }
    }

    pub fn update(&self) -> u64 {
        unsafe { mpv_render_context_update(self.handle) }
    }

    pub fn render(&self, width: i32, height: i32) {
        let fbo = mpv_opengl_fbo {
            fbo: 0,
            w: width,
            h: height,
            internal_format: 0,
        };
        let flip_y: i32 = 1;
        let block: i32 = 0;

        let mut params = [
            mpv_render_param {
                type_: mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_FBO,
                data: &fbo as *const _ as *mut c_void,
            },
            mpv_render_param {
                type_: mpv_render_param_type_MPV_RENDER_PARAM_FLIP_Y,
                data: &flip_y as *const _ as *mut c_void,
            },
            mpv_render_param {
                type_: mpv_render_param_type_MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
                data: &block as *const _ as *mut c_void,
            },
            mpv_render_param {
                type_: mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
                data: ptr::null_mut(),
            },
        ];

        let err = unsafe { mpv_render_context_render(self.handle, params.as_mut_ptr()) };
        if err < 0 {
            log_error!("Failed to render frame with mpv, {}", mpv_error_str(err));
        }
    }
}

impl Drop for MpvRenderContext {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                mpv_render_context_free(self.handle);
            }
        }
    }
}

extern "C" fn get_proc_address_mpv(ctx: *mut c_void, name: *const i8) -> *mut c_void {
    let get_proc_addr: GetProcAddr = unsafe { std::mem::transmute(ctx) };
    if name.is_null() {
        return ptr::null_mut();
    }
    let name_str = unsafe { CStr::from_ptr(name) };
    unsafe { get_proc_addr(name_str.as_ptr() as *const u8) as *mut c_void }
}

pub(crate) fn mpv_error_str(err: i32) -> &'static str {
    unsafe {
        let ptr = mpv_error_string(err);
        if ptr.is_null() {
            "unknown error"
        } else {
            CStr::from_ptr(ptr).to_str().unwrap_or("unknown error")
        }
    }
}
