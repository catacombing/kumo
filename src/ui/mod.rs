//! Non-engine UI.

use std::borrow::Cow;
use std::mem;
use std::ops::{Bound, Range, RangeBounds};
use std::rc::Rc;
use std::time::Instant;

use _text_input::zwp_text_input_v3::{ChangeCause, ContentHint, ContentPurpose};
use funq::MtQueueHandle;
use glib::{ControlFlow, Priority, Source, source};
use glutin::display::Display;
use pangocairo::cairo::LinearGradient;
use pangocairo::pango::{Alignment, SCALE as PANGO_SCALE};
use smallvec::SmallVec;
use smithay_client_toolkit::compositor::{CompositorState, Region};
use smithay_client_toolkit::reexports::client::protocol::wl_subsurface::WlSubsurface;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::protocols::wp::text_input::zv3::client as _text_input;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};

use crate::config::{CONFIG, FontFamily};
use crate::storage::history::{HistoryMatch, MAX_MATCHES};
use crate::ui::renderer::{Renderer, Svg, TextLayout, TextOptions, Texture, TextureBuilder};
use crate::window::{PasteTarget, TextInputChange, TextInputState, WindowHandler};
use crate::{History, Position, Size, State, WindowId, gl, rect_contains};

pub mod engine_backdrop;
pub mod overlay;
mod renderer;

/// Logical height of the non-browser UI.
pub const TOOLBAR_HEIGHT: u32 = 50;

/// Logical height of the UI/content separator.
pub const SEPARATOR_HEIGHT: u32 = 2;

/// Width of the web view zoom level indicator;
const ZOOM_LABEL_WIDTH: u32 = 35;

/// URI bar height percentage from UI.
const URIBAR_HEIGHT_PERCENTAGE: f64 = 0.6;

/// Icon button height percentage from UI.
const ICON_BUTTON_HEIGHT_PERCENTAGE: f64 = 0.8;

/// Padding around items in the URI bar.
const PADDING: f64 = 10.;

/// Separator characters for tab completion.
const AUTOCOMPLETE_SEPARATORS: &[u8] = b"/: ?&";

#[funq::callbacks(State)]
pub trait UiHandler {
    /// Change the active engine's URI.
    fn load_uri(&mut self, window: WindowId, uri: String);

    /// Load previous page.
    fn load_prev(&mut self, window: WindowId);

    /// Open tabs UI.
    fn show_tabs_ui(&mut self, window: WindowId);

    /// Show history suggestions popup.
    fn open_history_menu(
        &mut self,
        window_id: WindowId,
        matches: SmallVec<[HistoryMatch; MAX_MATCHES]>,
    );

    /// Hide history suggestions popup.
    fn close_history_menu(&mut self, window_id: WindowId);

    /// Show long-press text input popup.
    fn open_text_menu(
        &mut self,
        window_id: WindowId,
        position: Position,
        selection: Option<String>,
    );

    /// Set the active engine's zoom level.
    fn set_zoom_level(&mut self, window_id: WindowId, zoom_level: f64);

    /// Update the active engine's page search text.
    fn update_search_text(&mut self, window_id: WindowId, text: String);

    /// Notify engine about search termination.
    fn stop_search(&mut self, window_id: WindowId);

    /// Go to the next text search match.
    fn search_next(&mut self, window_id: WindowId);

    /// Go to the previous text search match.
    fn search_prev(&mut self, window_id: WindowId);
}

impl UiHandler for State {
    fn load_uri(&mut self, window_id: WindowId, uri: String) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.load_uri(uri, false);
        }
    }

    fn load_prev(&mut self, window_id: WindowId) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            if let Some(engine) = window.active_tab_mut() {
                engine.load_prev();
            }
        }
    }

    fn show_tabs_ui(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };
        window.set_tabs_ui_visible(true);
    }

    fn open_history_menu(
        &mut self,
        window_id: WindowId,
        matches: SmallVec<[HistoryMatch; MAX_MATCHES]>,
    ) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.open_history_menu(matches);
        }
    }

    fn close_history_menu(&mut self, window_id: WindowId) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.close_history_menu();
        }
    }

    fn open_text_menu(
        &mut self,
        window_id: WindowId,
        position: Position,
        selection: Option<String>,
    ) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.open_text_menu(position, selection);
        }
    }

    fn set_zoom_level(&mut self, window_id: WindowId, zoom_level: f64) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.set_zoom_level(zoom_level);
        }
    }

    fn update_search_text(&mut self, window_id: WindowId, text: String) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.update_search_text(&text);
        }
    }

    fn stop_search(&mut self, window_id: WindowId) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.stop_search();
        }
    }

    fn search_next(&mut self, window_id: WindowId) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.search_next();
        }
    }

    /// Go to the previous text search match.
    fn search_prev(&mut self, window_id: WindowId) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.search_prev();
        }
    }
}

pub struct Ui {
    renderer: Renderer,

    surface: WlSurface,
    subsurface: WlSubsurface,
    viewport: WpViewport,
    compositor: CompositorState,

    origin: Position,
    size: Size,
    scale: f64,

    search_stop_button: IconButton,
    search_next_button: IconButton,
    search_prev_button: IconButton,
    prev_button: IconButton,
    tabs_button: TabsButton,
    zoom_label: ZoomLabel,
    separator: Separator,
    uribar: Uribar,
    load_progress: f64,

    keyboard_focus: Option<KeyboardInputElement>,
    touch_focus: TouchFocusElement,
    touch_position: Position<f64>,
    touch_point: Option<i32>,

    queue: MtQueueHandle<State>,
    window_id: WindowId,

    last_has_history: bool,
    last_tab_count: usize,
    last_config: u32,
    dirty: bool,
}

