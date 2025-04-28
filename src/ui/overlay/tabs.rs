//! Tabs overlay.

use std::borrow::Cow;
use std::collections::HashMap;
use std::mem;
use std::time::{Duration, Instant};

use funq::MtQueueHandle;
use glib::{ControlFlow, Priority, Source, source};
use indexmap::IndexMap;
use pangocairo::pango::Alignment;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};

use crate::engine::{Engine, EngineId, Favicon, Group, GroupId, NO_GROUP, NO_GROUP_ID};
use crate::ui::overlay::Popup;
use crate::ui::renderer::{Renderer, Svg, TextLayout, TextOptions, Texture, TextureBuilder};
use crate::ui::{LONG_PRESS_MILLIS, MAX_TAP_DISTANCE, SvgButton, TextField};
use crate::window::TextInputChange;
use crate::{Position, Size, State, WindowId, gl, rect_contains};

/// Tab text color of active tab.
const ACTIVE_TAB_FG: [f64; 3] = [1., 1., 1.];
/// Tab text color of inactive tabs.
const INACTIVE_TAB_FG: [f64; 3] = [0.8, 0.8, 0.8];
/// Tab view background color.
const TABS_BG: [f64; 3] = [0.09, 0.09, 0.09];
/// New tab button background color.
const NEW_TAB_BG: [f64; 3] = [0.15, 0.15, 0.15];
/// Tab load progress highlight color.
const PROGRESS_TAB_BG: [f64; 4] = [0.46, 0.16, 0.16, 0.5];

/// Tab font size.
const FONT_SIZE: u8 = 20;

/// Horizontal tabbing around tabs.
const TABS_X_PADDING: f64 = 10.;

/// Vertical padding between tabs.
const TABS_Y_PADDING: f64 = 1.;

/// Horizontal padding around buttons.
const BUTTON_X_PADDING: f64 = 10.;

/// Vertical padding around buttons.
const BUTTON_Y_PADDING: f64 = 10.;

/// Padding around the tab "X" button.
const CLOSE_PADDING: f64 = 30.;

/// Logical height of each tab.
const TAB_HEIGHT: u32 = 50;

/// Logical height of the UI buttons.
const BUTTON_HEIGHT: u32 = 60;

/// Favicon width and height at scale 1.
const FAVICON_SIZE: f64 = 28.;

/// Scale of a tab being drag & dropped.
const REORDERING_SCALE: f32 = 0.9;

#[funq::callbacks(State)]
pub trait TabsHandler {
    /// Create a new tab and switch to it.
    fn add_tab(&mut self, window_id: WindowId, group_id: GroupId);

    /// Switch tabs.
    fn set_active_tab(&mut self, engine_id: EngineId);

    /// Close a tab.
    fn close_tab(&mut self, engine_id: EngineId);

    /// Move an existing tab to a new position in the tabs list.
    fn move_tab(&mut self, engine_id: EngineId, new_index: usize);

    /// Cycle overview to the next tab group.
    fn cycle_tab_group(&mut self, window_id: WindowId, group_id: GroupId);

    /// Set ephemeral mode of the active tab group.
    fn set_ephemeral_mode(&mut self, window_id: WindowId, group_id: GroupId, ephemeral: bool);

    /// Create a new tab group.
    fn create_tab_group(&mut self, window_id: WindowId);

    /// Delete a tab group.
    fn delete_tab_group(&mut self, window_id: WindowId, group_id: GroupId);

    /// Update a tab group's label.
    fn update_group_label(&mut self, window_id: WindowId, label: String);

    /// Open history UI.
    fn show_history_ui(&mut self, window_id: WindowId);

    /// Open downloads UI.
    fn show_downloads_ui(&mut self, window_id: WindowId);

    /// Close the tabs UI.
    fn close_tabs_ui(&mut self, window_id: WindowId);

    /// Start tab drop & drag.
    fn start_tab_reordering(&mut self, tab: EngineId);
}

impl TabsHandler for State {
    fn add_tab(&mut self, window_id: WindowId, group_id: GroupId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };

        let _ = window.add_tab(true, true, group_id);
        window.set_tabs_ui_visible(false);
    }

    fn set_active_tab(&mut self, engine_id: EngineId) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };

        window.set_active_tab(engine_id);
        window.set_tabs_ui_visible(false);
    }

    fn close_tab(&mut self, engine_id: EngineId) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };

        window.close_tab(engine_id);
    }

    fn move_tab(&mut self, engine_id: EngineId, new_index: usize) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };

        window.move_tab(engine_id, new_index);
    }

    fn cycle_tab_group(&mut self, window_id: WindowId, group_id: GroupId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };

        window.cycle_tab_group(group_id);
    }

    fn set_ephemeral_mode(&mut self, window_id: WindowId, group_id: GroupId, ephemeral: bool) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };

        window.set_ephemeral_mode(group_id, ephemeral);
    }

    fn create_tab_group(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };

        window.create_tab_group(None, true);
    }

    fn delete_tab_group(&mut self, window_id: WindowId, group_id: GroupId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };

        window.delete_tab_group(group_id);
    }

    fn update_group_label(&mut self, window_id: WindowId, label: String) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };

        window.update_group_label(label);
    }

    fn show_history_ui(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };
        window.set_history_ui_visibile(true);
    }

    fn show_downloads_ui(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };
        window.set_downloads_ui_visibile(true);
    }

    fn close_tabs_ui(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };
        window.set_tabs_ui_visible(false);
    }

    fn start_tab_reordering(&mut self, tab: EngineId) {
        let window = match self.windows.get_mut(&tab.window_id()) {
            Some(window) => window,
            None => return,
        };
        window.start_tab_reordering(tab);
    }
}

/// Tab overview UI.
pub struct Tabs {
    last_reordering_scroll: Option<Instant>,
    texture_cache: TextureCache,
    scroll_offset: f64,

    size: Size,
    scale: f64,

    queue: MtQueueHandle<State>,
    window_id: WindowId,

    close_group_button: SvgButton,
    cycle_group_button: SvgButton,
    persistent_button: SvgButton,
    new_group_button: PlusButton,
    new_tab_button: PlusButton,
    downloads_button: SvgButton,
    history_button: SvgButton,
    downloads_visible: bool,

    keyboard_focus: Option<KeyboardInputElement>,
    touch_state: TouchState,

    group_label: GroupLabel,
    group: GroupId,

    visible: bool,
    dirty: bool,
}

impl Tabs {
    pub fn new(window_id: WindowId, queue: MtQueueHandle<State>) -> Self {
        let group_label = GroupLabel::new(window_id, queue.clone());
        Self {
            group_label,
            window_id,
            queue,
            persistent_button: SvgButton::new_toggle(Svg::PersistentOn, Svg::PersistentOff),
            cycle_group_button: SvgButton::new(Svg::ArrowLeft),
            downloads_button: SvgButton::new(Svg::Download),
            close_group_button: SvgButton::new(Svg::Close),
            history_button: SvgButton::new(Svg::History),
            scale: 1.0,
            last_reordering_scroll: Default::default(),
            downloads_visible: Default::default(),
            new_group_button: Default::default(),
            keyboard_focus: Default::default(),
            new_tab_button: Default::default(),
            texture_cache: Default::default(),
            scroll_offset: Default::default(),
            touch_state: Default::default(),
            visible: Default::default(),
            group: Default::default(),
            dirty: Default::default(),
            size: Default::default(),
        }
    }

