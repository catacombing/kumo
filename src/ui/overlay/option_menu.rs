//! Dropdown like HTML <select> tags.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::{cmp, mem};

use funq::MtQueueHandle;
use pangocairo::cairo::{Context, Format, ImageSurface};
use pangocairo::pango::Layout;
use smithay_client_toolkit::seat::keyboard::Modifiers;

use crate::engine::EngineId;
use crate::ui::overlay::Popup;
use crate::ui::renderer::{Renderer, TextOptions, Texture, TextureBuilder};
use crate::ui::TOOLBAR_HEIGHT;
use crate::window::WindowId;
use crate::{gl, Position, Size, State};

// Option menu colors.
const FG: [f64; 3] = [1., 1., 1.];
const BG: [f64; 3] = [0.09, 0.09, 0.09];
const DISABLED_FG: [f64; 3] = [0.4, 0.4, 0.4];
const DISABLED_BG: [f64; 3] = BG;
const SELECTED_FG: [f64; 3] = [0.09, 0.09, 0.09];
const SELECTED_BG: [f64; 3] = [0.46, 0.16, 0.16];

// Option menu item padding.
const X_PADDING: f64 = 15.;
const Y_PADDING: f64 = 10.;

/// Option item font size.
const FONT_SIZE: u8 = 14;

/// Square of the maximum distance before touch input is considered a drag.
const MAX_TAP_DISTANCE: f64 = 400.;

#[funq::callbacks(State)]
trait OptionMenuHandler {
    /// Indicate selection of an option item.
    fn option_menu_submit(&mut self, menu_id: OptionMenuId, index: usize);
}

impl OptionMenuHandler for State {
    fn option_menu_submit(&mut self, menu_id: OptionMenuId, index: usize) {
        let window = match self.windows.get_mut(&menu_id.window_id()) {
            Some(window) => window,
            None => return,
        };
        let engine = match window.tabs_mut().get_mut(&menu_id.engine_id()) {
            Some(engine) => engine,
            None => return,
        };
        engine.option_menu_submit(menu_id, index);
        engine.option_menu_close(Some(menu_id));
    }
}

/// Option menu state.
pub struct OptionMenu {
    id: OptionMenuId,
    items: Vec<OptionMenuRenderItem>,
    selection_index: Option<usize>,

    queue: MtQueueHandle<State>,

    touch_state: TouchState,
    scroll_offset: f32,

    position: Position,
    max_height: u32,
    width: u32,
    scale: f64,

    dirty: bool,
}

impl OptionMenu {
    pub fn new<I>(
        id: OptionMenuId,
        queue: MtQueueHandle<State>,
        position: Position,
        item_size: Size,
        max_size: Size,
        scale: f64,
        items: I,
    ) -> Self
    where
        I: Iterator<Item = OptionMenuItem>,
    {
        let mut selection_index = None;
        let items: Vec<_> = items
            .enumerate()
            .map(|(i, item)| {
                // Update selected item.
                if item.selected {
                    selection_index = Some(i);
                }

                OptionMenuRenderItem::new(item, item_size, scale)
            })
            .collect();

        let mut menu = Self {
            selection_index,
            position,
            queue,
            items,
            scale,
            id,
            width: item_size.width,
            scroll_offset: Default::default(),
            touch_state: Default::default(),
            max_height: Default::default(),
            dirty: Default::default(),
        };

        // Set initial size constraints.
        menu.set_size(max_size);

        menu
    }

    /// Get the popup's ID,
    pub fn id(&self) -> OptionMenuId {
        self.id
    }

    /// Get index of item at the specified physical point.
    fn item_at(&self, mut position: Position<f64>) -> Option<usize> {
        // Apply current scroll offset.
        position.y += self.scroll_offset as f64;

        // Ignore points entirely outside the menu.
        let physical_width = self.width as f64 * self.scale;
        if position.x < 0. || position.y < 0. || position.x >= physical_width {
            return None;
        }

        // Find item at the point's Y position.
        let mut y_end = 0.;
        for (i, item) in self.items.iter().enumerate() {
            y_end += item.height() as f64;
            if position.y < y_end {
                return Some(i);
            }
        }

        None
    }

