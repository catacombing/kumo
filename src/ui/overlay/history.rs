//! History overlay.

use std::collections::HashMap;
use std::mem;

use chrono::{DateTime, Local};
use funq::MtQueueHandle;
use smithay_client_toolkit::seat::keyboard::Modifiers;

use crate::engine::{EngineHandler, NO_GROUP_ID};
use crate::storage::history::{History as HistoryDb, HistoryEntry, HistoryUri};
use crate::ui::overlay::tabs::TabsHandler;
use crate::ui::overlay::Popup;
use crate::ui::renderer::{Renderer, Svg, TextLayout, TextOptions, Texture, TextureBuilder};
use crate::ui::{SvgButton, MAX_TAP_DISTANCE};
use crate::window::WindowId;
use crate::{gl, rect_contains, Position, Size, State};

/// History view background color.
const HISTORY_BG: [f64; 3] = [0.09, 0.09, 0.09];

/// Logical height of the UI buttons.
const BUTTON_HEIGHT: u32 = 60;

/// Padding around buttons.
const BUTTON_PADDING: f64 = 10.;

/// Main history entry font size.
const FONT_SIZE: u8 = 18;

/// History entry subtitle font size.
const SECONDARY_FONT_SIZE: u8 = 10;

/// History list entry background color.
const ENTRY_BG: [f64; 3] = [0.15, 0.15, 0.15];

/// History entry subtitle foreground color.
const SUBTITLE_FG: [f64; 3] = [0.75, 0.75, 0.75];

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
}

impl HistoryHandler for State {
    fn close_history(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };
        window.set_history_ui_visibile(false);
    }
}

/// History UI.
pub struct History {
    history_textures: HistoryTextures,
    close_button: SvgButton,
    scroll_offset: f64,

    touch_state: TouchState,

    size: Size,
    scale: f64,

    queue: MtQueueHandle<State>,
    window_id: WindowId,

    history_db: HistoryDb,

    visible: bool,
    dirty: bool,
}

impl History {
    pub fn new(window_id: WindowId, queue: MtQueueHandle<State>, history_db: HistoryDb) -> Self {
        Self {
            history_db,
            window_id,
            queue,
            close_button: SvgButton::new(Svg::Close),
            history_textures: Default::default(),
            scroll_offset: Default::default(),
            touch_state: Default::default(),
            visible: Default::default(),
            dirty: Default::default(),
            scale: Default::default(),
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
        let width = self.button_size().width;
        let x = (self.size.width as f64 * self.scale).round() - width as f64;
        Position::new(x, 0.)
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
        let entry_start_y = self.close_button_position().y + self.button_size().height as f64;
        let entry_end_y = self.size.height as f64 * self.scale - x_padding;
        let entry_size_int = self.entry_size();
        let entry_size: Size<f64> = entry_size_int.into();

        // Check whether position is within history list boundaries.
        if position.x < x_padding
            || position.x >= x_padding + entry_size.width
            || position.y < entry_start_y
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
        self.dirty
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
        let close_button_position: Position<f32> = self.close_button_position().into();
        let ui_height = (self.size.height as f64 * self.scale).round() as f32;
        let history_start = close_button_position.y + self.button_size().height as f32;
        let button_height = self.button_size().height as i32;
        let entry_size = self.entry_size();

        // Get UI textures.
        //
        // This must happen with the renderer bound to ensure new textures are
        // associated with the correct program.
        self.history_textures.free_unused_textures();
        let close_button = self.close_button.texture();

        // Draw background.
        //
        // NOTE: This clears the entire surface, but works fine since the history popup
        // always fills the entire surface.
        let [r, g, b] = HISTORY_BG;
        unsafe {
            gl::ClearColor(r as f32, g as f32, b as f32, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
        }

        // Scissor crop top entry, to not overlap the buttons.
        let scissor_height = (ui_height - close_button_position.y) as i32 - button_height;
        unsafe {
            gl::Enable(gl::SCISSOR_TEST);
            gl::Scissor(0, 0, i32::MAX, scissor_height);
        }

        // Draw history list.
        let x_padding = (ENTRY_X_PADDING * self.scale) as f32;
        let mut texture_pos =
            Position::new(x_padding, ui_height - x_padding + self.scroll_offset as f32);
        for i in 0..self.history_textures.len() {
            // Render only entries within the viewport.
            texture_pos.y -= entry_size.height as f32;
            if texture_pos.y <= history_start - entry_size.height as f32 {
                break;
            } else if texture_pos.y < ui_height {
                let texture = self.history_textures.texture(i, entry_size, self.scale);
                unsafe { renderer.draw_texture_at(texture, texture_pos, None) };
            }

            // Add padding after the history entry.
            texture_pos.y -= (ENTRY_Y_PADDING * self.scale) as f32
        }

        unsafe { gl::Disable(gl::SCISSOR_TEST) };

        // Draw close button.
        unsafe { renderer.draw_texture_at(close_button, close_button_position, None) };
    }

    fn position(&self) -> Position {
        Position::new(0, 0)
    }

    fn set_size(&mut self, size: Size) {
        self.size = size;
        self.dirty = true;

        // Update UI element sizes.
        self.close_button.set_geometry(self.button_size(), self.scale);
    }

    fn size(&self) -> Size {
        self.size
    }

    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.dirty = true;

        // Update UI element scales.
        self.close_button.set_geometry(self.button_size(), self.scale);
    }

    fn opaque_region(&self) -> Size {
        self.size
    }

    fn touch_down(&mut self, _time: u32, id: i32, position: Position<f64>, _modifiers: Modifiers) {
        // Only accept a single touch point in the UI.
        if self.touch_state.slot.is_some() {
            return;
        }
        self.touch_state.slot = Some(id);

        // Convert position to physical space.
        let position = position * self.scale;
        self.touch_state.position = position;
        self.touch_state.start = position;

        // Get button geometries.
        let close_button_position = self.close_button_position();
        let button_size = self.button_size().into();

        if rect_contains(close_button_position, button_size, position) {
            self.touch_state.action = TouchAction::CloseTap;
        } else {
            self.touch_state.action = TouchAction::EntryTap;
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
            // Ignore drag when tap started on a UI element.
            _ => (),
        }
    }

    fn touch_up(&mut self, _time: u32, id: i32, _modifiers: Modifiers) {
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
                    self.queue.open_in_tab(self.window_id, NO_GROUP_ID, uri, true);

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
            // Close the history UI.
            TouchAction::CloseTap => self.queue.close_history(self.window_id),
            TouchAction::EntryDrag => (),
        }
    }
}

