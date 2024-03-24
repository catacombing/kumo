//! Non-browser UI.

use funq::MtQueueHandle;
use glutin::display::Display;
use pangocairo::cairo::{Context, Format, ImageSurface};
use pangocairo::pango::Layout;
use smithay_client_toolkit::reexports::client::protocol::wl_subsurface::WlSubsurface;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::{AxisScroll, BTN_LEFT};

use crate::ui::renderer::{Renderer, TextOptions, Texture, TextureBuilder};
use crate::{gl, Position, Size, State, WindowId};

mod renderer;

/// Logical height of the UI surface.
pub const UI_HEIGHT: u32 = 50;

/// Logical height of the UI/content separator.
const SEPARATOR_HEIGHT: f64 = 1.5;

/// Color of the UI/content separator.
const SEPARATOR_COLOR: [u8; 4] = [117, 42, 42, 255];

/// URI bar width percentage from UI.
const URIBAR_WIDTH_PERCENTAGE: f64 = 0.80;
/// URI bar height percentage from UI.
const URIBAR_HEIGHT_PERCENTAGE: f64 = 0.60;

/// URI bar text color.
const URIBAR_FG: [f64; 3] = [1., 1., 1.];
/// URI bar background color.
const URIBAR_BG: [f64; 3] = [0.15, 0.15, 0.15];

/// URI bar padding to left window edge.
const URIBAR_X_PADDING: f64 = 10.;

#[funq::callbacks(State)]
trait UiHandler {
    fn load_uri(&mut self, window: WindowId, uri: String);
}

impl UiHandler for State {
    fn load_uri(&mut self, window_id: WindowId, uri: String) {
        if let Some(window) = self.windows.get(&window_id) {
            if let Some(engine) = self.engines.get(&window.active_tab()) {
                engine.load_uri(&uri);
            }
        }
    }
}

pub struct Ui {
    renderer: Option<Renderer>,

    subsurface: WlSubsurface,
    surface: WlSurface,
    viewport: WpViewport,

    size: Size,
    scale: f64,

    separator: Separator,
    uribar: Uribar,

    keyboard_focus: Option<KeyboardInputElement>,
    touch_point: Option<(i32, Position<f64>)>,

    dirty: bool,
}

