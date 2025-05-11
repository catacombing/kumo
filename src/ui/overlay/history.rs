//! History overlay.

use std::collections::HashMap;
use std::mem;

use chrono::{DateTime, Local};
use funq::MtQueueHandle;
use pangocairo::pango::Alignment;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};

use crate::config::colors::{BG, FG, SECONDARY_BG, SECONDARY_FG};
use crate::config::font::font_size;
use crate::engine::NO_GROUP_ID;
use crate::storage::history::{History as HistoryDb, HistoryEntry, HistoryUri};
use crate::ui::overlay::Popup;
use crate::ui::overlay::tabs::TabsHandler;
use crate::ui::renderer::{Renderer, Svg, TextLayout, TextOptions, Texture, TextureBuilder};
use crate::ui::{MAX_TAP_DISTANCE, SvgButton, TextField};
use crate::window::{TextInputChange, WindowId};
use crate::{Position, Size, State, gl, rect_contains};

/// Logical height of the UI buttons.
const BUTTON_HEIGHT: u32 = 60;

/// Padding around buttons.
const BUTTON_PADDING: f64 = 10.;

/// Logical height of each history entry.
const ENTRY_HEIGHT: u32 = 65;

/// Horizontal tabbing around history entries.
const ENTRY_X_PADDING: f64 = 10.;

/// Vertical padding between history entries.
const ENTRY_Y_PADDING: f64 = 1.;

/// Padding around the history entry "X" button.
const ENTRY_CLOSE_PADDING: f64 = 40.;

#[funq::callbacks(State)]
trait HistoryHandler {
    /// Close the history UI.
    fn close_history(&mut self, window_id: WindowId);

    /// Update history filter.
    fn set_history_filter(&mut self, window_id: WindowId, filter: String);

    /// Open history URI in a new tab.
    fn open_history_in_tab(&mut self, window_id: WindowId, uri: String);
}

impl HistoryHandler for State {
    fn close_history(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };
        window.set_history_ui_visibile(false);
    }

    fn set_history_filter(&mut self, window_id: WindowId, filter: String) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };
        window.set_history_filter(filter);
    }

    fn open_history_in_tab(&mut self, window_id: WindowId, uri: String) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };

        let tab_id = window.add_tab(false, true, NO_GROUP_ID);
        if let Some(engine) = window.tab_mut(tab_id) {
            engine.load_uri(&uri);
        }
    }
}

/// History UI.
pub struct History {
    history_textures: HistoryTextures,
    delete_prompt: ConfirmationPrompt,
    confirm_button: SvgButton,
    delete_button: SvgButton,
    close_button: SvgButton,
    filter: HistoryFilter,
    scroll_offset: f64,

    keyboard_focus: Option<KeyboardInputElement>,
    touch_state: TouchState,

    size: Size,
    scale: f64,

    queue: MtQueueHandle<State>,
    window_id: WindowId,

    history_db: HistoryDb,

    pending_delete_confirmation: bool,

    visible: bool,
    dirty: bool,
}

impl History {
    pub fn new(window_id: WindowId, queue: MtQueueHandle<State>, history_db: HistoryDb) -> Self {
        let filter = HistoryFilter::new(window_id, queue.clone());

        Self {
            history_db,
            window_id,
            filter,
            queue,
            confirm_button: SvgButton::new(Svg::Checkmark),
            close_button: SvgButton::new(Svg::Close),
            delete_button: SvgButton::new(Svg::Bin),
            scale: 1.,
            pending_delete_confirmation: Default::default(),
            history_textures: Default::default(),
            keyboard_focus: Default::default(),
            delete_prompt: Default::default(),
            scroll_offset: Default::default(),
            touch_state: Default::default(),
            visible: Default::default(),
            dirty: Default::default(),
            size: Default::default(),
        }
    }

    /// Check whether the popup is active.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Show or hide a popup.
    pub fn set_visible(&mut self, visible: bool) {
        self.dirty |= self.visible != visible;
        self.visible = visible;

        // Update history and reset scroll offset when opening the UI.
        if self.visible {
            let entries = self.history_db.entries().unwrap_or_default();
            self.history_textures.set_entries(entries);
            self.scroll_offset = 0.;
        }
    }

