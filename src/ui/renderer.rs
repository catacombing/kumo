//! OpenGL UI rendering.

use std::ffi::{CStr, CString};
use std::num::NonZeroU32;
use std::{cmp, mem, ptr};

use glutin::config::{Api, ConfigTemplateBuilder};
use glutin::context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext, Version};
use glutin::display::Display;
use glutin::prelude::*;
use glutin::surface::{Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use pangocairo::cairo::{Context, Format, ImageSurface};
use pangocairo::pango::{Alignment, EllipsizeMode, FontDescription, Layout};
use raw_window_handle::{RawWindowHandle, WaylandWindowHandle};
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::Proxy;

use crate::gl::types::{GLfloat, GLint, GLuint};
use crate::{gl, Position, Size};

// OpenGL shader programs.
const VERTEX_SHADER: &str = include_str!("../../shaders/vertex.glsl");
const FRAGMENT_SHADER: &str = include_str!("../../shaders/fragment.glsl");

/// OpenGL renderer.
#[derive(Debug)]
pub struct Renderer {
    uniform_position: GLint,
    uniform_matrix: GLint,

    egl_surface: Surface<WindowSurface>,
    egl_context: PossiblyCurrentContext,

    size: Size<f32>,
}

impl Renderer {
    /// Initialize a new renderer.
    pub fn new(display: &Display, surface: &WlSurface, size: Size) -> Self {
        // Setup OpenGL symbol loader.
        gl::load_with(|symbol| {
            let symbol = CString::new(symbol).unwrap();
            display.get_proc_address(symbol.as_c_str()).cast()
        });

        // Create EGL surface.
        let (egl_surface, egl_context) = Self::create_surface(display, surface, size);

        // Setup OpenGL program.
        let (uniform_position, uniform_matrix) = Self::create_program();

        Renderer { uniform_position, uniform_matrix, egl_surface, egl_context, size: size.into() }
    }

    /// Perform drawing with this renderer.
    pub fn draw<F: FnMut(&Renderer)>(&self, mut fun: F) {
        self.egl_context.make_current(&self.egl_surface).unwrap();

        fun(self);

        unsafe { gl::Flush() };

        self.egl_surface.swap_buffers(&self.egl_context).unwrap();
    }

    /// Render texture at a position in viewport-coordinates.
    ///
    /// Specifying a `size` will automatically scale the texture to render at
    /// the desired size. Otherwise the texture's size will be used instead.
    pub unsafe fn draw_texture_at(
        &self,
        texture: &Texture,
        mut position: Position<f32>,
        size: impl Into<Option<Size<f32>>>,
    ) {
        let (width, height) = match size.into() {
            Some(Size { width, height }) => (width, height),
            None => (texture.width as f32, texture.height as f32),
        };

        // Matrix transforming vertex positions to desired size.
        let x_scale = width / self.size.width;
        let y_scale = height / self.size.height;
        let matrix = [x_scale, 0., 0., y_scale];
        gl::UniformMatrix2fv(self.uniform_matrix, 1, gl::FALSE, matrix.as_ptr());

        // Set texture position offset.
        position.x /= self.size.width / 2.;
        position.y /= self.size.height / 2.;
        gl::Uniform2fv(self.uniform_position, 1, [position.x, -position.y].as_ptr());

        gl::BindTexture(gl::TEXTURE_2D, texture.id);

        gl::DrawArrays(gl::TRIANGLES, 0, 6);
    }

    /// Update viewport size.
    pub fn set_size(&mut self, size: Size) {
        unsafe { gl::Viewport(0, 0, size.width as i32, size.height as i32) };

        // Update surface size.
        self.egl_surface.resize(
            &self.egl_context,
            NonZeroU32::new(size.width).unwrap(),
            NonZeroU32::new(size.height).unwrap(),
        );

        self.size = size.into();
    }

    /// Create a new EGL surface.
    fn create_surface(
        display: &Display,
        surface: &WlSurface,
        size: Size,
    ) -> (Surface<WindowSurface>, PossiblyCurrentContext) {
        assert!(size.width > 0 && size.height > 0);

        // Create EGL config.
        let config_template = ConfigTemplateBuilder::new().with_api(Api::GLES2).build();
        let egl_config = unsafe {
            display
                .find_configs(config_template)
                .ok()
                .and_then(|mut configs| configs.next())
                .unwrap()
        };

        // Create EGL context.
        let context_attributes = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(Version::new(2, 0))))
            .build(None);
        let egl_context =
            unsafe { display.create_context(&egl_config, &context_attributes).unwrap() };
        let egl_context = egl_context.treat_as_possibly_current();

        let mut raw_window_handle = WaylandWindowHandle::empty();
        raw_window_handle.surface = surface.id().as_ptr().cast();
        let raw_window_handle = RawWindowHandle::Wayland(raw_window_handle);
        let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            raw_window_handle,
            NonZeroU32::new(size.width).unwrap(),
            NonZeroU32::new(size.height).unwrap(),
        );

        let egl_surface =
            unsafe { display.create_window_surface(&egl_config, &surface_attributes).unwrap() };

        // Ensure rendering never blocks.
        egl_context.make_current(&egl_surface).unwrap();
        egl_surface.set_swap_interval(&egl_context, SwapInterval::DontWait).unwrap();

        (egl_surface, egl_context)
    }

    /// Create the OpenGL program.
    fn create_program() -> (GLint, GLint) {
        unsafe {
            // Create vertex shader.
            let vertex_shader = gl::CreateShader(gl::VERTEX_SHADER);
            gl::ShaderSource(
                vertex_shader,
                1,
                [VERTEX_SHADER.as_ptr()].as_ptr() as *const _,
                &(VERTEX_SHADER.len() as i32) as *const _,
            );
            gl::CompileShader(vertex_shader);

            // Create fragment shader.
            let fragment_shader = gl::CreateShader(gl::FRAGMENT_SHADER);
            gl::ShaderSource(
                fragment_shader,
                1,
                [FRAGMENT_SHADER.as_ptr()].as_ptr() as *const _,
                &(FRAGMENT_SHADER.len() as i32) as *const _,
            );
            gl::CompileShader(fragment_shader);

            // Create shader program.
            let program = gl::CreateProgram();
            gl::AttachShader(program, vertex_shader);
            gl::AttachShader(program, fragment_shader);
            gl::LinkProgram(program);
            gl::UseProgram(program);

            // Generate VBO.
            let mut vbo = 0;
            gl::GenBuffers(1, &mut vbo);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);

            // Fill VBO with vertex positions.
            #[rustfmt::skip]
            let vertices: [GLfloat; 12] = [
                -1.0,  1.0, // Top-left
                -1.0, -1.0, // Bottom-left
                 1.0, -1.0, // Bottom-right

                -1.0,  1.0, // Top-left
                 1.0, -1.0, // Bottom-right
                 1.0,  1.0, // Top-right
            ];
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (mem::size_of::<GLfloat>() * vertices.len()) as isize,
                vertices.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );

            // Define VBO layout.
            let name = CStr::from_bytes_with_nul(b"aVertexPosition\0").unwrap();
            let location = gl::GetAttribLocation(program, name.as_ptr()) as GLuint;
            gl::VertexAttribPointer(
                location,
                2,
                gl::FLOAT,
                gl::FALSE,
                2 * mem::size_of::<GLfloat>() as i32,
                ptr::null(),
            );
            gl::EnableVertexAttribArray(0);

            // Get uniform locations.
            let name = CStr::from_bytes_with_nul(b"uPosition\0").unwrap();
            let uniform_position = gl::GetUniformLocation(program, name.as_ptr());
            let name = CStr::from_bytes_with_nul(b"uMatrix\0").unwrap();
            let uniform_matrix = gl::GetUniformLocation(program, name.as_ptr());

            (uniform_position, uniform_matrix)
        }
    }
}

