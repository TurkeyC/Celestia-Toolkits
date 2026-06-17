use std::error::Error;
use std::ffi::c_void;

use crate::egl::EglState;
use crate::renderer::WallpaperRenderer;

/// Fullscreen quad vertices (clip space: -1 to 1).
/// Interleaved: position (x, y), texcoord (u, v)
const QUAD_VERTICES: [f32; 16] = [
    // positions     texcoords (Y flipped: image crate origin is top-left,
    //              OpenGL texture origin is bottom-left)
    -1.0,  1.0,  0.0, 0.0, // top-left
    -1.0, -1.0,  0.0, 1.0, // bottom-left
     1.0, -1.0,  1.0, 1.0, // bottom-right
     1.0,  1.0,  1.0, 0.0, // top-right
];

const QUAD_INDICES: [u32; 6] = [
    0, 1, 2,
    0, 2, 3,
];

/// Shader sources
const VERTEX_SHADER: &str = r#"
#version 330 core
layout (location = 0) in vec2 aPos;
layout (location = 1) in vec2 aTexCoord;
out vec2 vTexCoord;
void main() {
    gl_Position = vec4(aPos, 0.0, 1.0);
    vTexCoord = aTexCoord;
}
"#;

const FRAGMENT_SHADER: &str = r#"
#version 330 core
in vec2 vTexCoord;
out vec4 FragColor;
uniform sampler2D uTexture;
void main() {
    FragColor = texture(uTexture, vTexCoord);
}
"#;

pub struct ImageRenderer {
    file_path: String,
    verbose: u8,
    // OpenGL resources (initialized in init())
    texture: Option<u32>,
    vao: Option<u32>,
    vbo: Option<u32>,
    ebo: Option<u32>,
    program: Option<u32>,
}

impl ImageRenderer {
    pub fn new(file_path: String, verbose: u8) -> Self {
        Self {
            file_path,
            verbose,
            texture: None,
            vao: None,
            vbo: None,
            ebo: None,
            program: None,
        }
    }

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

    fn load_texture_from_file(path: &str) -> Result<u32, Box<dyn Error>> {
        let img = image::open(path)?
            .into_rgba8();
        let (w, h) = img.dimensions();
        let data = img.into_raw();

        let mut texture: u32 = 0;
        unsafe {
            gl::GenTextures(1, &mut texture);
            gl::BindTexture(gl::TEXTURE_2D, texture);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as i32,
                w as i32,
                h as i32,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                data.as_ptr() as *const c_void,
            );
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::BindTexture(gl::TEXTURE_2D, 0);
        }

        Ok(texture)
    }
}

impl WallpaperRenderer for ImageRenderer {
    fn name(&self) -> &'static str {
        "picture (static image)"
    }

    fn init(&mut self, _egl: &EglState, _wl_display: *mut c_void) -> Result<(), Box<dyn Error>> {
        if self.verbose > 0 {
            log_info!("Loading image: {}", self.file_path);
        }

        // Compile shaders
        let vs = Self::compile_shader(VERTEX_SHADER, gl::VERTEX_SHADER)?;
        let fs = Self::compile_shader(FRAGMENT_SHADER, gl::FRAGMENT_SHADER)?;
        let program = Self::link_program(vs, fs)?;
        unsafe {
            gl::DeleteShader(vs);
            gl::DeleteShader(fs);
        }

        // Load texture
        let texture = Self::load_texture_from_file(&self.file_path)?;

        if self.verbose > 0 {
            log_success!("Image loaded successfully");
        }

        // Set up VAO / VBO / EBO
        let mut vao: u32 = 0;
        let mut vbo: u32 = 0;
        let mut ebo: u32 = 0;

        unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::GenBuffers(1, &mut ebo);

            gl::BindVertexArray(vao);

            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                std::mem::size_of_val(&QUAD_VERTICES) as isize,
                QUAD_VERTICES.as_ptr() as *const c_void,
                gl::STATIC_DRAW,
            );

            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, ebo);
            gl::BufferData(
                gl::ELEMENT_ARRAY_BUFFER,
                std::mem::size_of_val(&QUAD_INDICES) as isize,
                QUAD_INDICES.as_ptr() as *const c_void,
                gl::STATIC_DRAW,
            );

            // Position attribute (location = 0)
            let pos_attr = 0;
            gl::VertexAttribPointer(pos_attr, 2, gl::FLOAT, 0, 4 * std::mem::size_of::<f32>() as i32, std::ptr::null());
            gl::EnableVertexAttribArray(pos_attr);

            // TexCoord attribute (location = 1)
            let tc_attr = 1;
            gl::VertexAttribPointer(
                tc_attr,
                2, gl::FLOAT, 0,
                4 * std::mem::size_of::<f32>() as i32,
                (2 * std::mem::size_of::<f32>()) as *const c_void,
            );
            gl::EnableVertexAttribArray(tc_attr);

            gl::BindVertexArray(0);
        }

        self.program = Some(program);
        self.texture = Some(texture);
        self.vao = Some(vao);
        self.vbo = Some(vbo);
        self.ebo = Some(ebo);

        Ok(())
    }

    fn render(&self, _output_idx: usize, _egl: &EglState, _width: i32, _height: i32) -> Result<(), Box<dyn Error>> {
        unsafe {
            // Clear the screen
            gl::Clear(gl::COLOR_BUFFER_BIT);

            // Use the shader program
            gl::UseProgram(self.program.unwrap());

            // Bind the texture
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, self.texture.unwrap());
            let tex_loc = gl::GetUniformLocation(self.program.unwrap(), c"uTexture".as_ptr());
            gl::Uniform1i(tex_loc, 0);

            // Draw the fullscreen quad
            gl::BindVertexArray(self.vao.unwrap());
            gl::DrawElements(gl::TRIANGLES, 6, gl::UNSIGNED_INT, std::ptr::null());
            gl::BindVertexArray(0);
        }

        Ok(())
    }
}

impl Drop for ImageRenderer {
    fn drop(&mut self) {
        unsafe {
            if let Some(id) = self.texture.take() {
                gl::DeleteTextures(1, &id);
            }
            if let Some(id) = self.vao.take() {
                gl::DeleteVertexArrays(1, &id);
            }
            if let Some(id) = self.vbo.take() {
                gl::DeleteBuffers(1, &id);
            }
            if let Some(id) = self.ebo.take() {
                gl::DeleteBuffers(1, &id);
            }
            if let Some(id) = self.program.take() {
                gl::DeleteProgram(id);
            }
        }
    }
}