    /// Set history filter.
    pub fn set_filter(&mut self, filter: String) {
        // Update current filter.
        self.dirty |= self.history_textures.filter != filter;
        self.history_textures.filter = filter;

        // Update entries to apply filter.
        let entries = self.history_db.entries().unwrap_or_default();
        self.history_textures.set_entries(entries);
    }

    /// Get default physical UI button size.
    ///
    /// This includes all padding, since that is part of the texture.
    fn button_size(&self) -> Size {
        let height = BUTTON_HEIGHT + (2. * BUTTON_PADDING).round() as u32;
        let width = BUTTON_HEIGHT + (2. * BUTTON_PADDING).round() as u32;
        Size::new(width, height) * self.scale
    }

    /// Physical position of the close button.
    ///
    /// This includes all padding since that is included in the texture.
    fn close_button_position(&self) -> Position<f64> {
        let button_size = self.button_size();
        let x = (self.size.width as f64 * self.scale).round() - button_size.width as f64;
        let y = (self.size.height as f64 * self.scale).round() - button_size.height as f64;
        Position::new(x, y)
    }

    /// Physical position of the bulk delete button.
    ///
    /// This includes all padding since that is included in the texture.
    fn delete_button_position(&self) -> Position<f64> {
        let button_size = self.button_size();
        let y = (self.size.height as f64 * self.scale).round() - button_size.height as f64;
        Position::new(0., y)
    }

    /// Get default physical UI text prompt size.
    ///
    /// This includes all padding, since that is part of the texture.
    fn delete_prompt_size(&self) -> Size {
        Size::new(self.size.width, self.size.height / 4) * self.scale
    }

    /// Physical position of the bulk delete confirmation prompt.
    ///
    /// This includes all padding since that is included in the texture.
    fn delete_prompt_position(&self) -> Position<f64> {
        let prompt_size = self.delete_prompt_size();
        let y = (self.size.height as f64 * self.scale - prompt_size.height as f64) / 2.;
        Position::new(0., y)
    }

    /// Get default physical history filter size.
    ///
    /// This includes all padding, since that is part of the texture.
    fn filter_size(&self) -> Size {
        let button_size = self.button_size();
        let width = self.size.width as f64 * self.scale - 2. * button_size.width as f64;
        let height = BUTTON_HEIGHT as f64 * self.scale;
        Size::new(width.round() as u32, height.round() as u32)
    }

    /// Physical position of the bulk delete confirmation prompt.
    ///
    /// This includes all padding since that is included in the texture.
    fn filter_position(&self) -> Position<f64> {
        let delete_button_position = self.delete_button_position();
        let x = delete_button_position.x + self.button_size().width as f64;
        let y = delete_button_position.y + BUTTON_PADDING * self.scale;
        Position::new(x, y)
    }

    /// Get physical size of the history entry close button.
    fn close_entry_button_size(entry_size: Size, scale: f64) -> Size<f64> {
        let size = entry_size.height as f64 - ENTRY_CLOSE_PADDING * scale;
        Size::new(size, size)
    }

    /// Get physical position of the close button within a history entry.
    fn close_entry_button_position(entry_size: Size, scale: f64) -> Position<f64> {
        let icon_size = Self::close_entry_button_size(entry_size, scale);
        let button_padding = (entry_size.height as f64 - icon_size.height) / 2.;
        let x = entry_size.width as f64 - button_padding - icon_size.width;
        Position::new(x, button_padding)
    }

    /// Physical size of each history entry.
    fn entry_size(&self) -> Size {
        let width = self.size.width - (2. * ENTRY_X_PADDING).round() as u32;
        Size::new(width, ENTRY_HEIGHT) * self.scale
    }

