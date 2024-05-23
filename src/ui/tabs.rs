//! Tabs UI.

use std::collections::HashMap;
use std::mem;

use funq::MtQueueHandle;
use glutin::display::Display;
use pangocairo::cairo::{Context, Format, ImageSurface};
use smithay_client_toolkit::compositor::{CompositorState, Region};
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::seat::keyboard::Modifiers;

use crate::engine::{Engine, EngineId};
use crate::ui::renderer::{Renderer, TextOptions, Texture, TextureBuilder};
use crate::{gl, rect_contains, Position, Size, State, WindowId};

/// Tab text color of active tab.
const ACTIVE_TAB_FG: [f64; 3] = [1., 1., 1.];
/// Tab text color of inactive tabs.
const INACTIVE_TAB_FG: [f64; 3] = [0.8, 0.8, 0.8];
/// Tab view background color.
const TABS_BG: [f64; 3] = [0.09, 0.09, 0.09];
/// New tab button background color.
const NEW_TAB_BG: [f64; 3] = [0.15, 0.15, 0.15];

/// Tab font size.
const FONT_SIZE: u8 = 20;

/// Horizontal tabbing around tabs.
const TABS_X_PADDING: f64 = 10.;

/// Vertical padding between tabs.
const TABS_Y_PADDING: f64 = 1.;

/// Horizontal padding around "New Tab" button.
const NEW_TAB_X_PADDING: f64 = 10.;

/// Vertical padding around "New Tab" button.
const NEW_TAB_Y_PADDING: f64 = 10.;

/// Padding around the tab "X" button.
const CLOSE_PADDING: f64 = 30.;

/// Logical height of each tab.
const TAB_HEIGHT: u32 = 50;

/// Logical height of the "New Tab" button.
const NEW_TAB_BUTTON_HEIGHT: u32 = 60;

/// Size of the "New Tab" button `+` icon.
const NEW_TAB_ICON_SIZE: f64 = 30.;

/// Square of the maximum distance before touch input is considered a drag.
const MAX_TAP_DISTANCE: f64 = 400.;

#[funq::callbacks(State)]
trait TabsUiHandler {
    /// Create a new tab and switch to it.
    fn add_tab(&mut self, window: WindowId);

    /// Switch tabs.
    fn set_active_tab(&mut self, engine_id: EngineId);

    /// Close a tab.
    fn close_tab(&mut self, engine_id: EngineId);
}

impl TabsUiHandler for State {
    fn add_tab(&mut self, window_id: WindowId) {
        let window = match self.windows.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };

        // Explicitly mark window as dirty since the tabs UI closed.
        window.mark_dirty();

        let _ = window.add_tab(true);
    }

    fn set_active_tab(&mut self, engine_id: EngineId) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };

        // Explicitly mark window as dirty since the tabs UI closed.
        window.mark_dirty();

        window.set_active_tab(engine_id);
    }

    fn close_tab(&mut self, engine_id: EngineId) {
        let window = match self.windows.get_mut(&engine_id.window_id()) {
            Some(window) => window,
            None => return,
        };

        window.close_tab(engine_id);
    }
}

/// Tab overview/creation UI.
pub struct TabsUi {
    texture_cache: TextureCache,
    renderer: Renderer,
    scroll_offset: f64,

    surface: WlSurface,
    viewport: WpViewport,

    size: Size,
    scale: f64,

    queue: MtQueueHandle<State>,
    window_id: WindowId,

    new_tab_button: NewTabButton,

    touch_state: TouchState,

    visible: bool,
    dirty: bool,
}

impl TabsUi {
    pub fn new(
        window_id: WindowId,
        queue: MtQueueHandle<State>,
        display: Display,
        surface: WlSurface,
        viewport: WpViewport,
    ) -> Self {
        let renderer = Renderer::new(display, surface.clone());

        Self {
            window_id,
            viewport,
            renderer,
            surface,
            queue,
            scale: 1.0,
            new_tab_button: Default::default(),
            texture_cache: Default::default(),
            scroll_offset: Default::default(),
            touch_state: Default::default(),
            visible: Default::default(),
            dirty: Default::default(),
            size: Default::default(),
        }
    }

    /// Update the surface size.
    pub fn set_size(&mut self, compositor: &CompositorState, size: Size) {
        self.size = size;
        self.dirty = true;

        // Update opaque region.
        if let Ok(region) = Region::new(compositor) {
            region.add(0, 0, size.width as i32, size.height as i32);
            self.surface.set_opaque_region(Some(region.wl_region()));
        }

        // Update UI element sizes.
        self.new_tab_button.set_geometry(self.new_tab_button_size(), self.scale);
        self.texture_cache.clear();
    }

