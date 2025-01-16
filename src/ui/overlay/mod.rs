//! Rendering to the overlay surface.

use funq::MtQueueHandle;
use glutin::display::Display;
use smithay_client_toolkit::compositor::{CompositorState, Region};
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};

use crate::storage::history::History as HistoryDb;
use crate::ui::overlay::history::History;
use crate::ui::overlay::option_menu::{
    OptionMenu, OptionMenuId, OptionMenuItem, OptionMenuPosition,
};
use crate::ui::overlay::tabs::Tabs;
use crate::ui::Renderer;
use crate::window::TextInputChange;
use crate::{gl, rect_contains, Position, Size, State, WindowId};

pub mod history;
pub mod option_menu;
pub mod tabs;

/// Overlay surface element.
pub trait Popup {
    /// Check whether the popup requires a redraw.
    fn dirty(&self) -> bool;

    /// Redraw the popup.
    fn draw(&mut self, renderer: &Renderer);

    /// Popup's logical location.
    fn position(&self) -> Position;

    /// Update maximum available size.
    fn set_size(&mut self, size: Size);

    /// Get the logical popup size.
    fn size(&self) -> Size;

    /// Update scale factor.
    fn set_scale(&mut self, scale: f64);

    /// Popup's logical opaque size.
    fn opaque_region(&self) -> Size;

    /// Handle touch press events.
    fn touch_down(
        &mut self,
        _time: u32,
        _id: i32,
        _position: Position<f64>,
        _modifiers: Modifiers,
    ) {
    }

    /// Handle touch motion events.
    fn touch_motion(
        &mut self,
        _time: u32,
        _id: i32,
        _position: Position<f64>,
        _modifiers: Modifiers,
    ) {
    }

    /// Handle new key press.
    fn press_key(&mut self, _raw: u32, _keysym: Keysym, _modifiers: Modifiers) {}

    /// Handle touch release events.
    fn touch_up(&mut self, _time: u32, _id: i32, _modifiers: Modifiers) {}

    /// Delete text around the current cursor position.
    fn delete_surrounding_text(&mut self, _before_length: u32, _after_length: u32) {}

    /// Insert IME text at the current cursor position.
    fn commit_string(&mut self, _text: String) {}

    /// Set preedit text at the current cursor position.
    fn set_preedit_string(&mut self, _text: String, _cursor_begin: i32, _cursor_end: i32) {}

    /// Get current IME text_input state.
    fn text_input_state(&mut self) -> TextInputChange {
        TextInputChange::Disabled
    }

    /// Paste text at the current cursor position.
    fn paste(&mut self, _text: String) {}

    /// Check if popup has keyboard input element focused.
    fn has_keyboard_focus(&self) -> bool {
        false
    }

    /// Handle keyboard focus loss.
    fn clear_keyboard_focus(&mut self) {}
}

/// Overlay UI surface.
pub struct Overlay {
    renderer: Renderer,

    surface: WlSurface,
    viewport: WpViewport,
    compositor: CompositorState,

    popups: Popups,

    queue: MtQueueHandle<State>,

    keyboard_focus: Option<usize>,
    touch_focus: Option<usize>,

    size: Size,
    scale: f64,

    dirty: bool,
}

impl Overlay {
    pub fn new(
        window_id: WindowId,
        queue: MtQueueHandle<State>,
        display: Display,
        surface: WlSurface,
        viewport: WpViewport,
        compositor: CompositorState,
        history_db: HistoryDb,
    ) -> Self {
        let popups = Popups::new(window_id, queue.clone(), history_db);
        let renderer = Renderer::new(display, surface.clone());

        Self {
            compositor,
            viewport,
            renderer,
            surface,
            popups,
            queue,
            scale: 1.0,
            keyboard_focus: Default::default(),
            touch_focus: Default::default(),
            dirty: Default::default(),
            size: Default::default(),
        }
    }

    /// Update the logical UI size.
    pub fn set_size(&mut self, size: Size) {
        self.size = size;

        // Update popups.
        self.popups.set_size(size);
    }

    /// Update Wayland surface regions.
    #[cfg_attr(feature = "profiling", profiling::function)]
    fn update_regions(&self) {
        // Update opaque region.
        if let Ok(region) = Region::new(&self.compositor) {
            for popup in self.popups.iter() {
                let pos = popup.position();
                let size: Size<i32> = popup.opaque_region().into();
                region.add(pos.x, pos.y, size.width, size.height);
            }
            self.surface.set_opaque_region(Some(region.wl_region()));
        }

        // Update input region.
        if let Ok(region) = Region::new(&self.compositor) {
            for popup in self.popups.iter() {
                let pos = popup.position();
                let size: Size<i32> = popup.size().into();
                region.add(pos.x, pos.y, size.width, size.height);
            }
            self.surface.set_input_region(Some(region.wl_region()));
        }
    }

    /// Update the render scale.
    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale;