    /// Clamp tabs view viewport offset.
    fn clamp_scroll_offset(&mut self) {
        let old_offset = self.scroll_offset;
        let max_offset = self.max_scroll_offset() as f32;
        self.scroll_offset = self.scroll_offset.clamp(0., max_offset);
        self.dirty |= old_offset != self.scroll_offset;
    }

    /// Get maximum tab scroll offset.
    fn max_scroll_offset(&self) -> u32 {
        let max_height = (self.max_height as f64 * self.scale).round() as u32;
        self.total_height().saturating_sub(max_height)
    }

    /// Get total option menu height in physical space.
    ///
    /// This is the height of the popup without maximum height constraints.
    fn total_height(&self) -> u32 {
        self.items.iter().map(|item| item.height() as u32).sum()
    }

    /// Get total option menu height in logical space.
    ///
    /// See [`Self::total_height`] for more details.
    fn total_logical_height(&self) -> u32 {
        (self.total_height() as f64 / self.scale).round() as u32
    }
}

impl Popup for OptionMenu {
    fn dirty(&self) -> bool {
        self.dirty || self.items.iter().any(|item| item.dirty)
    }

    fn draw(&mut self, renderer: &Renderer) {
        self.dirty = false;

        // Ensure offset is correct in case size changed.
        self.clamp_scroll_offset();

        // Setup scissoring to crop last element when it should be partially visible.
        unsafe {
            gl::Enable(gl::SCISSOR_TEST);
            let toolbar_height = (TOOLBAR_HEIGHT * self.scale).round() as i32;
            let height = (self.max_height as f64 * self.scale).round() as i32;
            gl::Scissor(0, toolbar_height, i32::MAX, height);
        }

        // Draw each option menu entry.
        let max_height = (self.max_height as f32 * self.scale as f32).round();
        let mut position: Position<f32> = (self.position() * self.scale).into();
        position.y -= self.scroll_offset;
        for (i, item) in self.items.iter_mut().enumerate() {
            // NOTE: This must be called on all textures to clear dirtiness flag.
            let selected = self.selection_index == Some(i);
            let texture = item.texture(selected);

            // Skip rendering out of bounds textures.
            if position.y + texture.height as f32 >= 0. && position.y < max_height {
                unsafe { renderer.draw_texture_at(texture, position, None) };
            }

            position.y += texture.height as f32;
        }

        // Disable scissoring again.
        unsafe { gl::Disable(gl::SCISSOR_TEST) };
    }

    fn position(&self) -> Position {
        // Ensure popup is within display area.
        let max_height = self.max_height as i32;
        let y_end = self.position.y + self.total_logical_height() as i32;
        let y = if y_end > max_height {
            cmp::max(self.position.y - y_end + max_height, 0)
        } else {
            self.position.y
        };

        Position::new(self.position.x, y)
    }

    fn set_size(&mut self, size: Size) {
        self.max_height = size.height - TOOLBAR_HEIGHT as u32;
        self.dirty = true;
    }

    fn size(&self) -> Size {
        let height = cmp::min(self.max_height, self.total_logical_height());
        Size::new(self.width, height)
    }

    fn set_scale(&mut self, scale: f64) {
        for item in &mut self.items {
            item.set_scale(scale);
        }
    }

    fn opaque_region(&self) -> Size {
        self.size()
    }