    /// Update the render scale.
    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.dirty = true;

        // Update UI element scales.
        self.new_tab_button.set_geometry(self.new_tab_button_size(), self.scale);
        self.texture_cache.clear();
    }

    /// Render current tabs UI state.
    ///
    /// Returns `true` if rendering was performed.
    pub fn draw<'a, T>(&mut self, tabs: T, active_tab: EngineId) -> bool
    where
        T: Iterator<Item = &'a Box<dyn Engine>>,
    {
        // Ensure offset is correct in case tabs were closed or window size changed.
        self.clamp_scroll_offset();

        // Abort early if UI is up to date.
        if !self.dirty {
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

        // Get geometry required for rendering.
        let new_tab_button_position: Position<f32> = self.new_tab_button_position().into();
        let tab_size = self.tab_size();

        // Render the tabs UI.
        let physical_size = self.size * self.scale;
        self.renderer.draw(physical_size, |renderer| {
            // Get textures for all tabs.
            //
            // This must happen with the renderer bound to ensure new textures are
            // associated with the correct program.
            let tab_textures = self.texture_cache.textures(tab_size, self.scale, tabs, active_tab);

            // Get "New Tab" button texture.
            let new_tab_button = self.new_tab_button.texture();

            unsafe {
                // Draw background.
                let [r, g, b] = TABS_BG;
                gl::ClearColor(r as f32, g as f32, b as f32, 1.0);
                gl::Clear(gl::COLOR_BUFFER_BIT);

                // Draw individual tabs.
                let mut texture_pos = new_tab_button_position;
                texture_pos.x += (TABS_X_PADDING * self.scale) as f32;
                texture_pos.y += self.scroll_offset as f32;
                for texture in tab_textures {
                    // Render only tabs within the viewport.
                    texture_pos.y -= texture.height as f32;
                    if texture_pos.y < new_tab_button_position.y
                        && texture_pos.y > -1. * texture.height as f32
                    {
                        renderer.draw_texture_at(texture, texture_pos, None);
                    }

                    // Add padding after the tab.
                    texture_pos.y -= (TABS_Y_PADDING * self.scale) as f32
                }

                // Draw "New Tab" button, last, to render over scrolled tabs.
                texture_pos = new_tab_button_position;
                renderer.draw_texture_at(new_tab_button, texture_pos, None);
            }
        });

        true
    }

    /// Force tabs UI redraw.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Handle touch press events.
    pub fn touch_down(
        &mut self,
        _time: u32,
        id: i32,
        position: Position<f64>,
        _modifiers: Modifiers,
    ) {
        // Only accept a single touch point in the UI.
        if self.touch_state.slot.is_some() {
            return;
        }
        self.touch_state.slot = Some(id);

        // Convert position to physical space.
        let position = position * self.scale;
        self.touch_state.position = position;
        self.touch_state.start = position;

        // Get new tab button geometry.
        let new_tab_button_position = self.new_tab_button_position();
        let new_tab_button_size = self.new_tab_button_size().into();

        if rect_contains(new_tab_button_position, new_tab_button_size, position) {
            self.touch_state.action = TouchAction::NewTabTap;
        } else {
            self.touch_state.action = TouchAction::TabTap;
        }
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
        if self.touch_state.slot != Some(id) {
            return;
        }

        // Update touch position.
        let position = position * self.scale;
        let old_position = mem::replace(&mut self.touch_state.position, position);

        // Ignore drag when tap started on "New Tab" button.
        if self.touch_state.action == TouchAction::NewTabTap {
            return;
        }

        // Switch to dragging once tap distance limit is exceeded.
        let delta = self.touch_state.position - self.touch_state.start;
        if delta.x.powi(2) + delta.y.powi(2) > MAX_TAP_DISTANCE {
            self.touch_state.action = TouchAction::TabDrag;

            // Immediately start moving the tabs list.
            let old_offset = self.scroll_offset;
            self.scroll_offset += self.touch_state.position.y - old_position.y;
            self.clamp_scroll_offset();
            self.dirty |= self.scroll_offset != old_offset;
        }
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, _time: u32, id: i32, _modifiers: Modifiers) {
        // Ignore all unknown touch points.
        if self.touch_state.slot != Some(id) {
            return;
        }
        self.touch_state.slot = None;

        match self.touch_state.action {
            // Open a new tab.
            TouchAction::NewTabTap => {
                let new_tab_button_position = self.new_tab_button_position();
                let new_tab_button_size = self.new_tab_button_size().into();
                let position = self.touch_state.position;

                if rect_contains(new_tab_button_position, new_tab_button_size, position) {
                    self.hide();
                    self.queue.add_tab(self.window_id);
                }
            },
            // Switch tabs for tap actions on a tab.
            TouchAction::TabTap => {
                if let Some((&RenderTab { engine, .. }, close)) =
                    self.tab_at(self.touch_state.start)
                {
                    if close {
                        self.queue.close_tab(engine);
                    } else {
                        self.hide();
                        self.queue.set_active_tab(engine);
                    }
                }
            },
            TouchAction::TabDrag => (),
        }
    }

    /// Show the tabs UI.
    pub fn show(&mut self) {
        self.dirty |= !self.visible;
        self.visible = true;
    }

    /// Hide the tabs UI.
    fn hide(&mut self) {
        self.surface.attach(None, 0, 0);
        self.surface.commit();

        self.visible = false;
    }

    /// Check tabs UI visibility.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Get underlying Wayland surface.
    pub fn surface(&self) -> &WlSurface {
        &self.surface
    }

    /// Check whether tabs UI needs redraw.
    pub fn dirty(&self) -> bool {
        self.visible && self.dirty
    }

    /// Physical size of the "New Tab" button bar.
    ///
    /// This includes all padding since that is included in the texture.
    fn new_tab_button_size(&self) -> Size {
        let height = NEW_TAB_BUTTON_HEIGHT + (2. * NEW_TAB_Y_PADDING).round() as u32;
        Size::new(self.size.width, height) * self.scale
    }

    /// Physical position of the "New Tab" button.
    ///
    /// This includes all padding since that is included in the texture.
    fn new_tab_button_position(&self) -> Position<f64> {
        let y = (self.size.height - NEW_TAB_BUTTON_HEIGHT) as f64 - 2. * NEW_TAB_Y_PADDING;
        Position::new(0., y) * self.scale
    }

    /// Physical size of each tab.
    fn tab_size(&self) -> Size {
        let width = self.size.width - (2. * TABS_X_PADDING).round() as u32;
        Size::new(width, TAB_HEIGHT) * self.scale
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

    /// Get tab at the specified location.
    ///
    /// The tuple's second element will be `true` when the position matches the
    /// close button of the tab.
    fn tab_at(&self, mut position: Position<f64>) -> Option<(&RenderTab, bool)> {
        let tabs_end_y = self.new_tab_button_position().y;
        let y_padding = TABS_Y_PADDING * self.scale;
        let x_padding = TABS_X_PADDING * self.scale;
        let tab_size_int = self.tab_size();
        let tab_size: Size<f64> = tab_size_int.into();

        // Check if position is beyond tabs list or outside of the horizontal
        // boundaries.
        if position.x < x_padding
            || position.x >= x_padding + tab_size.width
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
        let index = self.texture_cache.tabs.len().checked_sub(rindex + 1)?;
        let tab = self.texture_cache.tabs.get(index)?;

        // Check if click is within close button bounds.
        //
        // We include padding for the close button since it can be really hard to hit
        // otherwise.
        let close_position = Self::close_button_position(tab_size_int, self.scale);
        let tab_relative_x = position.x - x_padding;
        let close = tab_relative_x >= close_position.x - close_position.y;

        Some((tab, close))
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
        let available_height = self.new_tab_button_position().y.round();

        // Calculate height of all tabs.
        let num_tabs = self.texture_cache.tabs.len();
        let mut tabs_height =
            (num_tabs * (tab_height as usize + tab_padding)).saturating_sub(tab_padding);

        // Allow a bit of padding at the top.
        let new_tab_padding = (NEW_TAB_Y_PADDING * self.scale).round();
        tabs_height += new_tab_padding as usize;

        // Calculate tab content outside the viewport.
        tabs_height.saturating_sub(available_height as usize)
    }
}

/// Tab texture cache by URI.
#[derive(Default)]
struct TextureCache {
    textures: HashMap<(String, bool), Texture>,
    tabs: Vec<RenderTab>,
}

impl TextureCache {
    /// Get all textures for the specified list of tabs.
    ///
    /// This will automatically maintain an internal cache to avoid re-drawing
    /// textures for tabs that have not changed.
    fn textures<'a, T>(
        &mut self,
        tab_size: Size,
        scale: f64,
        tabs: T,
        active_tab: EngineId,
    ) -> impl Iterator<Item = &Texture>
    where
        T: Iterator<Item = &'a Box<dyn Engine>>,
    {
        // Get URIs for all tabs.
        self.tabs.clear();
        self.tabs.extend(tabs.map(|tab| RenderTab::new(tab.as_ref(), active_tab)));

        // Remove unused URIs from cache.
        self.textures.retain(|uri, texture| {
            let retain = self.tabs.iter().any(|tab| &tab.uri == uri);

            // Release OpenGL texture.
            if !retain {
                texture.delete();
            }

            retain
        });

        // Create textures for missing tabs.
        for tab in self.tabs.iter() {
            // Ignore tabs we already rendered.
            if self.textures.contains_key(&tab.uri) {
                continue;
            }

            // Create pango layout.
            let layout = {
                let image_surface = ImageSurface::create(Format::ARgb32, 0, 0).unwrap();
                let context = Context::new(&image_surface).unwrap();
                pangocairo::functions::create_layout(&context)
            };

            // Fallback to URI if title is empty.
            if tab.title.trim().is_empty() {
                layout.set_text(&tab.uri.0);
            } else {
                layout.set_text(&tab.title);
            }

            // Configure text rendering options.
            let mut text_options = TextOptions::new();
            text_options.set_font_size(FONT_SIZE);
            if tab.uri.1 {
                text_options.text_color(ACTIVE_TAB_FG);
            } else {
                text_options.text_color(INACTIVE_TAB_FG);
            }

            // Calculate available area font font rendering.
            let close_position = TabsUi::close_button_position(tab_size, scale);
            let text_width = (close_position.x - close_position.y).round() as i32;
            let text_size = Size::new(text_width, tab_size.height as i32);
            text_options.position(Position::new(close_position.y, 0.));
            text_options.size(text_size);

            // Render text to the texture.
            let builder = TextureBuilder::new(tab_size.into(), scale);
            builder.clear(NEW_TAB_BG);
            builder.rasterize(&layout, &text_options);

            // Render close `X`.
            let size = TabsUi::close_button_size(tab_size, scale);
            let context = builder.context();
            context.move_to(close_position.x, close_position.y);
            context.line_to(close_position.x + size.width, close_position.y + size.height);
            context.move_to(close_position.x + size.width, close_position.y);
            context.line_to(close_position.x, close_position.y + size.height);
            context.set_source_rgb(ACTIVE_TAB_FG[0], ACTIVE_TAB_FG[1], ACTIVE_TAB_FG[2]);
            context.set_line_width(scale);
            context.stroke().unwrap();

            self.textures.insert(tab.uri.clone(), builder.build());
        }

        // Get textures for all tabs in reverse order.
        self.tabs.iter().rev().map(|tab| self.textures.get(&tab.uri).unwrap())
    }

    /// Clear all cached textures.
    fn clear(&mut self) {
        self.textures.clear();
        self.tabs.clear();
    }
}