impl Ui {
    pub fn new(
        window: WindowId,
        queue: MtQueueHandle<State>,
        (subsurface, surface): (WlSubsurface, WlSurface),
        viewport: WpViewport,
    ) -> Self {
        // Focus URI bar on window creation.
        let keyboard_focus = Some(KeyboardInputElement::UriBar);
        let mut uribar = Uribar::new(window, queue);
        uribar.set_focused(true);

        Self {
            keyboard_focus,
            subsurface,
            viewport,
            surface,
            uribar,
            scale: 1.0,
            touch_point: Default::default(),
            separator: Default::default(),
            renderer: Default::default(),
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
    pub fn draw(&mut self) -> bool {
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

    /// Check if the keyboard focus is on a UI input element.
    pub fn has_keyboard_focus(&self) -> bool {
        self.keyboard_focus.is_some()
    }

    /// Check whether a surface is owned by this UI.
    pub fn owns_surface(&self, surface: &WlSurface) -> bool {
        &self.surface == surface
    }

    /// Handle new key press.
    pub fn press_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        if let Some(KeyboardInputElement::UriBar) = self.keyboard_focus {
            self.uribar.text_input.press_key(raw, keysym, modifiers)
        }
    }

    /// Handle key release.
    pub fn release_key(&self, _raw: u32, _keysym: Keysym, _modifiers: Modifiers) {}

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
        &mut self,
        time: u32,
        position: Position<f64>,
        button: u32,
        state: u32,
        modifiers: Modifiers,
    ) {
        // Emulate touch input using touch point `-1`.
        match state {
            0 if button == BTN_LEFT => self.touch_up(time, -1, modifiers),
            1 if button == BTN_LEFT => self.touch_down(time, -1, position, modifiers),
            _ => (),
        }
    }

    /// Handle pointer motion events.
    pub fn pointer_motion(&self, _time: u32, _position: Position<f64>, _modifiers: Modifiers) {}

    /// Handle touch press events.
    pub fn touch_down(
        &mut self,
        _time: u32,
        id: i32,
        position: Position<f64>,
        _modifiers: Modifiers,
    ) {
        // Only accept a single touch point in the UI.
        if self.touch_point.is_some() {
            return;
        }

        self.touch_point = Some((id, position));
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, _time: u32, id: i32, modifiers: Modifiers) {
        // Ignore all unknown touch points.
        let position = match self.touch_point {
            Some((ui_id, position)) if ui_id == id => position,
            _ => return,
        };
        self.touch_point = None;

        // Convert position to physical space.
        let position = position * self.scale;

        // Forward URI input clicks.
        let uribar_position = position - self.uribar_position(self.uribar.size).into();
        let uribar_size: Size<f64> = (self.uribar.size * self.scale).into();
        if (0.0..uribar_size.width).contains(&uribar_position.x)
            && (0.0..uribar_size.height).contains(&uribar_position.y)
        {
            self.keyboard_focus = Some(KeyboardInputElement::UriBar);

            // Forward mouse button.
            self.uribar.touch_up(uribar_position, modifiers);

            return;
        }

        self.clear_focus();
    }

    /// Handle touch motion events.
    pub fn touch_motion(
        &mut self,
        _time: u32,
        _id: i32,
        _position: Position<f64>,
        _modifiers: Modifiers,
    ) {
    }

    /// Update the URI bar's content.
    pub fn set_uri(&mut self, uri: &str) {
        self.uribar.set_uri(uri);
    }

    /// Clear UI keyboard focus.
    pub fn clear_focus(&mut self) {
        self.uribar.set_focused(false);
        self.keyboard_focus = None;
    }

    /// Check whether UI needs redraw.
    pub fn dirty(&self) -> bool {
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
    text_input: TextInput,
    focused: bool,
    size: Size,
    scale: f64,
}

impl Uribar {
    fn new(window: WindowId, mut queue: MtQueueHandle<State>) -> Self {
        // Setup text input with submission handling.
        let mut text_input = TextInput::new();
        text_input.set_submit_handler(Box::new(move |uri| queue.load_uri(window, uri)));

        Self {
            text_input,
            scale: 1.,
            focused: Default::default(),
            texture: Default::default(),
            size: Default::default(),
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
    fn set_uri(&mut self, uri: &str) {
        if uri == self.text_input.text() {
            return;
        }
        self.text_input.set_text(uri);

        // Force redraw.
        self.texture = None;
    }

    /// Set URI bar input focus.
    fn set_focused(&mut self, focused: bool) {
        self.text_input.dirty |= self.focused != focused;
        self.focused = focused;
    }

    /// Check if URI bar needs redraw.
    fn dirty(&self) -> bool {
        self.texture.is_none() || self.text_input.dirty
    }

    /// Get the OpenGL texture.
    fn texture(&mut self) -> &Texture {
        // Ensure texture is up to date.
        if self.texture.is_none() || self.text_input.dirty {
            self.texture = Some(self.draw());
            self.text_input.dirty = false;
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
        // Draw background color.
        let physical_size = self.size * self.scale;
        let builder = TextureBuilder::new(physical_size.into(), self.scale);
        builder.clear(URIBAR_BG);

        // Set text rendering options.
        let position = Position::new(URIBAR_X_PADDING * self.scale, 0.);
        let width = physical_size.width - 2 * position.x.round() as u32;
        let size = Size::new(width, physical_size.height);
        let mut text_options = TextOptions::new();
        text_options.position(position);
        text_options.size(size.into());
        text_options.text_color(URIBAR_FG);

        // Show cursor when focused.
        if self.focused {
            text_options.show_cursor(self.text_input.cursor_index());
        }

        // Draw URI bar.
        builder.rasterize(self.text_input.layout(), text_options);

        // Convert cairo buffer to texture.
        builder.build()
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, position: Position<f64>, modifiers: Modifiers) {
        self.set_focused(true);

        // Forward event to text input.
        let mut relative_position = position;
        relative_position.x -= URIBAR_X_PADDING * self.scale;
        self.text_input.touch_up(relative_position, modifiers);
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

/// Elements accepting keyboard focus.
enum KeyboardInputElement {
    UriBar,
}

/// Text input field.
struct TextInput {
    submit_handler: Box<dyn FnMut(String)>,
    layout: Layout,
    cursor_index: i32,
    cursor_offset: i32,
    dirty: bool,
}

impl TextInput {
    fn new() -> Self {
        // Create pango layout.
        let image_surface = ImageSurface::create(Format::ARgb32, 0, 0).unwrap();
        let context = Context::new(&image_surface).unwrap();
        let layout = pangocairo::functions::create_layout(&context);

        Self {
            layout,
            submit_handler: Box::new(|_| {}),
            cursor_offset: Default::default(),
            cursor_index: Default::default(),
            dirty: Default::default(),
        }
    }

    /// Update return key handler.
    fn set_submit_handler(&mut self, handler: Box<dyn FnMut(String)>) {
        self.submit_handler = handler;
    }

    /// Update the field's text.
    ///
    /// This automatically positions the cursor at the end of the text.
    fn set_text(&mut self, text: &str) {
        self.layout.set_text(text);

        // Move cursor to the beginning.
        if text.is_empty() {
            self.cursor_index = 0;
            self.cursor_offset = 0;
        } else {
            self.cursor_index = text.len() as i32 - 1;
            self.cursor_offset = 1;
        }

        self.dirty = true;
    }

    /// Get current text content.
    fn text(&self) -> String {
        self.layout.text().to_string()
    }

    /// Get underlying pango layout.
    fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Handle new key press.
    pub fn press_key(&mut self, _raw: u32, keysym: Keysym, modifiers: Modifiers) {
        // Ignore input with logo/alt key held.
        if modifiers.logo || modifiers.alt {
            return;
        }

        match (keysym, modifiers.shift, modifiers.ctrl) {
            (Keysym::Left, false, false) => {
                self.move_cursor(-1);
                self.dirty = true;
            },
            (Keysym::Right, false, false) => {
                self.move_cursor(1);
                self.dirty = true;
            },
            (Keysym::BackSpace, false, false) => {
                // Find byte index of character after the cursor.
                let end_index = self.cursor_index();

                // Find byte index of character before the cursor and update the cursor.
                self.move_cursor(-1);
                let start_index = self.cursor_index();

                // Remove all bytes in the range from the text.
                let mut text = self.text();
                for index in (start_index..end_index).rev() {
                    text.remove(index as usize);
                }
                self.layout.set_text(&text);

                self.dirty = true;
            },
            (Keysym::Return, false, false) => {
                let text = self.text();
                (self.submit_handler)(text);
            },
            (keysym, _, false) => {
                if let Some(key_char) = keysym.key_char() {
                    // Add character to text.
                    let index = self.cursor_index() as usize;
                    let mut text = self.text();
                    text.insert(index, key_char);
                    self.layout.set_text(&text);

                    // Move cursor behind the new character.
                    self.move_cursor(1);

                    self.dirty = true;
                }
            },
            _ => (),
        }
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, position: Position<f64>, modifiers: Modifiers) {
        if modifiers.logo || modifiers.shift {
            return;
        }

        // Get byte offset from X/Y position.
        let x = (position.x * pangocairo::pango::SCALE as f64).round() as i32;
        let y = (position.y * pangocairo::pango::SCALE as f64).round() as i32;
        let (_, index, offset) = self.layout.xy_to_index(x, y);

        // Update cursor index.
        self.cursor_index = index;
        self.cursor_offset = offset;
        self.dirty = true;
    }

    /// Move the text input cursor.
    fn move_cursor(&mut self, positions: i32) {
        let (cursor, offset) = self.layout.move_cursor_visually(
            true,
            self.cursor_index,
            self.cursor_offset,
            positions,
        );

        if (0..i32::MAX).contains(&cursor) {
            self.cursor_index = cursor;
            self.cursor_offset = offset;
        }
    }

    /// Get current cursor's byte offset.
    fn cursor_index(&self) -> i32 {
        self.cursor_index + self.cursor_offset
    }
}

impl Default for TextInput {
    fn default() -> Self {
        Self::new()
    }
}