    /// Update the tracked tabs.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn set_tabs<'a, T>(&mut self, tabs: T, active_tab: Option<EngineId>)
    where
        T: Iterator<Item = &'a Box<dyn Engine>>,
    {
        // Get index of tab being reordered.
        let mut old_reordering_tab = None;
        if let TouchAction::TabReordering(engine_id) = self.touch_state.action {
            old_reordering_tab =
                self.texture_cache.tabs.get_index_of(&engine_id).map(|i| (i, engine_id));
        }

        self.texture_cache.set_tabs(tabs, active_tab);

        // Adjust reordering touch origin to account for new tab location.
        if let Some((old_position, engine_id)) = old_reordering_tab {
            if let Some(new_position) = self.texture_cache.tabs.get_index_of(&engine_id) {
                let tabs_moved = new_position as f64 - old_position as f64;
                let tab_height = self.tab_size().height as f64;
                self.touch_state.start.y += tabs_moved * tab_height;
            }
        }

        self.dirty = true;
    }

    /// Reload a tab's favicon.
    #[allow(clippy::borrowed_box)]
    pub fn update_favicon(&mut self, tab: &Box<dyn Engine>) {
        self.dirty |= self.texture_cache.update_favicon(tab);
    }

    /// Update a tab's load progress.
    pub fn set_load_progress(&mut self, engine_id: EngineId, load_progress: f64) {
        self.dirty |= self.texture_cache.set_load_progress(engine_id, load_progress);
    }

    /// Update the active tab.
    pub fn set_active_tab(&mut self, active_tab: Option<EngineId>) {
        self.texture_cache.set_active_tab(active_tab);
        self.dirty = true;
    }

    /// Update the active tab group.
    pub fn set_active_tab_group(&mut self, group: &Group) {
        // Always close group label editor.
        self.group_label.stop_editing();

        let id = group.id();
        if id == self.group
            && self.group_label.text == group.label
            && self.persistent_button.enabled != group.ephemeral
        {
            return;
        }

        self.persistent_button.set_enabled(!group.ephemeral);
        self.group_label.set(group.label.clone());
        self.group = id;
        self.dirty = true;
    }

    /// Get the current tab group.
    pub fn active_tab_group(&self) -> GroupId {
        self.group
    }

    /// Show or hide the download UI button.
    pub fn set_downloads_button_visible(&mut self, visible: bool) {
        if self.downloads_visible == visible {
            return;
        }

        self.downloads_visible = visible;
        self.dirty = true;

        // Update new tab button width.
        self.new_tab_button.set_geometry(self.new_tab_button_size(), self.scale);
    }

    /// Check whether the popup is active.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Show or hide a popup.
    pub fn set_visible(&mut self, visible: bool) {
        self.dirty |= self.visible != visible;
        self.visible = visible;
    }

    /// Start tab drop & drag.
    pub fn start_tab_reordering(&mut self, tab: EngineId) {
        self.touch_state.action = TouchAction::TabReordering(tab);
        self.dirty = true;
    }

    /// Get current URI of a tab.
    pub fn tab_uri(&self, tab: EngineId) -> Option<&str> {
        self.texture_cache.tabs.get(&tab).map(|tab| tab.uri.as_str())
    }

    /// Physical size of the tab creation button bar.
    ///
    /// This includes all padding since that is included in the texture.
    fn new_tab_button_size(&self) -> Size {
        let visible_buttons = if self.downloads_visible { 2 } else { 1 };
        let height = BUTTON_HEIGHT + (2. * BUTTON_Y_PADDING).round() as u32;
        let width = self.size.width - BUTTON_HEIGHT * visible_buttons - BUTTON_X_PADDING as u32;
        Size::new(width, height) * self.scale
    }

    /// Physical position of the tab creation button.
    ///
    /// This includes all padding since that is included in the texture.
    fn new_tab_button_position(&self) -> Position<f64> {
        let visible_buttons = if self.downloads_visible { 2. } else { 1. };
        let y = (self.size.height - BUTTON_HEIGHT) as f64 - 2. * BUTTON_Y_PADDING;
        let x = BUTTON_HEIGHT as f64 * visible_buttons + BUTTON_X_PADDING;
        Position::new(x, y) * self.scale
    }

    /// Get default UI button size.
    fn button_size(scale: f64) -> Size {
        let height = BUTTON_HEIGHT + (2. * BUTTON_Y_PADDING).round() as u32;
        let width = BUTTON_HEIGHT + (2. * BUTTON_X_PADDING).round() as u32;
        Size::new(width, height) * scale
    }

    /// Physical size of the tab group cycle button.
    ///
    /// This includes all padding since that is included in the texture.
    fn cycle_group_button_size(&self) -> Size {
        Self::button_size(self.scale)
    }

    /// Physical position of the tab group cycle button.
    ///
    /// This includes all padding since that is included in the texture.
    fn cycle_group_button_position(&self) -> Position<f64> {
        Position::new(0., 0.)
    }

    /// Physical size of the browser history button.
    ///
    /// This includes all padding since that is included in the texture.
    fn history_button_size(&self) -> Size {
        Self::button_size(self.scale)
    }

    /// Physical position of the browser history button.
    ///
    /// This includes all padding since that is included in the texture.
    fn history_button_position(&self) -> Position<f64> {
        Position::new(0., self.new_tab_button_position().y)
    }

    /// Physical size of the persistent mode button.
    ///
    /// This includes all padding since that is included in the texture.
    fn persistent_button_size(&self) -> Size {
        self.history_button_size()
    }

    /// Physical position of the persistent mode button.
    ///
    /// This includes all padding since that is included in the texture.
    fn persistent_button_position(&self) -> Position<f64> {
        self.history_button_position()
    }

    /// Physical size of the downloads button.
    ///
    /// This includes all padding since that is included in the texture.
    fn downloads_button_size(&self) -> Size {
        Self::button_size(self.scale)
    }

    /// Physical position of the downloads button.
    ///
    /// This includes all padding since that is included in the texture.
    fn downloads_button_position(&self) -> Position<f64> {
        let x = (BUTTON_HEIGHT as f64 + BUTTON_X_PADDING) * self.scale;
        Position::new(x, self.new_tab_button_position().y)
    }

    /// Physical size of the tab group creation button.
    ///
    /// This includes all padding since that is included in the texture.
    fn new_group_button_size(&self) -> Size {
        Self::button_size(self.scale)
    }

    /// Physical position of the tab group creation button.
    ///
    /// This includes all padding since that is included in the texture.
    fn new_group_button_position(&self) -> Position<f64> {
        let width = self.new_group_button_size().width;
        let x = (self.size.width as f64 * self.scale).round() - width as f64;
        Position::new(x, 0.)
    }

