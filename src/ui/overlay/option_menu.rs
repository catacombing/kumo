//! Dropdown like HTML <select> tags.

use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};

use funq::MtQueueHandle;
use pangocairo::cairo::{Context, Format, ImageSurface};
use pangocairo::pango::Layout;
use smithay_client_toolkit::seat::keyboard::Modifiers;

use crate::engine::EngineId;
use crate::ui::overlay::Popup;
use crate::ui::renderer::{Renderer, TextOptions, Texture, TextureBuilder};
use crate::window::WindowId;
use crate::{Position, Size, State};

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

    position: Position,
    size: Size,
    scale: f64,
}

impl OptionMenu {
    pub fn new<I>(
        id: OptionMenuId,
        queue: MtQueueHandle<State>,
        position: Position,
        item_size: Size,
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
        let size = Size::new(item_size.width, item_size.height * items.len() as u32);

        Self {
            selection_index,
            position,
            queue,
            items,
            scale,
            size,
            id,
            touch_state: Default::default(),
        }
    }

    /// Get the popup's ID,
    pub fn id(&self) -> OptionMenuId {
        self.id
    }

    /// Get index of item at the specified point.
    fn item_at(&self, position: Position<f64>) -> Option<usize> {
        // Ignore points entirely outside the menu.
        let physical_width = self.size.width as f64 * self.scale;
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
}

impl Popup for OptionMenu {
    fn dirty(&self) -> bool {
        self.items.iter().any(|item| item.dirty)
    }

    fn draw(&mut self, renderer: &Renderer) {
        // Draw each option menu entry.
        let mut position = (self.position * self.scale).into();
        for (i, item) in self.items.iter_mut().enumerate() {
            let selected = self.selection_index == Some(i);
            let texture = item.texture(selected);
            unsafe { renderer.draw_texture_at(texture, position, None) };
            position.y += texture.height as f32;
        }
    }

    fn position(&self) -> Position {
        self.position
    }

    fn set_size(&mut self, _size: Size) {}

    fn size(&self) -> Size {
        let height = self.items.iter().map(|item| item.height() as u32).sum();
        Size::new(self.size.width, height)
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
        let physical_position = position * self.scale;
        self.touch_state.position = physical_position;

        // Update selected item.
        let new_selected = self.item_at(physical_position);
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
        let physical_position = position * self.scale;
        self.touch_state.position = physical_position;

        // Update selected item.
        let new_selected = self.item_at(physical_position);
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

    fn touch_up(&mut self, _time: u32, id: i32, _modifiers: Modifiers) {
        // Ignore all unknown touch points.
        if self.touch_state.slot != Some(id) {
            return;
        }

        if let Some(index) = self.item_at(self.touch_state.position) {
            self.queue.option_menu_submit(self.id, index);
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
    position: Position<f64>,
}