    /// Get entry at the specified location.
    ///
    /// The tuple's second element will be `true` when the position matches the
    /// close button of the history entry.
    fn entry_at(&self, mut position: Position<f64>) -> Option<(&HistoryUri, bool)> {
        let y_padding = ENTRY_Y_PADDING * self.scale;
        let x_padding = ENTRY_X_PADDING * self.scale;
        let entry_end_y = self.close_button_position().y;

        let entry_size_int = self.entry_size();
        let entry_size: Size<f64> = entry_size_int.into();

        // Check whether position is within history list boundaries.
        if position.x < x_padding
            || position.x >= x_padding + entry_size.width
            || position.y < 0.
            || position.y >= entry_end_y
        {
            return None;
        }

        // Apply current scroll offset.
        position.y -= self.scroll_offset;

        // Check if position is in the entry separator.
        let bottom_relative = (entry_end_y - position.y).round();
        let bottom_relative_y =
            entry_size.height - 1. - (bottom_relative % (entry_size.height + y_padding));
        if bottom_relative_y < 0. {
            return None;
        }

        // Find history entry at the specified offset.
        let index = (bottom_relative / (entry_size.height + y_padding).round()) as usize;
        let (entry, _) = self.history_textures.entries.get(index)?;

        // Check if click is within close button bounds.
        //
        // We include padding for the close button since it can be really hard to hit
        // otherwise.
        let close_position = Self::close_entry_button_position(entry_size_int, self.scale);
        let entry_relative_x = position.x - x_padding;
        let close = entry_relative_x >= close_position.x - close_position.y;

        Some((entry, close))
    }

    /// Clamp history list viewport offset.
    fn clamp_scroll_offset(&mut self) {
        let old_offset = self.scroll_offset;
        let max_offset = self.max_scroll_offset() as f64;
        self.scroll_offset = self.scroll_offset.clamp(0., max_offset);
        self.dirty |= old_offset != self.scroll_offset;
    }

    /// Get maximum history list scroll offset.
    fn max_scroll_offset(&self) -> usize {
        let entry_padding = (ENTRY_Y_PADDING * self.scale).round() as usize;
        let entry_height = self.entry_size().height;

        // Calculate height available for history entries.
        let ui_height = (self.size.height as f64 * self.scale).round() as usize;
        let close_button_height = self.button_size().height as usize;
        let available_height = ui_height - close_button_height;

        // Calculate height of all history entries.
        let num_entries = self.history_textures.len();
        let mut entries_height =
            (num_entries * (entry_height as usize + entry_padding)).saturating_sub(entry_padding);

        // Allow a bit of padding at the top.
        let top_padding = (BUTTON_PADDING * self.scale).round();
        entries_height += top_padding as usize;

        // Calculate history content outside the viewport.
        entries_height.saturating_sub(available_height)
    }
}

