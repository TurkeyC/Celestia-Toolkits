use std::cell::{Cell, RefCell};
use std::error::Error;
use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use serde::Deserialize;

use crate::egl::EglState;
use crate::renderer::WallpaperRenderer;
use crate::wayland::HaltInfo;
use crate::WAKEUP_FD;

#[cfg(feature = "lua-scripting")]
use crate::renderers::spine_lua::{Command as LuaCommand, LuaRuntime};

use rusty_spine::{
    self,
    atlas::AtlasPage,
    extension,
    AnimationStateData, Atlas, BlendMode, SkeletonBinary, SkeletonController, SkeletonJson,
};

// ======================================================================
// Config types (deserialised from .spine.toml)
// ======================================================================

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct SpineConfig {
    /// Path to the skeleton file (relative to config file directory, or absolute).
    skeleton: String,
    /// Animation playback sequence.
    #[serde(default)]
    anim: Vec<AnimEntry>,
    /// Display settings.
    #[serde(default)]
    display: DisplayConfig,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct AnimEntry {
    /// Name of the animation in the Spine skeleton.
    name: String,
    /// Whether to loop this animation while it is playing.
    #[serde(default)]
    loop_anim: bool,
    /// How long to play this animation before advancing (seconds).
    /// 0 = play until natural end (looping animations will never expire).
    #[serde(default)]
    duration: f32,
}

#[derive(Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
struct DisplayConfig {
    /// Horizontal offset from centre, in skeleton coordinates.
    #[serde(default)]
    offset_x: f32,
    /// Vertical offset from centre, in skeleton coordinates.
    #[serde(default)]
    offset_y: f32,
    /// Scale override.  0 = auto-fill (cover mode).
    #[serde(default)]
    scale: f32,
}

// ======================================================================
// Animation sequencer
// ======================================================================

struct AnimationSequence {
    entries: Vec<AnimEntry>,
    current_index: usize,
    elapsed: f32,
}

impl AnimationSequence {
    fn new(entries: Vec<AnimEntry>) -> Self {
        Self {
            entries,
            current_index: 0,
            elapsed: 0.0,
        }
    }

    /// Advance the animation timer and switch to the next entry when the
    /// current one expires.  Only entries with `duration > 0` trigger
    /// switches; entries with `duration == 0` hold indefinitely.
    fn advance(&mut self, controller: &mut SkeletonController, dt: f32) {
        if self.entries.is_empty() {
            return;
        }
        let dur = self.entries[self.current_index].duration;
        if dur > 0.0 {
            self.elapsed += dt;
            while self.elapsed >= dur && dur > 0.0 {
                self.elapsed -= dur;
                self.current_index = (self.current_index + 1) % self.entries.len();
                let entry = &self.entries[self.current_index];
                let _ = controller
                    .animation_state
                    .set_animation_by_name(0, &entry.name, entry.loop_anim);
            }
        }
    }
}

// ======================================================================
// GL helper
// ======================================================================

/// OpenGL texture handle stored in AtlasPage renderer_object.
struct SpineTexture(pub u32);

// ======================================================================
// Shaders
// ======================================================================

const VERTEX_SHADER: &str = r#"
#version 330 core
layout (location = 0) in vec2 aPos;
layout (location = 1) in vec2 aTexCoord;
uniform vec2 uOffset;
uniform vec2 uScale_WH;
out vec2 vTexCoord;

void main() {
    vec2 pos = aPos * uScale_WH + uOffset;
    gl_Position = vec4(pos, 0.0, 1.0);
    vTexCoord = aTexCoord;
}
"#;

const FRAGMENT_SHADER: &str = r#"
#version 330 core
in vec2 vTexCoord;
out vec4 FragColor;
uniform sampler2D uTexture;
uniform vec4 uColor;
uniform vec4 uDarkColor;

void main() {
    vec4 texColor = texture(uTexture, vTexCoord);
    vec4 color = uColor * texColor;
    FragColor = vec4(color.rgb + uDarkColor.rgb * (1.0 - texColor.a), color.a);
}
"#;

// ======================================================================
// Blend mode helpers
// ======================================================================

fn set_blend_mode(mode: BlendMode) {
    unsafe {
        match mode {
            BlendMode::Normal => gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA),
            BlendMode::Additive => gl::BlendFunc(gl::SRC_ALPHA, gl::ONE),
            BlendMode::Multiply => gl::BlendFunc(gl::DST_COLOR, gl::ONE_MINUS_SRC_ALPHA),
            BlendMode::Screen => gl::BlendFunc(gl::ONE_MINUS_DST_COLOR, gl::ONE),
        }
    }
}