impl Ui {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        window_id: WindowId,
        queue: MtQueueHandle<State>,
        display: Display,
        surface: WlSurface,
        subsurface: WlSubsurface,
        viewport: WpViewport,
        compositor: CompositorState,
        history: History,
    ) -> Self {
        let uribar = Uribar::new(window_id, history, queue.clone());
        let renderer = Renderer::new(display, surface.clone());

        let mut ui = Self {
            compositor,
            subsurface,
            window_id,
            viewport,
            renderer,
            surface,
            uribar,
            queue,
            search_next_button: IconButton::new(Icon::ArrowRight),
            search_prev_button: IconButton::new(Icon::ArrowLeft),
            search_stop_button: IconButton::new(Icon::X),
            prev_button: IconButton::new(Icon::ArrowLeft),
            load_progress: 1.0,
            scale: 1.0,
            last_has_history: Default::default(),
            last_tab_count: Default::default(),
            keyboard_focus: Default::default(),
            touch_position: Default::default(),
            touch_focus: Default::default(),
            touch_point: Default::default(),
            last_config: Default::default(),
            tabs_button: Default::default(),
            zoom_label: Default::default(),
            separator: Default::default(),
            origin: Default::default(),
            dirty: Default::default(),
            size: Default::default(),
        };

        // Focus URI bar on window creation.
        ui.keyboard_focus_uribar();

        ui
    }

    /// Update the logical UI size.
    pub fn set_size(&mut self, size: Size) {
        self.origin = Position::new(0, (size.height - TOOLBAR_HEIGHT) as i32);
        self.subsurface.set_position(self.origin.x, self.origin.y);

        self.size = Size::new(size.width, TOOLBAR_HEIGHT);
        self.dirty = true;

        // Update opaque region.
        if let Ok(region) = Region::new(&self.compositor) {
            region.add(0, 0, size.width as i32, size.height as i32);
            self.surface.set_opaque_region(Some(region.wl_region()));
        }

        // Update UI elements.
        self.search_stop_button.set_geometry(self.icon_button_size(), self.scale);
        self.search_next_button.set_geometry(self.icon_button_size(), self.scale);
        self.search_prev_button.set_geometry(self.icon_button_size(), self.scale);
        self.tabs_button.set_geometry(self.tabs_button_size(), self.scale);
        self.zoom_label.set_geometry(self.zoom_label_size(), self.scale);
        self.prev_button.set_geometry(self.icon_button_size(), self.scale);
        self.uribar.set_geometry(self.uribar_size(), self.scale);
    }

    /// Update the render scale.
    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.dirty = true;

        // Update UI elements.

        // Update uribar last, to ensure all element scales are updated.
        self.search_stop_button.set_geometry(self.icon_button_size(), self.scale);
        self.search_next_button.set_geometry(self.icon_button_size(), self.scale);
        self.search_prev_button.set_geometry(self.icon_button_size(), self.scale);
        self.tabs_button.set_geometry(self.tabs_button_size(), self.scale);
        self.zoom_label.set_geometry(self.zoom_label_size(), self.scale);
        self.prev_button.set_geometry(self.icon_button_size(), self.scale);
        self.uribar.set_geometry(self.uribar_size(), scale);
    }

    /// Update the engine's load progress.
    pub fn set_load_progress(&mut self, load_progress: f64) {
        self.dirty |= self.load_progress != load_progress;
        self.load_progress = load_progress;
    }

    /// Update the engine's zoom level.
    pub fn set_zoom_level(&mut self, zoom_level: f64) {
        let label_was_visible = self.zoom_label.level != 1.;
        let label_is_visible = zoom_level != 1.;

        self.zoom_label.set_level(zoom_level);

        // Update URI input size if label appeared or disappeared.
        if label_was_visible != label_is_visible {
            self.zoom_label.set_geometry(self.zoom_label_size(), self.scale);
            self.uribar.set_geometry(self.uribar_size(), self.scale);
        }
    }

    /// Render current UI state.
    ///
    /// Returns `true` if rendering was performed.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn draw(&mut self, tab_count: usize, has_history: bool) -> bool {
        // Abort early if UI is up to date.
        let dirty = self.dirty();
        if !dirty && self.last_tab_count == tab_count && self.last_has_history == has_history {
            return false;
        }
        self.last_has_history = has_history;
        self.last_tab_count = tab_count;
        self.dirty = false;

        // Force UI element redraw on config change.
        let config = CONFIG.read().unwrap();
        if self.last_config != config.generation {
            self.last_config = config.generation;

            self.search_stop_button.dirty = true;
            self.search_next_button.dirty = true;
            self.search_prev_button.dirty = true;
            self.prev_button.dirty = true;
            self.tabs_button.dirty = true;
            self.zoom_label.dirty = true;
            self.uribar.dirty = true;
        }

        // Update viewporter logical render size.
        //
        // NOTE: This must be done every time we draw with Sway; it is not correctly
        // persisted when drawing with the same surface multiple times.
        self.viewport.set_destination(self.size.width as i32, self.size.height as i32);

        // Mark entire UI as damaged.
        self.surface.damage(0, 0, self.size.width as i32, self.size.height as i32);

        // Calculate target positions/sizes before partial mutable borrows.
        let search_stop_button_pos = self.search_stop_button_position().into();
        let search_next_button_pos = self.search_next_button_position().into();
        let search_prev_button_pos = self.search_prev_button_position().into();
        let prev_button_pos = self.prev_button_position().into();
        let tabs_button_pos = self.tabs_button_position().into();
        let zoom_label_pos = self.zoom_label_position().into();
        let separator_pos = self.separator_position().into();
        let uribar_pos = self.uribar_position().into();
        let separator_size = self.separator_size();

        // Render the UI.
        let physical_size = self.size * self.scale;
        self.renderer.draw(physical_size, |renderer| {
            unsafe {
                // Draw background.
                let [r, g, b] = config.colors.background.as_f32();
                gl::ClearColor(r, g, b, 1.0);
                gl::Clear(gl::COLOR_BUFFER_BIT);

                // Draw UI elements.

                if separator_size.width > 0. {
                    let texture = self.separator.texture();
                    renderer.draw_texture_at(texture, separator_pos, separator_size);
                }

                if !self.uribar.searching {
                    if has_history {
                        let texture = self.prev_button.texture();
                        renderer.draw_texture_at(texture, prev_button_pos, None);
                    }
                    if let Some(zoom_label_texture) = self.zoom_label.texture() {
                        renderer.draw_texture_at(zoom_label_texture, zoom_label_pos, None);
                    }

                    let tabs_button_texture = self.tabs_button.texture(tab_count);
                    renderer.draw_texture_at(tabs_button_texture, tabs_button_pos, None);
                } else {
                    let stop_button_texture = self.search_stop_button.texture();
                    renderer.draw_texture_at(stop_button_texture, search_stop_button_pos, None);

                    let next_button_texture = self.search_next_button.texture();
                    renderer.draw_texture_at(next_button_texture, search_next_button_pos, None);

                    let prev_button_texture = self.search_prev_button.texture();
                    renderer.draw_texture_at(prev_button_texture, search_prev_button_pos, None);
                }

                renderer.draw_texture_at(self.uribar.texture(), uribar_pos, None);
            }
        });

        true
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

    /// Handle touch press events.
    pub fn touch_down(
        &mut self,
        time: u32,
        id: i32,
        logical_position: Position<f64>,
        _modifiers: Modifiers,
    ) {
        // Only accept a single touch point in the UI.
        if self.touch_point.is_some() {
            return;
        }
        self.touch_point = Some(id);

        // Convert position to physical space.
        self.touch_position = logical_position * self.scale;

        // Get uribar geometry.
        let uribar_position = self.uribar_position();
        let uribar_size = self.uribar.size.into();

        // Get geometry of currently visible UI buttons and labels.
        type Element = TouchFocusElement;
        let button_size = self.icon_button_size();
        let ui_elements = if !self.uribar.searching {
            [
                (self.prev_button_position(), button_size, Element::PrevButton),
                (self.tabs_button_position(), self.tabs_button_size(), Element::TabsButton),
                (self.zoom_label_position(), self.zoom_label_size(), Element::ZoomLabel),
            ]
        } else {
            [
                (self.search_stop_button_position(), button_size, Element::SearchStopButton),
                (self.search_next_button_position(), button_size, Element::SearchNextButton),
                (self.search_prev_button_position(), button_size, Element::SearchPrevButton),
            ]
        };

        if rect_contains(uribar_position, uribar_size, self.touch_position) {
            // Forward touch event.
            let absolute_logical_position = logical_position + self.origin.into();
            let relative_position = self.touch_position - uribar_position;
            self.uribar.touch_down(time, absolute_logical_position, relative_position);

            self.touch_focus = TouchFocusElement::UriBar;
            self.keyboard_focus_uribar();
        } else {
            for (position, size, focus) in ui_elements {
                if rect_contains(position, size.into(), self.touch_position) {
                    self.touch_focus = focus;
                    return;
                }
            }

            self.touch_focus = TouchFocusElement::None;
            self.clear_keyboard_focus();
        }
    }

    /// Handle touch motion events.
    pub fn touch_motion(
        &mut self,
        _time: u32,
        id: i32,
        logical_position: Position<f64>,
        _modifiers: Modifiers,
    ) {
        // Ignore all unknown touch points.
        if self.touch_point != Some(id) {
            return;
        }

        // Convert position to physical space.
        self.touch_position = logical_position * self.scale;

        if let TouchFocusElement::UriBar = &self.touch_focus {
            // Forward touch event.
            let uribar_position = self.touch_position - self.uribar_position();
            self.uribar.touch_motion(uribar_position);
        }
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, time: u32, id: i32, _modifiers: Modifiers) {
        // Ignore all unknown touch points.
        if self.touch_point != Some(id) {
            return;
        }
        self.touch_point = None;

        match self.touch_focus {
            // Forward touch event.
            TouchFocusElement::UriBar => self.uribar.touch_up(time),
            TouchFocusElement::TabsButton => {
                let zoom_label_position = self.zoom_label_position();
                let zoom_label_size: Size<f64> = self.zoom_label_size().into();

                if self.touch_position.x >= zoom_label_position.x + zoom_label_size.width {
                    self.queue.show_tabs_ui(self.window_id);
                }
            },
            TouchFocusElement::PrevButton => {
                let uribar_position = self.uribar_position();

                if self.touch_position.x < uribar_position.x {
                    self.queue.load_prev(self.window_id);
                }
            },
            TouchFocusElement::ZoomLabel => {
                let zoom_label_position = self.zoom_label_position();
                let zoom_label_size = self.zoom_label_size().into();

                if rect_contains(zoom_label_position, zoom_label_size, self.touch_position) {
                    self.queue.set_zoom_level(self.window_id, 1.);
                }
            },
            TouchFocusElement::SearchStopButton => {
                let button_position = self.search_stop_button_position();
                let button_size = self.icon_button_size().into();

                if rect_contains(button_position, button_size, self.touch_position) {
                    self.queue.stop_search(self.window_id);
                    self.set_searching(false);
                }
            },
            TouchFocusElement::SearchNextButton => {
                let button_position = self.search_next_button_position();
                let button_size = self.icon_button_size().into();

                if rect_contains(button_position, button_size, self.touch_position) {
                    self.queue.search_next(self.window_id);
                }
            },
            TouchFocusElement::SearchPrevButton => {
                let button_position = self.search_prev_button_position();
                let button_size = self.icon_button_size().into();

                if rect_contains(button_position, button_size, self.touch_position) {
                    self.queue.search_prev(self.window_id);
                }
            },
            TouchFocusElement::None => (),
        }
    }

    /// Delete text around the current cursor position.
    pub fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32) {
        if let Some(KeyboardInputElement::UriBar) = self.keyboard_focus {
            self.uribar.text_field.delete_surrounding_text(before_length, after_length);
        }
    }

    /// Insert IME text at the current cursor position.
    pub fn commit_string(&mut self, text: &str) {
        if let Some(KeyboardInputElement::UriBar) = self.keyboard_focus {
            self.uribar.text_field.commit_string(text);
        }
    }

    /// Set preedit text at the current cursor position.
    pub fn set_preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32) {
        if let Some(KeyboardInputElement::UriBar) = self.keyboard_focus {
            self.uribar.text_field.set_preedit_string(text, cursor_begin, cursor_end);
        }
    }

    /// Get current IME text_input state.
    pub fn text_input_state(&mut self) -> TextInputChange {
        match self.keyboard_focus {
            Some(KeyboardInputElement::UriBar) => {
                let uribar_pos = self.uribar_position();
                self.uribar.text_field.text_input_state(uribar_pos)
            },
            _ => TextInputChange::Disabled,
        }
    }

    /// Paste text at the current cursor position.
    pub fn paste(&mut self, text: &str) {
        if let Some(KeyboardInputElement::UriBar) = self.keyboard_focus {
            self.uribar.text_field.paste(text);
        }
    }

    /// Update the URI bar's content.
    pub fn set_uri(&mut self, uri: Cow<'_, str>) {
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
        self.dirty
            || (!self.uribar.searching && self.zoom_label.dirty)
            || self.uribar.dirty()
            || self.last_config != CONFIG.read().unwrap().generation
    }

    /// Start/Stop text search.
    pub fn set_searching(&mut self, searching: bool) {
        if self.uribar.searching == searching {
            return;
        }
        self.uribar.set_searching(searching);

        self.uribar.set_geometry(self.uribar_size(), self.scale);

        if searching {
            self.keyboard_focus_uribar();
        } else {
            self.clear_keyboard_focus();
        }

        self.dirty = true;
    }

    /// Update the current number of search matches.
    pub fn set_search_match_count(&mut self, count: usize) {
        let has_matches = count != 0 || self.uribar.text_field.is_empty();
        self.dirty |= self.uribar.search_has_matches != has_matches;

        self.uribar.search_has_matches = has_matches;
        self.uribar.dirty = true;
    }

    /// Physical height of the toolbar without the separator.
    pub fn content_height(scale: f64) -> f64 {
        let separator_height = Self::separator_height(scale);
        (TOOLBAR_HEIGHT as f64 * scale).round() - separator_height
    }

    /// Physical height of the separator.
    fn separator_height(scale: f64) -> f64 {
        (SEPARATOR_HEIGHT as f64 * scale).round()
    }

    /// Physical position of the toolbar separator.
    fn separator_position(&self) -> Position<f64> {
        Position::new(0., 0.)
    }

    /// Physical size of the toolbar separator.
    fn separator_size(&self) -> Size<f32> {
        let mut physical_size = self.size * self.scale;
        physical_size.width = (physical_size.width as f64 * self.load_progress).round() as u32;
        physical_size.height = Self::separator_height(self.scale) as u32;
        physical_size.into()
    }

    /// Physical position of the URI bar.
    fn uribar_position(&self) -> Position<f64> {
        let separator_height = self.separator_size().height as f64;
        let content_height = Self::content_height(self.scale);
        let button_width = self.icon_button_size().width as f64;
        let prev_button_x = self.prev_button_position().x;

        let x = prev_button_x + button_width;
        let y = content_height * (1. - URIBAR_HEIGHT_PERCENTAGE) / 2. + separator_height;

        Position::new(x, y.round())
    }

    /// Physical size of the URI bar.
    fn uribar_size(&self) -> Size {
        let zoom_label_size = self.zoom_label_size();
        let uribar_end = if self.uribar.searching {
            self.search_prev_button_position().x
        } else {
            self.tabs_button_position().x - zoom_label_size.width as f64
        };
        let uribar_start = self.uribar_position().x;

        Size::new((uribar_end - uribar_start) as u32, zoom_label_size.height)
    }

    /// Physical position of the zoom level label
    fn zoom_label_position(&self) -> Position<f64> {
        let mut position = self.uribar_position();
        position.x += self.uribar_size().width as f64;
        position
    }

    /// Physical size of the zoom level label.
    fn zoom_label_size(&self) -> Size {
        let height = Self::content_height(self.scale) * URIBAR_HEIGHT_PERCENTAGE;
        let width =
            if self.zoom_label.level != 1. { ZOOM_LABEL_WIDTH as f64 * self.scale } else { 0. };
        Size::new(width.round() as u32, height.round() as u32)
    }

    /// Physical position of the tabs button.
    fn tabs_button_position(&self) -> Position<f64> {
        let separator_height = self.separator_size().height;
        let button_width = self.tabs_button_size().width;

        let x = (self.size.width as f64 * self.scale).round() - button_width as f64;
        let y = separator_height as f64;

        Position::new(x, y)
    }

    /// Physical size of the tabs button including its padding.
    fn tabs_button_size(&self) -> Size {
        let size = Self::content_height(self.scale) as u32;
        Size::new(size, size)
    }

    /// Y offset for the icon buttons.
    fn icon_button_y(&self) -> f64 {
        let separator_height = self.separator_size().height as f64;
        let button_height = self.icon_button_size().height as f64;
        let content_height = Self::content_height(self.scale);

        (content_height - button_height) / 2. + separator_height
    }

    /// Physical size of the icon buttons including their padding.
    fn icon_button_size(&self) -> Size {
        let content_height = Self::content_height(self.scale);
        let size = (content_height * ICON_BUTTON_HEIGHT_PERCENTAGE).round() as u32;
        Size::new(size, size)
    }

    /// Physical position of the previous page button.
    fn prev_button_position(&self) -> Position<f64> {
        Position::new(0., self.icon_button_y())
    }

    /// Physical position of the search stop button.
    fn search_stop_button_position(&self) -> Position<f64> {
        Position::new(0., self.icon_button_y())
    }

    /// Physical position of the search next button.
    fn search_next_button_position(&self) -> Position<f64> {
        let button_width = self.icon_button_size().width;
        let x = (self.size.width as f64 * self.scale).round() - button_width as f64;
        Position::new(x, self.icon_button_y())
    }

    /// Physical position of the search previous button.
    fn search_prev_button_position(&self) -> Position<f64> {
        let mut position = self.search_next_button_position();
        position.x -= self.icon_button_size().width as f64;
        position
    }
}

