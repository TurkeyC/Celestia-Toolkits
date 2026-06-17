use std::error::Error;
use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::egl::EglState;
use crate::monitor;
use crate::mpv_ctx::{MpvContext, MpvRenderContext};
use crate::renderer::WallpaperRenderer;
use crate::wayland::HaltInfo;
use crate::WAKEUP_FD;

pub struct MpvVideoRenderer {
    mpv: Option<Arc<MpvContext>>,
    render_ctx: Option<MpvRenderContext>,
    video_path: String,
    slideshow_time: u32,
    mpv_options_config: Option<String>,
    save_info: Option<String>,
    verbose: u8,
    halt_info: Arc<HaltInfo>,
}

impl MpvVideoRenderer {
    pub fn new(
        video_path: String,
        slideshow_time: u32,
        mpv_options_config: Option<String>,
        save_info: Option<String>,
        verbose: u8,
        halt_info: Arc<HaltInfo>,
    ) -> Self {
        Self {
            mpv: None,
            render_ctx: None,
            video_path,
            slideshow_time,
            mpv_options_config,
            save_info,
            verbose,
            halt_info,
        }
    }

    pub fn mpv(&self) -> &Arc<MpvContext> {
        self.mpv.as_ref().expect("MpvVideoRenderer not initialized")
    }

    fn render_ctx(&self) -> &MpvRenderContext {
        self.render_ctx.as_ref().expect("MpvVideoRenderer not initialized")
    }
}

impl WallpaperRenderer for MpvVideoRenderer {
    fn name(&self) -> &'static str {
        "video (mpv)"
    }

    fn needs_eventfd(&self) -> bool {
        true
    }

    fn init(&mut self, egl: &EglState, wl_display: *mut c_void) -> Result<(), Box<dyn Error>> {
        let mpv = Arc::new(MpvContext::new());
        mpv.set_init_options(self.slideshow_time, self.mpv_options_config.as_deref());
        mpv.initialize();
        mpv.set_init_options(self.slideshow_time, self.mpv_options_config.as_deref());
        mpv.force_libmpv_vo(self.verbose);

        let render_ctx = MpvRenderContext::new(&mpv, wl_display, egl.get_proc_address);

        if let Some(ref info) = self.save_info {
            let parts: Vec<&str> = info.split_whitespace().collect();
            if parts.len() >= 2 {
                if self.verbose > 0 {
                    log_info!(
                        "Restoring previous time: {} and playlist position: {}",
                        parts[0],
                        parts[1]
                    );
                }
                let default_start = mpv.get_property_string("start");
                mpv.command(&["set", "start", parts[0]]);
                mpv.command(&["set", "playlist-start", parts[1]]);
                mpv.load_media(&self.video_path);
                mpv.wait_for_file_loaded(self.verbose, &self.video_path);
                if let Some(ref ds) = default_start {
                    mpv.command(&["set", "start", ds]);
                }
            } else {
                mpv.load_media(&self.video_path);
                mpv.wait_for_file_loaded(self.verbose, &self.video_path);
            }
        } else {
            mpv.load_media(&self.video_path);
            mpv.wait_for_file_loaded(self.verbose, &self.video_path);
        }

        mpv.command(&["set", "idle", "no"]);
        render_ctx.set_update_callback(render_update_callback, std::ptr::null_mut());

        if self.verbose > 0 {
            log_success!("mpv initialized");
        }

        self.mpv = Some(mpv);
        self.render_ctx = Some(render_ctx);

        Ok(())
    }

    fn render(&self, _output_idx: usize, _egl: &EglState, width: i32, height: i32) -> Result<(), Box<dyn Error>> {
        // The caller has already made the EGL context current for this output
        // and set the viewport. We pass the actual dimensions to mpv's FBO.
        self.render_ctx().update();
        self.render_ctx().render(width, height);
        Ok(())
    }

    fn spawn_threads(&mut self, _halt_info: Arc<HaltInfo>) -> Vec<std::thread::JoinHandle<()>> {
        let mut handles = Vec::new();
        let mpv = self.mpv();

        // Always spawn the mpv event thread
        handles.push(monitor::spawn_mpv_event_thread(
            self.halt_info.clone(),
            mpv.clone(),
            self.slideshow_time,
        ));

        // Auto-pause / auto-stop threads
        if self.halt_info.auto_pause.load(Ordering::Relaxed) {
            handles.push(monitor::spawn_auto_pause_thread(
                self.halt_info.clone(),
                mpv.clone(),
            ));
        } else if self.halt_info.auto_stop.load(Ordering::Relaxed) {
            handles.push(monitor::spawn_auto_stop_thread(
                self.halt_info.clone(),
            ));
        }

        // Pauselist / stoplist threads
        if let Some(list) = monitor::load_watch_list("pauselist") {
            if self.verbose > 0 {
                log_info!("pauselist found and will be monitored");
            }
            handles.push(monitor::spawn_pauselist_thread(
                self.halt_info.clone(),
                list,
                mpv.clone(),
            ));
        }

        if let Some(list) = monitor::load_watch_list("stoplist") {
            if self.verbose > 0 {
                log_info!("stoplist found and will be monitored");
            }
            handles.push(monitor::spawn_stoplist_thread(list));
        }

        handles
    }

    fn on_eventfd_wakeup(&self) {
        // The main loop handles reading the eventfd and calling render().
        // We don't need to do anything here.
    }
}

extern "C" fn render_update_callback(_ctx: *mut c_void) {
    if let Some(&fd) = WAKEUP_FD.get() {
        let inc: u64 = 1;
        unsafe {
            libc::write(fd, &inc as *const u64 as *const c_void, 8);
        }
    }
}
