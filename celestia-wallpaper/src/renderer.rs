use std::error::Error;

pub enum WallpaperType {
    Video,
    Picture,
    Spine,
    Web,
}

impl WallpaperType {
    /// Auto-detect wallpaper type from file path extension.
    /// Returns None when detection is ambiguous (no extension or unknown).
    pub fn from_path(path: &str) -> Option<Self> {
        let lower = path.to_lowercase();
        if lower.ends_with(".mp4")
            || lower.ends_with(".webm")
            || lower.ends_with(".mkv")
            || lower.ends_with(".avi")
            || lower.ends_with(".mov")
            || lower.ends_with(".gif")
        {
            Some(WallpaperType::Video)
        } else if lower.ends_with(".png")
            || lower.ends_with(".jpg")
            || lower.ends_with(".jpeg")
            || lower.ends_with(".bmp")
            || lower.ends_with(".webp")
        {
            Some(WallpaperType::Picture)
        } else if lower.ends_with(".spine.toml") || lower.ends_with(".json") || lower.ends_with(".skel") {
            Some(WallpaperType::Spine)
        } else if lower.ends_with(".html") || lower.ends_with(".htm") {
            Some(WallpaperType::Web)
        } else {
            None
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            WallpaperType::Video => "video",
            WallpaperType::Picture => "picture",
            WallpaperType::Spine => "spine",
            WallpaperType::Web => "web",
        }
    }
}

/// Common interface for all wallpaper renderers.
///
/// Each renderer takes a file path and renders it to the Wayland/EGL
/// surfaces managed by the main loop. The main loop handles Wayland event
/// dispatch and EGL surface management; the renderer handles everything
/// that happens ON the EGL surface.
pub trait WallpaperRenderer {
    /// Unique name for this renderer type.
    #[allow(dead_code)]
    fn name(&self) -> &'static str;

    /// Returns true if this renderer needs the eventfd wakeup mechanism.
    /// Video (mpv) needs it; static images do not.
    #[allow(dead_code)]
    fn needs_eventfd(&self) -> bool {
        false
    }

    /// Called once after EGL is initialized, before Wayland roundtrip.
    fn init(&mut self, _egl: &super::egl::EglState, _wl_display: *mut std::ffi::c_void) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    /// Called when an output has its EGL window created and needs an EGL surface.
    /// The renderer can set up per-output state here.
    #[allow(dead_code)]
    fn on_output_egl_window(
        &mut self,
        _output_idx: usize,
        _egl_window: *mut std::ffi::c_void,
        _width: i32,
        _height: i32,
        _scale: i32,
    ) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    /// Render one frame for a specific output.
    /// Called from the main loop when it's time to render.
    /// The caller has already made the EGL context current and set the viewport.
    /// `width` and `height` are the framebuffer dimensions in pixels (already scaled).
    fn render(&self, _output_idx: usize, _egl: &super::egl::EglState, _width: i32, _height: i32) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    /// Called when the eventfd is signaled (only if needs_eventfd() returns true).
    fn on_eventfd_wakeup(&self) {}

    /// Spawn any background threads and return their join handles.
    fn spawn_threads(
        &mut self,
        _halt_info: std::sync::Arc<super::wayland::HaltInfo>,
    ) -> Vec<std::thread::JoinHandle<()>> {
        Vec::new()
    }
}