/// Information required to render a tab.
#[derive(Debug)]
struct RenderTab {
    // Engine URI and its activity state.
    uri: (String, bool),
    engine: EngineId,
    title: String,
}

impl RenderTab {
    fn new(engine: &dyn Engine, active_tab: EngineId) -> Self {
        let engine_id = engine.id();
        Self {
            uri: (engine.uri(), engine_id == active_tab),
            title: engine.title(),
            engine: engine_id,
        }
    }
}

/// Tab creation button.
struct NewTabButton {
    texture: Option<Texture>,
    dirty: bool,
    size: Size,
    scale: f64,
}

impl Default for NewTabButton {
    fn default() -> Self {
        Self { dirty: true, scale: 1., texture: Default::default(), size: Default::default() }
    }
}

impl NewTabButton {
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
    fn draw(&mut self) -> Texture {
        // Clear with background color.
        let builder = TextureBuilder::new(self.size.into(), self.scale);
        builder.clear(TABS_BG);

        // Draw button background.
        let x_padding = NEW_TAB_X_PADDING * self.scale;
        let y_padding = NEW_TAB_Y_PADDING * self.scale;
        let width = self.size.width as f64 - 2. * x_padding;
        let height = self.size.height as f64 - 2. * y_padding;
        builder.context().rectangle(x_padding, y_padding, width.round(), height.round());
        builder.context().set_source_rgb(NEW_TAB_BG[0], NEW_TAB_BG[1], NEW_TAB_BG[2]);
        builder.context().fill().unwrap();

        // Set general stroke properties.
        let icon_size = NEW_TAB_ICON_SIZE * self.scale;
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
    TabTap,
    TabDrag,
    NewTabTap,
}
