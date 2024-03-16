//! Non-browser UI.

use glutin::display::Display;
use smithay_client_toolkit::reexports::client::protocol::wl_subsurface::WlSubsurface;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::seat::keyboard::Modifiers;
use smithay_client_toolkit::seat::pointer::AxisScroll;

use crate::engine::Engine;
use crate::ui::renderer::{Renderer, Texture, TextureBuilder};
use crate::{gl, Position, Size};

mod renderer;

/// Logical height of the UI surface.
pub const UI_HEIGHT: u32 = 35;

/// Logical height of the UI/content separator.
const SEPARATOR_HEIGHT: f64 = 1.5;

/// Color of the UI/content separator.
const SEPARATOR_COLOR: [u8; 4] = [117, 42, 42, 255];

/// URI bar width percentage from UI.
const URIBAR_WIDTH_PERCENTAGE: f64 = 0.80;
/// URI bar height percentage from UI.
const URIBAR_HEIGHT_PERCENTAGE: f64 = 0.75;

/// URI bar text color.
const URIBAR_FG: [f64; 3] = [1., 1., 1.];
/// URI bar background color.
const URIBAR_BG: [f64; 3] = [0.15, 0.15, 0.15];

/// URI bar padding to left window edge.
const URIBAR_X_PADDING: f64 = 10.;

pub struct Ui {
    renderer: Option<Renderer>,

    subsurface: WlSubsurface,
    surface: WlSurface,
    viewport: WpViewport,

    size: Size,
    scale: f64,

    separator: Separator,
    uribar: Uribar,

    dirty: bool,
}

impl Ui {
    pub fn new((subsurface, surface): (WlSubsurface, WlSurface), viewport: WpViewport) -> Self {
        Self {
            subsurface,
            viewport,
            surface,
            scale: 1.0,
            separator: Default::default(),
            renderer: Default::default(),
            uribar: Default::default(),
            dirty: Default::default(),
            size: Default::default(),
        }
    }

    /// Update the surface geometry.
    pub fn set_geometry(&mut self, display: &Display, position: Position, size: Size) {
        self.size = size;
        self.dirty = true;

        // Update subsurface location.
        self.subsurface.set_position(position.x, position.y);

        // Update renderer and its EGL surface
        let physical_size = size * self.scale;
        match &mut self.renderer {
            Some(renderer) => renderer.set_size(physical_size),
            None => self.renderer = Some(Renderer::new(display, &self.surface, physical_size)),
        }

        // Update UI elements.
        self.uribar.set_size(Uribar::size(size));
    }

    /// Update the render scale.
    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.dirty = true;

        // Resize the renderer and underlying surface.
        let physical_size = self.size * scale;
        if let Some(renderer) = &mut self.renderer {
            renderer.set_size(physical_size);
        }