/// URI input UI.
struct Uribar {
    texture: Option<Texture>,
    dirty: bool,

    queue: MtQueueHandle<State>,
    window_id: WindowId,

    text_field: TextField,
    uri: String,

    search_has_matches: bool,
    searching: bool,

    autocomplete_handler: Rc<dyn Fn(&mut TextField)>,

    size: Size,
    scale: f64,
}

impl Uribar {
    fn new(window_id: WindowId, history: History, queue: MtQueueHandle<State>) -> Self {
        // Setup text input with submission handling.
        let font_size = CONFIG.read().unwrap().font.size(1.);
        let mut text_field = TextField::new(window_id, queue.clone(), font_size);
        let mut submit_queue = queue.clone();
        let _ = text_field
            .set_submit_handler(Box::new(move |uri| submit_queue.load_uri(window_id, uri)));
        text_field.set_purpose(ContentPurpose::Url);

        // Setup autocomplete suggestion on text change.
        let matches_queue = queue.clone();
        let autocomplete_handler = Rc::new(move |text_field: &mut TextField| {
            let text = text_field.text();

            // Get matches for history popup.
            if text_field.focused {
                let matches = history.matches(&text);
                matches_queue.clone().open_history_menu(window_id, matches);
            }

            // Get suggestion for autocomplete.
            let suggestion = match history.autocomplete(&text) {
                Some(mut suggestion) if suggestion.len() > text.len() => {
                    suggestion.split_off(text.len())
                },
                _ => String::new(),
            };
            text_field.set_autocomplete(suggestion);
        });
        let handler = autocomplete_handler.clone();
        let _ = text_field.set_text_change_handler(Box::new(move |text_field| handler(text_field)));

        Self {
            autocomplete_handler,
            text_field,
            window_id,
            queue,
            dirty: true,
            scale: 1.,
            search_has_matches: Default::default(),
            searching: Default::default(),
            texture: Default::default(),
            size: Default::default(),
            uri: Default::default(),
        }
    }