    /// Physical size of the tab group label.
    ///
    /// This includes all padding since that is included in the texture.
    fn group_label_size(&self) -> Size {
        let height = BUTTON_HEIGHT + (2. * BUTTON_Y_PADDING).round() as u32;
        Size::new(self.size.width, height) * self.scale
    }

    /// Physical position of the tab group label.
    ///
    /// This includes all padding since that is included in the texture.
    fn group_label_position(&self) -> Position<f64> {
        Position::new(0., 0.)
    }

    /// Get physical size of the close button.
    fn close_button_size(tab_size: Size, scale: f64) -> Size<f64> {
        let size = tab_size.height as f64 - CLOSE_PADDING * scale;
        Size::new(size, size)
    }

    /// Get physical position of the close button within a tab.
    fn close_button_position(tab_size: Size, scale: f64) -> Position<f64> {
        let icon_size = Self::close_button_size(tab_size, scale);
        let button_padding = (tab_size.height as f64 - icon_size.height) / 2.;
        let x = tab_size.width as f64 - button_padding - icon_size.width;
        Position::new(x, button_padding)
    }

    /// Size of the favicon image.
    fn favicon_size(&self) -> Size {
        let size = (FAVICON_SIZE * self.scale).round() as u32;
        Size::new(size, size)
    }

    /// Physical size of each tab.
    fn tab_size(&self) -> Size {
        let width = self.size.width - (2. * TABS_X_PADDING).round() as u32;
        Size::new(width, TAB_HEIGHT) * self.scale
    }

    /// Get tab at the specified location.
    ///
    /// The tuple's second element will be `true` when the position matches the
    /// close button of the tab.
    fn tab_at(&self, mut position: Position<f64>) -> Option<(EngineId, bool)> {
        let tabs_start_y = self.group_label_position().y + self.group_label_size().height as f64;
        let tabs_end_y = self.new_tab_button_position().y;
        let y_padding = TABS_Y_PADDING * self.scale;
        let x_padding = TABS_X_PADDING * self.scale;
        let tab_size_int = self.tab_size();
        let tab_size: Size<f64> = tab_size_int.into();

        // Check if position is beyond tabs list or outside of the horizontal
        // boundaries.
        if position.x < x_padding
            || position.x >= x_padding + tab_size.width
            || position.y < tabs_start_y
            || position.y >= tabs_end_y
        {
            return None;
        }

        // Apply current scroll offset.
        position.y -= self.scroll_offset;

        // Check if position is in the tab separator.
        let new_tab_relative = (tabs_end_y - position.y).round();
        let tab_relative_y =
            tab_size.height - 1. - (new_tab_relative % (tab_size.height + y_padding));
        if tab_relative_y < 0. {
            return None;
        }

        // Find tab at the specified offset.
        let rindex = (new_tab_relative / (tab_size.height + y_padding).round()) as usize;
        let tabs = group_tabs(&self.texture_cache.tabs, self.group);
        let (tab, _) = tabs.rev().nth(rindex)?;

        // Check if click is within close button bounds.
        //
        // We include padding for the close button since it can be really hard to hit
        // otherwise.
        let close_position = Self::close_button_position(tab_size_int, self.scale);
        let tab_relative_x = position.x - x_padding;
        let close = tab_relative_x >= close_position.x - close_position.y;

        Some((*tab, close))
    }

    /// Clamp tabs view viewport offset.
    fn clamp_scroll_offset(&mut self) {
        let old_offset = self.scroll_offset;
        let max_offset = self.max_scroll_offset() as f64;
        self.scroll_offset = self.scroll_offset.clamp(0., max_offset);
        self.dirty |= old_offset != self.scroll_offset;
    }

    /// Get maximum tab scroll offset.
    fn max_scroll_offset(&self) -> usize {
        let tab_padding = (TABS_Y_PADDING * self.scale).round() as usize;
        let tab_height = self.tab_size().height;

        // Calculate height available for tabs.
        let new_tab_button_position_y = self.new_tab_button_position().y.round() as usize;
        let group_label_height = self.group_label_size().height as usize;
        let available_height = new_tab_button_position_y - group_label_height;

        // Calculate height of all tabs.
        let num_tabs = group_tabs(&self.texture_cache.tabs, self.group).count();
        let mut tabs_height =
            (num_tabs * (tab_height as usize + tab_padding)).saturating_sub(tab_padding);

        // Allow a bit of padding at the top.
        let new_tab_padding = (BUTTON_Y_PADDING * self.scale).round();
        tabs_height += new_tab_padding as usize;

        // Calculate tab content outside the viewport.
        tabs_height.saturating_sub(available_height)
    }

    /// Handle scrolling for the tab reordering.
    ///
    /// This will automatically scroll the tabs view when the tab being
    /// reordered is at the top or bottom of the tabs list while scrolling
    /// is available.
    fn add_reordering_scroll_offset(&mut self, engine_id: EngineId) {
        let group_label_position = self.group_label_position();
        let tabs_start = group_label_position.y + self.group_label_size().height as f64;
        let tabs_end = self.new_tab_button_position().y;
        let tab_height = self.tab_size().height as f64;

        if self.touch_state.position.y < tabs_start + tab_height {
            if let Some(last_scroll) = self.last_reordering_scroll {
                // Scroll tabs list, clamping the offset to get the real distance travelled.
                let old_offset = self.scroll_offset;
                self.scroll_offset += last_scroll.elapsed().as_millis() as f64;
                self.clamp_scroll_offset();

                // Adjust tab position and touch offset.
                self.touch_state.start.y += self.scroll_offset - old_offset;
                self.shift_reordering_tab(engine_id);
            }

            self.last_reordering_scroll = Some(Instant::now());
            self.dirty = true;
        } else if self.touch_state.position.y >= tabs_end - tab_height {
            if let Some(last_scroll) = self.last_reordering_scroll {
                // Scroll tabs list, clamping the offset to get the real distance travelled.
                let old_offset = self.scroll_offset;
                self.scroll_offset -= last_scroll.elapsed().as_millis() as f64;
                self.clamp_scroll_offset();

                // Adjust tab position and touch offset.
                self.touch_state.start.y += self.scroll_offset - old_offset;
                self.shift_reordering_tab(engine_id);
            }

            self.last_reordering_scroll = Some(Instant::now());
            self.dirty = true;
        } else {
            self.last_reordering_scroll = None;
        }
    }

    /// Shift the tab being reordered to its target position.
    fn shift_reordering_tab(&mut self, engine_id: EngineId) {
        // Calculate number of tab positions the reordering tab needs to be moved by.
        let delta = self.touch_state.position.y - self.touch_state.start.y;
        let tab_height = self.tab_size().height as f64;
        let tabs_moved = (delta / tab_height).round() as isize;

        // Calculate tab's target index.
        let tabs = &self.texture_cache.tabs;
        let tab_index = match tabs.get_index_of(&engine_id) {
            Some(index) => index as isize,
            None => return,
        };
        let new_index = (tab_index + tabs_moved).clamp(0, tabs.len() as isize - 1);

        // If the tab moved by at least half a tab's height, move the tab's position and
        // placeholder to the new target location.
        if new_index != tab_index {
            self.queue.move_tab(engine_id, new_index as usize);
        }
    }
}