        // Update UI elements.
        self.uribar.set_scale(scale);
    }

    /// Render current UI state.
    ///
    /// Returns `true` if rendering was performed.
    pub fn draw(&mut self, engine: &dyn Engine) -> bool {
        // Ensure URI is up to date.
        self.uribar.set_uri(engine.uri());

        // Abort early if UI is up to date.
        let dirty = self.dirty();
        let renderer = match &self.renderer {
            Some(renderer) if dirty => renderer,
            _ => return false,
        };
        self.dirty = false;

        // Update browser's viewporter logical render size.
        //
        // NOTE: This must be done every time we draw with Sway; it is not correctly
        // persisted when drawing with the same surface multiple times.
        self.viewport.set_destination(self.size.width as i32, self.size.height as i32);

        // Calculate target positions/sizes before partial mutable borrows.
        let separator_size = self.separator_size();
        let uribar_pos = self.uribar_position(self.uribar.size);

        // Get UI element textures.
        let separator_texture = self.separator.texture();
        let uribar_texture = self.uribar.texture();

        renderer.draw(|renderer| {
            // Render the UI.
            unsafe {
                // Draw background.
                gl::ClearColor(0.1, 0.1, 0.1, 1.0);
                gl::Clear(gl::COLOR_BUFFER_BIT);

                // Draw UI elements.
                renderer.draw_texture_at(separator_texture, (0., 0.).into(), separator_size);
                renderer.draw_texture_at(uribar_texture, uribar_pos.into(), None);
            }
        });

        true
    }

    /// Check whether a surface is owned by this UI.
    pub fn owns_surface(&self, surface: &WlSurface) -> bool {
        &self.surface == surface
    }

    /// Handle scroll axis events.
    pub fn pointer_axis(
        &self,
        _time: u32,
        _position: Position<f64>,
        _horizontal: AxisScroll,
        _vertical: AxisScroll,
        _modifiers: Modifiers,
    ) {
    }

    /// Handle pointer button events.
    pub fn pointer_button(
        &self,
        _time: u32,
        _position: Position<f64>,
        _button: u32,
        _state: u32,
        _modifiers: Modifiers,
    ) {
    }

    /// Handle pointer motion events.
    pub fn pointer_motion(&self, _time: u32, _position: Position<f64>, _modifiers: Modifiers) {}

    /// Handle touch press events.
    pub fn touch_down(
        &mut self,
        _time: u32,
        _id: i32,
        _position: Position<f64>,
        _modifiers: Modifiers,
    ) {
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, _time: u32, _id: i32, _modifiers: Modifiers) {}

    /// Handle touch motion events.
    pub fn touch_motion(
        &mut self,
        _time: u32,
        _id: i32,
        _position: Position<f64>,
        _modifiers: Modifiers,
    ) {
    }

    /// Check whether UI needs redraw.
    fn dirty(&self) -> bool {
        self.dirty || self.uribar.dirty()
    }

    /// Position URI bar.
    fn uribar_position(&self, uribar_size: Size) -> Position {
        let height = self.size.height as f64;
        let logical_y = (SEPARATOR_HEIGHT + height - uribar_size.height as f64) / 2.;
        let y = (logical_y * self.scale).round() as i32;
        Position::new(y, y)
    }

    /// Size of the UI/content separator.
    fn separator_size(&self) -> Size<f32> {
        let mut physical_size = self.size * self.scale;
        physical_size.height = (SEPARATOR_HEIGHT * self.scale).round() as u32;
        physical_size.into()
    }
}

/// URI input UI.
struct Uribar {
    texture: Option<Texture>,
    uri: String,
    size: Size,
    scale: f64,
}

impl Uribar {
    fn new() -> Self {
        Self {
            scale: 1.,
            texture: Default::default(),
            size: Default::default(),
            uri: Default::default(),
        }
    }

    /// Update the output texture size.
    fn set_size(&mut self, size: Size) {
        self.size = size;

        // Force redraw.
        self.texture = None;
    }

    /// Update the output texture scale.
    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;

        // Force redraw.
        self.texture = None;
    }

    /// Update the URI bar's content.
    fn set_uri(&mut self, uri: String) {
        if uri == self.uri {
            return;
        }
        self.uri = uri;

        // Force redraw.
        self.texture = None;
    }

    /// Check if URI bar needs redraw.
    fn dirty(&self) -> bool {
        self.texture.is_none()
    }

    /// Get the OpenGL texture.
    fn texture(&mut self) -> &Texture {
        // Ensure texture is up to date.
        if self.texture.is_none() {
            self.texture = Some(self.draw());
        }

        self.texture.as_ref().unwrap()
    }

    /// Get URI bar size based on its parent.
    fn size(parent_size: Size) -> Size {
        let width = (parent_size.width as f64 * URIBAR_WIDTH_PERCENTAGE).round();
        let height = (parent_size.height as f64 * URIBAR_HEIGHT_PERCENTAGE).round();

        Size::new(width as u32, height as u32)
    }

    /// Draw the URI bar into an OpenGL texture.
    fn draw(&self) -> Texture {
        let physical_size = self.size * self.scale;
        let builder = TextureBuilder::new(physical_size.into(), self.scale);
        builder.clear(URIBAR_BG);

        let position = Position::new(URIBAR_X_PADDING * self.scale, 0.);
        let width = physical_size.width - 2 * position.x.round() as u32;
        let size = Size::new(width, physical_size.height);
        builder.rasterize(&self.uri, URIBAR_FG, position, size.into());

        builder.build()
    }
}

impl Default for Uribar {
    fn default() -> Self {
        Self::new()
    }
}

/// Separator between UI and browser content.
#[derive(Default)]
struct Separator {
    texture: Option<Texture>,
}

impl Separator {
    fn texture(&mut self) -> &Texture {
        // Ensure texture is initialized.
        if self.texture.is_none() {
            self.texture = Some(Texture::new(&SEPARATOR_COLOR, 1, 1));
        }

        self.texture.as_ref().unwrap()
    }
}