impl Popup for History {
    fn dirty(&self) -> bool {
        self.dirty || self.filter.input.dirty
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn draw(&mut self, renderer: &Renderer) {
        self.dirty = false;

        // Don't render anything when hidden.
        if !self.visible {
            return;
        }

        // Ensure offset is correct in case entries or window size changed.
        self.clamp_scroll_offset();

        // Get geometry required for rendering.
        let x_padding = (ENTRY_X_PADDING * self.scale) as f32;
        let delete_prompt_position: Position<f32> = self.delete_prompt_position().into();
        let delete_button_position: Position<f32> = self.delete_button_position().into();
        let close_button_position: Position<f32> = self.close_button_position().into();
        let ui_height = (self.size.height as f64 * self.scale).round() as f32;
        let filter_position: Position<f32> = self.filter_position().into();
        let button_height = self.button_size().height as i32;
        let entry_size = self.entry_size();

        // Get UI textures.
        //
        // This must happen with the renderer bound to ensure new textures are
        // associated with the correct program.
        unsafe { self.history_textures.free_unused_textures() };
        let delete_button = if self.pending_delete_confirmation {
            self.confirm_button.texture()
        } else {
            self.delete_button.texture()
        };
        let close_button = self.close_button.texture();
        let filter_label = self.filter.texture();

        // Draw background.
        //
        // NOTE: This clears the entire surface, but works fine since the history popup
        // always fills the entire surface.
        let [r, g, b] = BG.as_f32();
        unsafe {
            gl::ClearColor(r, g, b, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
        }

        if !self.pending_delete_confirmation {
            // Scissor crop bottom entry, to not overlap the buttons.
            unsafe {
                gl::Enable(gl::SCISSOR_TEST);
                gl::Scissor(0, button_height, i32::MAX, ui_height as i32);
            }

            // Draw history list.
            let mut texture_pos =
                Position::new(x_padding, close_button_position.y + self.scroll_offset as f32);
            for i in 0..self.history_textures.len() {
                // Render only entries within the viewport.
                texture_pos.y -= entry_size.height as f32;
                if texture_pos.y <= -(entry_size.height as f32) {
                    break;
                } else if texture_pos.y < close_button_position.y {
                    let texture = self.history_textures.texture(i, entry_size, self.scale);
                    renderer.draw_texture_at(texture, texture_pos, None);
                }

                // Add padding after the history entry.
                texture_pos.y -= (ENTRY_Y_PADDING * self.scale) as f32
            }

            unsafe { gl::Disable(gl::SCISSOR_TEST) };
        }

        // Render delete confirmation text.
        if self.pending_delete_confirmation {
            let entry_count = self.history_textures.len();
            let delete_prompt = self.delete_prompt.texture(entry_count);
            renderer.draw_texture_at(delete_prompt, delete_prompt_position, None);
        }

        // Draw buttons.
        renderer.draw_texture_at(delete_button, delete_button_position, None);
        renderer.draw_texture_at(close_button, close_button_position, None);

        // Draw filter text.
        renderer.draw_texture_at(filter_label, filter_position, None);
    }

    fn position(&self) -> Position {
        Position::new(0, 0)
    }

    fn set_size(&mut self, size: Size) {
        self.size = size;
        self.dirty = true;

        // Update UI element sizes.
        self.delete_prompt.set_geometry(self.delete_prompt_size(), self.scale);
        self.confirm_button.set_geometry(self.button_size(), self.scale);
        self.delete_button.set_geometry(self.button_size(), self.scale);
        self.close_button.set_geometry(self.button_size(), self.scale);
        self.filter.set_geometry(self.filter_size(), self.scale);
    }

    fn size(&self) -> Size {
        self.size
    }

    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.dirty = true;

        // Update UI element scales.
        self.delete_prompt.set_geometry(self.delete_prompt_size(), self.scale);
        self.confirm_button.set_geometry(self.button_size(), self.scale);
        self.delete_button.set_geometry(self.button_size(), self.scale);
        self.close_button.set_geometry(self.button_size(), self.scale);
        self.filter.set_geometry(self.filter_size(), self.scale);
    }

    fn opaque_region(&self) -> Size {
        self.size
    }

    fn press_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        if let Some(KeyboardInputElement::Filter) = self.keyboard_focus {
            self.filter.input.press_key(raw, keysym, modifiers)
        }
    }

    fn touch_down(
        &mut self,
        time: u32,
        id: i32,
        logical_position: Position<f64>,
        _modifiers: Modifiers,
    ) {
        // Only accept a single touch point in the UI.
        if self.touch_state.slot.is_some() {
            return;
        }
        self.touch_state.slot = Some(id);

        // Convert position to physical space.
        let position = logical_position * self.scale;
        self.touch_state.position = position;
        self.touch_state.start = position;

        // Get button geometries.
        let delete_button_position = self.delete_button_position();
        let close_button_position = self.close_button_position();
        let filter_position = self.filter_position();
        let filter_size = self.filter_size().into();
        let button_size = self.button_size().into();

        if rect_contains(delete_button_position, button_size, position) {
            self.touch_state.action = TouchAction::DeleteTap;
            self.clear_keyboard_focus();
        } else if rect_contains(close_button_position, button_size, position) {
            self.touch_state.action = TouchAction::CloseTap;
            self.clear_keyboard_focus();
        } else if rect_contains(filter_position, filter_size, position) {
            let (text_position, _) = self.filter.text_geometry();
            let relative_position = position - filter_position - text_position;
            self.filter.input.touch_down(time, logical_position, relative_position);

            self.filter.input.set_focus(true);
            self.touch_state.action = TouchAction::FilterTouch;
            self.keyboard_focus = Some(KeyboardInputElement::Filter);
        } else {
            self.touch_state.action = TouchAction::EntryTap;
            self.clear_keyboard_focus();
        }
    }