impl Popup for Tabs {
    fn dirty(&self) -> bool {
        self.dirty || self.group_label.dirty()
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn draw(&mut self, renderer: &Renderer) {
        self.dirty = false;

        // Don't render anything when hidden.
        if !self.visible {
            return;
        }

        // Ensure offset is correct in case tabs were closed or window size changed.
        self.clamp_scroll_offset();

        // Get geometry required for rendering.
        let cycle_group_button_position: Position<f32> = self.cycle_group_button_position().into();
        let persistent_button_position: Position<f32> = self.persistent_button_position().into();
        let new_group_button_position: Position<f32> = self.new_group_button_position().into();
        let downloads_button_position: Position<f32> = self.downloads_button_position().into();
        let new_tab_button_position: Position<f32> = self.new_tab_button_position().into();
        let history_button_position: Position<f32> = self.history_button_position().into();
        let group_label_position: Position<f32> = self.group_label_position().into();
        let tabs_start = group_label_position.y + self.group_label_size().height as f32;
        let favicon_size: Size<f32> = self.favicon_size().into();
        let tab_size = self.tab_size();

        // Get reordering tab and animate reordering scroll offset.
        let reordering_tab = match (self.touch_state.action, self.touch_state.slot) {
            (TouchAction::TabReordering(tab), Some(_)) => {
                self.add_reordering_scroll_offset(tab);
                Some(tab)
            },
            _ => {
                self.last_reordering_scroll = None;
                None
            },
        };
        let mut reordering_tab_position = Position::default();

        // Get UI textures.
        //
        // This must happen with the renderer bound to ensure new textures are
        // associated with the correct program.
        let tab_textures = self.texture_cache.textures(tab_size, self.scale, self.group);
        let cycle_group_button = self.cycle_group_button.texture();
        let close_group_button = self.close_group_button.texture();
        let persistent_button = self.persistent_button.texture();
        let new_group_button = self.new_group_button.texture();
        let downloads_button = self.downloads_button.texture();
        let new_tab_button = self.new_tab_button.texture();
        let history_button = self.history_button.texture();
        let group_label = self.group_label.texture();

        // Draw background.
        //
        // NOTE: This clears the entire surface, but works fine since the tabs popup
        // always fills the entire surface.
        let [r, g, b] = TABS_BG;
        unsafe {
            gl::ClearColor(r as f32, g as f32, b as f32, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
        }

        // Enable blending for favicons.
        unsafe {
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
        }

        // Draw individual tabs.
        let mut texture_pos = history_button_position;
        texture_pos.x += (TABS_X_PADDING * self.scale) as f32;
        texture_pos.y += self.scroll_offset as f32;
        for tab_textures in tab_textures {
            // Render only tabs within the viewport.
            texture_pos.y -= tab_textures.tab.height as f32;
            if texture_pos.y < new_tab_button_position.y
                && texture_pos.y > tabs_start - tab_textures.tab.height as f32
                && reordering_tab != Some(tab_textures.engine_id)
            {
                renderer.draw_texture_at(tab_textures.tab, texture_pos, None);

                if let Some(favicon_texture) = tab_textures.favicon {
                    // Center favicon within the tab.
                    let offset = (tab_textures.tab.height as f32 - favicon_size.height) / 2.;
                    let favicon_pos = texture_pos + Position::new(offset, offset);

                    renderer.draw_texture_at(favicon_texture, favicon_pos, favicon_size);
                }
            } else if reordering_tab == Some(tab_textures.engine_id) {
                reordering_tab_position = texture_pos;
            }

            // Add padding after the tab.
            texture_pos.y -= (TABS_Y_PADDING * self.scale) as f32
        }

        // Draw tab in the process of reordering.
        if let Some(textures) = reordering_tab.and_then(|tab| self.texture_cache.tab_textures(tab))
        {
            // Add drag offset to tab position.
            let start: Position<f32> = self.touch_state.start.into();
            let end: Position<f32> = self.touch_state.position.into();
            reordering_tab_position.y += end.y - start.y;

            // Ensure at least half the tab stays within tabs list.
            let half_height = (tab_size.height / 2) as f32;
            reordering_tab_position.y = reordering_tab_position
                .y
                .clamp(tabs_start - half_height, new_tab_button_position.y - half_height);

            // Downscale tab texture to distinguish it from the other tabs.
            let width = (textures.tab.width as f32 * REORDERING_SCALE).round();
            let height = (textures.tab.height as f32 * REORDERING_SCALE).round();
            let size = Size::new(width, height);
            reordering_tab_position.x += (textures.tab.width as f32 - width) / 2.;
            reordering_tab_position.y += (textures.tab.height as f32 - height) / 2.;

            renderer.draw_texture_at(textures.tab, reordering_tab_position, size);

            if let Some(favicon_texture) = textures.favicon {
                // Downscale favicon size.
                let favicon_size = favicon_size * REORDERING_SCALE;

                // Center favicon within the tab.
                let offset = (height - favicon_size.height) / 2.;
                let favicon_position = reordering_tab_position + Position::new(offset, offset);

                renderer.draw_texture_at(favicon_texture, favicon_position, favicon_size);
            }
        }

        // Draw tab group label.
        unsafe { gl::Disable(gl::BLEND) };

        renderer.draw_texture_at(group_label, group_label_position, None);

        // Draw buttons last, to render over scrolled tabs and label.
        renderer.draw_texture_at(new_tab_button, new_tab_button_position, None);
        renderer.draw_texture_at(cycle_group_button, cycle_group_button_position, None);
        if self.downloads_visible {
            renderer.draw_texture_at(downloads_button, downloads_button_position, None);
        }

        // Change new group to close group while editing the group label.
        if self.group_label.editing {
            renderer.draw_texture_at(close_group_button, new_group_button_position, None);
        } else {
            renderer.draw_texture_at(new_group_button, new_group_button_position, None);
        }

        // Show history button for default group, and persistency button for all others.
        if self.group == NO_GROUP_ID {
            renderer.draw_texture_at(history_button, history_button_position, None);
        } else {
            renderer.draw_texture_at(persistent_button, persistent_button_position, None);
        }
    }

    fn position(&self) -> Position {
        Position::new(0, 0)
    }

    fn set_size(&mut self, size: Size) {
        self.size = size;
        self.dirty = true;

        // Update UI element sizes.
        self.cycle_group_button.set_geometry(self.cycle_group_button_size(), self.scale);
        self.persistent_button.set_geometry(self.persistent_button_size(), self.scale);
        self.close_group_button.set_geometry(self.new_group_button_size(), self.scale);
        self.new_group_button.set_geometry(self.new_group_button_size(), self.scale);
        self.downloads_button.set_geometry(self.downloads_button_size(), self.scale);
        self.new_tab_button.set_geometry(self.new_tab_button_size(), self.scale);
        self.history_button.set_geometry(self.history_button_size(), self.scale);
        self.group_label.set_geometry(self.group_label_size(), self.scale);
        self.texture_cache.clear_textures();
    }

    fn size(&self) -> Size {
        self.size
    }

    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.dirty = true;

        // Update UI element scales.
        self.cycle_group_button.set_geometry(self.cycle_group_button_size(), self.scale);
        self.persistent_button.set_geometry(self.persistent_button_size(), self.scale);
        self.close_group_button.set_geometry(self.new_group_button_size(), self.scale);
        self.new_group_button.set_geometry(self.new_group_button_size(), self.scale);
        self.downloads_button.set_geometry(self.downloads_button_size(), self.scale);
        self.new_tab_button.set_geometry(self.new_tab_button_size(), self.scale);
        self.history_button.set_geometry(self.history_button_size(), self.scale);
        self.group_label.set_geometry(self.group_label_size(), self.scale);
        self.texture_cache.clear_textures();
    }

    fn opaque_region(&self) -> Size {
        self.size
    }

    fn press_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        if let Some(KeyboardInputElement::GroupLabel) = self.keyboard_focus {
            self.group_label.input.press_key(raw, keysym, modifiers)
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
        let cycle_group_button_position = self.cycle_group_button_position();
        let cycle_group_button_size = self.cycle_group_button_size().into();
        let new_group_button_position = self.new_group_button_position();
        let new_group_button_size = self.new_group_button_size().into();
        let downloads_button_position = self.downloads_button_position();
        let downloads_button_size = self.downloads_button_size().into();
        let new_tab_button_position = self.new_tab_button_position();
        let new_tab_button_size = self.new_tab_button_size().into();
        let history_button_position = self.history_button_position();
        let history_button_size = self.history_button_size().into();
        let group_label_position = self.group_label_position();
        let group_label_size = self.group_label_size().into();

        if rect_contains(cycle_group_button_position, cycle_group_button_size, position) {
            self.touch_state.action = TouchAction::CycleGroupTap;
            self.clear_keyboard_focus();
        } else if rect_contains(history_button_position, history_button_size, position) {
            if self.group == NO_GROUP_ID {
                self.touch_state.action = TouchAction::HistoryTap;
                self.clear_keyboard_focus();
            } else {
                self.touch_state.action = TouchAction::PersistentTap;
                self.clear_keyboard_focus();
            }
        } else if rect_contains(downloads_button_position, downloads_button_size, position) {
            self.touch_state.action = TouchAction::DownloadsTap;
            self.clear_keyboard_focus();
        } else if rect_contains(new_group_button_position, new_group_button_size, position) {
            // Close on new group button tap while editing the group label.
            //
            // This action must be set during the initial touch action since the `editing`
            // status will be cleared before the action is dispatched.
            if self.group_label.editing {
                self.touch_state.action = TouchAction::CloseGroupTap;
            } else {
                self.touch_state.action = TouchAction::NewGroupTap;
            }
            self.clear_keyboard_focus();
        } else if rect_contains(new_tab_button_position, new_tab_button_size, position) {
            self.touch_state.action = TouchAction::NewTabTap;
            self.clear_keyboard_focus();
        } else if rect_contains(group_label_position, group_label_size, position)
            && self.group != NO_GROUP_ID
        {
            self.group_label.touch_down(time, logical_position, position - group_label_position);
            self.touch_state.action = TouchAction::GroupLabelTouch;
            self.keyboard_focus = Some(KeyboardInputElement::GroupLabel);
        } else if let Some((engine_id, close)) = self.tab_at(position) {
            self.touch_state.action = TouchAction::TabTap(engine_id, close);
            self.clear_keyboard_focus();

            // Create timer for tab reordering.
            let mut queue = self.queue.clone();
            self.touch_state.stage_long_press_callback(move || {
                queue.start_tab_reordering(engine_id);
            });
        } else {
            self.touch_state.action = TouchAction::None;
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
            TouchAction::TabTap(..) | TouchAction::TabDrag => {
                // Ignore dragging until tap distance limit is exceeded.
                let delta = self.touch_state.position - self.touch_state.start;
                if delta.x.powi(2) + delta.y.powi(2) <= MAX_TAP_DISTANCE {
                    return;
                }
                self.touch_state.action = TouchAction::TabDrag;

                // Stop long-press timeout.
                self.touch_state.clear_long_press_timeout();

                // Immediately start moving the tabs list.
                let old_offset = self.scroll_offset;
                self.scroll_offset += self.touch_state.position.y - old_position.y;
                self.clamp_scroll_offset();
                self.dirty |= self.scroll_offset != old_offset;
            },
            // Handle tab drag & drop.
            TouchAction::TabReordering(engine_id) => {
                self.shift_reordering_tab(engine_id);
                self.dirty = true;
            },
            // Forward group label events.
            TouchAction::GroupLabelTouch => {
                let group_label_position = self.group_label_position();
                self.group_label.touch_motion(position - group_label_position);

                // Stop long-press timeout.
                self.touch_state.clear_long_press_timeout();
            },
            _ => {
                // Stop long-press timeout.
                self.touch_state.clear_long_press_timeout();
            },
        }
    }

    fn touch_up(&mut self, time: u32, id: i32, _modifiers: Modifiers) {
        // Ignore all unknown touch points.
        if self.touch_state.slot != Some(id) {
            return;
        }
        self.touch_state.slot = None;

        self.touch_state.clear_long_press_timeout();

        match self.touch_state.action {
            // Cycle through tab groups.
            TouchAction::CycleGroupTap => {
                let cycle_group_button_position = self.cycle_group_button_position();
                let cycle_group_button_size = self.cycle_group_button_size().into();
                let position = self.touch_state.position;

                if rect_contains(cycle_group_button_position, cycle_group_button_size, position) {
                    self.queue.cycle_tab_group(self.window_id, self.group);
                }
            },
            // Open browser history.
            TouchAction::HistoryTap => {
                let history_button_position = self.history_button_position();
                let history_button_size = self.history_button_size().into();
                let position = self.touch_state.position;

                if rect_contains(history_button_position, history_button_size, position) {
                    self.queue.show_history_ui(self.window_id);
                }
            },
            // Open downloads UI.
            TouchAction::DownloadsTap => {
                let downloads_button_position = self.downloads_button_position();
                let downloads_button_size = self.downloads_button_size().into();
                let position = self.touch_state.position;

                if rect_contains(downloads_button_position, downloads_button_size, position) {
                    self.queue.show_downloads_ui(self.window_id);
                }
            },
            // Toggle group's persistent mode.
            TouchAction::PersistentTap => {
                let persistent_button_position = self.persistent_button_position();
                let persistent_button_size = self.persistent_button_size().into();
                let position = self.touch_state.position;

                if rect_contains(persistent_button_position, persistent_button_size, position) {
                    let ephemeral = self.persistent_button.enabled;
                    self.queue.set_ephemeral_mode(self.window_id, self.group, ephemeral);
                }
            },
            // Create new tab group.
            TouchAction::NewGroupTap => {
                let new_group_button_position = self.new_group_button_position();
                let new_group_button_size = self.new_group_button_size().into();
                let position = self.touch_state.position;

                if rect_contains(new_group_button_position, new_group_button_size, position) {
                    self.queue.create_tab_group(self.window_id);
                }
            },
            // Create new tab group.
            TouchAction::CloseGroupTap => {
                let new_group_button_position = self.new_group_button_position();
                let new_group_button_size = self.new_group_button_size().into();
                let position = self.touch_state.position;

                if rect_contains(new_group_button_position, new_group_button_size, position) {
                    // Close tab group on new group button press while editing.
                    self.queue.delete_tab_group(self.window_id, self.group);
                }
            },
            // Open a new tab.
            TouchAction::NewTabTap => {
                let new_tab_button_position = self.new_tab_button_position();
                let new_tab_button_size = self.new_tab_button_size().into();
                let position = self.touch_state.position;

                if rect_contains(new_tab_button_position, new_tab_button_size, position) {
                    self.queue.add_tab(self.window_id, self.group);
                }
            },
            // Forward group label events.
            TouchAction::GroupLabelTouch => self.group_label.touch_up(time),
            // Switch tabs for tap actions on a tab.
            TouchAction::TabTap(engine_id, close) => {
                if close {
                    self.queue.close_tab(engine_id);
                } else {
                    self.queue.set_active_tab(engine_id);
                }
            },
            TouchAction::TabReordering(_) => self.dirty = true,
            TouchAction::TabDrag | TouchAction::None => (),
        }
    }

    fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32) {
        if let Some(KeyboardInputElement::GroupLabel) = self.keyboard_focus {
            self.group_label.input.delete_surrounding_text(before_length, after_length);
        }
    }

