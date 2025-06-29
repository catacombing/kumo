//! Menu overlay, for opening other overlays.

use std::collections::HashMap;
use std::mem;

use funq::MtQueueHandle;
use smithay_client_toolkit::seat::keyboard::Modifiers;

use crate::config::{CONFIG, Config};
use crate::ui::overlay::Popup;
use crate::ui::renderer::{Renderer, Svg, TextLayout, TextOptions, Texture, TextureBuilder};
use crate::ui::{ScrollVelocity, SvgButton};
use crate::window::WindowId;
use crate::{Position, Size, State, gl, rect_contains};

/// Logical height of the UI buttons.
const BUTTON_HEIGHT: u32 = 60;

/// Padding around buttons.
const BUTTON_PADDING: f64 = 10.;

/// Logical height of each entry.
const ENTRY_HEIGHT: u32 = 65;

/// Horizontal tabbing around entries.
const ENTRY_X_PADDING: f64 = 10.;

/// Vertical padding between entries.
const ENTRY_Y_PADDING: f64 = 1.;

/// Menu item SVG icon width and height at scale 1.
const ICON_SIZE: f64 = 28.;

#[funq::callbacks(State)]
trait MenuHandler {
    /// Close the menu overlay.
    fn close_menu(&mut self, window_id: WindowId);

    /// Open history UI.
    fn show_history_ui(&mut self, window_id: WindowId);

    /// Open downloads UI.
    fn show_downloads_ui(&mut self, window_id: WindowId);

    /// Open settings UI.
    fn show_settings_ui(&mut self, window_id: WindowId);
}

impl MenuHandler for State {
    fn close_menu(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };
        window.set_menu_ui_visible(false);
    }

    fn show_history_ui(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };
        window.set_history_ui_visible(true);
    }

    fn show_downloads_ui(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };
        window.set_downloads_ui_visible(true);
    }

    fn show_settings_ui(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };
        window.set_settings_ui_visible(true);
    }
}

/// Menu UI.
pub struct Menu {
    texture_cache: TextureCache,
    close_button: SvgButton,
    scroll_offset: f64,

    touch_state: TouchState,
    velocity: ScrollVelocity,

    size: Size,
    scale: f64,

    queue: MtQueueHandle<State>,
    window_id: WindowId,

    download_count: usize,

    last_config: u32,
    visible: bool,
    dirty: bool,
}