/// OpenGL texture.
#[derive(Debug)]
pub struct Texture {
    id: u32,
    pub width: usize,
    pub height: usize,
}

impl Drop for Texture {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteTextures(1, &self.id);
        }
    }
}

impl Texture {
    /// Load a buffer as texture into OpenGL.
    pub fn new(buffer: &[u8], width: usize, height: usize) -> Self {
        assert!(buffer.len() == width * height * 4);

        unsafe {
            let mut id = 0;
            gl::GenTextures(1, &mut id);
            gl::BindTexture(gl::TEXTURE_2D, id);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as i32,
                width as i32,
                height as i32,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                buffer.as_ptr() as *const _,
            );
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            Self { id, width, height }
        }
    }
}

/// Cairo-based graphics rendering.
pub struct TextureBuilder {
    image_surface: ImageSurface,
    font: FontDescription,
    context: Context,
    size: Size<i32>,
}

impl TextureBuilder {
    pub fn new(size: Size<i32>, scale: f64) -> Self {
        let image_surface = ImageSurface::create(Format::ARgb32, size.width, size.height).unwrap();
        let context = Context::new(&image_surface).unwrap();

        let mut font = FontDescription::from_string("sans 16px");
        font.set_absolute_size(font.size() as f64 * scale);

        Self { image_surface, context, size, font }
    }