    fn commit_string(&mut self, text: &str) {
        if let Some(KeyboardInputElement::GroupLabel) = self.keyboard_focus {
            self.group_label.input.commit_string(text);
        }
    }

    fn set_preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32) {
        if let Some(KeyboardInputElement::GroupLabel) = self.keyboard_focus {
            self.group_label.input.set_preedit_string(text, cursor_begin, cursor_end);
        }
    }

    fn text_input_state(&mut self) -> TextInputChange {
        match self.keyboard_focus {
            Some(KeyboardInputElement::GroupLabel) => {
                let group_label_position = self.group_label_position();
                let x = group_label_position.x.round() as i32;
                let y = group_label_position.y.round() as i32;
                self.group_label.input.text_input_state(Position::new(x, y))
            },
            _ => TextInputChange::Disabled,
        }
    }

    fn paste(&mut self, text: &str) {
        if let Some(KeyboardInputElement::GroupLabel) = self.keyboard_focus {
            self.group_label.input.paste(text);
        }
    }

    fn has_keyboard_focus(&self) -> bool {
        self.keyboard_focus.is_some()
    }

    fn clear_keyboard_focus(&mut self) {
        // Automatically confirm input on focus loss.
        if self.group_label.editing {
            self.group_label.input.submit();
        }

        self.keyboard_focus = None;
    }
}