    /// Update the output texture size and scale.
    fn set_geometry(&mut self, size: Size, scale: f64) {
        self.scale = scale;
        self.size = size;

        // Update text field dimensions.
        let field_width = self.size.width as f64 - (2. * PADDING * scale).round();
        self.text_field.set_width(field_width);
        self.text_field.set_scale(scale);

        // Force redraw.
        self.dirty = true;
    }

    /// Update the URI bar's content.
    fn set_uri(&mut self, uri: Cow<'_, str>) {
        if uri == self.uri {
            return;
        }
        self.uri = uri.to_string();

        if !self.searching {
            self.text_field.set_text(&self.uri);

            // Force redraw.
            self.dirty = true;
        }
    }

    /// Set URI bar input focus.
    fn set_focused(&mut self, focused: bool) {
        if !focused {
            self.queue.close_history_menu(self.window_id);
        }

        self.text_field.set_focus(focused);
    }

    /// Switch input between URI and Search mode.
    fn set_searching(&mut self, searching: bool) {
        self.searching = searching;

        if self.searching {
            // Switch text field to search mode.
            let mut queue = self.queue.clone();
            let window_id = self.window_id;
            let _ = self.text_field.set_text_change_handler(Box::new(move |text| {
                queue.update_search_text(window_id, text.text())
            }));
            let _ = self.text_field.set_submit_handler(Box::new(|_| {}));
            self.text_field.set_purpose(ContentPurpose::Normal);
            self.text_field.set_autocomplete("");
            self.text_field.set_text("");

            // Ensure we don't show errors on search start.
            self.search_has_matches = true;
        } else {
            // Switch text field to URI mode.
            let handler = self.autocomplete_handler.clone();
            let mut queue = self.queue.clone();
            let window_id = self.window_id;
            let _ = self
                .text_field
                .set_submit_handler(Box::new(move |uri| queue.load_uri(window_id, uri)));
            let _ = self.text_field.set_text_change_handler(Box::new(move |field| handler(field)));
            self.text_field.set_purpose(ContentPurpose::Url);
            self.text_field.set_text(&self.uri);
        }
    }

    /// Check if URI bar needs redraw.
    fn dirty(&self) -> bool {
        self.dirty || self.text_field.dirty
    }

    /// Get the OpenGL texture.
    fn texture(&mut self) -> &Texture {
        // Ensure texture is up to date.
        if self.dirty || self.text_field.dirty {
            if let Some(texture) = self.texture.take() {
                texture.delete();
            }
            self.texture = Some(self.draw());

            self.text_field.dirty = false;
            self.dirty = false;
        }

        self.texture.as_ref().unwrap()
    }

    /// Draw the URI bar into an OpenGL texture.
    fn draw(&mut self) -> Texture {
        // Draw background color.
        let config = CONFIG.read().unwrap();
        let size = self.size.into();
        let builder = TextureBuilder::new(size);
        let bg = if self.searching && !self.search_has_matches {
            config.colors.highlight.as_f64()
        } else {
            config.colors.alt_background.as_f64()
        };
        builder.clear(bg);

        // Set text rendering options.
        let position: Position<f64> = self.text_position().into();
        let mut text_options = TextOptions::new();
        text_options.cursor_position(self.text_field.cursor_index());
        text_options.autocomplete(self.text_field.autocomplete().into());
        text_options.preedit(self.text_field.preedit.clone());
        text_options.position(position);
        text_options.size(size);
        text_options.text_color(config.colors.foreground.as_f64());
        text_options.set_ellipsize(false);

        // Show cursor or selection when focused.
        if self.text_field.focused {
            if self.text_field.selection.is_some() {
                text_options.selection(self.text_field.selection.clone());
            } else {
                text_options.show_cursor();
            }
        }

        // Ensure font family, size, and scale are up to date.
        let font_size = config.font.size(1.);
        let layout = self.text_field.layout();
        layout.set_font(&config.font.family, font_size);
        layout.set_scale(self.scale);

        // Draw URI bar.
        builder.rasterize(layout, &text_options);

        // Draw start gradient to indicate available scroll content.
        let context = builder.context();
        let size: Size<f64> = self.size.into();
        let x_padding = (PADDING * self.scale).round();
        let gradient = LinearGradient::new(0., 0., x_padding, 0.);
        gradient.add_color_stop_rgba(0., bg[0], bg[1], bg[2], 255.);
        gradient.add_color_stop_rgba(x_padding, bg[0], bg[1], bg[2], 0.);
        context.rectangle(0., 0., x_padding, size.height);
        context.set_source(gradient).unwrap();
        context.fill().unwrap();

        // Draw end gradient to indicate available scroll content.
        let x = size.width - x_padding;
        let gradient = LinearGradient::new(x, 0., size.width, 0.);
        gradient.add_color_stop_rgba(0., bg[0], bg[1], bg[2], 0.);
        gradient.add_color_stop_rgba(x_padding, bg[0], bg[1], bg[2], 255.);
        context.rectangle(x, 0., size.width, size.height);
        context.set_source(gradient).unwrap();
        context.fill().unwrap();

        // Convert cairo buffer to texture.
        builder.build()
    }

    /// Get relative position of the text.
    fn text_position(&self) -> Position {
        let x = (PADDING * self.scale + self.text_field.scroll_offset).round() as i32;
        Position::new(x, 0)
    }

    /// Handle touch press events.
    pub fn touch_down(
        &mut self,
        time: u32,
        absolute_logical_position: Position<f64>,
        mut position: Position<f64>,
    ) {
        // Forward event to text field.
        position.x -= PADDING * self.scale;
        self.text_field.touch_down(time, absolute_logical_position, position);
    }

    /// Handle touch motion events.
    pub fn touch_motion(&mut self, mut position: Position<f64>) {
        // Forward event to text field.
        position.x -= PADDING * self.scale;
        self.text_field.touch_motion(position);
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, time: u32) {
        // Forward event to text field.
        self.text_field.touch_up(time);
    }
}

/// Separator between UI and browser content.
#[derive(Default)]
struct Separator {
    texture: Option<Texture>,
    color: [u8; 4],
}