    /// Fill entire buffer with a single color.
    pub fn clear(&self, color: [f64; 3]) {
        self.context.set_source_rgb(color[0], color[1], color[2]);
        self.context.paint().unwrap();
    }

    /// Draw text within the specified bounds.
    pub fn rasterize(&self, layout: &Layout, text_options: TextOptions) {
        layout.set_font_description(Some(&self.font));

        // Limit text size to builder limits.
        let position = text_options.position;
        let size = match text_options.size {
            Some(mut size) => {
                size.width = cmp::min(size.width, self.size.width - position.x.round() as i32);
                size.height = cmp::min(size.height, self.size.height - position.y.round() as i32);
                size
            },
            None => {
                let width = self.size.width - position.x.round() as i32;
                let height = self.size.height - position.y.round() as i32;
                Size::new(width, height)
            },
        };

        // Truncate text beyond specified bounds.
        layout.set_width(size.width * pangocairo::pango::SCALE);
        layout.set_height(size.height * pangocairo::pango::SCALE);
        layout.set_ellipsize(EllipsizeMode::End);

        // Set text position.
        layout.set_alignment(Alignment::Left);
        let (_, text_height) = layout.pixel_size();
        let y = position.y + size.height as f64 / 2. - text_height as f64 / 2.;
        self.context.move_to(position.x, y);

        // Set foreground color.
        let color = text_options.text_color;
        self.context.set_source_rgb(color[0], color[1], color[2]);

        pangocairo::functions::show_layout(&self.context, layout);

        // Draw text input cursor.
        if (0..i32::MAX).contains(&text_options.cursor_pos) {
            // Get cursor rect and convert it from pango coordinates.
            let (cursor_rect, _) = layout.cursor_pos(text_options.cursor_pos);
            let cursor_x = position.x + cursor_rect.x() as f64 / pangocairo::pango::SCALE as f64;
            let cursor_y = y + cursor_rect.y() as f64 / pangocairo::pango::SCALE as f64;
            let cursor_height = cursor_rect.height() as f64 / pangocairo::pango::SCALE as f64;

            // Draw cursor line.
            self.context.move_to(cursor_x, cursor_y);
            self.context.line_to(cursor_x, cursor_y + cursor_height);
            self.context.stroke_preserve().unwrap();
        }
    }

    /// Finalize the output texture.
    pub fn build(self) -> Texture {
        drop(self.context);

        let width = self.image_surface.width() as usize;
        let height = self.image_surface.height() as usize;
        let data = self.image_surface.take_data().unwrap();
        Texture::new(&data, width, height)
    }
}

/// Options for text rendering.
pub struct TextOptions {
    text_color: [f64; 3],
    position: Position<f64>,
    size: Option<Size<i32>>,
    cursor_pos: i32,
}

impl TextOptions {
    pub fn new() -> Self {
        Self {
            text_color: [1.; 3],
            cursor_pos: -1,
            position: Default::default(),
            size: Default::default(),
        }
    }

    /// Set text color.
    pub fn text_color(&mut self, color: [f64; 3]) {
        self.text_color = color;
    }

    /// Set text position.
    pub fn position(&mut self, position: Position<f64>) {
        self.position = position;
    }

    /// Set text size.
    pub fn size(&mut self, size: Size<i32>) {
        self.size = Some(size);
    }

    /// Show text input cursor.
    pub fn show_cursor(&mut self, pos: i32) {
        self.cursor_pos = pos;
    }
}

impl Default for TextOptions {
    fn default() -> Self {
        Self::new()
    }
}
