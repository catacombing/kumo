//! Non-engine UI.

use std::borrow::Cow;
use std::mem;
use std::ops::{Bound, Range, RangeBounds};

use funq::MtQueueHandle;
use glutin::display::Display;
use pangocairo::cairo::{Context, Format, ImageSurface};
use pangocairo::pango::Layout;
use smithay_client_toolkit::reexports::client::protocol::wl_subsurface::WlSubsurface;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::{AxisScroll, BTN_LEFT};

use crate::tlds::TLDS;
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

/// Maximum interval between taps to be considered a double/trible-tap.
const MAX_MULTI_TAP_MILLIS: u32 = 300;

/// Search engine base URI.
const SEARCH_URI: &str = "https://duckduckgo.com/?q=";

#[funq::callbacks(State)]
trait UiHandler {
    fn load_uri(&mut self, window: WindowId, uri: String);
}

impl UiHandler for State {
    fn load_uri(&mut self, window_id: WindowId, uri: String) {
        // Perform search if URI is not a recognized URI.
        let uri = match build_uri(uri.trim()) {
            Some(uri) => uri,
            None => Cow::Owned(format!("{SEARCH_URI}{uri}")),
        };

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
    touch_point: Option<i32>,

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
    pub fn pointer_motion(&mut self, time: u32, position: Position<f64>, modifiers: Modifiers) {
        self.touch_motion(time, -1, position, modifiers);
    }

    /// Handle touch press events.
    pub fn touch_down(
        &mut self,
        time: u32,
        id: i32,
        position: Position<f64>,
        modifiers: Modifiers,
    ) {
        // Only accept a single touch point in the UI.
        if self.touch_point.is_some() {
            return;
        }
        self.touch_point = Some(id);

        // Convert position to physical space.
        let position = position * self.scale;

        let uribar_position = position - self.uribar_position(self.uribar.size).into();
        let uribar_size: Size<f64> = (self.uribar.size * self.scale).into();
        if (0.0..uribar_size.width).contains(&uribar_position.x)
            && (0.0..uribar_size.height).contains(&uribar_position.y)
        {
            self.keyboard_focus = Some(KeyboardInputElement::UriBar);

            // Forward touch event.
            self.uribar.touch_down(time, uribar_position, modifiers);

            return;
        }

        self.clear_focus();
    }

    /// Handle touch motion events.
    pub fn touch_motion(
        &mut self,
        _time: u32,
        id: i32,
        position: Position<f64>,
        _modifiers: Modifiers,
    ) {
        // Ignore all unknown touch points.
        if self.touch_point != Some(id) {
            return;
        }

        // Convert position to physical space.
        let position = position * self.scale;

        if let Some(KeyboardInputElement::UriBar) = self.keyboard_focus {
            // Forward touch event.
            let uribar_position = position - self.uribar_position(self.uribar.size).into();
            self.uribar.touch_motion(uribar_position);
        }
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, _time: u32, id: i32, _modifiers: Modifiers) {
        // Ignore all unknown touch points.
        if self.touch_point != Some(id) {
            return;
        }
        self.touch_point = None;

        if let Some(KeyboardInputElement::UriBar) = self.keyboard_focus {
            // Forward touch event.
            self.uribar.touch_up();
        }
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
    size: Size,
    scale: f64,
}

impl Uribar {
    fn new(window: WindowId, mut queue: MtQueueHandle<State>) -> Self {
        // Setup text input with submission handling.
        let mut text_input = TextInput::new();
        text_input.set_submit_handler(Box::new(move |uri| queue.load_uri(window, uri)));

        Self { text_input, scale: 1., texture: Default::default(), size: Default::default() }
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
        self.text_input.set_focus(focused);
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

        // Show cursor or selection when focused.
        if self.text_input.focused {
            if self.text_input.selection.is_some() {
                text_options.selection(self.text_input.selection.clone());
            } else {
                text_options.show_cursor(self.text_input.cursor_index());
            }
        }

        // Draw URI bar.
        builder.rasterize(self.text_input.layout(), text_options);

        // Convert cairo buffer to texture.
        builder.build()
    }

    /// Handle touch press events.
    pub fn touch_down(&mut self, time: u32, position: Position<f64>, modifiers: Modifiers) {
        // Forward event to text input.
        let mut relative_position = position;
        relative_position.x -= URIBAR_X_PADDING * self.scale;
        self.text_input.touch_down(time, relative_position, modifiers);
    }

    /// Handle touch motion events.
    pub fn touch_motion(&mut self, position: Position<f64>) {
        // Forward event to text input.
        let mut relative_position = position;
        relative_position.x -= URIBAR_X_PADDING * self.scale;
        self.text_input.touch_motion(relative_position);
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self) {
        // Forward event to text input.
        self.text_input.touch_up();
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
    selection: Option<Range<i32>>,
    submit_handler: Box<dyn FnMut(String)>,
    last_touch: TouchHistory,
    layout: Layout,
    cursor_index: i32,
    cursor_offset: i32,
    focused: bool,
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
            last_touch: Default::default(),
            selection: Default::default(),
            focused: Default::default(),
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

        // Clear selection.
        self.selection = None;

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

    /// Modify text selection.
    pub fn select<R>(&mut self, range: R)
    where
        R: RangeBounds<i32>,
    {
        let mut start = match range.start_bound() {
            Bound::Included(start) => *start,
            Bound::Excluded(start) => *start + 1,
            Bound::Unbounded => i32::MIN,
        };
        start = start.max(0);
        let mut end = match range.end_bound() {
            Bound::Included(end) => *end + 1,
            Bound::Excluded(end) => *end,
            Bound::Unbounded => i32::MAX,
        };
        end = end.min(self.text().len() as i32);

        if start < end {
            self.selection = Some(start..end);
            self.dirty = true;
        } else {
            self.clear_selection();
        }
    }

    /// Clear text selection.
    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.dirty = true;
    }

    /// Handle new key press.
    pub fn press_key(&mut self, _raw: u32, keysym: Keysym, modifiers: Modifiers) {
        // Ignore input with logo/alt key held.
        if modifiers.logo || modifiers.alt {
            return;
        }

        match (keysym, modifiers.shift, modifiers.ctrl) {
            (Keysym::Return, false, false) => {
                let text = self.text();
                (self.submit_handler)(text);

                self.set_focus(false);
            },
            (Keysym::Left, false, false) => {
                match self.selection.take() {
                    Some(selection) => {
                        self.cursor_index = selection.start;
                        self.cursor_offset = 0;
                    },
                    None => self.move_cursor(-1),
                }
                self.dirty = true;
            },
            (Keysym::Right, false, false) => {
                match self.selection.take() {
                    Some(selection) => {
                        let text_len = self.text().len() as i32;
                        if selection.end >= text_len {
                            self.cursor_index = text_len - 1;
                            self.cursor_offset = 1;
                        } else {
                            self.cursor_index = selection.end;
                            self.cursor_offset = 0;
                        }
                    },
                    None => self.move_cursor(1),
                }
                self.dirty = true;
            },
            (Keysym::BackSpace, false, false) => {
                match self.selection.take() {
                    Some(selection) => self.delete_selected(selection),
                    None => {
                        // Find byte index of character after the cursor.
                        let end_index = self.cursor_index() as usize;

                        // Find byte index of character before the cursor and update the cursor.
                        self.move_cursor(-1);
                        let start_index = self.cursor_index() as usize;

                        // Remove all bytes in the range from the text.
                        let mut text = self.text();
                        text.drain(start_index..end_index);
                        self.layout.set_text(&text);
                    },
                }

                self.dirty = true;
            },
            (Keysym::Delete, false, false) => {
                match self.selection.take() {
                    Some(selection) => self.delete_selected(selection),
                    None => {
                        // Ignore DEL if cursor is the end of the input.
                        let mut text = self.text();
                        if text.len() as i32 == self.cursor_index + self.cursor_offset {
                            return;
                        }

                        // Find byte index of character after the cursor.
                        let start_index = self.cursor_index() as usize;

                        // Find byte index of end of the character after the cursor.
                        //
                        // We use cursor motion here to ensure grapheme clusters are handled
                        // appropriately.
                        self.move_cursor(1);
                        let end_index = self.cursor_index() as usize;
                        self.move_cursor(-1);

                        // Remove all bytes in the range from the text.
                        text.drain(start_index..end_index);
                        self.layout.set_text(&text);
                    },
                }

                self.dirty = true;
            },
            (keysym, _, false) => {
                // Delete selection before writing new text.
                if let Some(selection) = self.selection.take() {
                    self.delete_selected(selection);
                }

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

    /// Delete the selected text.
    ///
    /// This automatically places the cursor at the start of the selection.
    pub fn delete_selected(&mut self, selection: Range<i32>) {
        // Remove selected text from input.
        let range = selection.start as usize..selection.end as usize;
        let mut text = self.text().to_string();
        text.drain(range);
        self.layout.set_text(&text);

        // Update cursor.
        if selection.start > 0 && selection.start == text.len() as i32 {
            self.cursor_index = selection.start - 1;
            self.cursor_offset = 1;
        } else {
            self.cursor_index = selection.start;
            self.cursor_offset = 0;
        }
    }

    /// Handle touch press events.
    pub fn touch_down(&mut self, time: u32, position: Position<f64>, modifiers: Modifiers) {
        if modifiers.logo || modifiers.shift {
            return;
        }

        // Get byte offset from X/Y position.
        let x = (position.x * pangocairo::pango::SCALE as f64).round() as i32;
        let y = (position.y * pangocairo::pango::SCALE as f64).round() as i32;
        let (_, index, offset) = self.layout.xy_to_index(x, y);

        // Update touch history.
        let multi_taps = self.last_touch.push(time, index, offset);

        // Handle single/double/triple-taps.
        match multi_taps {
            0 => {
                // Whether touch is modifying one of the selection boundaries.
                if let Some(selection) = self.selection.as_ref() {
                    self.last_touch.moving_selection_start = selection.start == index + offset;
                    self.last_touch.moving_selection_end = selection.end == index + offset;
                }

                if !self.last_touch.moving_selection_start && !self.last_touch.moving_selection_end
                {
                    // Update cursor index.
                    self.cursor_index = index;
                    self.cursor_offset = offset;

                    // Clear selection.
                    self.selection = None;
                }
            },
            1 => {
                // Select entire word at touch location.
                let text = self.text();
                let mut word_start = 0;
                let mut word_end = text.len() as i32;
                for (i, c) in text.char_indices() {
                    let i = i as i32;
                    if i + 1 < index + offset && !c.is_alphanumeric() {
                        word_start = i + 1;
                    } else if i > index + offset && !c.is_alphanumeric() {
                        word_end = i;
                        break;
                    }
                }
                self.select(word_start..word_end);
            },
            2 => {
                // Select everything.
                self.select(..);
            },
            _ => unreachable!(),
        }

        // Ensure focus when receiving touch input.
        self.set_focus(true);

        self.dirty = true;
    }

    /// Handle touch motion events.
    pub fn touch_motion(&mut self, position: Position<f64>) {
        let selection = match &mut self.selection {
            Some(selection)
                if self.last_touch.moving_selection_start
                    || self.last_touch.moving_selection_end =>
            {
                selection
            },
            _ => return,
        };

        // Get byte offset from X/Y position.
        let x = (position.x * pangocairo::pango::SCALE as f64).round() as i32;
        let y = (position.y * pangocairo::pango::SCALE as f64).round() as i32;
        let (_, index, offset) = self.layout.xy_to_index(x, y);

        // Update selection if it is at least one character wide.
        if self.last_touch.moving_selection_start && index + offset != selection.end {
            selection.start = index + offset;
        } else if self.last_touch.moving_selection_end && index + offset != selection.start {
            selection.end = index + offset;
        }

        // Swap modified side when input carets "overtake" each other.
        if selection.start > selection.end {
            mem::swap(&mut selection.start, &mut selection.end);
            mem::swap(
                &mut self.last_touch.moving_selection_start,
                &mut self.last_touch.moving_selection_end,
            );
        }

        self.dirty = true;
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self) {
        self.dirty = true;
    }

    /// Set input focus.
    fn set_focus(&mut self, focused: bool) {
        // Update selection on focus change.
        if focused && !self.focused {
            self.select(..);
        } else if !focused && self.focused {
            self.clear_selection();
        }

        self.focused = focused;
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

/// Simplified touch history for double/triple-tap tracking.
#[derive(Default)]
struct TouchHistory {
    last_touch: u32,
    cursor_index: i32,
    cursor_offset: i32,
    repeats: usize,
    moving_selection_start: bool,
    moving_selection_end: bool,
}

impl TouchHistory {
    /// Add a new touch event.
    ///
    /// This returns the number of times consecutive taps (0-2).
    pub fn push(&mut self, time: u32, cursor_index: i32, cursor_offset: i32) -> usize {
        if self.repeats < 2
            && self.last_touch + MAX_MULTI_TAP_MILLIS >= time
            && cursor_index == self.cursor_index
        {
            self.repeats += 1;
        } else {
            self.cursor_index = cursor_index;
            self.cursor_offset = cursor_offset;
            self.moving_selection_start = false;
            self.moving_selection_end = false;
            self.repeats = 0;
        }

        self.last_touch = time;

        self.repeats
    }
}

#[allow(rustdoc::bare_urls)]
/// Extract HTTP URI from uri bar input.
///
/// # Examples
///
/// | input                         | output                                      |
/// | ----------------------------- | ------------------------------------------- |
/// | `"https://example.org"`       | `Some("https://example.org")`               |
/// | `"example.org"`               | `Some("https://example.org")`               |
/// | `"example.org/space in path"` | `Some("https://example.org/space in path")` |
/// | `"/home"`                     | `Some("file:///home")`                      |
/// | `"example org"`               | `None`                                      |
/// | `"ftp://example.org"`         | `None`                                      |
fn build_uri(mut input: &str) -> Option<Cow<'_, str>> {
    let uri = Cow::Borrowed(input);

    // If input starts with `/`, we assume it's a path.
    if input.starts_with('/') {
        return Some(Cow::Owned(format!("file://{uri}")));
    }

    // Parse scheme, short-circuiting if an unknown scheme was found.
    const ALLOWED_SCHEMES: &[&str] = &["http", "https", "file"];
    let mut has_scheme = false;
    let mut has_port = false;
    if let Some(index) = input.find(|c: char| !c.is_alphabetic()) {
        if input[index..].starts_with(':') {
            has_scheme = ALLOWED_SCHEMES.contains(&&input[..index]);
            if has_scheme {
                // Allow arbitrary number of slashes after the scheme.
                input = input[index + 1..].trim_start_matches('/');
            } else {
                // Check if we're dealing with a local address + port, instead of scheme.
                // Example: "localhost:80/index"
                has_port = index + 1 < input.len()
                    && &input[index + 1..index + 2] != "/"
                    && input[index + 1..].chars().take_while(|c| *c != '/').all(|c| c.is_numeric());

                if has_port {
                    input = &input[..index];
                } else {
                    return None;
                }
            }
        }
    }

    if !has_port {
        // Allow all characters after a slash.
        if let Some(index) = input.find('/') {
            input = &input[..index];
        }

        // Parse port.
        if let Some(index) = input.rfind(':') {
            has_port =
                index + 1 < input.len() && input[index + 1..].chars().all(|c| c.is_numeric());
            if has_port {
                input = &input[..index];
            }
        }
    }

    // Abort if the domain contains any illegal characters.
    if input.find(|c: char| !c.is_alphanumeric() && c != '-' && c != '.').is_some() {
        return None;
    }

    // Skip TLD check if scheme was explicitly specified.
    if has_scheme {
        return Some(uri);
    }

    // Check for valid TLD.
    match input.rfind('.') {
        Some(tld_index) if TLDS.contains(&input[tld_index + 1..].to_uppercase().as_str()) => {
            Some(Cow::Owned(format!("https://{uri}")))
        },
        // Accept no TLD only if a port was explicitly specified.
        None if has_port => Some(Cow::Owned(format!("https://{uri}"))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_uri() {
        assert_eq!(build_uri("https://example.org").as_deref(), Some("https://example.org"));
        assert_eq!(build_uri("example.org").as_deref(), Some("https://example.org"));
        assert_eq!(build_uri("x.org/space path").as_deref(), Some("https://x.org/space path"));
        assert_eq!(build_uri("/home/user").as_deref(), Some("file:///home/user"));
        assert_eq!(build_uri("https://x.org:666").as_deref(), Some("https://x.org:666"));
        assert_eq!(build_uri("example.org:666").as_deref(), Some("https://example.org:666"));
        assert_eq!(build_uri("https://example:666").as_deref(), Some("https://example:666"));
        assert_eq!(build_uri("example:666").as_deref(), Some("https://example:666"));
        assert_eq!(build_uri("example:666/x").as_deref(), Some("https://example:666/x"));
        assert_eq!(build_uri("https://exa-mple.org").as_deref(), Some("https://exa-mple.org"));
        assert_eq!(build_uri("exa-mple.org").as_deref(), Some("https://exa-mple.org"));
        assert_eq!(build_uri("https:123").as_deref(), Some("https:123"));
        assert_eq!(build_uri("https:123:456").as_deref(), Some("https:123:456"));
        assert_eq!(build_uri("/test:123").as_deref(), Some("file:///test:123"));

        assert_eq!(build_uri("example org").as_deref(), None);
        assert_eq!(build_uri("ftp://example.org").as_deref(), None);
        assert_eq!(build_uri("space in scheme:example.org").as_deref(), None);
        assert_eq!(build_uri("example.invalidtld").as_deref(), None);
        assert_eq!(build_uri("example.org:/").as_deref(), None);
        assert_eq!(build_uri("example:/").as_deref(), None);
        assert_eq!(build_uri("xxx:123:456").as_deref(), None);
    }
}