impl Separator {
    fn texture(&mut self) -> &Texture {
        // Invalidate texture if highlight color changed.
        let hl = CONFIG.read().unwrap().colors.highlight.as_u8();
        if self.color != hl {
            if let Some(texture) = self.texture.take() {
                texture.delete();
            }
        }

        // Ensure texture is up to date.
        if self.texture.is_none() {
            self.texture = Some(Texture::new(&hl, 1, 1));
            self.color = hl;
        }

        self.texture.as_ref().unwrap()
    }
}

/// Tab overview button.
struct TabsButton {
    texture: Option<Texture>,
    dirty: bool,
    tab_count: usize,
    size: Size,
    scale: f64,
}

impl Default for TabsButton {
    fn default() -> Self {
        Self {
            dirty: true,
            scale: 1.,
            tab_count: Default::default(),
            texture: Default::default(),
            size: Default::default(),
        }
    }
}

impl TabsButton {
    fn texture(&mut self, tab_count: usize) -> &Texture {
        // Ensure texture is up to date.
        let tab_count = tab_count.min(100);
        if self.dirty || tab_count != self.tab_count {
            // Get tab count text.
            let label = if tab_count == 100 {
                Cow::Borrowed("∞")
            } else {
                Cow::Owned(tab_count.to_string())
            };

            // Redraw texture.
            if let Some(texture) = self.texture.take() {
                texture.delete();
            }
            self.texture = Some(self.draw(&label));

            self.tab_count = tab_count;
            self.dirty = false;
        }

        self.texture.as_ref().unwrap()
    }

    /// Draw the tabs button into an OpenGL texture.
    fn draw(&mut self, tab_count_label: &str) -> Texture {
        let config = CONFIG.read().unwrap();
        let padding = (PADDING * self.scale).round();

        // Render button outline.
        let fg = config.colors.foreground.as_f64();
        let builder = TextureBuilder::new(self.size.into());
        let context = builder.context();
        builder.clear(config.colors.background.as_f64());
        context.set_source_rgb(fg[0], fg[1], fg[2]);
        context.rectangle(
            padding,
            padding,
            self.size.width as f64 - 2. * padding,
            self.size.height as f64 - 2. * padding,
        );
        context.set_line_width(self.scale);
        context.stroke().unwrap();

        // Render tab count text.
        let layout = TextLayout::new(config.font.size(1.), self.scale);
        layout.set_alignment(Alignment::Center);
        layout.set_text(tab_count_label);
        let mut text_options = TextOptions::new();
        text_options.text_color(fg);
        builder.rasterize(&layout, &text_options);

        builder.build()
    }

    /// Update the output texture scale.
    fn set_geometry(&mut self, size: Size, scale: f64) {
        self.scale = scale;
        self.size = size;

        // Force redraw.
        self.dirty = true;
    }
}

/// Button with a simple icon label.
struct IconButton {
    texture: Option<Texture>,
    dirty: bool,
    size: Size,
    scale: f64,
    icon: Icon,
}

impl IconButton {
    fn new(icon: Icon) -> Self {
        Self { icon, dirty: true, scale: 1., texture: Default::default(), size: Default::default() }
    }

    fn texture(&mut self) -> &Texture {
        // Ensure texture is up to date.
        if mem::take(&mut self.dirty) {
            if let Some(texture) = self.texture.take() {
                texture.delete();
            }
            self.texture = Some(self.draw());
        }

        self.texture.as_ref().unwrap()
    }

    /// Draw the button into an OpenGL texture.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn draw(&mut self) -> Texture {
        let colors = &CONFIG.read().unwrap().colors;
        let builder = TextureBuilder::new(self.size.into());
        builder.clear(colors.background.as_f64());

        // Set line drawing properties.
        let fg = colors.foreground.as_f64();
        let context = builder.context();
        context.set_source_rgb(fg[0], fg[1], fg[2]);
        context.set_line_width(self.scale);

        // Draw button symbol.
        let padding = (PADDING * self.scale).round();
        let icon_width = self.size.width as f64 - 2. * padding;
        let icon_height = self.size.height as f64 - 2. * padding;
        match self.icon {
            Icon::ArrowLeft => {
                context.move_to(padding + icon_width * 0.75, padding);
                context.line_to(padding + icon_width * 0.25, padding + icon_height / 2.);
                context.line_to(padding + icon_width * 0.75, padding + icon_height);
                context.stroke().unwrap();
            },
            Icon::ArrowRight => {
                context.move_to(padding + icon_width * 0.25, padding);
                context.line_to(padding + icon_width * 0.75, padding + icon_height / 2.);
                context.line_to(padding + icon_width * 0.25, padding + icon_height);
                context.stroke().unwrap();
            },
            Icon::X => {
                context.move_to(padding, padding);
                context.line_to(padding + icon_width, padding + icon_height);
                context.stroke().unwrap();

                context.move_to(padding + icon_width, padding);
                context.line_to(padding, padding + icon_height);
                context.stroke().unwrap();
            },
        }

        builder.build()
    }

    /// Update the output texture scale.
    fn set_geometry(&mut self, size: Size, scale: f64) {
        self.scale = scale;
        self.size = size;

        // Force redraw.
        self.dirty = true;
    }
}

/// Icon for an icon button.
enum Icon {
    ArrowLeft,
    ArrowRight,
    X,
}

/// Elements accepting keyboard focus.
#[derive(Debug)]
enum KeyboardInputElement {
    UriBar,
}

/// Elements accepting touch input.
#[derive(Default)]
enum TouchFocusElement {
    None,
    SearchStopButton,
    SearchNextButton,
    SearchPrevButton,
    TabsButton,
    PrevButton,
    ZoomLabel,
    #[default]
    UriBar,
}

/// Text input field.
pub struct TextField {
    layout: TextLayout,
    cursor_index: i32,
    cursor_offset: i32,
    scroll_offset: f64,

    width: f64,

    selection: Option<Range<i32>>,

    touch_state: TouchState,

    text_change_handler: Box<dyn FnMut(&mut Self)>,
    submit_handler: Box<dyn FnMut(String)>,

    queue: MtQueueHandle<State>,
    window_id: WindowId,

    autocomplete: String,

    preedit: (String, i32, i32),
    change_cause: ChangeCause,
    purpose: ContentPurpose,

    focused: bool,

    text_input_dirty: bool,
    dirty: bool,
}

impl TextField {
    fn new(window_id: WindowId, queue: MtQueueHandle<State>, font_size: u8) -> Self {
        let font_family = CONFIG.read().unwrap().font.family.clone();
        Self::with_family(window_id, queue, font_family, font_size)
    }

    fn with_family(
        window_id: WindowId,
        queue: MtQueueHandle<State>,
        family: FontFamily,
        font_size: u8,
    ) -> Self {
        Self {
            window_id,
            queue,
            layout: TextLayout::with_family(family, font_size, 1.),
            text_change_handler: Box::new(|_| {}),
            submit_handler: Box::new(|_| {}),
            change_cause: ChangeCause::Other,
            purpose: ContentPurpose::Normal,
            text_input_dirty: Default::default(),
            cursor_offset: Default::default(),
            scroll_offset: Default::default(),
            autocomplete: Default::default(),
            cursor_index: Default::default(),
            touch_state: Default::default(),
            selection: Default::default(),
            preedit: Default::default(),
            focused: Default::default(),
            width: Default::default(),
            dirty: Default::default(),
        }
    }

    /// Update return key handler.
    fn set_submit_handler(&mut self, handler: Box<dyn FnMut(String)>) -> Box<dyn FnMut(String)> {
        mem::replace(&mut self.submit_handler, handler)
    }

    /// Update text change handler.
    fn set_text_change_handler(
        &mut self,
        handler: Box<dyn FnMut(&mut Self)>,
    ) -> Box<dyn FnMut(&mut Self)> {
        mem::replace(&mut self.text_change_handler, handler)
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

        self.clear_selection();

        // Reset scroll offset.
        self.scroll_offset = 0.;

        self.text_input_dirty = true;
        self.dirty = true;

        self.emit_text_changed();
    }

    /// Set the field width in pixels.
    fn set_width(&mut self, width: f64) {
        self.width = width;

        // Ensure cursor is visible.
        self.update_scroll_offset();

        self.dirty = true;
    }

