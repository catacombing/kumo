//! Rendering to the overlay surface.

use funq::MtQueueHandle;
use glutin::display::Display;
use smithay_client_toolkit::compositor::{CompositorState, Region};
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use smithay_client_toolkit::seat::keyboard::Modifiers;

use crate::ui::overlay::option_menu::{OptionMenu, OptionMenuId, OptionMenuItem};
use crate::ui::overlay::tabs::Tabs;
use crate::ui::Renderer;
use crate::{gl, rect_contains, Position, Size, State, WindowId};

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

    /// Handle touch release events.
    fn touch_up(&mut self, _time: u32, _id: i32, _modifiers: Modifiers) {}
}

/// Overlay UI surface.
pub struct Overlay {
    renderer: Renderer,

    surface: WlSurface,
    viewport: WpViewport,
    compositor: CompositorState,

    popups: Popups,

    queue: MtQueueHandle<State>,

    touch_focus: Option<usize>,

    size: Size,
    scale: f64,
}

impl Overlay {
    pub fn new(
        window_id: WindowId,
        queue: MtQueueHandle<State>,
        display: Display,
        surface: WlSurface,
        viewport: WpViewport,
        compositor: CompositorState,
    ) -> Self {
        let renderer = Renderer::new(display, surface.clone());
        let popups = Popups::new(window_id, queue.clone());

        Self {
            compositor,
            viewport,
            renderer,
            surface,
            popups,
            queue,
            scale: 1.0,
            touch_focus: Default::default(),
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
        if popups.all(|popup| !popup.dirty()) {
            return false;
        }
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
                // NOTE: We still draw invisible popups to allow them to clear their dirty flags
                // after going invisible. Popups must not draw anything while invisible.
                for popup in self.popups.iter_mut() {
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
        self.popups.tabs.visible()
    }

    /// Handle touch press events.
    pub fn touch_down(
        &mut self,
        time: u32,
        id: i32,
        position: Position<f64>,
        modifiers: Modifiers,
    ) {
        self.touch_focus = None;
        for (i, popup) in self.popups.iter_mut().enumerate() {
            let popup_position = popup.position().into();
            if rect_contains(popup_position, popup.size().into(), position) {
                popup.touch_down(time, id, position - popup_position, modifiers);
                self.touch_focus = Some(i);
                break;
            }
        }
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

    /// Show an option menu.
    #[cfg_attr(feature = "profiling", profiling::function)]
    pub fn open_option_menu<I>(
        &mut self,
        id: OptionMenuId,
        position: Position,
        item_width: u32,
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

    /// Hide an option menu.
    pub fn close_option_menu(&mut self, id: OptionMenuId) {
        self.popups.option_menus.retain(|menu| menu.id() != id);
    }
}

/// Overlay popup windows.
struct Popups {
    option_menus: Vec<OptionMenu>,
    tabs: Tabs,
}

impl Popups {
    fn new(window_id: WindowId, queue: MtQueueHandle<State>) -> Self {
        let tabs = Tabs::new(window_id, queue);
        Self { tabs, option_menus: Default::default() }
    }

    /// Update logical popup size.
    fn set_size(&mut self, size: Size) {
        for menu in &mut self.option_menus {
            menu.set_size(size);
        }
        self.tabs.set_size(size);
    }

    /// Update popup scale.
    fn set_scale(&mut self, scale: f64) {
        for menu in &mut self.option_menus {
            menu.set_scale(scale);
        }
        self.tabs.set_scale(scale);
    }

    /// Non-mutable popup iterator.
    fn iter(&self) -> Box<dyn Iterator<Item = &dyn Popup> + '_> {
        if self.tabs.visible() {
            Box::new(self.option_menus.iter().map(|menu| menu as _).chain([&self.tabs as _]))
        } else {
            Box::new(self.option_menus.iter().map(|menu| menu as _))
        }
    }

    /// Mutable popup iterator.
    fn iter_mut(&mut self) -> Box<dyn Iterator<Item = &mut dyn Popup> + '_> {
        if self.tabs.visible() {
            Box::new(
                self.option_menus.iter_mut().map(|menu| menu as _).chain([&mut self.tabs as _]),
            )
        } else {
            Box::new(self.option_menus.iter_mut().map(|menu| menu as _))
        }
    }
}