    fn touch_motion(
        &mut self,
        _time: u32,
        id: i32,
        position: Position<f64>,
        _modifiers: Modifiers,
    ) {
        // Ignore all unknown touch points.
        if self.touch_state.slot != Some(id) {
            return;
        }

        // Update touch position.
        let position = position * self.scale;
        let old_position = mem::replace(&mut self.touch_state.position, position);

        match self.touch_state.action {
            // Handle transition from tap to drag.
            TouchAction::EntryTap | TouchAction::EntryDrag => {
                // Ignore dragging until tap distance limit is exceeded.
                let delta = self.touch_state.position - self.touch_state.start;
                if delta.x.powi(2) + delta.y.powi(2) <= MAX_TAP_DISTANCE {
                    return;
                }
                self.touch_state.action = TouchAction::EntryDrag;

                // Immediately start moving the history list.
                let old_offset = self.scroll_offset;
                self.scroll_offset += self.touch_state.position.y - old_position.y;
                self.clamp_scroll_offset();
                self.dirty |= self.scroll_offset != old_offset;
            },
            // Forward filter label events.
            TouchAction::FilterTouch => {
                let (text_position, _) = self.filter.text_geometry();
                let relative_position = position - self.filter_position() - text_position;
                self.filter.input.touch_motion(relative_position);
            },
            // Ignore drag when tap started on a UI element.
            _ => (),
        }
    }

    fn touch_up(&mut self, time: u32, id: i32, _modifiers: Modifiers) {
        // Ignore all unknown touch points.
        if self.touch_state.slot != Some(id) {
            return;
        }
        self.touch_state.slot = None;

        match self.touch_state.action {
            TouchAction::EntryTap => match self.entry_at(self.touch_state.start) {
                // Open history entry in a new tab.
                Some((uri, false)) => {
                    // Create new tab for the selected URI.
                    let uri = uri.to_string(true);
                    self.queue.open_history_in_tab(self.window_id, uri);

                    // Close all overlay windows.
                    self.queue.close_history(self.window_id);
                    self.queue.close_tabs_ui(self.window_id);
                },
                // Remove entry from the history.
                Some((uri, true)) => {
                    // Delete the uri from storage.
                    let uri = uri.to_string(true);
                    self.history_db.delete(&uri);

                    // Update the history entries.
                    let entries = self.history_db.entries().unwrap_or_default();
                    self.history_textures.set_entries(entries);
                    self.dirty = true;
                },
                None => (),
            },
            // Abort deletion if confirmation is pending.
            TouchAction::CloseTap if self.pending_delete_confirmation => {
                self.pending_delete_confirmation = false;
                self.dirty = true;
            },
            // Close the history UI.
            TouchAction::CloseTap => self.queue.close_history(self.window_id),
            // Prompt for confirmation on first press.
            TouchAction::DeleteTap if !self.pending_delete_confirmation => {
                if !self.history_textures.is_empty() {
                    self.pending_delete_confirmation = true;
                    self.dirty = true;
                } else {
                    // Clear filter if there are no matching entries.
                    self.history_textures.filter.clear();
                    self.filter.input.set_text("");

                    // Update the history entries.
                    let entries = self.history_db.entries().unwrap_or_default();
                    self.history_textures.set_entries(entries);
                }
            },
            // Confirm deletion on second press.
            TouchAction::DeleteTap => {
                // Delete all history entries.
                let filter = (!self.history_textures.filter.is_empty())
                    .then_some(self.history_textures.filter.as_str());
                self.history_db.bulk_delete(filter);

                // Clear active filter.
                self.history_textures.filter.clear();
                self.filter.input.set_text("");

                // Update the history entries.
                let entries = self.history_db.entries().unwrap_or_default();
                self.history_textures.set_entries(entries);

                // Clear confirmation prompt.
                self.pending_delete_confirmation = false;
                self.dirty = true;
            },
            // Forward filter label events.
            TouchAction::FilterTouch => self.filter.input.touch_up(time),
            TouchAction::EntryDrag => (),
        }
    }

    fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32) {
        if let Some(KeyboardInputElement::Filter) = self.keyboard_focus {
            self.filter.input.delete_surrounding_text(before_length, after_length);
        }
    }

    fn commit_string(&mut self, text: &str) {
        if let Some(KeyboardInputElement::Filter) = self.keyboard_focus {
            self.filter.input.commit_string(text);
        }
    }

    fn set_preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32) {
        if let Some(KeyboardInputElement::Filter) = self.keyboard_focus {
            self.filter.input.set_preedit_string(text, cursor_begin, cursor_end);
        }
    }

    fn text_input_state(&mut self) -> TextInputChange {
        match self.keyboard_focus {
            Some(KeyboardInputElement::Filter) => {
                self.filter.input.text_input_state(self.filter_position())
            },
            _ => TextInputChange::Disabled,
        }
    }

    fn paste(&mut self, text: &str) {
        if let Some(KeyboardInputElement::Filter) = self.keyboard_focus {
            self.filter.input.paste(text);
        }
    }

    fn has_keyboard_focus(&self) -> bool {
        self.keyboard_focus.is_some()
    }

    fn clear_keyboard_focus(&mut self) {
        self.filter.input.set_focus(false);
        self.filter.input.submit();

        self.keyboard_focus = None;
    }
}

/// History texture cache by URI.
#[derive(Default)]
struct HistoryTextures {
    textures: HashMap<HistoryUri, (HistoryEntry, Texture)>,
    entries: Vec<(HistoryUri, HistoryEntry)>,
    filter: String,
}

impl HistoryTextures {
    /// Update the history entries.
    fn set_entries(&mut self, mut entries: Vec<(HistoryUri, HistoryEntry)>) {
        entries.retain(|(uri, entry)| {
            uri.to_string(true).contains(&self.filter) || entry.title.contains(&self.filter)
        });

        self.entries = entries;
    }

    /// Cleanup unused textures.
    ///
    /// # Safety
    ///
    /// The correct OpenGL context **must** be current or this will attempt to
    /// delete invalid OpenGL textures.
    #[cfg_attr(feature = "profiling", profiling::function)]
    unsafe fn free_unused_textures(&mut self) {
        // Remove unused URIs from cache.
        self.textures.retain(|uri, (entry, texture)| {
            // Only retain items with unchanged URI, Title, and Access Time.
            let retain = self.entries.iter().any(|(history_uri, history_entry)| {
                uri == history_uri
                    && entry.title == history_entry.title
                    && entry.last_access == history_entry.last_access
            });

            // Release OpenGL texture.
            if !retain {
                texture.delete();
            }

            retain
        });
    }

    /// Get the texture for a history entry.
    ///
    /// This will automatically take care of caching rendered textures.
    ///
    /// ### Panics
    ///
    /// Panics if `index >= self.len()`.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn texture(&mut self, index: usize, entry_size: Size, scale: f64) -> &Texture {
        let (uri, entry) = &self.entries[index];