    fn touch_down(&mut self, _time: u32, id: i32, position: Position<f64>, _modifiers: Modifiers) {
        // Only accept a single touch point.
        if self.touch_state.slot.is_some() {
            return;
        }
        self.touch_state.slot = Some(id);

        // Keep track of touch position for release.
        let position = position * self.scale;
        self.touch_state.position = position;
        self.touch_state.start = position;

        // Reset touch action.
        self.touch_state.action = TouchAction::Tap;

        // Update selected item.
        let new_selected = self.item_at(position);
        if new_selected != self.selection_index {
            // Always clear currently selected item.
            if let Some(old_index) = self.selection_index {
                self.items[old_index].dirty = true;
            }

            // Select new item if there is one under the touch point.
            if let Some(new_index) = new_selected {
                self.items[new_index].dirty = true;
            }

            self.selection_index = new_selected;
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

        // Keep track of touch position for release.
        let position = position * self.scale;
        let old_position = mem::replace(&mut self.touch_state.position, position);

        // Switch to dragging once tap distance limit is exceeded.
        let delta = self.touch_state.position - self.touch_state.start;
        if delta.x.powi(2) + delta.y.powi(2) > MAX_TAP_DISTANCE {
            self.touch_state.action = TouchAction::Drag;

            // Immediately start scrolling the menu.
            let old_offset = self.scroll_offset;
            self.scroll_offset += (old_position.y - self.touch_state.position.y) as f32;
            self.clamp_scroll_offset();
            self.dirty |= self.scroll_offset != old_offset;
        }
    }

    fn touch_up(&mut self, _time: u32, id: i32, _modifiers: Modifiers) {
        // Ignore all unknown touch points.
        if self.touch_state.slot != Some(id) {
            return;
        }
        self.touch_state.slot = None;

        if self.touch_state.action == TouchAction::Tap {
            if let Some(index) = self.item_at(self.touch_state.position) {
                self.queue.option_menu_submit(self.id, index);
            }
        }
    }
}

/// Entry in an option menu.
pub struct OptionMenuItem {
    /// Option menu text.
    pub label: String,
    /// Whether item is selectable.
    pub disabled: bool,
    /// Whether item is selected.
    pub selected: bool,
}

/// State for rendering an option menu entry.
struct OptionMenuRenderItem {
    /// Item texture cache.
    texture: Option<Texture>,
    dirty: bool,

    /// Desired logical texture size.
    size: Size,
    /// Render scale.
    scale: f64,

    /// Pango text layout.
    layout: Layout,
    /// Whether item is selectable.
    disabled: bool,
}

impl OptionMenuRenderItem {
    fn new(item: OptionMenuItem, item_size: Size, scale: f64) -> Self {
        let layout = {
            let image_surface = ImageSurface::create(Format::ARgb32, 0, 0).unwrap();
            let context = Context::new(&image_surface).unwrap();
            pangocairo::functions::create_layout(&context)
        };
        let font = TextureBuilder::font_description(FONT_SIZE, scale);
        layout.set_font_description(Some(&font));
        layout.set_text(&item.label);
        layout.set_height(0);

        OptionMenuRenderItem {
            layout,
            scale,
            disabled: item.disabled,
            size: item_size,
            texture: None,
            dirty: true,
        }
    }

    fn texture(&mut self, selected: bool) -> &Texture {
        // Ensure texture is up to date.
        if mem::take(&mut self.dirty) {
            if let Some(texture) = self.texture.take() {
                texture.delete();
            }
            self.texture = Some(self.draw(selected));
        }

        self.texture.as_ref().unwrap()
    }

    fn draw(&self, selected: bool) -> Texture {
        // Determine item colors.
        let (fg, bg) = if self.disabled {
            (DISABLED_FG, DISABLED_BG)
        } else if selected {
            (SELECTED_FG, SELECTED_BG)
        } else {
            (FG, BG)
        };

        // Configure text rendering options.
        let mut text_options = TextOptions::new();
        text_options.set_font_size(FONT_SIZE);
        text_options.text_color(fg);

        // Calculate physical item size.
        let width = (self.size.width as f64 * self.scale).round() as i32;
        let physical_size = Size::new(width, self.height());

        // Calculate text placement.
        let x_padding = (X_PADDING * self.scale).round() as i32;
        let mut text_size = physical_size;
        text_size.width -= 2 * x_padding;
        text_options.position(Position::new(x_padding, 0).into());
        text_options.size(text_size);

        // Render text to texture.
        let builder = TextureBuilder::new(physical_size, self.scale);
        builder.clear(bg);
        builder.rasterize(&self.layout, &text_options);

        builder.build()
    }

    /// Get the item's height.
    fn height(&self) -> i32 {
        let y_padding = (Y_PADDING * self.scale).round() as i32;
        self.layout.pixel_size().1 + y_padding
    }

    /// Update item scale.
    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.dirty = true;
    }
}

/// Unique identifier for an option menu.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OptionMenuId {
    engine_id: EngineId,
    id: usize,
}

impl OptionMenuId {
    pub fn new(engine_id: EngineId) -> Self {
        static NEXT_MENU_ID: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT_MENU_ID.fetch_add(1, Ordering::Relaxed);
        Self { engine_id, id }
    }

    /// Get the menu's engine.
    pub fn engine_id(&self) -> EngineId {
        self.engine_id
    }

    /// Get the menu's window.
    pub fn window_id(&self) -> WindowId {
        self.engine_id.window_id()
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
    Tap,
    Drag,
}