    /// Set the text's scale.
    fn set_scale(&mut self, scale: f64) {
        self.layout().set_scale(scale);
        self.dirty = true;
    }

    /// Get current text content.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn text(&self) -> String {
        self.layout.text().to_string()
    }

    /// Check if the input's text is empty.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn is_empty(&self) -> bool {
        self.layout.text().is_empty()
    }

    /// Get underlying pango layout.
    fn layout(&mut self) -> &mut TextLayout {
        &mut self.layout
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

            // Ensure selection end is visible.
            self.update_scroll_offset();

            self.text_input_dirty = true;
            self.dirty = true;
        } else {
            self.clear_selection();
        }
    }

    /// Clear text selection.
    pub fn clear_selection(&mut self) {
        self.selection = None;

        self.text_input_dirty = true;
        self.dirty = true;
    }

    /// Get selection text.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn selection_text(&self) -> Option<String> {
        let selection = self.selection.as_ref()?;
        let range = selection.start as usize..selection.end as usize;
        Some(self.text()[range].to_owned())
    }

    /// Submit current text input.
    pub fn submit(&mut self) {
        let text = self.text();
        (self.submit_handler)(text);

        self.set_focus(false);
    }

    /// Handle new key press.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn press_key(&mut self, _raw: u32, keysym: Keysym, modifiers: Modifiers) {
        // Ignore input with logo/alt key held.
        if modifiers.logo || modifiers.alt {
            return;
        }

        match (keysym, modifiers.shift, modifiers.ctrl) {
            (Keysym::Return, false, false) => self.submit(),
            (Keysym::Left, false, false) => {
                match self.selection.take() {
                    Some(selection) => {
                        self.cursor_index = selection.start;
                        self.cursor_offset = 0;
                    },
                    None => self.move_cursor(-1),
                }

                self.text_input_dirty = true;
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

                self.text_input_dirty = true;
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

                        // Ensure cursor is still visible.
                        self.update_scroll_offset();

                        self.emit_text_changed();
                    },
                }

                self.text_input_dirty = true;
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

                        self.emit_text_changed();
                    },
                }

                self.text_input_dirty = true;
                self.dirty = true;
            },
            (Keysym::Tab, false, false) => {
                // Ignore tab without completion available.
                let mut text = self.text();
                if self.autocomplete.is_empty() || self.cursor_index() < text.len() as i32 {
                    // Insert `/` when at the end of input without a suggestion.
                    if text.len() as i32 == self.cursor_index + self.cursor_offset {
                        text.push('/');
                        self.set_text(&text);

                        self.emit_text_changed();
                    }

                    return;
                }

                // Add all text up to and including the next separator characters.
                let complete_index = self
                    .autocomplete
                    .bytes()
                    .enumerate()
                    .skip_while(|(_, b)| !AUTOCOMPLETE_SEPARATORS.contains(b))
                    .find_map(|(i, b)| (!AUTOCOMPLETE_SEPARATORS.contains(&b)).then_some(i))
                    .unwrap_or(self.autocomplete.len());
                text.push_str(&self.autocomplete[..complete_index]);
                self.set_text(&text);

                self.emit_text_changed();
            },
            (Keysym::XF86_Copy, ..) | (Keysym::C, true, true) => {
                if let Some(text) = self.selection_text() {
                    self.queue.set_clipboard(text);
                }
            },
            (Keysym::XF86_Paste, ..) | (Keysym::V, true, true) => {
                self.queue.request_paste(PasteTarget::Ui(self.window_id))
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

                    self.text_input_dirty = true;
                    self.dirty = true;

                    self.emit_text_changed();
                }
            },
            _ => (),
        }
    }

    /// Delete the selected text.
    ///
    /// This automatically places the cursor at the start of the selection.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn delete_selected(&mut self, selection: Range<i32>) {
        // Remove selected text from input.
        let range = selection.start as usize..selection.end as usize;
        let mut text = self.text();
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

        self.text_input_dirty = true;
        self.dirty = true;

        self.emit_text_changed();
    }

    /// Handle touch press events.
    ///
    /// The `absolute_logical_position` should be the text input's global
    /// location in logical space and is used for opening popups.
    pub fn touch_down(
        &mut self,
        time: u32,
        absolute_logical_position: Position<f64>,
        position: Position<f64>,
    ) {
        // Get byte offset from X/Y position.
        let x = ((position.x - self.scroll_offset) * PANGO_SCALE as f64).round() as i32;
        let y = (position.y * PANGO_SCALE as f64).round() as i32;
        let (_, index, offset) = self.layout.xy_to_index(x, y);
        let byte_index = self.cursor_byte_index(index, offset);

        // Update touch state.
        self.touch_state.down(time, position, byte_index, self.focused);

        // Stage timer for option menu popup.
        if self.touch_state.action == TouchAction::Tap {
            let position = absolute_logical_position.i32_round();
            let mut selection = self.selection_text();
            let mut queue = self.queue.clone();
            let window_id = self.window_id;

            self.touch_state.stage_long_press_callback(move || {
                queue.open_text_menu(window_id, position, selection.take());
            });
        }
    }

    /// Handle touch motion events.
    pub fn touch_motion(&mut self, position: Position<f64>) {
        // Update touch state.
        let delta = self.touch_state.motion(position, self.selection.as_ref());

        // Handle touch drag actions.
        let action = self.touch_state.action;
        match action {
            // Scroll through URI text.
            TouchAction::Drag => {
                self.scroll_offset += delta.x;
                self.clamp_scroll_offset();

                self.touch_state.clear_long_press_timeout();

                self.dirty = true;
            },
            // Modify selection boundaries.
            TouchAction::DragSelectionStart | TouchAction::DragSelectionEnd
                if self.selection.is_some() =>
            {
                // Get byte offset from X/Y position.
                let x = ((position.x - self.scroll_offset) * PANGO_SCALE as f64).round() as i32;
                let y = (position.y * PANGO_SCALE as f64).round() as i32;
                let (_, index, offset) = self.layout.xy_to_index(x, y);
                let byte_index = self.cursor_byte_index(index, offset);

                // Update selection if it is at least one character wide.
                let selection = self.selection.as_mut().unwrap();
                let modifies_start = action == TouchAction::DragSelectionStart;
                if modifies_start && byte_index != selection.end {
                    selection.start = byte_index;
                } else if !modifies_start && byte_index != selection.start {
                    selection.end = byte_index;
                }

                // Swap modified side when input carets "overtake" each other.
                if selection.start > selection.end {
                    mem::swap(&mut selection.start, &mut selection.end);
                    self.touch_state.action = if modifies_start {
                        TouchAction::DragSelectionEnd
                    } else {
                        TouchAction::DragSelectionStart
                    };
                }

                // Ensure selection end stays visible.
                self.update_scroll_offset();

                self.touch_state.clear_long_press_timeout();

                self.text_input_dirty = true;
                self.dirty = true;
            },
            // Ignore touch motion for tap actions.
            _ => (),
        }
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, time: u32) {
        // Always reset long-press timers.
        self.touch_state.clear_long_press_timeout();

        // Ignore release handling for drag actions.
        if matches!(
            self.touch_state.action,
            TouchAction::Drag
                | TouchAction::DragSelectionStart
                | TouchAction::DragSelectionEnd
                | TouchAction::Focus
        ) {
            return;
        }

        // Get byte offset from X/Y position.
        let position = self.touch_state.last_position;
        let x = ((position.x - self.scroll_offset) * PANGO_SCALE as f64).round() as i32;
        let y = (position.y * PANGO_SCALE as f64).round() as i32;
        let (_, index, offset) = self.layout.xy_to_index(x, y);
        let byte_index = self.cursor_byte_index(index, offset);

        // Handle single/double/triple-taps.
        let ms_since_down = (time - self.touch_state.last_time) as u128;
        match self.touch_state.action {
            TouchAction::Tap => {
                // Move cursor to tap location, ignoring long presses.
                let long_press = CONFIG.read().unwrap().input.long_press.as_millis();
                if ms_since_down < long_press {
                    // Update cursor index.
                    self.cursor_index = index;
                    self.cursor_offset = offset;

                    self.clear_selection();

                    self.text_input_dirty = true;
                    self.dirty = true;
                }
            },
            // Select entire word at touch location.
            TouchAction::DoubleTap => {
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
            // Select everything.
            TouchAction::TripleTap => self.select(..),
            TouchAction::Drag
            | TouchAction::DragSelectionStart
            | TouchAction::DragSelectionEnd
            | TouchAction::Focus => {
                unreachable!()
            },
        }

        // Ensure focus when receiving touch input.
        self.set_focus(true);
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

        // Ensure cursor is visible.
        self.update_scroll_offset();

        // Set reason for next IME update.
        self.change_cause = ChangeCause::InputMethod;

        self.text_input_dirty = true;
        self.dirty = true;

        self.emit_text_changed();
    }

    /// Insert text at the current cursor position.
    fn commit_string(&mut self, text: &str) {
        // Set reason for next IME update.
        self.change_cause = ChangeCause::InputMethod;

        self.paste(text);
    }

    /// Set preedit text at the current cursor position.
    fn set_preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32) {
        // Delete selection as soon as preedit starts.
        if !text.is_empty() {
            if let Some(selection) = self.selection.take() {
                self.delete_selected(selection);
            }
        }

        self.preedit = (text, cursor_begin, cursor_end);

        // Ensure preedit end is visible.
        self.update_scroll_offset();

        self.text_input_dirty = true;
        self.dirty = true;
    }

    /// Paste text into the input element.
    fn paste(&mut self, text: &str) {
        // Delete selection before writing new text.
        if let Some(selection) = self.selection.take() {
            self.delete_selected(selection);
        }

        // Add text to input element.
        let index = self.cursor_index() as usize;
        let mut input_text = self.text();
        input_text.insert_str(index, text);
        self.layout.set_text(&input_text);

        // Move cursor behind the new characters.
        self.cursor_index += text.len() as i32;

        // Ensure cursor is visible.
        self.update_scroll_offset();

        self.text_input_dirty = true;
        self.dirty = true;

        self.emit_text_changed();
    }

    /// Set autocomplete text.
    ///
    /// This is expected to have the common prefix removed already.
    fn set_autocomplete(&mut self, autocomplete: impl Into<String>) {
        self.autocomplete = autocomplete.into();
    }

    /// Get autocomplete text.
    ///
    /// This will return the text to be appended behind the cursor when an
    /// autocomplete suggestion is available.
    fn autocomplete(&self) -> &str {
        if self.focused && self.selection.is_none() { &self.autocomplete } else { "" }
    }

    /// Get current IME text_input state.
    fn text_input_state(&mut self, position: Position<f64>) -> TextInputChange {
        // Send disabled if input is not focused.
        if !self.focused {
            return TextInputChange::Disabled;
        }

        // Skip expensive surrounding_text clone without changes.
        if !mem::take(&mut self.text_input_dirty) {
            return TextInputChange::Unchanged;
        }

        // Get reason for this change.
        let change_cause = mem::replace(&mut self.change_cause, ChangeCause::Other);

        // Calculate cursor rectangle.
        let position = position.i32_round();
        let cursor_index = self.cursor_index();
        let (cursor_rect, _) = self.layout.cursor_pos(self.cursor_index());
        let cursor_x = position.x + cursor_rect.x() / PANGO_SCALE;
        let cursor_y = position.y + cursor_rect.y() / PANGO_SCALE;
        let cursor_height = cursor_rect.height() / PANGO_SCALE;
        let cursor_width = cursor_rect.width() / PANGO_SCALE;
        let cursor_rect = (cursor_x, cursor_y, cursor_width, cursor_height);

        // Skip if nothing has changed.
        let surrounding_text = self.text();
        TextInputChange::Dirty(TextInputState {
            change_cause,
            cursor_index,
            cursor_rect,
            surrounding_text: surrounding_text.clone(),
            selection: self.selection.clone(),
            hint: ContentHint::None,
            purpose: self.purpose,
        })
    }

    /// Set IME input field purpose hint.
    fn set_purpose(&mut self, purpose: ContentPurpose) {
        self.purpose = purpose;

        self.text_input_dirty = true;
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

        self.text_input_dirty = true;
        self.dirty = true;
    }

    /// Move the text input cursor.
    #[cfg_attr(feature = "profiling", profiling::function)]
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

        // Ensure cursor is always visible.
        self.update_scroll_offset();

        self.text_input_dirty = true;
        self.dirty = true;
    }

    /// Call text change handler.
    fn emit_text_changed(&mut self) {
        let mut text_change_handler = mem::replace(&mut self.text_change_handler, Box::new(|_| {}));
        (text_change_handler)(self);
        self.text_change_handler = text_change_handler;
    }

    /// Get current cursor's byte offset.
    #[cfg_attr(feature = "profiling", profiling::function)]
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

    /// Update the scroll offset based on cursor position.
    ///
    /// This will scroll towards the cursor to ensure it is always visible.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn update_scroll_offset(&mut self) {
        // For cursor ranges we jump twice to make both ends visible when possible.
        if let Some(selection) = &self.selection {
            let end = selection.end;
            self.update_scroll_offset_to(selection.start);
            self.update_scroll_offset_to(end);
        } else if self.preedit.0.is_empty() {
            self.update_scroll_offset_to(self.cursor_index());
        } else {
            self.update_scroll_offset_to(self.preedit.1);
            self.update_scroll_offset_to(self.preedit.2);
        }
    }

    /// Update the scroll offset to include a specific cursor index.
    fn update_scroll_offset_to(&mut self, cursor_index: i32) {
        let (cursor_rect, _) = self.layout.cursor_pos(cursor_index);
        let cursor_x = cursor_rect.x() as f64 / PANGO_SCALE as f64;

        // Scroll cursor back into the visible range.
        let delta = cursor_x + self.scroll_offset - self.width;
        if delta > 0. {
            self.scroll_offset -= delta;
            self.dirty = true;
        } else if cursor_x + self.scroll_offset < 0. {
            self.scroll_offset = -cursor_x;
            self.dirty = true;
        }

        self.clamp_scroll_offset();
    }

    /// Clamp the scroll offset to the field's limits.
    fn clamp_scroll_offset(&mut self) {
        let min_offset = -(self.layout.pixel_size().0 as f64 - self.width).max(0.);
        let clamped_offset = self.scroll_offset.min(0.).max(min_offset);
        self.dirty |= clamped_offset != self.scroll_offset;
        self.scroll_offset = clamped_offset;
    }
}