        // Create and cache texture if necessary.
        if !self.textures.contains_key(uri) {
            // Create title pango layout.
            let layout = TextLayout::new(font_size(1.13), scale);
            let title_height = layout.line_height();

            // Create timestamp layout.
            let timestamp_layout = TextLayout::new(font_size(0.63), scale);
            let timestamp_height = timestamp_layout.line_height();
            let timestamp = DateTime::from_timestamp(entry.last_access, 0)
                .unwrap_or_default()
                .with_timezone(&Local);
            let timestamp_text = timestamp.format("%Y-%m-%d %H:%M").to_string();
            timestamp_layout.set_text(&timestamp_text);

            // Get Y text padding above title.
            let mut y_padding =
                ((entry_size.height as i32 - title_height - timestamp_height) / 2) as f64;

            // Set title layout text and setup subtitle layout if necessary.
            let mut subtitle_height = 0;
            let subtitle_layout = if !entry.title.trim().is_empty() {
                layout.set_text(&entry.title);

                // Create subtitle layout, to get its line height.
                let subtitle_layout = TextLayout::new(font_size(0.63), scale);
                subtitle_layout.set_text(&uri.to_string(true));

                // Calculate y padding from title and subtitle size.
                subtitle_height = subtitle_layout.line_height();
                y_padding -= (subtitle_height / 2) as f64;

                Some(subtitle_layout)
            } else {
                layout.set_text(&uri.to_string(true));

                None
            };

            // Configure text rendering options.
            let mut text_options = TextOptions::new();

            // Calculate available area for font rendering.
            let close_position = History::close_entry_button_position(entry_size, scale);
            let text_width = (close_position.x - close_position.y * 2.).round() as i32;
            let title_size = Size::new(text_width, title_height);
            text_options.position(Position::new(close_position.y, y_padding));
            text_options.size(title_size);

            // Render text to the texture.
            let builder = TextureBuilder::new(entry_size.into());
            builder.clear(SECONDARY_BG.as_f64());
            builder.rasterize(&layout, &text_options);

            // Also render URI if main label was title.
            if let Some(subtitle_layout) = subtitle_layout {
                // Calculate URI text placement.
                let subtitle_size = Size::new(text_width, subtitle_height);
                let subtitle_y = title_height as f64 + y_padding;
                text_options.position(Position::new(close_position.y, subtitle_y));
                text_options.size(subtitle_size);

                // Render URI to texture.
                text_options.text_color(SECONDARY_FG.as_f64());
                builder.rasterize(&subtitle_layout, &text_options);
            }

            // Render human-readable time since last visit.
            let timestamp_size = Size::new(text_width, timestamp_height);
            let timestamp_y = entry_size.height as f64 - y_padding - timestamp_height as f64;
            text_options.position(Position::new(close_position.y, timestamp_y));
            text_options.size(timestamp_size);
            text_options.text_color(SECONDARY_FG.as_f64());
            builder.rasterize(&timestamp_layout, &text_options);

            // Render close `X`.
            let fg = FG.as_f64();
            let size = History::close_entry_button_size(entry_size, scale);
            let context = builder.context();
            context.move_to(close_position.x, close_position.y);
            context.line_to(close_position.x + size.width, close_position.y + size.height);
            context.move_to(close_position.x + size.width, close_position.y);
            context.line_to(close_position.x, close_position.y + size.height);
            context.set_source_rgb(fg[0], fg[1], fg[2]);
            context.set_line_width(scale);
            context.stroke().unwrap();

            self.textures.insert(uri.clone(), (entry.clone(), builder.build()));
        }

        &self.textures.get(uri).unwrap().1
    }

    /// Get the number of entries.
    fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check whether there are any entries.
    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Deletion confirmation text dialog.
#[derive(Default)]
struct ConfirmationPrompt {
    texture: Option<Texture>,

    last_entry_count: usize,
    dirty: bool,
    size: Size,
    scale: f64,
}

impl ConfirmationPrompt {
    /// Get this text's OpenGL texture.
    pub fn texture(&mut self, entry_count: usize) -> &Texture {
        // Ensure texture is up to date.
        if mem::take(&mut self.dirty) || self.last_entry_count != entry_count {
            // Ensure texture is cleared while program is bound.
            if let Some(texture) = self.texture.take() {
                texture.delete();
            }
            self.texture = Some(self.draw(entry_count));
            self.last_entry_count = entry_count;
        }

        self.texture.as_ref().unwrap()
    }