/// Tab texture cache by URI.
#[derive(Default)]
struct TextureCache {
    textures: HashMap<TabTextureCacheKey<'static>, Texture>,
    favicons: HashMap<glib::GString, Texture>,
    tabs: IndexMap<EngineId, RenderTab>,
}

impl TextureCache {
    /// Update the tabs tracked by this cache.
    fn set_tabs<'a, T>(&mut self, tabs: T, active_tab: Option<EngineId>)
    where
        T: Iterator<Item = &'a Box<dyn Engine>>,
    {
        self.tabs.clear();
        self.tabs.extend(tabs.map(|tab| {
            let engine_id = tab.id();
            let tab = RenderTab::new(tab.as_ref(), active_tab == Some(engine_id));
            (engine_id, tab)
        }));
    }

    /// Update the active tab for this cache.
    fn set_active_tab(&mut self, active_tab: Option<EngineId>) {
        for (engine_id, tab) in &mut self.tabs {
            tab.active = Some(*engine_id) == active_tab;
        }
    }

    /// Update a tab's load progress.
    ///
    /// Returns `true` if the load progress of a tab was updated.
    fn set_load_progress(&mut self, engine_id: EngineId, load_progress: f64) -> bool {
        match self.tabs.get_mut(&engine_id) {
            Some(tab) => {
                tab.load_progress = (load_progress * 100.).ceil() as u8;
                true
            },
            None => false,
        }
    }

    /// Update an existing engine's favicon.
    ///
    /// Returns `true` if a tab's favicon was reloaded.
    #[allow(clippy::borrowed_box)]
    fn update_favicon(&mut self, tab: &Box<dyn Engine>) -> bool {
        let render_tab = match self.tabs.get_mut(&tab.id()) {
            Some(render_tab) => render_tab,
            None => return false,
        };

        let resource_uri = match tab.favicon_uri() {
            Some(resource_uri) => resource_uri,
            None => return false,
        };

        // Update favicon unless it was already loaded.
        let changed = render_tab.favicon.as_ref().is_none_or(|f| f.resource_uri != resource_uri);
        if changed {
            render_tab.favicon = tab.favicon();
        }

        changed
    }

    /// Clear all cached textures.
    fn clear_textures(&mut self) {
        self.textures.clear();
    }