/// Touch event tracking.
#[derive(Default)]
struct TouchState {
    action: TouchAction,
    last_time: u32,
    last_position: Position<f64>,
    last_motion_position: Position<f64>,
    start_byte_index: i32,
    long_press_source: Option<Source>,
}

impl TouchState {
    /// Update state from touch down event.
    fn down(&mut self, time: u32, position: Position<f64>, byte_index: i32, focused: bool) {
        // Update touch action.
        let input = &CONFIG.read().unwrap().input;
        let delta = position - self.last_position;
        self.action = if !focused {
            TouchAction::Focus
        } else if self.last_time + input.max_multi_tap.as_millis() as u32 >= time
            && delta.x.powi(2) + delta.y.powi(2) <= input.max_tap_distance
        {
            match self.action {
                TouchAction::Tap => TouchAction::DoubleTap,
                TouchAction::DoubleTap => TouchAction::TripleTap,
                _ => TouchAction::Tap,
            }
        } else {
            TouchAction::Tap
        };

        // Reset touch origin state.
        self.start_byte_index = byte_index;
        self.last_motion_position = position;
        self.last_position = position;
        self.last_time = time;
    }

    /// Update state from touch motion event.
    ///
    /// Returns the distance moved since the last touch down or motion.
    fn motion(&mut self, position: Position<f64>, selection: Option<&Range<i32>>) -> Position<f64> {
        // Update incremental delta.
        let delta = position - self.last_motion_position;
        self.last_motion_position = position;

        // Never transfer out of drag/multi-tap states.
        if self.action != TouchAction::Tap {
            return delta;
        }

        // Ignore drags below the tap deadzone.
        let max_tap_distance = CONFIG.read().unwrap().input.max_tap_distance;
        let delta = position - self.last_position;
        if delta.x.powi(2) + delta.y.powi(2) <= max_tap_distance {
            return delta;
        }

        // Check whether drag modifies selection or scrolls the URI bar.
        self.action = match selection {
            Some(selection) if selection.start == self.start_byte_index => {
                TouchAction::DragSelectionStart
            },
            Some(selection) if selection.end == self.start_byte_index => {
                TouchAction::DragSelectionEnd
            },
            _ => TouchAction::Drag,
        };

        delta
    }

