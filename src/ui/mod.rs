//! Non-engine UI.

use std::borrow::Cow;
use std::mem;
use std::ops::{Bound, Range, RangeBounds};

use _text_input::zwp_text_input_v3::{ChangeCause, ContentHint, ContentPurpose};
use funq::MtQueueHandle;
use glutin::display::Display;
use pangocairo::cairo::{Context, Format, ImageSurface};
use pangocairo::pango::{Alignment, Layout, SCALE as PANGO_SCALE};
use smithay_client_toolkit::reexports::client::protocol::wl_subsurface::WlSubsurface;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::protocols::wp::text_input::zv3::client as _text_input;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};

use crate::tlds::TLDS;
use crate::ui::renderer::{Renderer, TextOptions, Texture, TextureBuilder};
use crate::window::TextInput;
use crate::{gl, Position, Size, State, WindowId};

mod renderer;
pub mod tabs;

/// Logical height of the UI surface.
pub const UI_HEIGHT: u32 = 50;

/// Logical height of the UI/content separator.
const SEPARATOR_HEIGHT: f64 = 1.5;

/// Logical width and height of the tabs button.
const TABS_BUTTON_SIZE: u32 = 29;

/// Color of the UI/content separator.
const SEPARATOR_COLOR: [u8; 4] = [117, 42, 42, 255];

/// URI bar height percentage from UI.
const URIBAR_HEIGHT_PERCENTAGE: f64 = 0.6;

/// UI background color.
const UI_BG: [f64; 3] = [0.1, 0.1, 0.1];

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
    /// Change the active engine's URI.
    fn load_uri(&mut self, window: WindowId, uri: String);

    /// Open tabs UI.
    fn show_tabs(&mut self, window: WindowId);
}

impl UiHandler for State {
    fn load_uri(&mut self, window_id: WindowId, uri: String) {
        // Perform search if URI is not a recognized URI.
        let uri = match build_uri(uri.trim()) {
            Some(uri) => uri,
            None => Cow::Owned(format!("{SEARCH_URI}{uri}")),
        };

        if let Some(window) = self.windows.get(&window_id) {
            if let Some(engine) = window.tabs().get(&window.active_tab()) {
                engine.load_uri(&uri);
            }
        }
    }

    fn show_tabs(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };
        window.show_tabs_ui();
        window.unstall();
    }
}

pub struct Ui {
    renderer: Renderer,

    subsurface: WlSubsurface,
    surface: WlSurface,
    viewport: WpViewport,

    size: Size,
    scale: f64,

    tabs_button: TabsButton,
    separator: Separator,
    uribar: Uribar,

    keyboard_focus: Option<KeyboardInputElement>,
    touch_focus: TouchFocusElement,
    touch_point: Option<i32>,

    queue: MtQueueHandle<State>,
    window_id: WindowId,

    dirty: bool,
}

impl Ui {
    pub fn new(
        window_id: WindowId,
        queue: MtQueueHandle<State>,
        display: Display,
        (subsurface, surface): (WlSubsurface, WlSurface),
        viewport: WpViewport,
    ) -> Self {
        let uribar = Uribar::new(window_id, queue.clone());
        let renderer = Renderer::new(display, surface.clone());

        let mut ui = Self {
            subsurface,
            window_id,
            viewport,
            renderer,
            surface,
            uribar,
            queue,
            touch_focus: TouchFocusElement::UriBar,
            scale: 1.0,
            keyboard_focus: Default::default(),
            touch_point: Default::default(),
            tabs_button: Default::default(),
            separator: Default::default(),
            dirty: Default::default(),
            size: Default::default(),
        };

        // Focus URI bar on window creation.
        ui.keyboard_focus_uribar();

        ui
    }

    /// Update the surface geometry.
    pub fn set_geometry(&mut self, position: Position, size: Size) {
        self.size = size;
        self.dirty = true;

        // Update subsurface location.
        self.subsurface.set_position(position.x, position.y);

        // Update UI elements.
        self.uribar.set_geometry(self.uribar_size(), self.scale);
    }

    /// Update the render scale.
    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.dirty = true;