    /// Get all textures for the specified list of tabs.
    ///
    /// This will automatically maintain an internal cache to avoid re-drawing
    /// textures for tabs that have not changed.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn textures(
        &mut self,
        tab_size: Size,
        scale: f64,
        group: GroupId,
    ) -> impl Iterator<Item = TabTextures> {
        // Remove unused textures from cache.
        self.textures.retain(|cache_key, texture| {
            let retain = self.tabs.values().any(|tab| &tab.key() == cache_key);

            // Release OpenGL texture.
            if !retain {
                texture.delete();
            }

            retain
        });

        // Remove unused favicons from cache.
        self.favicons.retain(|resource_uri, texture| {
            let retain = self.tabs.values().any(|tab| {
                tab.favicon.as_ref().is_some_and(|favicon| &favicon.resource_uri == resource_uri)
            });

            // Release OpenGL texture.
            if !retain {
                texture.delete();
            }

            retain
        });

        // Create textures for missing tabs.
        for (_, tab) in group_tabs(&self.tabs, group) {
            // Create favicon texture.
            if let Some(favicon) = tab.favicon.as_ref() {
                if !self.favicons.contains_key(&favicon.resource_uri) {
                    // Add favicon to texture cache.
                    let texture = Texture::new_with_format(
                        &favicon.bytes,
                        favicon.width,
                        favicon.height,
                        gl::BGRA_EXT,
                    );
                    self.favicons.insert(favicon.resource_uri.clone(), texture);
                }
            }

            // Ignore tabs we already rendered.
            if self.textures.contains_key(&tab.key()) {
                continue;
            }

            // Create pango layout.
            let layout = TextLayout::new(FONT_SIZE, scale);

            // Fallback to URI if title is empty.
            layout.set_text(tab.label());

            // Configure text rendering options.
            let mut text_options = TextOptions::new();
            if tab.active {
                text_options.text_color(ACTIVE_TAB_FG);
            } else {
                text_options.text_color(INACTIVE_TAB_FG);
            }

            // Calculate spacing to the left of tab text.
            let close_position = Tabs::close_button_position(tab_size, scale);
            let x_offset =
                if tab.favicon.is_some() { tab_size.height as f64 } else { close_position.y };

            // Calculate available area font font rendering.
            let text_width = close_position.x - close_position.y - x_offset;
            let text_size = Size::new(text_width.round() as i32, tab_size.height as i32);
            text_options.position(Position::new(x_offset, 0.));
            text_options.size(text_size);

            // Render background with load progress indication.
            let builder = TextureBuilder::new(tab_size.into());
            let context = builder.context();
            builder.clear(NEW_TAB_BG);
            if tab.load_progress < 100 {
                let width = tab_size.width as f64 / 100. * tab.load_progress as f64;
                let [r, g, b, a] = PROGRESS_TAB_BG;

                context.rectangle(0., 0., width, tab_size.height as f64);
                context.set_source_rgba(r, g, b, a);
                context.fill().unwrap();
            }

            // Render text to the texture.
            builder.rasterize(&layout, &text_options);

            // Render close `X`.
            let size = Tabs::close_button_size(tab_size, scale);
            context.move_to(close_position.x, close_position.y);
            context.line_to(close_position.x + size.width, close_position.y + size.height);
            context.move_to(close_position.x + size.width, close_position.y);
            context.line_to(close_position.x, close_position.y + size.height);
            context.set_source_rgb(ACTIVE_TAB_FG[0], ACTIVE_TAB_FG[1], ACTIVE_TAB_FG[2]);
            context.set_line_width(scale);
            context.stroke().unwrap();

            self.textures.insert(tab.owned_key(), builder.build());
        }

        // Get textures for all tabs in reverse order.
        group_tabs(&self.tabs, group).rev().map(|(engine_id, tab)| {
            TabTextures::new(&self.textures, &self.favicons, *engine_id, tab)
        })
    }

    /// Get the texture for one specific tab.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn tab_textures(&mut self, engine_id: EngineId) -> Option<TabTextures> {
        let tab = self.tabs.get(&engine_id)?;
        Some(TabTextures::new(&self.textures, &self.favicons, engine_id, tab))
    }
}

/// Textures required for rendering a tab.
struct TabTextures<'a> {
    engine_id: EngineId,
    tab: &'a Texture,
    favicon: Option<&'a Texture>,
}

impl<'a> TabTextures<'a> {
    /// Get the textures for the tab.
    ///
    /// # Panics
    ///
    /// Panics if the tab's main texture doesn't exist.
    fn new(
        tabs: &'a HashMap<TabTextureCacheKey<'static>, Texture>,
        favicons: &'a HashMap<glib::GString, Texture>,
        engine_id: EngineId,
        render_tab: &'a RenderTab,
    ) -> Self {
        let favicon = render_tab.favicon.as_ref().and_then(|f| favicons.get(&*f.resource_uri));
        let tab = tabs.get(&render_tab.key()).unwrap();
        Self { engine_id, favicon, tab }
    }
}

/// Information required to render a tab.
#[derive(Debug)]
struct RenderTab {
    uri: String,
    title: String,
    active: bool,
    favicon: Option<Favicon>,
    load_progress: u8,
}

impl RenderTab {
    fn new(engine: &dyn Engine, active: bool) -> Self {
        Self {
            active,
            title: engine.title().into(),
            favicon: engine.favicon(),
            uri: engine.uri().into(),
            load_progress: 100,
        }
    }

    /// Get the tab's rendered text.
    fn label(&self) -> &str {
        if self.title.trim().is_empty() { &self.uri } else { &self.title }
    }

    /// Get a borrowed texture cache key.
    fn key<'a>(&'a self) -> TabTextureCacheKey<'a> {
        TabTextureCacheKey {
            favicon_uri: self.favicon.as_ref().map(|f| Cow::Borrowed(&f.resource_uri)),
            label: Cow::Borrowed(self.label()),
            load_progress: self.load_progress,
            active: self.active,
        }
    }

    /// Get an owned texture cache key.
    fn owned_key(&self) -> TabTextureCacheKey<'static> {
        TabTextureCacheKey {
            favicon_uri: self.favicon.as_ref().map(|f| Cow::Owned(f.resource_uri.clone())),
            label: Cow::Owned(self.label().to_string()),
            load_progress: self.load_progress,
            active: self.active,
        }
    }
}

/// Indexing key for the tab texture cache.
#[derive(Hash, PartialEq, Eq, Debug)]
struct TabTextureCacheKey<'a> {
    favicon_uri: Option<Cow<'a, glib::GString>>,
    label: Cow<'a, str>,
    load_progress: u8,
    active: bool,
}

/// Button with a `+` as icon.
struct PlusButton {
    texture: Option<Texture>,
    dirty: bool,
    size: Size,
    scale: f64,
}

impl Default for PlusButton {
    fn default() -> Self {
        Self { dirty: true, scale: 1., texture: Default::default(), size: Default::default() }
    }
}

impl PlusButton {
    fn texture(&mut self) -> &Texture {
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
    fn draw(&self) -> Texture {
        // Clear with background color.
        let builder = TextureBuilder::new(self.size.into());
        builder.clear(TABS_BG);

        // Draw button background.
        let x_padding = BUTTON_X_PADDING * self.scale;
        let y_padding = BUTTON_Y_PADDING * self.scale;
        let width = self.size.width as f64 - 2. * x_padding;
        let height = self.size.height as f64 - 2. * y_padding;
        builder.context().rectangle(x_padding, y_padding, width.round(), height.round());
        builder.context().set_source_rgb(NEW_TAB_BG[0], NEW_TAB_BG[1], NEW_TAB_BG[2]);
        builder.context().fill().unwrap();

        // Set general stroke properties.
        let icon_size = height * 0.5;
        let line_width = self.scale;
        let center_x = self.size.width as f64 / 2.;
        let center_y = self.size.height as f64 / 2.;
        builder.context().set_source_rgb(ACTIVE_TAB_FG[0], ACTIVE_TAB_FG[1], ACTIVE_TAB_FG[2]);
        builder.context().set_line_width(line_width);

        // Draw vertical line of `+`.
        let start_y = center_y - icon_size / 2.;
        let end_y = center_y + icon_size / 2.;
        builder.context().move_to(center_x, start_y);
        builder.context().line_to(center_x, end_y);
        builder.context().stroke().unwrap();

        // Draw horizontal line of `+`.
        let start_x = center_x - icon_size / 2.;
        let end_x = center_x + icon_size / 2.;
        builder.context().move_to(start_x, center_y);
        builder.context().line_to(end_x, center_y);
        builder.context().stroke().unwrap();

        builder.build()
    }

    /// Set the physical size and scale of the button.
    fn set_geometry(&mut self, size: Size, scale: f64) {
        self.size = size;
        self.scale = scale;

        // Force redraw.
        self.dirty = true;
    }
}

/// Active tab group label.
struct GroupLabel {
    texture: Option<Texture>,
    text: Cow<'static, str>,