// ======================================================================
// SpineRenderer
// ======================================================================

pub struct SpineRenderer {
    file_path: String,
    verbose: u8,

    // GL objects (0 = uninitialised).
    program: u32,
    vao: u32,
    vbo: u32,
    ebo: u32,

    /// Spine runtime controller.
    controller: RefCell<Option<SkeletonController>>,
    /// Wall-clock instant of the last render call.
    last_time: Cell<Instant>,
    /// Skeleton bounding-box width.
    skel_width: Cell<f32>,
    /// Skeleton bounding-box height.
    skel_height: Cell<f32>,

    /// Animation sequence from config (None = default: loop first anim).
    anim_sequence: RefCell<Option<AnimationSequence>>,
    /// Display overrides from config.
    display_config: DisplayConfig,

    /// Lua scripting runtime (only for `.spine.lua` configs).
    #[cfg(feature = "lua-scripting")]
    lua_runtime: RefCell<Option<LuaRuntime>>,

    initialized: bool,
}

impl SpineRenderer {
    pub fn new(file_path: String, verbose: u8) -> Self {
        Self {
            file_path,
            verbose,
            program: 0,
            vao: 0,
            vbo: 0,
            ebo: 0,
            controller: RefCell::new(None),
            last_time: Cell::new(Instant::now()),
            skel_width: Cell::new(0.0),
            skel_height: Cell::new(0.0),
            anim_sequence: RefCell::new(None),
            display_config: DisplayConfig::default(),
            #[cfg(feature = "lua-scripting")]
            lua_runtime: RefCell::new(None),
            initialized: false,
        }
    }

    // ---- helpers ---------------------------------------------------------