    /// Draw the text into an OpenGL texture.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn draw(&self, entry_count: usize) -> Texture {
        // Clear with background color.
        let builder = TextureBuilder::new(self.size.into());
        builder.clear(BG.as_f64());

        // Render confirmation prompt text.
        let layout = TextLayout::new(font_size(1.13), self.scale);
        layout.set_alignment(Alignment::Center);
        layout.set_text(&format!("Confirm deleting {entry_count} history entries?"));
        builder.rasterize(&layout, &TextOptions::new());

        builder.build()
    }

    /// Set the physical size and scale of the text.
    fn set_geometry(&mut self, size: Size, scale: f64) {
        self.size = size;
        self.scale = scale;

        // Force redraw.
        self.dirty = true;
    }
}

/// History filter text input.
struct HistoryFilter {
    input: TextField,

    texture: Option<Texture>,

    size: Size,
    scale: f64,
}

impl HistoryFilter {
    fn new(window_id: WindowId, mut queue: MtQueueHandle<State>) -> Self {
        let mut input = TextField::new(window_id, queue.clone(), font_size(1.13));
        input.set_text_change_handler(Box::new(move |label| {
            queue.set_history_filter(window_id, label.text())
        }));
        Self { input, scale: 1., texture: Default::default(), size: Default::default() }
    }

    fn texture(&mut self) -> &Texture {
        // Ensure texture is up to date.
        if mem::take(&mut self.input.dirty) {
            // Ensure texture is cleared while program is bound.
            if let Some(texture) = self.texture.take() {
                texture.delete();
            }
            self.texture = Some(self.draw());
        }

        self.texture.as_ref().unwrap()
    }

    /// Draw the label into an OpenGL texture.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn draw(&mut self) -> Texture {
        // Add padding to text dimensions.
        let (mut text_position, text_size) = self.text_geometry();
        text_position.x += self.input.scroll_offset;

        // Set text rendering options.
        let mut text_options = TextOptions::new();
        text_options.cursor_position(self.input.cursor_index());
        text_options.autocomplete(self.input.autocomplete().into());
        text_options.preedit(self.input.preedit.clone());
        text_options.position(text_position);
        text_options.size(text_size.into());
        text_options.set_ellipsize(false);

        if self.input.focused {
            // Show cursor or selection when focused.
            if self.input.selection.is_some() {
                text_options.selection(self.input.selection.clone());
            } else {
                text_options.show_cursor();
            }
        } else {
            // Show placeholder without focus.
            text_options.set_placeholder("Filter…");
        }

        // Rasterize the text field.
        let layout = self.input.layout();
        layout.set_scale(self.scale);
        let builder = TextureBuilder::new(self.size.into());
        builder.clear(SECONDARY_BG.as_f64());
        builder.rasterize(layout, &text_options);

        builder.build()
    }

    /// Set the physical size and scale of the text field.
    fn set_geometry(&mut self, size: Size, scale: f64) {
        self.size = size;
        self.scale = scale;

        // Update text input width.
        let (_, text_size) = self.text_geometry();
        self.input.set_width(text_size.width as f64);

        // Force redraw.
        self.input.dirty = true;
    }

    /// Get physical geometry of the text input area.
    fn text_geometry(&self) -> (Position<f64>, Size) {
        let padding = (BUTTON_PADDING * self.scale).round();
        let width = (self.size.width as f64 - 2. * padding).round();
        let size = Size::new(width as u32, self.size.height);
        (Position::new(padding, 0.), size)
    }
}

/// Touch event tracking.
#[derive(Default)]
struct TouchState {
    slot: Option<i32>,
    action: TouchAction,
    start: Position<f64>,
    position: Position<f64>,
}

/// Intention of a touch sequence.
#[derive(Default, Copy, Clone, PartialEq, Eq, Debug)]
enum TouchAction {
    #[default]
    EntryTap,
    EntryDrag,
    CloseTap,
    DeleteTap,
    FilterTouch,
}

/// Elements accepting keyboard focus.
enum KeyboardInputElement {
    Filter,
}