    input: TextField,
    editing: bool,

    size: Size,
    scale: f64,

    dirty: bool,
}

impl GroupLabel {
    fn new(window_id: WindowId, mut queue: MtQueueHandle<State>) -> Self {
        let mut input = TextField::new(window_id, queue.clone(), FONT_SIZE);
        input.set_submit_handler(Box::new(move |label| queue.update_group_label(window_id, label)));

        Self {
            input,
            text: NO_GROUP.label,
            dirty: true,
            scale: 1.,
            editing: Default::default(),
            texture: Default::default(),
            size: Default::default(),
        }
    }

    fn texture(&mut self) -> &Texture {
        // Ensure texture is up to date.
        if self.dirty() {
            // Ensure texture is cleared while program is bound.
            if let Some(texture) = self.texture.take() {
                texture.delete();
            }
            self.texture = Some(self.draw());

            self.input.dirty = false;
            self.dirty = false;
        }

        self.texture.as_ref().unwrap()
    }

    /// Draw the label into an OpenGL texture.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn draw(&mut self) -> Texture {
        // Clear with background color.
        let builder = TextureBuilder::new(self.size.into());
        builder.clear(TABS_BG);

        // Render group label text.
        if self.editing {
            // Get text position with scroll offset applied.
            let (mut position, size) = self.text_geometry();
            position.x += self.input.scroll_offset;

            // Set text rendering options.
            let mut text_options = TextOptions::new();
            text_options.cursor_position(self.input.cursor_index());
            text_options.autocomplete(self.input.autocomplete().into());
            text_options.preedit(self.input.preedit.clone());
            text_options.position(position);
            text_options.size(size.into());
            text_options.set_ellipsize(false);

            // Show cursor or selection when focused.
            if self.input.focused {
                if self.input.selection.is_some() {
                    text_options.selection(self.input.selection.clone());
                } else {
                    text_options.show_cursor();
                }
            }

            // Rasterize the text field.
            let layout = self.input.layout();
            layout.set_scale(self.scale);
            builder.rasterize(layout, &text_options);
        } else if !self.text.is_empty() {
            let layout = TextLayout::new(FONT_SIZE, self.scale);
            layout.set_alignment(Alignment::Center);
            layout.set_text(&self.text);

            // Truncate label to be within persistence/new group buttons.
            let (position, size) = self.text_geometry();
            let mut text_options = TextOptions::new();
            text_options.position(position);
            text_options.size(size.into());

            builder.rasterize(&layout, &text_options);
        }

        builder.build()
    }

    /// Set the physical size and scale of the button.
    fn set_geometry(&mut self, size: Size, scale: f64) {
        self.size = size;
        self.scale = scale;

        // Update text input width.
        let (_, text_size) = self.text_geometry();
        self.input.set_width(text_size.width as f64);

        // Force redraw.
        self.dirty = true;
    }

    /// Set the active tab group label.
    fn set(&mut self, text: Cow<'static, str>) {
        self.editing = false;
        self.text = text;

        self.dirty = true;
    }

    /// Leave text input view.
    fn stop_editing(&mut self) {
        self.editing = false;
        self.dirty = true;
    }

    // Check if the group label requires a redraw.
    fn dirty(&self) -> bool {
        self.dirty || (self.editing && self.input.dirty)
    }

    /// Handle touch press events.
    pub fn touch_down(
        &mut self,
        time: u32,
        absolute_position: Position<f64>,
        position: Position<f64>,
    ) {
        if !self.editing {
            return;
        }

        // Forward event to text field.
        let (text_position, _) = self.text_geometry();
        self.input.touch_down(time, absolute_position, position - text_position);
    }

    /// Handle touch motion events.
    pub fn touch_motion(&mut self, position: Position<f64>) {
        if !self.editing {
            return;
        }

        // Forward event to text field.
        let (text_position, _) = self.text_geometry();
        self.input.touch_motion(position - text_position);
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, time: u32) {
        // Enable editing on touch release.
        if !self.editing {
            self.input.set_text(&self.text);
            self.input.set_focus(true);
            self.editing = true;
            return;
        }

        // Forward event to text field.
        self.input.touch_up(time);
    }

    /// Get physical geometry of the text input area.
    fn text_geometry(&self) -> (Position<f64>, Size) {
        let text_padding = (BUTTON_X_PADDING * self.scale).round() as u32;
        let button_width = Tabs::button_size(self.scale).width + text_padding;
        let width = self.size.width - button_width * 2;

        let position = Position::new(button_width as f64, 0.);
        let size = Size::new(width, self.size.height);

        (position, size)
    }
}

/// Touch event tracking.
#[derive(Default)]
struct TouchState {
    slot: Option<i32>,
    action: TouchAction,
    start: Position<f64>,
    position: Position<f64>,
    long_press_source: Option<Source>,
}

impl TouchState {
    /// Set a new callback to be executed once the long-press timeout elapses.
    fn stage_long_press_callback<F>(&mut self, mut callback: F)
    where
        F: FnMut() + Send + 'static,
    {
        // Clear old timout.
        self.clear_long_press_timeout();

        // Stage new timeout callback.
        let delay = Duration::from_millis(LONG_PRESS_MILLIS as u64);
        let source = source::timeout_source_new(delay, None, Priority::DEFAULT, move || {
            callback();
            ControlFlow::Break
        });
        source.attach(None);

        self.long_press_source = Some(source);
    }

    /// Cancel active long-press timers.
    fn clear_long_press_timeout(&mut self) {
        if let Some(source) = self.long_press_source.take() {
            source.destroy();
        }
    }
}

/// Intention of a touch sequence.
#[derive(Default, Copy, Clone, PartialEq, Eq, Debug)]
enum TouchAction {
    #[default]
    None,
    TabTap(EngineId, bool),
    TabReordering(EngineId),
    TabDrag,
    HistoryTap,
    NewTabTap,
    NewGroupTap,
    CloseGroupTap,
    PersistentTap,
    CycleGroupTap,
    GroupLabelTouch,
    DownloadsTap,
}

/// Elements accepting keyboard focus.
enum KeyboardInputElement {
    GroupLabel,
}

/// Get iterator over tabs in a tab group.
fn group_tabs(
    tabs: &IndexMap<EngineId, RenderTab>,
    group: GroupId,
) -> impl DoubleEndedIterator<Item = (&EngineId, &RenderTab)> {
    tabs.iter().filter(move |(engine_id, _)| engine_id.group_id() == group)
}