        // Update UI elements.
        self.uribar.set_geometry(self.uribar_size(), scale);
        self.tabs_button.set_scale(scale);
    }

    /// Render current UI state.
    ///
    /// Returns `true` if rendering was performed.
    pub fn draw(&mut self, tab_count: usize, force_redraw: bool) -> bool {
        // Abort early if UI is up to date.
        let dirty = self.dirty();
        if !dirty && !force_redraw {
            return false;
        }
        self.dirty = false;

        // Update browser's viewporter logical render size.
        //
        // NOTE: This must be done every time we draw with Sway; it is not correctly
        // persisted when drawing with the same surface multiple times.
        self.viewport.set_destination(self.size.width as i32, self.size.height as i32);

        // Mark entire UI as damaged.
        self.surface.damage(0, 0, self.size.width as i32, self.size.height as i32);

        // Calculate target positions/sizes before partial mutable borrows.
        let tabs_button_pos = self.tabs_button_position();
        let separator_size = self.separator_size();
        let uribar_pos = self.uribar_position();

        // Render the UI.
        let physical_size = self.size * self.scale;
        self.renderer.draw(physical_size, |renderer| {
            // Get UI element textures.
            //
            // This must happen with the renderer bound to ensure new textures are
            // associated with the correct program.
            let tabs_button_texture = self.tabs_button.texture(tab_count);
            let separator_texture = self.separator.texture();
            let uribar_texture = self.uribar.texture();

            unsafe {
                // Draw background.
                let [r, g, b] = UI_BG;
                gl::ClearColor(r as f32, g as f32, b as f32, 1.0);
                gl::Clear(gl::COLOR_BUFFER_BIT);

                // Draw UI elements.
                renderer.draw_texture_at(tabs_button_texture, tabs_button_pos.into(), None);
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

    /// Get underlying Wayland surface.
    pub fn surface(&self) -> &WlSurface {
        &self.surface
    }

    /// Handle new key press.
    pub fn press_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        if let Some(KeyboardInputElement::UriBar) = self.keyboard_focus {
            self.uribar.text_field.press_key(raw, keysym, modifiers)
        }
    }

    /// Handle key release.
    pub fn release_key(&self, _raw: u32, _keysym: Keysym, _modifiers: Modifiers) {}

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

        // URI bar relative position.
        let uribar_position = position - self.uribar_position().into();
        let uribar_size: Size<f64> = self.uribar.size.into();

        // Tab button relative position.
        let tabs_button_position = position - self.tabs_button_position().into();
        let tabs_button_size: Size<f64> = self.tabs_button.size().into();

        if (0.0..uribar_size.width).contains(&uribar_position.x)
            && (0.0..uribar_size.height).contains(&uribar_position.y)
        {
            self.keyboard_focus_uribar();
            self.touch_focus = TouchFocusElement::UriBar;

            // Forward touch event.
            self.uribar.touch_down(time, uribar_position, modifiers);

            return;
        } else if (0.0..tabs_button_size.width).contains(&tabs_button_position.x)
            && (0.0..tabs_button_size.height).contains(&tabs_button_position.y)
        {
            self.touch_focus = TouchFocusElement::TabsButton(position);
            return;
        }

        self.clear_keyboard_focus();
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

        match &mut self.touch_focus {
            TouchFocusElement::UriBar => {
                // Forward touch event.
                let uribar_position = position - self.uribar_position().into();
                self.uribar.touch_motion(uribar_position);
            },
            TouchFocusElement::TabsButton(touch_position) => {
                *touch_position = position;
            },
        }
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, _time: u32, id: i32, _modifiers: Modifiers) {
        // Ignore all unknown touch points.
        if self.touch_point != Some(id) {
            return;
        }
        self.touch_point = None;

        match self.touch_focus {
            // Forward touch event.
            TouchFocusElement::UriBar => (),
            TouchFocusElement::TabsButton(position) => {
                // Tab button relative position.
                let tabs_button_position = position - self.tabs_button_position().into();
                let tabs_button_size: Size<f64> = self.tabs_button.size().into();

                if (0.0..tabs_button_size.width).contains(&tabs_button_position.x)
                    && (0.0..tabs_button_size.height).contains(&tabs_button_position.y)
                {
                    self.queue.show_tabs(self.window_id);
                }
            },
        }
    }

    /// Delete text around the current cursor position.
    pub fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32) {
        if let Some(KeyboardInputElement::UriBar) = self.keyboard_focus {
            self.uribar.text_field.delete_surrounding_text(before_length, after_length);
        }
    }

    /// Insert text at the current cursor position.
    pub fn commit_string(&mut self, text: String) {
        if let Some(KeyboardInputElement::UriBar) = self.keyboard_focus {
            self.uribar.text_field.commit_string(text);
        }
    }

    /// Set preedit text at the current cursor position.
    pub fn preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32) {
        if let Some(KeyboardInputElement::UriBar) = self.keyboard_focus {
            self.uribar.text_field.preedit_string(text, cursor_begin, cursor_end);
        }
    }

    /// Send current IME state to the compositor.
    pub fn commit_ime_state(&mut self, text_input: &mut TextInput) {
        // Update IME state or disable it if no text field is focused.
        match self.keyboard_focus {
            Some(KeyboardInputElement::UriBar) => {
                let uribar_pos = self.uribar_position();
                self.uribar.text_field.commit_ime_state(text_input, uribar_pos);
            },
            _ => text_input.disable(),
        }
    }

    /// Update the URI bar's content.
    pub fn set_uri(&mut self, uri: &str) {
        self.uribar.set_uri(uri);
    }

    /// Set keyboard focus to URI bar.
    pub fn keyboard_focus_uribar(&mut self) {
        self.uribar.set_focused(true);
        self.keyboard_focus = Some(KeyboardInputElement::UriBar);
    }

    /// Clear UI keyboard focus.
    pub fn clear_keyboard_focus(&mut self) {
        self.uribar.set_focused(false);
        self.keyboard_focus = None;
    }

    /// Check whether UI needs redraw.
    pub fn dirty(&self) -> bool {
        self.dirty || self.uribar.dirty()
    }

    /// Physical position of the URI bar.
    fn uribar_position(&self) -> Position {
        let available_height = (self.size.height as f64 - SEPARATOR_HEIGHT) * self.scale;
        let padding = available_height * (1. - URIBAR_HEIGHT_PERCENTAGE) / 2.;
        let y = SEPARATOR_HEIGHT * self.scale + padding;
        Position::new(padding.round() as i32, y.round() as i32)
    }

    /// Physical size of the URI bar.
    fn uribar_size(&self) -> Size {
        let available_height = (self.size.height as f64 - SEPARATOR_HEIGHT) * self.scale;
        let height = available_height * URIBAR_HEIGHT_PERCENTAGE;

        let tabs_button_x = self.tabs_button_position().x as f64;
        let width = tabs_button_x - available_height * (1. - URIBAR_HEIGHT_PERCENTAGE);

        Size::new(width.round() as u32, height.round() as u32)
    }

    /// Physical position of the tabs button.
    fn tabs_button_position(&self) -> Position {
        let available_height = (self.size.height as f64 - SEPARATOR_HEIGHT) * self.scale;
        let padding = (available_height - TABS_BUTTON_SIZE as f64 * self.scale) / 2.;
        let y = SEPARATOR_HEIGHT * self.scale + padding;
        let x = (self.size.width - TABS_BUTTON_SIZE) as f64 * self.scale - padding;
        Position::new(x.round() as i32, y.round() as i32)
    }

    /// Physical size of the UI/content separator.
    fn separator_size(&self) -> Size<f32> {
        let mut physical_size = self.size * self.scale;
        physical_size.height = (SEPARATOR_HEIGHT * self.scale).round() as u32;
        physical_size.into()
    }
}