    fn compile_shader(src: &str, shader_type: u32) -> Result<u32, Box<dyn Error>> {
        let shader = unsafe { gl::CreateShader(shader_type) };
        unsafe {
            gl::ShaderSource(shader, 1, &(src.as_ptr() as *const i8), &(src.len() as i32));
            gl::CompileShader(shader);
            let mut success: i32 = 0;
            gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success);
            if success == 0 {
                let mut log_len: i32 = 0;
                gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut log_len);
                let mut log = vec![0u8; log_len as usize];
                gl::GetShaderInfoLog(shader, log_len, std::ptr::null_mut(), log.as_mut_ptr() as *mut i8);
                let msg = String::from_utf8_lossy(&log).to_string();
                return Err(format!("Shader compile error: {}", msg).into());
            }
        }
        Ok(shader)
    }

    fn link_program(vs: u32, fs: u32) -> Result<u32, Box<dyn Error>> {
        let program = unsafe { gl::CreateProgram() };
        unsafe {
            gl::AttachShader(program, vs);
            gl::AttachShader(program, fs);
            gl::LinkProgram(program);
            let mut success: i32 = 0;
            gl::GetProgramiv(program, gl::LINK_STATUS, &mut success);
            if success == 0 {
                let mut log_len: i32 = 0;
                gl::GetProgramiv(program, gl::INFO_LOG_LENGTH, &mut log_len);
                let mut log = vec![0u8; log_len as usize];
                gl::GetProgramInfoLog(program, log_len, std::ptr::null_mut(), log.as_mut_ptr() as *mut i8);
                let msg = String::from_utf8_lossy(&log).to_string();
                return Err(format!("Program link error: {}", msg).into());
            }
        }
        Ok(program)
    }

    // ---- texture callbacks -----------------------------------------------

    fn setup_texture_callbacks() {
        extension::set_create_texture_cb(move |page: &mut AtlasPage, path: &str| {
            let pixel_data: Vec<u8>;
            let (mut w, mut h) = (1u32, 1u32);

            match image::open(path) {
                Ok(img) => {
                    let rgba = img.into_rgba8();
                    let dims = rgba.dimensions();
                    w = dims.0;
                    h = dims.1;
                    pixel_data = rgba.into_raw();
                }
                Err(e) => {
                    log_error!("Failed to load Spine texture '{}': {}", path, e);
                    pixel_data = vec![255, 0, 255, 255];
                }
            }

            let mut texture: u32 = 0;
            unsafe {
                gl::GenTextures(1, &mut texture);
                gl::BindTexture(gl::TEXTURE_2D, texture);
                gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGBA as i32, w as i32, h as i32, 0, gl::RGBA, gl::UNSIGNED_BYTE, pixel_data.as_ptr() as *const c_void);
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
                gl::BindTexture(gl::TEXTURE_2D, 0);
            }
            page.renderer_object().set(SpineTexture(texture));
        });

        extension::set_dispose_texture_cb(|page: &mut AtlasPage| unsafe {
            if let Some(tex) = page.renderer_object().get::<SpineTexture>() {
                gl::DeleteTextures(1, &tex.0);
            }
            page.renderer_object().dispose::<SpineTexture>();
        });
    }

    // ---- path helpers ----------------------------------------------------

    fn atlas_path_from_skeleton(path: &str) -> String {
        let p = Path::new(path);
        let parent = p.parent().unwrap_or(Path::new("."));
        let stem = p.file_stem().unwrap_or_default().to_str().unwrap_or("unknown");
        parent.join(format!("{}.atlas", stem)).to_string_lossy().to_string()
    }

    /// Resolve `skeleton` field from config relative to the config file's parent dir.
    fn resolve_skel_path(config_path: &str, skel_rel: &str) -> String {
        let p = Path::new(config_path);
        let parent = p.parent().unwrap_or(Path::new("."));
        parent.join(skel_rel).to_string_lossy().to_string()
    }

    // ---- Lua config helpers (only for .spine.lua) -----------------------

    /// Extract `skeleton = "path"` from a Lua script without executing it.
    #[cfg(feature = "lua-scripting")]
    fn extract_lua_skeleton_path(script: &str) -> Option<String> {
        for line in script.lines() {
            let s = line.trim();
            if s.starts_with("--") {
                continue;
            }
            if let Some(eq_pos) = s.find('=') {
                let key = s[..eq_pos].trim();
                if key == "skeleton" {
                    let val = s[eq_pos + 1..].trim().trim_matches('"').trim_matches('\'').trim();
                    if !val.is_empty() {
                        return Some(val.to_string());
                    }
                }
            }
        }
        None
    }

    /// Parse `scale`, `offset_x`, `offset_y` from a Lua script text.
    #[cfg(feature = "lua-scripting")]
    fn parse_lua_display_config(script: &str) -> DisplayConfig {
        let mut dc = DisplayConfig::default();
        for line in script.lines() {
            let s = line.trim();
            if s.starts_with("--") { continue; }
            if let Some(eq_pos) = s.find('=') {
                let key = s[..eq_pos].trim();
                let val_str = s[eq_pos + 1..].trim().trim_matches('"').trim().to_string();
                match key {
                    "scale" => dc.scale = val_str.parse().unwrap_or(0.0),
                    "offset_x" => dc.offset_x = val_str.parse().unwrap_or(0.0),
                    "offset_y" => dc.offset_y = val_str.parse().unwrap_or(0.0),
                    _ => {}
                }
            }
        }
        dc
    }

    // ---- bounding box ----------------------------------------------------

    fn renderable_bounds(renderables: &[rusty_spine::SkeletonRenderable]) -> (f32, f32, f32, f32) {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for rend in renderables {
            for v in &rend.vertices {
                min_x = min_x.min(v[0]);
                min_y = min_y.min(v[1]);
                max_x = max_x.max(v[0]);
                max_y = max_y.max(v[1]);
            }
        }
        if min_x > max_x { (0.0, 0.0, 1.0, 1.0) } else { (min_x, min_y, max_x, max_y) }
    }

    fn pick_animation_name(names: &[String]) -> Option<String> {
        if names.is_empty() { return None; }
        for preferred in &["Idle_01", "idle", "Idle", "idle_01", "walk", "Walk", "animation"] {
            if names.iter().any(|n| n == preferred) {
                return Some(preferred.to_string());
            }
        }
        Some(names[0].clone())
    }

    // ---- skeleton loader -------------------------------------------------

    fn load_skeleton(
        skeleton_path: &str,
        atlas_path: &str,
    ) -> Result<(SkeletonController, f32, f32), Box<dyn Error>> {
        let atlas = Arc::new(Atlas::new_from_file(atlas_path)?);

        let skeleton_data = {
            let lower = skeleton_path.to_lowercase();
            if lower.ends_with(".skel") {
                let bin = SkeletonBinary::new(atlas);
                Arc::new(bin.read_skeleton_data_file(skeleton_path)?)
            } else {
                let json = SkeletonJson::new(atlas);
                Arc::new(json.read_skeleton_data_file(skeleton_path)?)
            }
        };

        let mut anim_state_data = AnimationStateData::new(skeleton_data.clone());
        anim_state_data.set_default_mix(0.2);
        let anim_state_data = Arc::new(anim_state_data);

        let mut controller = SkeletonController::new(skeleton_data, anim_state_data);

        // Sample one frame for bounds.
        controller.update(0.0);
        let renderables = controller.renderables();
        let (min_x, min_y, max_x, max_y) = Self::renderable_bounds(&renderables);
        let centre_x = (min_x + max_x) / 2.0;
        let centre_y = (min_y + max_y) / 2.0;
        let w = max_x - min_x;
        let h = max_y - min_y;

        controller.skeleton.set_x(-centre_x);
        controller.skeleton.set_y(-centre_y);

        Ok((controller, w, h))
    }
}