    /// Set a new callback to be executed once the long-press timeout elapses.
    fn stage_long_press_callback<F>(&mut self, mut callback: F)
    where
        F: FnMut() + Send + 'static,
    {
        // Clear old timout.
        self.clear_long_press_timeout();

        // Stage new timeout callback.
        let long_press = CONFIG.read().unwrap().input.long_press;
        let source = source::timeout_source_new(*long_press, None, Priority::DEFAULT, move || {
            callback();
            ControlFlow::Break
        });
        source.attach(None);

        self.long_press_source = Some(source);
    }

    /// Cancel active long-press popup timers.
    fn clear_long_press_timeout(&mut self) {
        if let Some(source) = self.long_press_source.take() {
            source.destroy();
        }
    }
}

/// Intention of a touch sequence.
#[derive(Default, PartialEq, Eq, Copy, Clone, Debug)]
enum TouchAction {
    #[default]
    Tap,
    DoubleTap,
    TripleTap,
    Drag,
    DragSelectionStart,
    DragSelectionEnd,
    Focus,
}

/// Button with an SVG icon.
pub struct SvgButton {
    texture: Option<Texture>,

    on_svg: Svg,
    off_svg: Option<Svg>,
    enabled: bool,

    padding_size: f64,

    dirty: bool,
    size: Size,
    scale: f64,
}

impl SvgButton {
    pub fn new(svg: Svg) -> Self {
        Self {
            padding_size: 10.,
            enabled: true,
            on_svg: svg,
            dirty: true,
            scale: 1.,
            off_svg: Default::default(),
            texture: Default::default(),
            size: Default::default(),
        }
    }

    /// Create a new SVG button with separate on/off state.
    pub fn new_toggle(on_svg: Svg, off_svg: Svg) -> Self {
        Self {
            on_svg,
            off_svg: Some(off_svg),
            padding_size: 10.,
            enabled: true,
            dirty: true,
            scale: 1.,
            texture: Default::default(),
            size: Default::default(),
        }
    }

    /// Get this button's OpenGL texture.
    pub fn texture(&mut self) -> &Texture {
        // Ensure texture is up to date.
        if mem::take(&mut self.dirty) {
            // Ensure texture is cleared while program is bound.
            if let Some(texture) = self.texture.take() {
                texture.delete();
            }
            self.texture = Some(self.draw());
        }

        self.texture.as_ref().unwrap()
    }

    /// Draw the button into an OpenGL texture.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn draw(&self) -> Texture {
        // Clear with background color.
        let colors = &CONFIG.read().unwrap().colors;
        let builder = TextureBuilder::new(self.size.into());
        builder.clear(colors.background.as_f64());

        // Draw button background.
        let bg = colors.alt_background.as_f64();
        let padding = self.padding_size * self.scale;
        let width = self.size.width as f64 - 2. * padding;
        let height = self.size.height as f64 - 2. * padding;
        let context = builder.context();
        context.rectangle(padding, padding, width.round(), height.round());
        context.set_source_rgb(bg[0], bg[1], bg[2]);
        context.fill().unwrap();

        // Draw button's icon.
        let svg = self.off_svg.filter(|_| !self.enabled).unwrap_or(self.on_svg);
        let icon_size = width.min(height) * 0.5;
        let icon_x = padding + (width - icon_size) / 2.;
        let icon_y = padding + (height - icon_size) / 2.;
        builder.rasterize_svg(svg, icon_x, icon_y, icon_size, icon_size);

        builder.build()
    }

    /// Set the physical size and scale of the button.
    fn set_geometry(&mut self, size: Size, scale: f64) {
        self.size = size;
        self.scale = scale;

        // Force redraw.
        self.dirty = true;
    }

    /// Update toggle state.
    fn set_enabled(&mut self, enabled: bool) {
        self.dirty |= self.enabled != enabled;
        self.enabled = enabled;
    }

    /// Set the padding at scale 1.
    fn set_padding(&mut self, padding: f64) {
        self.dirty |= self.padding_size != padding;
        self.padding_size = padding;
    }
}

/// Zoom level label.
pub struct ZoomLabel {
    texture: Option<Texture>,
    level: f64,

    dirty: bool,
    size: Size,
    scale: f64,
}

impl Default for ZoomLabel {
    fn default() -> Self {
        Self {
            level: 1.,
            scale: 1.,
            texture: Default::default(),
            dirty: Default::default(),
            size: Default::default(),
        }
    }
}

impl ZoomLabel {
    /// Get this label's OpenGL texture.
    pub fn texture(&mut self) -> Option<&Texture> {
        // Ensure texture is up to date.
        if mem::take(&mut self.dirty) {
            // Ensure texture is cleared while program is bound.
            if let Some(texture) = self.texture.take() {
                texture.delete();
            }

            // Skip rendering label at 100% scale.
            self.texture = (self.level != 1.).then(|| self.draw());
        }

        self.texture.as_ref()
    }

    /// Draw the label into an OpenGL texture.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn draw(&self) -> Texture {
        // Clear with background color.
        let config = CONFIG.read().unwrap();
        let secondary_bg = config.colors.alt_background.as_f64();
        let builder = TextureBuilder::new(self.size.into());
        builder.clear(secondary_bg);

        // Render current zoom level percentage.
        let text = format!("{}%", (self.level * 100.).round());
        let layout = TextLayout::new(config.font.size(0.6), self.scale);
        layout.set_alignment(Alignment::Center);
        layout.set_text(&text);
        builder.rasterize(&layout, &TextOptions::new());

        builder.build()
    }

    /// Set the physical size and scale of the label.
    fn set_geometry(&mut self, size: Size, scale: f64) {
        self.size = size;
        self.scale = scale;

        // Force redraw.
        self.dirty = true;
    }

    /// Update zoom label's level.
    fn set_level(&mut self, level: f64) {
        self.dirty |= self.level != level;
        self.level = level;
    }
}

/// Scroll velocity state.
#[derive(Default)]
pub struct ScrollVelocity {
    last_tick: Option<Instant>,
    velocity: f64,
}

impl ScrollVelocity {
    /// Check if there is any velocity active.
    pub fn is_moving(&self) -> bool {
        self.velocity != 0.
    }

    /// Set the velocity.
    pub fn set(&mut self, velocity: f64) {
        self.velocity = velocity;
        self.last_tick = None;
    }

    /// Apply and update the current scroll velocity.
    pub fn apply(&mut self, scroll_offset: &mut f64) {
        // No-op without velocity.
        if self.velocity == 0. {
            return;
        }

        // Initialize velocity on the first tick.
        //
        // This avoids applying velocity while the user is still actively scrolling.
        let last_tick = match self.last_tick.take() {
            Some(last_tick) => last_tick,
            None => {
                self.last_tick = Some(Instant::now());
                return;
            },
        };

        // Calculate velocity steps since last tick.
        let input = &CONFIG.read().unwrap().input;
        let now = Instant::now();
        let interval =
            (now - last_tick).as_micros() as f64 / (input.velocity_interval as f64 * 1_000.);

        // Apply and update velocity.
        *scroll_offset += self.velocity * (1. - input.velocity_friction.powf(interval + 1.))
            / (1. - input.velocity_friction);
        self.velocity *= input.velocity_friction.powf(interval);

        // Request next tick if velocity is significant.
        if self.velocity.abs() > 1. {
            self.last_tick = Some(now);
        } else {
            self.velocity = 0.
        }
    }
}