/// URI input UI.
struct Uribar {
    texture: Option<Texture>,
    text_field: TextField,
    size: Size,
    scale: f64,
}

impl Uribar {
    fn new(window: WindowId, mut queue: MtQueueHandle<State>) -> Self {
        // Setup text input with submission handling.
        let mut text_field = TextField::new();
        text_field.set_submit_handler(Box::new(move |uri| queue.load_uri(window, uri)));
        text_field.set_purpose(ContentPurpose::Url);

        Self { text_field, scale: 1., texture: Default::default(), size: Default::default() }
    }

    /// Update the output texture size and scale.
    fn set_geometry(&mut self, size: Size, scale: f64) {
        self.scale = scale;
        self.size = size;

        // Force redraw.
        self.texture = None;
    }

    /// Update the URI bar's content.
    fn set_uri(&mut self, uri: &str) {
        if uri == self.text_field.text() {
            return;
        }
        self.text_field.set_text(uri);

        // Force redraw.
        self.texture = None;
    }

    /// Set URI bar input focus.
    fn set_focused(&mut self, focused: bool) {
        self.text_field.set_focus(focused);
    }

    /// Check if URI bar needs redraw.
    fn dirty(&self) -> bool {
        self.texture.is_none() || self.text_field.dirty
    }

    /// Get the OpenGL texture.
    fn texture(&mut self) -> &Texture {
        // Ensure texture is up to date.
        if self.texture.is_none() || self.text_field.dirty {
            self.texture = Some(self.draw());
            self.text_field.dirty = false;
        }

        self.texture.as_ref().unwrap()
    }