        // Update popups.
        self.popups.set_scale(scale);
    }

    /// Render current overlay state.
    ///
    /// Returns `true` if rendering was performed.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn draw(&mut self) -> bool {
        let mut popups = self.popups.iter().peekable();

        // Hide surface if there's no visible popups.
        if popups.peek().is_none() {
            self.surface.attach(None, 0, 0);
            self.surface.commit();
            return false;
        }

        // Don't redraw if rendering is up to date.
        if !self.dirty && popups.all(|popup| !popup.dirty()) {
            return false;
        }
        self.dirty = false;
        drop(popups);

        self.update_regions();

        // Update viewporter logical render size.
        //
        // NOTE: This must be done every time we draw with Sway; it is not correctly
        // persisted when drawing with the same surface multiple times.
        self.viewport.set_destination(self.size.width as i32, self.size.height as i32);

        // Mark entire surface as damaged.
        self.surface.damage(0, 0, self.size.width as i32, self.size.height as i32);

        // Redraw all popups.
        let physical_size = self.size * self.scale;
        self.renderer.draw(physical_size, |renderer| {
            unsafe {
                // Clear background.
                gl::ClearColor(0., 0., 0., 0.);
                gl::Clear(gl::COLOR_BUFFER_BIT);

                // Draw the popups.
                //
                // The drawing happens in reverse order to ensure consistency with touch
                // handling which happens in traditional iterator order.
                //
                // NOTE: We still draw invisible popups to allow them to clear their dirty flags
                // after going invisible. Popups must not draw anything while invisible.
                for popup in self.popups.iter_mut().rev() {
                    popup.draw(renderer);
                }
            }
        });

        true
    }

    /// Check if the popup surface is fully opaque.
    pub fn opaque(&self) -> bool {
        // NOTE: This is a simplification of actual popup opaque region combination
        // since it's currently not possible to make the overlay surface fully
        // opaque by combining multiple smaller popups.
        self.popups.tabs.visible() || self.popups.history.visible()
    }

    /// Handle new key press.
    pub fn press_key(&mut self, raw: u32, keysym: Keysym, modifiers: Modifiers) {
        let focused_popup = self.keyboard_focus.and_then(|focus| self.popups.iter_mut().nth(focus));
        if let Some(popup) = focused_popup {
            popup.press_key(raw, keysym, modifiers);
        }
    }

    /// Handle touch press events.
    pub fn touch_down(
        &mut self,
        time: u32,
        id: i32,
        position: Position<f64>,
        modifiers: Modifiers,
    ) {
        // Close untouched option menus.
        let mut menu_closed = false;
        self.popups.option_menus.retain(|menu| {
            let popup_position = menu.position().into();
            let touched = rect_contains(popup_position, menu.size().into(), position);
            menu_closed |= !touched;
            touched
        });
        self.dirty |= menu_closed;

        // Focus touched popup.
        for (i, popup) in self.popups.iter_mut().enumerate() {
            let popup_position = popup.position().into();
            if rect_contains(popup_position, popup.size().into(), position) {
                popup.touch_down(time, id, position - popup_position, modifiers);
                self.keyboard_focus = Some(i);
                self.touch_focus = Some(i);
                return;
            }
        }

        // Clear focus if no popup was touched.
        self.clear_keyboard_focus();
        self.touch_focus = None;
    }

    /// Handle touch motion events.
    pub fn touch_motion(
        &mut self,
        time: u32,
        id: i32,
        position: Position<f64>,
        modifiers: Modifiers,
    ) {
        let focused_popup = self.touch_focus.and_then(|focus| self.popups.iter_mut().nth(focus));
        if let Some(popup) = focused_popup {
            let popup_position = popup.position().into();
            popup.touch_motion(time, id, position - popup_position, modifiers);
        }
    }

    /// Handle touch release events.
    pub fn touch_up(&mut self, time: u32, id: i32, modifiers: Modifiers) {
        let focused_popup = self.touch_focus.and_then(|focus| self.popups.iter_mut().nth(focus));
        if let Some(popup) = focused_popup {
            popup.touch_up(time, id, modifiers);
        }
    }

    /// Delete text around the current cursor position.
    pub fn delete_surrounding_text(&mut self, before_length: u32, after_length: u32) {
        let focused_popup = self.keyboard_focus.and_then(|focus| self.popups.iter_mut().nth(focus));
        if let Some(popup) = focused_popup {
            popup.delete_surrounding_text(before_length, after_length);
        }
    }

    /// Insert text at the current cursor position.
    pub fn commit_string(&mut self, text: String) {
        let focused_popup = self.keyboard_focus.and_then(|focus| self.popups.iter_mut().nth(focus));
        if let Some(popup) = focused_popup {
            popup.commit_string(text);
        }
    }

    /// Insert text at the current cursor position.
    pub fn paste(&mut self, text: String) {
        let focused_popup = self.keyboard_focus.and_then(|focus| self.popups.iter_mut().nth(focus));
        if let Some(popup) = focused_popup {
            popup.paste(text);
        }
    }

    /// Set preedit text at the current cursor position.
    pub fn set_preedit_string(&mut self, text: String, cursor_begin: i32, cursor_end: i32) {
        let focused_popup = self.keyboard_focus.and_then(|focus| self.popups.iter_mut().nth(focus));
        if let Some(popup) = focused_popup {
            popup.set_preedit_string(text, cursor_begin, cursor_end);
        }
    }

    /// Get current IME text_input state.
    pub fn text_input_state(&mut self) -> TextInputChange {
        let focused_popup = self.keyboard_focus.and_then(|focus| self.popups.iter_mut().nth(focus));
        match focused_popup {
            Some(popup) => popup.text_input_state(),
            None => TextInputChange::Disabled,
        }
    }

    /// Check whether a popup has an input element focused.
    pub fn has_keyboard_focus(&self) -> bool {
        let focused_popup = self.keyboard_focus.and_then(|focus| self.popups.iter().nth(focus));
        focused_popup.is_some_and(Popup::has_keyboard_focus)
    }

    /// Clear Overlay keyboard focus.
    pub fn clear_keyboard_focus(&mut self) {
        let focused_popup =
            self.keyboard_focus.take().and_then(|focus| self.popups.iter_mut().nth(focus));
        if let Some(popup) = focused_popup {
            popup.clear_keyboard_focus();
        }
    }

    /// Check if any popup is dirty.
    pub fn dirty(&self) -> bool {
        self.popups.iter().any(|popup| popup.dirty())
    }

    /// Get the underlying Wayland surface.
    pub fn surface(&self) -> &WlSurface {
        &self.surface
    }

    /// Get mutable access to the tabs popup.
    pub fn tabs_mut(&mut self) -> &mut Tabs {
        &mut self.popups.tabs
    }

    /// Change the history UI visibility.
    pub fn set_history_visible(&mut self, visible: bool) {
        self.dirty |= visible != self.popups.history.visible();
        self.popups.history.set_visible(visible);
    }

    /// Show an option menu.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn open_option_menu<I>(
        &mut self,
        id: OptionMenuId,
        position: impl Into<OptionMenuPosition>,
        item_width: impl Into<Option<u32>>,
        scale: f64,
        items: I,
    ) -> &mut OptionMenu
    where
        I: Iterator<Item = OptionMenuItem>,
    {
        let queue = self.queue.clone();
        let option_menu = OptionMenu::new(id, queue, position, item_width, self.size, scale, items);
        self.popups.option_menus.push(option_menu);
        self.popups.option_menus.last_mut().unwrap()
    }

    /// Get mutable reference to an option menu.
    pub fn option_menu(&mut self, id: OptionMenuId) -> Option<&mut OptionMenu> {
        self.popups.option_menus.iter_mut().find(|menu| menu.id() == id)
    }

    /// Permanently discard an option menu.
    pub fn close_option_menu(&mut self, id: OptionMenuId) {
        self.popups.option_menus.retain(|menu| menu.id() != id);
        self.dirty = true;
    }
}