impl WallpaperRenderer for SpineRenderer {
    fn name(&self) -> &'static str { "spine (2D skeleton animation)" }
    fn needs_eventfd(&self) -> bool { true }

    fn init(&mut self, _egl: &EglState, _wl_display: *mut c_void) -> Result<(), Box<dyn Error>> {
        if self.verbose > 0 {
            log_info!("Loading Spine skeleton: {}", self.file_path);
        }

        // ---- determine skeleton path & config ------------------------------
        let lower_path = self.file_path.to_lowercase();
        let is_toml = lower_path.ends_with(".spine.toml");
        let is_lua = lower_path.ends_with(".spine.lua");

        // For Lua mode we need to keep the script content across the
        // skeleton-loading boundary.
        #[cfg(feature = "lua-scripting")]
        let mut lua_script_content: Option<String> = None;

        // Read config (if applicable) so we have display / anim settings.
        if is_toml {
            let content = std::fs::read_to_string(&self.file_path)?;
            let cfg: SpineConfig = toml::from_str(&content)?;
            self.display_config = cfg.display;
            let skel_path = Self::resolve_skel_path(&self.file_path, &cfg.skeleton);

            // Override file_path so the rest of init uses the resolved skeleton.
            self.file_path = skel_path.clone();

            // Set up animation sequence.
            if !cfg.anim.is_empty() {
                *self.anim_sequence.borrow_mut() = Some(AnimationSequence::new(cfg.anim));
            }
        } else if is_lua {
            #[cfg(not(feature = "lua-scripting"))]
            return Err("Lua scripting is not enabled.  Rebuild with --features lua-scripting \
                         or install luajit-devel (Fedora: dnf install luajit-devel)"
                .into());

            #[cfg(feature = "lua-scripting")]
            {
                let content = std::fs::read_to_string(&self.file_path)?;
                let skel_rel = Self::extract_lua_skeleton_path(&content)
                    .ok_or_else(|| ".spine.lua must define skeleton = \"path\"".to_string())?;
                self.file_path = Self::resolve_skel_path(&self.file_path, &skel_rel);
                self.display_config = Self::parse_lua_display_config(&content);
                lua_script_content = Some(content);

                if self.verbose > 0 {
                    log_info!("Lua config: skeleton={}, scale={}, offset=({},{})",
                        self.file_path, self.display_config.scale,
                        self.display_config.offset_x, self.display_config.offset_y);
                }
            }
        }

        // ---- atlas ---------------------------------------------------------
        let atlas_path_str = Self::atlas_path_from_skeleton(&self.file_path);
        let atlas_path = Path::new(&atlas_path_str);
        if !atlas_path.exists() {
            return Err(format!(
                "Atlas file not found: {}.  \
                 Spine wallpapers need a .atlas file next to the skeleton file.",
                atlas_path_str
            ).into());
        }

        Self::setup_texture_callbacks();

        // ---- load skeleton -------------------------------------------------
        let (mut controller, skel_w, skel_h) =
            Self::load_skeleton(&self.file_path, &atlas_path_str)?;
        self.skel_width.set(skel_w);
        self.skel_height.set(skel_h);

        // ---- start animation(s) / Lua init --------------------------------
        if is_toml || !is_lua {
            // TOML-driven or default
            let anim_names: Vec<String> =
                controller.skeleton.data().animations().map(|a| a.name().to_string()).collect();

            if let Some(ref seq) = *self.anim_sequence.borrow() {
                if let Some(first) = seq.entries.first() {
                    let _ = controller
                        .animation_state
                        .set_animation_by_name(0, &first.name, first.loop_anim);
                    if self.verbose > 0 {
                        log_info!("Anim sequence: {} entries, starting with '{}'",
                            seq.entries.len(), first.name);
                    }
                }
            } else {
                if let Some(ref name) = Self::pick_animation_name(&anim_names) {
                    let _ = controller
                        .animation_state
                        .set_animation_by_name(0, name, true);
                    if self.verbose > 0 {
                        log_info!("Spine skeleton loaded: {}x{}, animation: {}",
                            skel_w as i32, skel_h as i32, name);
                    }
                }
            }
        }

        #[cfg(feature = "lua-scripting")]
        if is_lua {
            if let Some(ref script) = lua_script_content {
                let runtime = LuaRuntime::new(script, &mut controller)
                    .map_err(|e| format!("Lua init error: {e}"))?;
                runtime.call_init();

                // Process play commands from on_init immediately so the first
                // render frame has an animation to show.
                for cmd in runtime.drain_commands() {
                    match cmd {
                        LuaCommand::Play { track, name, looping } => {
                            let _ = controller.animation_state
                                .set_animation_by_name(track, &name, looping);
                        }
                        LuaCommand::Add { track, name, looping, delay } => {
                            let _ = controller.animation_state
                                .add_animation_by_name(track, &name, looping, delay);
                        }
                        LuaCommand::ClearTrack(track) => {
                            controller.animation_state.clear_track(track);
                        }
                        LuaCommand::Empty { track, mix_duration } => {
                            controller.animation_state.set_empty_animation(track, mix_duration);
                        }
                    }
                }

                *self.lua_runtime.borrow_mut() = Some(runtime);

                if self.verbose > 0 {
                    log_success!("Lua scripting runtime initialised");
                }
            }
        }

        // ---- compile shaders -----------------------------------------------
        let vs = Self::compile_shader(VERTEX_SHADER, gl::VERTEX_SHADER)?;
        let fs = Self::compile_shader(FRAGMENT_SHADER, gl::FRAGMENT_SHADER)?;
        let program = Self::link_program(vs, fs)?;
        unsafe { gl::DeleteShader(vs); gl::DeleteShader(fs); }

        // ---- GL buffers ----------------------------------------------------
        let mut vao: u32 = 0;
        let mut vbo: u32 = 0;
        let mut ebo: u32 = 0;
        unsafe { gl::GenVertexArrays(1, &mut vao); gl::GenBuffers(1, &mut vbo); gl::GenBuffers(1, &mut ebo); }

        self.program = program;
        self.vao = vao;
        self.vbo = vbo;
        self.ebo = ebo;
        self.last_time.set(Instant::now());
        *self.controller.borrow_mut() = Some(controller);
        self.initialized = true;

        if self.verbose > 0 {
            log_success!("Spine renderer initialised");
        }
        Ok(())
    }