/// History texture cache by URI.
#[derive(Default)]
struct HistoryTextures {
    textures: HashMap<HistoryUri, (HistoryEntry, Texture)>,
    entries: Vec<(HistoryUri, HistoryEntry)>,
}

impl HistoryTextures {
    /// Update the history entries.
    fn set_entries(&mut self, entries: Vec<(HistoryUri, HistoryEntry)>) {
        self.entries = entries;
    }

    /// Cleanup unused textures.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn free_unused_textures(&mut self) {
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
            let layout = TextLayout::new(FONT_SIZE, scale);
            let title_height = layout.line_height();

            // Create timestamp layout.
            let timestamp_layout = TextLayout::new(SECONDARY_FONT_SIZE, scale);
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
                let subtitle_layout = TextLayout::new(SECONDARY_FONT_SIZE, scale);
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

            // Calculate available area font font rendering.
            let close_position = History::close_entry_button_position(entry_size, scale);
            let text_width = (close_position.x - close_position.y * 2.).round() as i32;
            let title_size = Size::new(text_width, title_height);
            text_options.position(Position::new(close_position.y, y_padding));
            text_options.size(title_size);

            // Render text to the texture.
            let builder = TextureBuilder::new(entry_size.into());
            builder.clear(ENTRY_BG);
            builder.rasterize(&layout, &text_options);

            // Also render URI if main label was title.
            if let Some(subtitle_layout) = subtitle_layout {
                // Calculate URI text placement.
                let subtitle_size = Size::new(text_width, subtitle_height);
                let subtitle_y = title_height as f64 + y_padding;
                text_options.position(Position::new(close_position.y, subtitle_y));
                text_options.size(subtitle_size);

                // Render URI to texture.
                text_options.text_color(SUBTITLE_FG);
                builder.rasterize(&subtitle_layout, &text_options);
            }

            // Render human-readable time since last visit.
            let timestamp_size = Size::new(text_width, timestamp_height);
            let timestamp_y = entry_size.height as f64 - y_padding - timestamp_height as f64;
            text_options.position(Position::new(close_position.y, timestamp_y));
            text_options.size(timestamp_size);
            text_options.text_color(SUBTITLE_FG);
            builder.rasterize(&timestamp_layout, &text_options);

            // Render close `X`.
            let size = History::close_entry_button_size(entry_size, scale);
            let context = builder.context();
            context.move_to(close_position.x, close_position.y);
            context.line_to(close_position.x + size.width, close_position.y + size.height);
            context.move_to(close_position.x + size.width, close_position.y);
            context.line_to(close_position.x, close_position.y + size.height);
            context.set_source_rgb(1., 1., 1.);
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
}