/// Overlay popup windows.
struct Popups {
    option_menus: Vec<OptionMenu>,
    history: History,
    tabs: Tabs,
}

impl Popups {
    fn new(window_id: WindowId, queue: MtQueueHandle<State>, history_db: HistoryDb) -> Self {
        let history = History::new(window_id, queue.clone(), history_db);
        let tabs = Tabs::new(window_id, queue);
        Self { history, tabs, option_menus: Default::default() }
    }

    /// Update logical popup size.
    fn set_size(&mut self, size: Size) {
        for menu in &mut self.option_menus {
            menu.set_size(size);
        }
        self.history.set_size(size);
        self.tabs.set_size(size);
    }

    /// Update popup scale.
    fn set_scale(&mut self, scale: f64) {
        for menu in &mut self.option_menus {
            menu.set_scale(scale);
        }
        self.history.set_scale(scale);
        self.tabs.set_scale(scale);
    }

    /// Non-mutable popup iterator.
    fn iter(&self) -> Box<dyn PopupIterator + '_> {
        let option_menus = self.option_menus.iter().filter(|m| m.visible()).map(|m| m as _);
        if self.history.visible() {
            Box::new(option_menus.chain([&self.history as _]))
        } else if self.tabs.visible() {
            Box::new(option_menus.chain([&self.tabs as _]))
        } else {
            Box::new(option_menus)
        }
    }

    /// Mutable popup iterator.
    fn iter_mut(&mut self) -> Box<dyn PopupIteratorMut<'_> + '_> {
        let option_menus = self.option_menus.iter_mut().filter(|m| m.visible()).map(|m| m as _);
        if self.history.visible() {
            Box::new(option_menus.chain([&mut self.history as _]))
        } else if self.tabs.visible() {
            Box::new(option_menus.chain([&mut self.tabs as _]))
        } else {
            Box::new(option_menus)
        }
    }
}

// Wrappers for combining Iterator + DoubleEndedIterator trait bounds.
trait PopupIterator<'a>: Iterator<Item = &'a dyn Popup> + DoubleEndedIterator {}
impl<'a, T: Iterator<Item = &'a dyn Popup> + DoubleEndedIterator> PopupIterator<'a> for T {}
trait PopupIteratorMut<'a>: Iterator<Item = &'a mut dyn Popup> + DoubleEndedIterator {}
impl<'a, T: Iterator<Item = &'a mut dyn Popup> + DoubleEndedIterator> PopupIteratorMut<'a> for T {}