impl Menu {
    pub fn new(window_id: WindowId, queue: MtQueueHandle<State>) -> Self {
        Self {
            window_id,
            queue,
            close_button: SvgButton::new(Svg::Close),
            scale: 1.,
            download_count: Default::default(),
            texture_cache: Default::default(),
            scroll_offset: Default::default(),
            last_config: Default::default(),
            touch_state: Default::default(),
            velocity: Default::default(),
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
    }

    /// Update the number of tracked downloads.
    pub fn set_download_count(&mut self, download_count: usize) {
        self.dirty |= self.download_count != download_count;
        self.download_count = download_count;
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

    /// Physical size of each entry.
    fn entry_size(&self) -> Size {
        let width = self.size.width - (2. * ENTRY_X_PADDING).round() as u32;
        Size::new(width, ENTRY_HEIGHT) * self.scale
    }

    /// Get entry at the specified location.
    fn entry_at(&self, mut position: Position<f64>) -> Option<MenuItem> {
        let y_padding = ENTRY_Y_PADDING * self.scale;
        let x_padding = ENTRY_X_PADDING * self.scale;
        let entry_end_y = self.close_button_position().y;

        let entry_size_int = self.entry_size();
        let entry_size: Size<f64> = entry_size_int.into();

        // Check whether position is within list's boundaries.
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

        // Find entry at the specified offset.
        let items = MenuItem::items();
        let rindex = (bottom_relative / (entry_size.height + y_padding).round()) as usize;
        let index = items.len() - 1 - rindex;
        let entry = items.get(index)?;

        Some(*entry)
    }

    /// Clamp list's viewport offset.
    fn clamp_scroll_offset(&mut self) {
        let old_offset = self.scroll_offset;
        let max_offset = self.max_scroll_offset() as f64;
        self.scroll_offset = self.scroll_offset.clamp(0., max_offset);

        // Cancel velocity after reaching the scroll limit.
        if old_offset != self.scroll_offset {
            self.velocity.set(0.);
            self.dirty = true;
        }
    }

    /// Get maximum scroll offset.
    fn max_scroll_offset(&self) -> usize {
        let entry_padding = (ENTRY_Y_PADDING * self.scale).round() as usize;
        let entry_height = self.entry_size().height;

        // Calculate height available for entries.
        let ui_height = (self.size.height as f64 * self.scale).round() as usize;
        let close_button_height = self.button_size().height as usize;
        let available_height = ui_height - close_button_height;

        // Calculate height of all entries.
        let num_entries = MenuItem::items().len();
        let mut entries_height =
            (num_entries * (entry_height as usize + entry_padding)).saturating_sub(entry_padding);

        // Allow a bit of padding at the top.
        let top_padding = (BUTTON_PADDING * self.scale).round();
        entries_height += top_padding as usize;

        // Calculate content outside the viewport.
        entries_height.saturating_sub(available_height)
    }
}

impl Popup for Menu {
    fn dirty(&self) -> bool {
        self.dirty
            || self.velocity.is_moving()
            || CONFIG.read().unwrap().generation != self.last_config
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn draw(&mut self, renderer: &Renderer) {
        self.dirty = false;

        // Don't render anything when hidden.
        if !self.visible {
            return;
        }

        // Animate scroll velocity.
        self.velocity.apply(&mut self.scroll_offset);

        // Ensure offset is correct in case entries or window size changed.
        self.clamp_scroll_offset();

        // Update config version ID.
        let config = CONFIG.read().unwrap();
        if self.last_config != config.generation {
            self.last_config = config.generation;

            // Clear texture cache to handle color changes.
            for (_, texture) in self.texture_cache.textures.drain() {
                texture.delete();
            }
            self.close_button.dirty = true;
        }

        // Get geometry required for rendering.
        let x_padding = (ENTRY_X_PADDING * self.scale) as f32;
        let close_button_position: Position<f32> = self.close_button_position().into();
        let ui_height = (self.size.height as f64 * self.scale).round() as f32;
        let button_height = self.button_size().height as i32;
        let entry_size = self.entry_size();

        // Get UI textures.
        //
        // This must happen with the renderer bound to ensure new textures are
        // associated with the correct program.
        let close_button = self.close_button.texture();

        // Draw background.
        //
        // NOTE: This clears the entire surface, but works fine since the popup always
        // fills the entire surface.
        let [r, g, b] = config.colors.background.as_f32();
        unsafe {
            gl::ClearColor(r, g, b, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
        }

        // Scissor crop bottom entry, to not overlap the buttons.
        unsafe {
            gl::Enable(gl::SCISSOR_TEST);
            gl::Scissor(0, button_height, i32::MAX, ui_height as i32);
        }

        // Draw menu list.
        let mut texture_pos =
            Position::new(x_padding, close_button_position.y + self.scroll_offset as f32);
        for item in MenuItem::items().into_iter().rev() {
            // Render only entries within the viewport.
            texture_pos.y -= entry_size.height as f32;
            if texture_pos.y <= -(entry_size.height as f32) {
                break;
            } else if texture_pos.y < close_button_position.y {
                let texture = self.texture_cache.texture(
                    &config,
                    item,
                    entry_size,
                    self.scale,
                    self.download_count,
                );
                renderer.draw_texture_at(texture, texture_pos, None);
            }

            // Add padding after the entry.
            texture_pos.y -= (ENTRY_Y_PADDING * self.scale) as f32
        }

        unsafe { gl::Disable(gl::SCISSOR_TEST) };

        // Draw buttons.
        renderer.draw_texture_at(close_button, close_button_position, None);
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

    fn touch_down(
        &mut self,
        _time: u32,
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
                let max_tap_distance = CONFIG.read().unwrap().input.max_tap_distance;
                let delta = self.touch_state.position - self.touch_state.start;
                if delta.x.powi(2) + delta.y.powi(2) <= max_tap_distance {
                    return;
                }
                self.touch_state.action = TouchAction::EntryDrag;

                // Calculate current scroll velocity.
                let delta = self.touch_state.position.y - old_position.y;
                self.velocity.set(delta);

                // Immediately start moving the entries.
                let old_offset = self.scroll_offset;
                self.scroll_offset += delta;
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
                Some(MenuItem::Downloads) => self.queue.show_downloads_ui(self.window_id),
                Some(MenuItem::Settings) => self.queue.show_settings_ui(self.window_id),
                Some(MenuItem::History) => self.queue.show_history_ui(self.window_id),
                None => (),
            },
            TouchAction::CloseTap => self.queue.close_menu(self.window_id),
            TouchAction::EntryDrag => (),
        }
    }
}

/// Menu entry texture cache.
#[derive(Default)]
struct TextureCache {
    textures: HashMap<MenuItem, Texture>,
    download_count: usize,
    entry_size: Size,
    scale: f64,
}

impl TextureCache {
    /// Get the texture for a menu entry.
    ///
    /// This will automatically take care of caching rendered textures.
    ///
    /// ### Panics
    ///
    /// Panics if `index >= self.len()`.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn texture(
        &mut self,
        config: &Config,
        item: MenuItem,
        entry_size: Size,
        scale: f64,
        download_count: usize,
    ) -> &Texture {
        // Clear cache if redraw is required.
        if entry_size != self.entry_size || scale != self.scale {
            for (_, texture) in self.textures.drain() {
                texture.delete();
            }
            self.entry_size = entry_size;
            self.scale = scale;
        }

        // Clear downloads texture on count change.
        if self.download_count != download_count {
            self.textures.remove(&MenuItem::Downloads);
            self.download_count = download_count;
        }

        // Create and cache texture if necessary.
        self.textures.entry(item).or_insert_with(|| {
            // Create cleared canvas.
            let builder = TextureBuilder::new(entry_size.into());
            builder.clear(config.colors.alt_background.as_f64());

            // Draw menu item icon.
            let icon_size = (ICON_SIZE * scale).round();
            let padding = ((entry_size.height as f64 - icon_size) / 2.).round();
            builder.rasterize_svg(item.svg(), padding, padding, icon_size, icon_size);

            // Configure menu item label geometry.
            let mut text_options = TextOptions::new();
            let label_x = 2. * padding + icon_size;
            let label_width = entry_size.width - label_x as u32 - padding as u32;
            text_options.position(Position::new(label_x, 0.));
            text_options.size(Size::new(label_width as i32, entry_size.height as i32));

            // Render menu item label.
            let layout = TextLayout::new(config.font.size(1.25), scale);
            match item {
                MenuItem::Downloads => {
                    let label = format!("{} ({})", item.label(), download_count);
                    layout.set_text(&label);
                },
                _ => layout.set_text(item.label()),
            }
            builder.rasterize(&layout, &text_options);

            builder.build()
        })
    }
}

/// Entries in the menu.
#[derive(Copy, Clone, Hash, PartialEq, Eq)]
enum MenuItem {
    Downloads,
    Settings,
    History,
}

impl MenuItem {
    /// Get all available menu items.
    const fn items() -> [Self; 3] {
        [Self::Downloads, Self::Settings, Self::History]
    }

    /// Get the menu item's entry text.
    const fn label(&self) -> &'static str {
        match self {
            Self::Downloads => "Downloads",
            Self::Settings => "Settings",
            Self::History => "History",
        }
    }

    /// Get SVG associated with this item.
    const fn svg(&self) -> Svg {
        match self {
            Self::Downloads => Svg::Download,
            Self::Settings => Svg::Settings,
            Self::History => Svg::History,
        }
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