    /// Draw the URI bar into an OpenGL texture.
    fn draw(&self) -> Texture {
        // Draw background color.
        let builder = TextureBuilder::new(self.size.into(), self.scale);
        builder.clear(URIBAR_BG);

        // Set text rendering options.
        let position: Position<f64> = self.text_position().into();
        let width = self.size.width - 2 * position.x.round() as u32;
        let size = Size::new(width, self.size.height);
        let mut text_options = TextOptions::new();
        text_options.cursor_position(self.text_field.cursor_index());
        text_options.preedit(self.text_field.preedit.clone());
        text_options.position(position);
        text_options.size(size.into());
        text_options.text_color(URIBAR_FG);

        // Show cursor or selection when focused.
        if self.text_field.focused {
            if self.text_field.selection.is_some() {
                text_options.selection(self.text_field.selection.clone());
            } else {
                text_options.show_cursor();
            }
        }

        // Draw URI bar.
        builder.rasterize(self.text_field.layout(), &text_options);

        // Convert cairo buffer to texture.
        builder.build()
    }

    /// Get relative position of the text.
    fn text_position(&self) -> Position {
        Position::new((URIBAR_X_PADDING * self.scale).round() as i32, 0)
    }

    /// Handle touch press events.
    pub fn touch_down(&mut self, time: u32, position: Position<f64>, modifiers: Modifiers) {
        // Forward event to text field.
        let mut relative_position = position;
        relative_position.x -= URIBAR_X_PADDING * self.scale;
        self.text_field.touch_down(time, relative_position, modifiers);
    }