    fn render(&self, _output_idx: usize, _egl: &EglState, width: i32, height: i32) -> Result<(), Box<dyn Error>> {
        if !self.initialized { return Ok(()); }

        unsafe { gl::Clear(gl::COLOR_BUFFER_BIT); gl::Enable(gl::BLEND); }

        // Borrow controller.
        let mut guard = self.controller.borrow_mut();
        let controller = match guard.as_mut() {
            Some(c) => c,
            None => return Ok(()),
        };

        // --- delta time -----------------------------------------------------
        let now = Instant::now();
        let dt = now.duration_since(self.last_time.get()).as_secs_f32().min(0.1);
        self.last_time.set(now);

        // --- advance animation ----------------------------------------------
        controller.update(dt);

        // Lua-driven or TOML-sequence advancement.
        #[cfg(feature = "lua-scripting")]
        if let Some(ref lua) = *self.lua_runtime.borrow() {
            lua.call_completions();
            lua.call_update(dt);
            for cmd in lua.drain_commands() {
                match cmd {
                    LuaCommand::Play { track, name, looping } => {
                        let _ = controller
                            .animation_state
                            .set_animation_by_name(track, &name, looping);
                    }
                    LuaCommand::Add { track, name, looping, delay } => {
                        let _ = controller
                            .animation_state
                            .add_animation_by_name(track, &name, looping, delay);
                    }
                    LuaCommand::ClearTrack(track) => {
                        controller.animation_state.clear_track(track);
                    }
                    LuaCommand::Empty { track, mix_duration } => {
                        controller.animation_state.set_empty_animation(track, mix_duration);
                    }
                }
            }
        }

        // TOML / default sequence (also runs when Lua feature is present
        // but no lua_runtime is active, i.e. a .spine.toml or bare skeleton).
        #[cfg(feature = "lua-scripting")]
        let use_sequence = self.lua_runtime.borrow().is_none();
        #[cfg(not(feature = "lua-scripting"))]
        let use_sequence = true;

        if use_sequence {
            if let Some(ref mut seq) = *self.anim_sequence.borrow_mut() {
                seq.advance(controller, dt);
            }
        }

        let renderables = controller.renderables();
        if renderables.is_empty() { return Ok(()); }

        // --- uniforms -------------------------------------------------------
        let program = self.program;
        unsafe { gl::UseProgram(program) };

        let fw = width as f32;
        let fh = height as f32;

        // Cover scale: fill the entire viewport.
        let skel_w = self.skel_width.get();
        let skel_h = self.skel_height.get();
        let mut scale = if skel_w > 0.0 && skel_h > 0.0 {
            (fw / skel_w).max(fh / skel_h)
        } else {
            1.0
        };

        // User scale override.
        let dc = &self.display_config;
        if dc.scale > 0.0 {
            scale = dc.scale;
        }

        // Offset (user config applied in clip space).
        let offset_x = dc.offset_x * scale / fw * 2.0;
        let offset_y = dc.offset_y * scale / fh * 2.0;

        unsafe {
            let u_offset = gl::GetUniformLocation(program, c"uOffset".as_ptr());
            let u_scale_wh = gl::GetUniformLocation(program, c"uScale_WH".as_ptr());
            gl::Uniform2f(u_offset, offset_x, offset_y);
            gl::Uniform2f(u_scale_wh, scale / fw * 2.0, scale / fh * 2.0);

            let u_color = gl::GetUniformLocation(program, c"uColor".as_ptr());
            let u_dark_color = gl::GetUniformLocation(program, c"uDarkColor".as_ptr());

            for rend in &renderables {
                // Texture (null-safe).
                match rend.attachment_renderer_object {
                    Some(ptr) if !ptr.is_null() => {
                        let tex = &*(ptr as *const SpineTexture);
                        gl::BindTexture(gl::TEXTURE_2D, tex.0);
                    }
                    _ => gl::BindTexture(gl::TEXTURE_2D, 0),
                }

                set_blend_mode(rend.blend_mode);

                gl::Uniform4f(u_color, rend.color.r, rend.color.g, rend.color.b, rend.color.a);
                gl::Uniform4f(u_dark_color, rend.dark_color.r, rend.dark_color.g, rend.dark_color.b, rend.dark_color.a);

                let n = rend.vertices.len();

                // Interleave: pos (2) + uv (2) — flip V for OpenGL.
                let mut data = Vec::with_capacity(n * 4);
                for i in 0..n {
                    data.push(rend.vertices[i][0]);
                    data.push(rend.vertices[i][1]);
                    data.push(rend.uvs[i][0]);
                    data.push(rend.uvs[i][1]); // V (runtime already handles OpenGL flip)
                }

                gl::BindVertexArray(self.vao);
                gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
                gl::BufferData(gl::ARRAY_BUFFER, (data.len() * std::mem::size_of::<f32>()) as isize,
                    std::ptr::null(), gl::STREAM_DRAW);
                gl::BufferSubData(gl::ARRAY_BUFFER, 0,
                    (data.len() * std::mem::size_of::<f32>()) as isize,
                    data.as_ptr() as *const c_void);

                let stride = 4 * std::mem::size_of::<f32>() as i32;
                gl::VertexAttribPointer(0, 2, gl::FLOAT, 0, stride, std::ptr::null());
                gl::EnableVertexAttribArray(0);
                gl::VertexAttribPointer(1, 2, gl::FLOAT, 0, stride,
                    (2 * std::mem::size_of::<f32>()) as *const c_void);
                gl::EnableVertexAttribArray(1);

                gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, self.ebo);
                gl::BufferData(gl::ELEMENT_ARRAY_BUFFER,
                    (rend.indices.len() * std::mem::size_of::<u16>()) as isize,
                    rend.indices.as_ptr() as *const c_void, gl::STREAM_DRAW);

                gl::DrawElements(gl::TRIANGLES, rend.indices.len() as i32, gl::UNSIGNED_SHORT, std::ptr::null());
            }
        }

        unsafe { gl::BindVertexArray(0); gl::UseProgram(0); gl::Disable(gl::BLEND); }
        Ok(())
    }

    fn on_eventfd_wakeup(&self) {}

    fn spawn_threads(&mut self, halt_info: Arc<HaltInfo>) -> Vec<std::thread::JoinHandle<()>> {
        if let Some(&fd) = WAKEUP_FD.get() {
            vec![std::thread::spawn(move || {
                loop {
                    if halt_info.stop_render_loop.load(Ordering::Relaxed) { break; }
                    std::thread::sleep(std::time::Duration::from_millis(16));
                    let inc: u64 = 1;
                    unsafe { libc::write(fd, &inc as *const u64 as *const c_void, 8); }
                }
            })]
        } else {
            Vec::new()
        }
    }
}

impl Drop for SpineRenderer {
    fn drop(&mut self) {
        unsafe {
            if self.program != 0 { gl::DeleteProgram(self.program); }
            if self.vao != 0 { gl::DeleteVertexArrays(1, &self.vao); }
            if self.vbo != 0 { gl::DeleteBuffers(1, &self.vbo); }
            if self.ebo != 0 { gl::DeleteBuffers(1, &self.ebo); }
        }
    }
}