    /// Handle touch motion events.
    pub fn touch_motion(&mut self, position: Position<f64>) {
        // Forward event to text field.
        let mut relative_position = position;
        relative_position.x -= URIBAR_X_PADDING * self.scale;
        self.text_field.touch_motion(relative_position);
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

/// Tab overview button.
struct TabsButton {
    texture: Option<Texture>,
    tab_count: usize,
    scale: f64,
}

impl Default for TabsButton {
    fn default() -> Self {
        Self { scale: 1., tab_count: Default::default(), texture: Default::default() }
    }
}

impl TabsButton {
    fn texture(&mut self, tab_count: usize) -> &Texture {
        // Ensure texture is up to date.
        let tab_count = tab_count.min(100);
        if self.texture.is_none() || tab_count != self.tab_count {
            let label = if tab_count == 100 {
                Cow::Borrowed("∞")
            } else {
                Cow::Owned(tab_count.to_string())
            };
            self.texture = Some(self.draw(&label));
            self.tab_count = tab_count;
        }

        self.texture.as_ref().unwrap()
    }

    /// Draw the tabs button into an OpenGL texture.
    fn draw(&mut self, tab_count_label: &str) -> Texture {
        // Render button outline.
        let size = self.size();
        let builder = TextureBuilder::new(size.into(), self.scale);
        builder.clear(UI_BG);
        builder.context().set_source_rgb(URIBAR_FG[0], URIBAR_FG[1], URIBAR_FG[2]);
        builder.context().rectangle(0., 0., size.width as f64, size.height as f64);
        builder.context().set_line_width(self.scale * 2.);
        builder.context().stroke().unwrap();

        // Render tab count text.
        let layout = {
            let image_surface = ImageSurface::create(Format::ARgb32, 0, 0).unwrap();
            let context = Context::new(&image_surface).unwrap();
            pangocairo::functions::create_layout(&context)
        };
        layout.set_alignment(Alignment::Center);
        layout.set_text(tab_count_label);
        let mut text_options = TextOptions::new();
        text_options.text_color(URIBAR_FG);
        builder.rasterize(&layout, &text_options);

        builder.build()
    }

    /// Get the physical size of the button.
    fn size(&self) -> Size {
        Size::new(TABS_BUTTON_SIZE, TABS_BUTTON_SIZE) * self.scale
    }

    /// Update the output texture scale.
    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;

        // Force redraw.
        self.texture = None;
    }
}

/// Elements accepting keyboard focus.
enum KeyboardInputElement {
    UriBar,
}

/// Elements accepting touch input.
enum TouchFocusElement {
    UriBar,
    TabsButton(Position<f64>),
}

/// Text input field.
struct TextField {
    selection: Option<Range<i32>>,
    submit_handler: Box<dyn FnMut(String)>,
    preedit: (String, i32, i32),
    last_touch: TouchHistory,
    change_cause: ChangeCause,
    last_ime_state: ImeState,
    purpose: ContentPurpose,
    layout: Layout,
    cursor_index: i32,
    cursor_offset: i32,
    focused: bool,
    dirty: bool,
}

impl TextField {
    fn new() -> Self {
        // Create pango layout.
        let image_surface = ImageSurface::create(Format::ARgb32, 0, 0).unwrap();
        let context = Context::new(&image_surface).unwrap();
        let layout = pangocairo::functions::create_layout(&context);

        Self {
            layout,
            submit_handler: Box::new(|_| {}),
            change_cause: ChangeCause::Other,
            purpose: ContentPurpose::Normal,
            last_ime_state: Default::default(),
            cursor_offset: Default::default(),
            cursor_index: Default::default(),
            last_touch: Default::default(),
            selection: Default::default(),
            preedit: Default::default(),
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
        let byte_index = self.cursor_byte_index(index, offset);

        // Update touch history.
        let multi_taps = self.last_touch.push(time, index, offset);

        // Handle single/double/triple-taps.
        match multi_taps {
            0 => {
                // Whether touch is modifying one of the selection boundaries.
                if let Some(selection) = self.selection.as_ref() {
                    self.last_touch.moving_selection_start = selection.start == byte_index;
                    self.last_touch.moving_selection_end = selection.end == byte_index;
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
                    if i + 1 < byte_index && !c.is_alphanumeric() {
                        word_start = i + 1;
                    } else if i > byte_index && !c.is_alphanumeric() {
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
        // Ignore if neither selection end is being moved.
        if self.selection.is_none()
            || (!self.last_touch.moving_selection_start && !self.last_touch.moving_selection_end)
        {
            return;
        }

        // Get byte offset from X/Y position.
        let x = (position.x * pangocairo::pango::SCALE as f64).round() as i32;
        let y = (position.y * pangocairo::pango::SCALE as f64).round() as i32;
        let (_, index, offset) = self.layout.xy_to_index(x, y);
        let byte_index = self.cursor_byte_index(index, offset);

        let selection = self.selection.as_mut().unwrap();

        // Update selection if it is at least one character wide.
        if self.last_touch.moving_selection_start && byte_index != selection.end {
            selection.start = byte_index;
        } else if self.last_touch.moving_selection_end && byte_index != selection.start {
            selection.end = byte_index;
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

    /// Delete text around the current cursor position.
    fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32) {
        // Calculate removal boundaries.
        let mut text = self.text();
        let index = self.cursor_index() as usize;
        let end = (index + after_length as usize).min(text.len());
        let start = index.saturating_sub(before_length as usize);

        // Remove all bytes in the range from the text.
        text.drain(index..end);
        text.drain(start..index);
        self.layout.set_text(&text);

        // Update cursor position.
        self.cursor_index = start as i32;
        self.cursor_offset = 0;

        // Set reason for next IME update.
        self.change_cause = ChangeCause::InputMethod;

        self.dirty = true;
    }

    /// Insert text at the current cursor position.
    fn commit_string(&mut self, text: String) {
        // Delete selection before writing new text.
        if let Some(selection) = self.selection.take() {
            self.delete_selected(selection);
        }

        // Add text to input element.
        let index = self.cursor_index() as usize;
        let mut input_text = self.text();
        input_text.insert_str(index, &text);
        self.layout.set_text(&input_text);

        // Move cursor behind the new characters.
        self.cursor_index += text.len() as i32;

        // Set reason for next IME update.
        self.change_cause = ChangeCause::InputMethod;

        self.dirty = true;
    }

    /// Set preedit text at the current cursor position.
    fn preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32) {
        // Delete selection as soon as preedit starts.
        if !text.is_empty() {
            if let Some(selection) = self.selection.take() {
                self.delete_selected(selection);
            }
        }

        self.preedit = (text, cursor_begin, cursor_end);
        self.dirty = true;
    }

    /// Send current IME state to the compositor.
    fn commit_ime_state(&mut self, text_input: &mut TextInput, position: Position) {
        // Only send disable event if input is not focused.
        if !self.focused {
            text_input.disable();
            return;
        }

        // Skip if nothing has changed.
        let cursor_index = self.cursor_index();
        let surrounding_text = self.text();
        let ime_state = ImeState {
            cursor_index,
            surrounding_text: surrounding_text.clone(),
            selection: self.selection.clone(),
            purpose: self.purpose,
        };
        if text_input.enabled() && self.last_ime_state == ime_state {
            return;
        }
        self.last_ime_state = ime_state;

        // Enable IME if necessary.
        text_input.enable();

        // Offer the entire input text as surrounding text hint.
        //
        // NOTE: This request is technically limited to 4000 bytes, but that is unlikely
        // to be an issue for our purposes.
        let (cursor_index, selection_anchor) = match &self.selection {
            Some(selection) => (selection.end, selection.start),
            None => (cursor_index, cursor_index),
        };
        text_input.set_surrounding_text(surrounding_text, cursor_index, selection_anchor);

        // Set reason for this update.
        let cause = mem::replace(&mut self.change_cause, ChangeCause::Other);
        text_input.set_text_change_cause(cause);

        // Set text input field type.
        text_input.set_content_type(ContentHint::None, self.purpose);

        // Set cursor position.
        let (cursor_rect, _) = self.layout.cursor_pos(self.cursor_index());
        let cursor_x = position.x + cursor_rect.x() / PANGO_SCALE;
        let cursor_y = position.y + cursor_rect.y() / PANGO_SCALE;
        let cursor_height = cursor_rect.height() / PANGO_SCALE;
        let cursor_width = cursor_rect.width() / PANGO_SCALE;
        text_input.set_cursor_rectangle(cursor_x, cursor_y, cursor_width, cursor_height);

        text_input.commit();
    }

    /// Set IME input field purpose hint.
    fn set_purpose(&mut self, purpose: ContentPurpose) {
        self.purpose = purpose;
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
        for _ in 0..positions.abs() {
            let direction = positions;
            let (cursor, offset) = self.layout.move_cursor_visually(
                true,
                self.cursor_index,
                self.cursor_offset,
                direction,
            );

            if (0..i32::MAX).contains(&cursor) {
                self.cursor_index = cursor;
                self.cursor_offset = offset;
            } else {
                break;
            }
        }
    }

    /// Get current cursor's byte offset.
    fn cursor_index(&self) -> i32 {
        self.cursor_byte_index(self.cursor_index, self.cursor_offset)
    }

    /// Convert a cursor's index and offset to a byte offset.
    fn cursor_byte_index(&self, index: i32, mut offset: i32) -> i32 {
        // Offset is character based, so we translate it to bytes here.
        if offset > 0 {
            let text = self.text();
            while !text.is_char_boundary((index + offset) as usize) {
                offset += 1;
            }
        }

        index + offset
    }
}

impl Default for TextField {
    fn default() -> Self {
        Self::new()
    }
}

/// IME state for text input field.
#[derive(PartialEq, Eq)]
struct ImeState {
    cursor_index: i32,
    selection: Option<Range<i32>>,
    surrounding_text: String,
    purpose: ContentPurpose,
}

impl Default for ImeState {
    fn default() -> Self {
        Self {
            purpose: ContentPurpose::Normal,
            cursor_index: -1,
            surrounding_text: Default::default(),
            selection: Default::default(),
        }
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
